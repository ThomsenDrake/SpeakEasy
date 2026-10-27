//! Single-instance coordination.
//!
//! Every coding agent spawns this binary over MCP stdio, but the approval
//! popup, the Slack client, and the pending-request queue must live in
//! exactly one process. The first process to start becomes the *instance*:
//! it holds `gateway.lock` in the state dir, listens for MCP connections on
//! `gateway.sock` (a Unix socket), and serves both its own stdio and every
//! socket connection with the same JSON-RPC handler. Any later process
//! connects to the socket and becomes a *bridge*: a pump between the
//! agent's stdio and the instance, so the agent's MCP session behaves
//! exactly as if it had spawned the instance itself. The bridge tracks
//! in-flight request ids and, if the instance connection drops with
//! requests still pending (the instance quit or crashed), answers each
//! with a JSON-RPC error — the agent is told nothing was sent instead of
//! hanging on a dead session.
//!
//! All state (cooldowns, sent records, locks) already lives in the shared
//! state dir, so even if two instances ever raced into existence, deny
//! cooldowns and duplicate sends would still be enforced across processes.

use crate::core::Gateway;
use crate::mcp;
use serde_json::Value;
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// How long a starter waits for a lock holder that has not bound its socket
/// yet (or died before binding) before stealing the lock.
const STEAL_AFTER: Duration = Duration::from_secs(2);
/// Total bound on lock acquisition attempts.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
/// `sun_path` capacity on macOS (104 bytes including the NUL); Linux allows
/// 108. A state dir deeper than this cannot host the socket.
const SUN_PATH_LIMIT: usize = 103;

pub fn socket_path(state_dir: &Path) -> PathBuf {
    state_dir.join("gateway.sock")
}

fn lock_path(state_dir: &Path) -> PathBuf {
    state_dir.join("gateway.lock")
}

/// Try to connect to a running instance. `None` means no live instance
/// (nothing listening, or a stale socket file left by a crash).
pub fn connect_existing(state_dir: &Path) -> Option<UnixStream> {
    UnixStream::connect(socket_path(state_dir)).ok()
}

/// Why [`acquire`] failed.
#[derive(Debug)]
pub enum AcquireError {
    /// Another live instance holds the lock and answers on the socket; the
    /// caller should bridge to it instead of starting.
    AlreadyRunning,
    /// Lock or socket setup failed.
    Io(std::io::Error),
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcquireError::AlreadyRunning => write!(
                f,
                "another gateway instance is already running in this state dir"
            ),
            AcquireError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// RAII guard for the single instance. Dropping it removes the lock and
/// socket files; a crash leaves them behind and the next starter steals the
/// lock (the socket is only trusted when it accepts connections).
pub struct InstanceGuard {
    state_dir: PathBuf,
    _lock: std::fs::File,
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(lock_path(&self.state_dir));
        let _ = std::fs::remove_file(socket_path(&self.state_dir));
    }
}

/// Become the single instance: take `gateway.lock` (atomically, stealing it
/// from a dead holder), clear any stale socket file, and bind the listener.
pub fn acquire(state_dir: &Path) -> Result<(InstanceGuard, UnixListener), AcquireError> {
    let sock = socket_path(state_dir);
    if sock.as_os_str().len() > SUN_PATH_LIMIT {
        return Err(AcquireError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "state dir path is too long for a Unix socket ({} > {SUN_PATH_LIMIT} bytes): \
                 use a shorter SLACK_GATEWAY_STATE_DIR",
                sock.as_os_str().len()
            ),
        )));
    }
    let lock_path = lock_path(state_dir);
    let deadline = Instant::now() + ACQUIRE_TIMEOUT;
    let steal_after = Instant::now() + STEAL_AFTER;
    loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut lock) => {
                let _ = writeln!(lock, "{}", std::process::id());
                let _ = lock.flush();
                // The lock is ours. A socket file that does not accept
                // connections is stale (the previous holder is gone).
                let _ = std::fs::remove_file(socket_path(state_dir));
                return match UnixListener::bind(socket_path(state_dir)) {
                    Ok(listener) => Ok((
                        InstanceGuard {
                            state_dir: state_dir.to_path_buf(),
                            _lock: lock,
                        },
                        listener,
                    )),
                    Err(e) => {
                        let _ = std::fs::remove_file(&lock_path);
                        Err(AcquireError::Io(e))
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if connect_existing(state_dir).is_some() {
                    return Err(AcquireError::AlreadyRunning);
                }
                if Instant::now() > deadline {
                    return Err(AcquireError::Io(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "timed out acquiring the gateway instance lock",
                    )));
                }
                if Instant::now() > steal_after {
                    // The holder never bound its socket: it is starting up
                    // very slowly or it is dead. Steal the lock.
                    eprintln!(
                        "slack-approval-gateway: stealing stale instance lock in {}",
                        state_dir.display()
                    );
                    let _ = std::fs::remove_file(&lock_path);
                    continue;
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(AcquireError::Io(e)),
        }
    }
}

/// Accept MCP connections on the instance's socket forever. Each connection
/// is served by the same JSON-RPC handler as stdio, on its own thread, so
/// one agent waiting for a human never blocks another agent.
pub fn serve_connections(gateway: Arc<Gateway>, listener: UnixListener) {
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let gateway = Arc::clone(&gateway);
                    thread::spawn(move || {
                        let writer = match stream.try_clone() {
                            Ok(w) => w,
                            Err(_) => return,
                        };
                        if let Err(e) = mcp::run(gateway, BufReader::new(stream), writer) {
                            eprintln!("slack-approval-gateway: socket MCP error: {e}");
                        }
                    });
                }
                Err(e) => {
                    eprintln!("slack-approval-gateway: socket accept error: {e}");
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
    });
}

/// Pump newline-delimited JSON-RPC between an agent's stdio and the
/// instance's socket. Runs until the instance closes the connection; exits
/// the process. Protocol bytes are the only thing written to stdout;
/// diagnostics go to stderr.
pub fn run_bridge(stream: UnixStream) -> ! {
    eprintln!("slack-approval-gateway: bridging MCP stdio to the running instance");
    // BufReader<Stdin> (not Stdin::lock, a MutexGuard): it must be Send to
    // move into the forwarding thread.
    let stdin = BufReader::new(std::io::stdin());
    let stdout = Arc::new(Mutex::new(std::io::stdout()));
    let unanswered = pump_jsonrpc(stdin, stdout, stream);
    if !unanswered.is_empty() {
        eprintln!(
            "slack-approval-gateway: instance connection closed with {} pending request(s); \
             answered each with an error (nothing was sent)",
            unanswered.len()
        );
    }
    std::process::exit(0);
}

/// The `id` of a JSON-RPC request (a message with a method and an id and no
/// result or error), if the line parses as one.
fn request_id(line: &str) -> Option<Value> {
    let msg: Value = serde_json::from_str(line).ok()?;
    if msg.get("method").is_some()
        && msg.get("id").is_some()
        && msg.get("result").is_none()
        && msg.get("error").is_none()
    {
        msg.get("id").cloned()
    } else {
        None
    }
}

/// The `requestId` of a `notifications/cancelled` message, if the line is
/// one: the agent gave up on that request, and gets no answer to it.
fn cancelled_id(line: &str) -> Option<Value> {
    let msg: Value = serde_json::from_str(line).ok()?;
    if msg.get("method").and_then(Value::as_str) == Some("notifications/cancelled") {
        msg.get("params")?.get("requestId").cloned()
    } else {
        None
    }
}

/// The `id` of a JSON-RPC response (a message with an id and a result or an
/// error), if the line parses as one.
fn response_id(line: &str) -> Option<Value> {
    let msg: Value = serde_json::from_str(line).ok()?;
    if msg.get("id").is_some() && (msg.get("result").is_some() || msg.get("error").is_some()) {
        msg.get("id").cloned()
    } else {
        None
    }
}

/// Requests an agent sent that the instance has not answered. `closed`
/// flips once the instance connection is gone; the forwarding side then
/// fails requests fast instead of writing them into the void.
#[derive(Default)]
struct InFlight {
    ids: HashSet<Value>,
    closed: bool,
}

fn write_line<W: Write>(output: &Arc<Mutex<W>>, bytes: &[u8]) -> std::io::Result<()> {
    let mut out = output.lock().unwrap_or_else(PoisonError::into_inner);
    out.write_all(bytes)?;
    out.flush()
}

/// Answer a request the instance will never answer: it exited before a
/// decision was made, so nothing was sent.
fn write_synthesized_error<W: Write>(output: &Arc<Mutex<W>>, id: &Value) {
    let response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32000,
            "message": "slack-approval-gateway: the gateway instance exited before a \
                        decision was made; nothing was sent",
        }
    });
    let Ok(mut line) = serde_json::to_vec(&response) else {
        return;
    };
    line.push(b'\n');
    let _ = write_line(output, &line);
}

/// The testable core of [`run_bridge`]: forward `input` (the agent's stdio)
/// to the instance's socket and the instance's replies to `output`, line by
/// line, until the instance closes the connection. Request ids are tracked
/// so that when the connection drops — the instance quit or crashed — every
/// request still waiting for a decision is answered with a JSON-RPC error
/// ("nothing was sent"): an agent is told, never left hanging on a dead
/// session. Requests arriving after the drop are answered the same way,
/// immediately. Returns the ids the read side answered (the write side
/// answers its own late requests directly).
fn pump_jsonrpc<R: BufRead + Send + 'static, W: Write + Send + 'static>(
    mut input: R,
    output: Arc<Mutex<W>>,
    mut stream: UnixStream,
) -> Vec<Value> {
    let mut to_instance = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("slack-approval-gateway: cannot bridge: {e}");
            std::process::exit(1);
        }
    };
    let in_flight = Arc::new(Mutex::new(InFlight::default()));
    let tracked = Arc::clone(&in_flight);
    let out = Arc::clone(&output);
    let copy_in = thread::spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match input.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    // The agent is gone: the instance withdraws its pending
                    // calls without answering, and no one is left to tell.
                    if let Ok(mut f) = tracked.lock() {
                        f.ids.clear();
                    }
                    break;
                }
                Ok(_) => {
                    let late = match tracked.lock() {
                        Ok(mut f) => {
                            if f.closed {
                                true
                            } else {
                                if let Some(id) = request_id(&line) {
                                    f.ids.insert(id);
                                } else if let Some(id) = cancelled_id(&line) {
                                    // the instance won't answer it either
                                    f.ids.remove(&id);
                                }
                                false
                            }
                        }
                        Err(_) => break,
                    };
                    if late {
                        // The instance is gone: fail this request right away.
                        if let Some(id) = request_id(&line) {
                            write_synthesized_error(&out, &id);
                        }
                        continue;
                    }
                    if to_instance.write_all(line.as_bytes()).is_err() {
                        break;
                    }
                }
            }
        }
        // The agent closed its MCP session: tell the instance, then let the
        // read side finish the shutdown. Never joined: with the instance
        // gone this thread can be parked on stdin, and exiting the process
        // below is what ends it.
        let _ = to_instance.shutdown(std::net::Shutdown::Write);
    });
    let mut from_instance = BufReader::new(&mut stream);
    let mut line = String::new();
    loop {
        line.clear();
        match from_instance.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if let Some(id) = response_id(&line) {
                    if let Ok(mut f) = in_flight.lock() {
                        f.ids.remove(&id);
                    }
                }
                if write_line(&output, line.as_bytes()).is_err() {
                    break;
                }
            }
        }
    }
    // The instance is gone (or closed the connection). Flip `closed` and
    // answer every request still waiting for a decision: the human never
    // saw it, nothing was sent.
    let unanswered: Vec<Value> = match in_flight.lock() {
        Ok(mut f) => {
            f.closed = true;
            f.ids.drain().collect()
        }
        Err(_) => Vec::new(),
    };
    for id in &unanswered {
        write_synthesized_error(&output, id);
    }
    drop(copy_in);
    unanswered
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn temp_dir(tag: &str) -> PathBuf {
        // Unix socket paths are limited (~104 bytes on macOS), so the test
        // state dir must be short: /tmp, not the long per-user temp dir.
        let dir = PathBuf::from(format!(
            "/tmp/sag-it-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn acquire_binds_and_connects() {
        let dir = temp_dir("acquire");
        let (guard, listener) = acquire(&dir).expect("acquire");
        assert!(socket_path(&dir).exists());
        assert!(connect_existing(&dir).is_some());
        drop(listener);
        drop(guard);
        assert!(!socket_path(&dir).exists());
        assert!(!lock_path(&dir).exists());
        assert!(connect_existing(&dir).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn second_acquire_reports_already_running() {
        let dir = temp_dir("running");
        let (_guard, _listener) = acquire(&dir).expect("first acquire");
        match acquire(&dir) {
            Err(AcquireError::AlreadyRunning) => {}
            Err(e) => panic!("expected AlreadyRunning, got {e}"),
            Ok(_) => panic!("expected AlreadyRunning, got a second instance"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stale_lock_and_socket_are_stolen() {
        let dir = temp_dir("stale");
        // Simulate a crash: lock file and a socket file nobody listens on.
        std::fs::write(lock_path(&dir), "999999999\n").unwrap();
        {
            let _listener = UnixListener::bind(socket_path(&dir)).unwrap();
            // Drop without accepting: connecting now would succeed, so
            // instead simulate a dead socket by binding and immediately
            // closing the listener but keeping the file.
            drop(_listener);
        }
        // A bound-then-dropped listener leaves a socket file that refuses
        // connections, exactly like a crashed instance.
        assert!(connect_existing(&dir).is_none());
        let (_guard, _listener) = acquire(&dir).expect("steal stale lock");
        assert!(connect_existing(&dir).is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pump_forwards_both_directions_until_eof() {
        let (client, server) = UnixStream::pair().unwrap();
        // Echo server: copy everything back until the client half-closes.
        let echo = thread::spawn(move || {
            let mut reader = server.try_clone().unwrap();
            let mut writer = server;
            let _ = std::io::copy(&mut reader, &mut writer);
        });
        let input = Cursor::new(b"ping\n".to_vec());
        let output = Arc::new(Mutex::new(Vec::new()));
        let unanswered = pump_jsonrpc(input, Arc::clone(&output), client);
        let bytes = std::mem::take(&mut *output.lock().unwrap());
        assert_eq!(bytes, b"ping\n");
        assert!(unanswered.is_empty());
        echo.join().unwrap();
    }

    #[test]
    fn pump_answers_in_flight_requests_when_the_instance_dies() {
        let (client, server) = UnixStream::pair().unwrap();
        // A fake instance that reads the request and then dies without
        // answering, like a SIGKILLed gateway.
        let instance = thread::spawn(move || {
            let mut reader = BufReader::new(server);
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            assert!(line.contains("tools/call"));
            // Dropping the connection is the whole point: the instance is gone.
        });
        // The agent stays connected (its input open) while the instance dies.
        let (mut agent, agent_side) = UnixStream::pair().unwrap();
        writeln!(
            agent,
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{}}}}"
        )
        .unwrap();
        let input = BufReader::new(agent_side);
        let output = Arc::new(Mutex::new(Vec::new()));
        let unanswered = pump_jsonrpc(input, Arc::clone(&output), client);
        instance.join().unwrap();
        drop(agent);
        assert_eq!(unanswered, vec![serde_json::json!(7)]);
        let bytes = std::mem::take(&mut *output.lock().unwrap());
        let text = String::from_utf8(bytes).unwrap();
        let msg: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(msg["id"], serde_json::json!(7));
        assert!(
            msg["error"]["message"]
                .as_str()
                .unwrap()
                .contains("nothing was sent"),
            "{text}"
        );
    }

    #[test]
    fn pump_does_not_reanswer_responded_requests() {
        let (client, server) = UnixStream::pair().unwrap();
        // A fake instance that answers id 7 and then closes the connection.
        let instance = thread::spawn(move || {
            let mut reader = BufReader::new(server.try_clone().unwrap());
            let mut writer = server;
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            let _ = writeln!(
                writer,
                "{{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{{\"ok\":true}}}}"
            );
            // Dropping the connection is a graceful shutdown here.
        });
        let input = Cursor::new(
            b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{}}\n".to_vec(),
        );
        let output = Arc::new(Mutex::new(Vec::new()));
        let unanswered = pump_jsonrpc(input, Arc::clone(&output), client);
        instance.join().unwrap();
        assert!(unanswered.is_empty());
        let bytes = std::mem::take(&mut *output.lock().unwrap());
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"result\""), "{text}");
        assert!(!text.contains("nothing was sent"), "{text}");
    }

    #[test]
    fn pump_does_not_answer_requests_of_an_agent_that_left() {
        let (client, server) = UnixStream::pair().unwrap();
        // The instance reads the call, sees the agent's side close, withdraws
        // the call without answering, and closes the connection.
        let instance = thread::spawn(move || {
            let mut reader = BufReader::new(server);
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            line.clear();
            assert_eq!(reader.read_line(&mut line).unwrap(), 0);
        });
        let input = Cursor::new(
            b"{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{}}\n".to_vec(),
        );
        let output = Arc::new(Mutex::new(Vec::new()));
        let unanswered = pump_jsonrpc(input, Arc::clone(&output), client);
        instance.join().unwrap();
        assert!(unanswered.is_empty(), "{unanswered:?}");
        assert!(output.lock().unwrap().is_empty());
    }

    #[test]
    fn pump_does_not_answer_requests_the_agent_cancelled() {
        let (client, server) = UnixStream::pair().unwrap();
        // A fake instance that reads the call and its cancellation, answers
        // neither (as the real one does), and closes the connection.
        let instance = thread::spawn(move || {
            let mut reader = BufReader::new(server.try_clone().unwrap());
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            line.clear();
            let _ = reader.read_line(&mut line);
            assert!(line.contains("notifications/cancelled"), "{line}");
        });
        let input = Cursor::new(
            b"{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{}}\n\
              {\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":9}}\n"
                .to_vec(),
        );
        let output = Arc::new(Mutex::new(Vec::new()));
        let unanswered = pump_jsonrpc(input, Arc::clone(&output), client);
        instance.join().unwrap();
        assert!(unanswered.is_empty(), "{unanswered:?}");
        let text = String::from_utf8(std::mem::take(&mut *output.lock().unwrap())).unwrap();
        assert!(text.is_empty(), "{text}");
    }

    #[test]
    fn pump_fails_requests_arriving_after_the_instance_died() {
        let (client, server) = UnixStream::pair().unwrap();
        // A fake instance that reads one request, then dies without answering.
        let instance = thread::spawn(move || {
            let mut reader = BufReader::new(server);
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
        });
        // The agent (still connected) sends id 7 (never answered) and then
        // id 8, which the bridge must fail fast instead of forwarding into
        // the dead socket.
        let (mut agent, agent_side) = UnixStream::pair().unwrap();
        agent
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{}}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/call\",\"params\":{}}\n",
            )
            .unwrap();
        let input = BufReader::new(agent_side);
        let output = Arc::new(Mutex::new(Vec::new()));
        let unanswered = pump_jsonrpc(input, Arc::clone(&output), client);
        instance.join().unwrap();
        // id 8 may still be on its way through the write side: give it a
        // moment before the agent leaves.
        std::thread::sleep(std::time::Duration::from_millis(200));
        drop(agent);
        // Which side answers id 8 is timing-dependent (the read side's drain
        // or the write side's fail-fast); what matters is that both requests
        // are answered exactly once.
        assert!(unanswered.contains(&serde_json::json!(7)), "{unanswered:?}");
        assert!(unanswered.len() <= 2, "{unanswered:?}");
        assert!(
            unanswered
                .iter()
                .all(|id| *id == serde_json::json!(7) || *id == serde_json::json!(8)),
            "{unanswered:?}"
        );
        let bytes = std::mem::take(&mut *output.lock().unwrap());
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(text.matches("nothing was sent").count(), 2, "{text}");
        assert!(text.contains("\"id\":7"), "{text}");
        assert!(text.contains("\"id\":8"), "{text}");
    }
}
