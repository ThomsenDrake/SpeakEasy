//! Single-instance coordination.
//!
//! Every coding agent spawns this binary over MCP stdio, but the approval
//! popup, the Slack client, and the pending-request queue must live in
//! exactly one process. The first process to start becomes the *instance*:
//! it holds `gateway.lock` in the state dir, listens for MCP connections on
//! `gateway.sock` (a Unix socket), and serves both its own stdio and every
//! socket connection with the same JSON-RPC handler. Any later process
//! connects to the socket and becomes a *bridge*: a byte-transparent pump
//! between the agent's stdio and the instance, so the agent's MCP session
//! behaves exactly as if it had spawned the instance itself.
//!
//! All state (cooldowns, sent records, locks) already lives in the shared
//! state dir, so even if two instances ever raced into existence, deny
//! cooldowns and duplicate sends would still be enforced across processes.

use crate::core::Gateway;
use crate::mcp;
use std::io::{BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

/// Pump bytes between an agent's stdio and the instance's socket. Runs
/// until the instance closes the connection; exits the process. Protocol
/// bytes are the only thing written to stdout; diagnostics go to stderr.
pub fn run_bridge(mut stream: UnixStream) -> ! {
    eprintln!("slack-approval-gateway: bridging MCP stdio to the running instance");
    let mut to_instance = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("slack-approval-gateway: cannot bridge: {e}");
            std::process::exit(1);
        }
    };
    thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut stdin = stdin.lock();
        let _ = std::io::copy(&mut stdin, &mut to_instance);
        // The agent closed its MCP session: tell the instance, then let the
        // read side finish the shutdown.
        let _ = to_instance.shutdown(std::net::Shutdown::Write);
    });
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let _ = std::io::copy(&mut stream, &mut stdout);
    let _ = stdout.flush();
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    /// The testable core of [`run_bridge`]: forward `input` to the instance
    /// and the instance's replies to `output` until the instance closes the
    /// connection. Returns when the read side reaches EOF.
    fn pump<R: Read + Send + 'static, W: Write>(mut input: R, mut output: W, mut stream: UnixStream) {
        let mut to_instance = stream.try_clone().expect("clone stream");
        let copy_in = thread::spawn(move || {
            let _ = std::io::copy(&mut input, &mut to_instance);
            let _ = to_instance.shutdown(std::net::Shutdown::Write);
        });
        let _ = std::io::copy(&mut stream, &mut output);
        let _ = output.flush();
        let _ = copy_in.join();
    }

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
        let mut output: Vec<u8> = Vec::new();
        pump(input, &mut output, client);
        assert_eq!(output, b"ping\n");
        echo.join().unwrap();
    }
}
