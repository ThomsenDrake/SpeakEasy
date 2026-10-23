//! Shared helpers for the integration tests.
//!
//! - [`MockSlackServer`]: a local stand-in for the Slack Web API. It records
//!   every request and replies with canned JSON.
//! - [`ScriptedProvider`]: a [`DecisionProvider`] that replays scripted
//!   human decisions and records what it was asked about.
//! - [`temp_state_dir`]: a fresh, unique gateway state directory.
//!
//! Each test binary compiles this module separately (`mod common;`), so not
//! every binary uses every helper.
#![allow(dead_code)]

use serde_json::{json, Value};
use slack_approval_gateway::core::{Decision, DecisionProvider, MessageRequest, ProviderError};
use slack_approval_gateway::slack::UreqSlackClient;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Bot token used by [`MockSlackServer::client`]. Requests made with it carry
/// `Authorization: Bearer xoxb-test-token`.
pub const TEST_TOKEN: &str = "xoxb-test-token";

/// Lock a mutex, ignoring poisoning (a panicking test thread must not cascade
/// into unrelated assertions).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------
// Mock Slack Web API
// ---------------------------------------------------------------------------

/// One HTTP request received by [`MockSlackServer`].
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    /// HTTP method, e.g. `POST`.
    pub method: String,
    /// Request path without the query string, e.g. `/chat.postMessage`.
    pub path: String,
    /// Raw query string without the leading `?`, if any.
    pub query: Option<String>,
    /// Value of the `Authorization` header, if present.
    pub authorization: Option<String>,
    /// Value of the `Content-Type` header, if present.
    pub content_type: Option<String>,
    /// Parsed JSON body; `Value::Null` if the body was empty or not JSON.
    pub body: Value,
}

impl RecordedRequest {
    /// A string field of the JSON body, e.g. `req.body_str("text")`.
    pub fn body_str(&self, key: &str) -> Option<&str> {
        self.body.get(key).and_then(Value::as_str)
    }
}

/// State shared between the server handle and its connection threads.
struct MockState {
    requests: Mutex<Vec<RecordedRequest>>,
    post_message_error: Mutex<Option<String>>,
    post_message_raw_reply: Mutex<Option<String>>,
    permalink_error: Mutex<Option<String>>,
    /// Counter for generated message timestamps.
    next_ts: AtomicU64,
    shutdown: AtomicBool,
}

/// A mock Slack Web API on `127.0.0.1:<random port>`.
///
/// Routes (anything else gets `404 Not Found`):
/// - `POST /chat.postMessage` replies
///   `{"ok":true,"channel":<mock_channel_id(request channel)>,"ts":"1700000000.00000N",...}`
///   with a fresh `ts` per post (`N` = 1, 2, 3, …), or
///   `{"ok":false,"error":...}` after [`set_post_message_error`](Self::set_post_message_error).
///   Like real Slack, the reply carries a channel *ID*: `C0123` is echoed,
///   `#general` becomes `CGENERAL` (see [`mock_channel_id`]). The recorded
///   request body always keeps the exact destination the client sent.
/// - `POST /chat.getPermalink` replies
///   `{"ok":true,"permalink":"https://mock.slack.com/archives/{channel}/p{ts without '.'}"}`,
///   or `{"ok":false,"error":...}` after [`set_permalink_error`](Self::set_permalink_error).
///
/// Every request is recorded before the reply is written, so it is visible in
/// [`requests`](Self::requests) as soon as the client call returns. Each
/// connection is served on its own thread (concurrent clients are fine) and
/// gets `Connection: close`. Dropping the server stops the accept loop
/// without blocking the test.
///
/// The client honors the standard proxy variables (`HTTP_PROXY`,
/// `HTTPS_PROXY`, `ALL_PROXY`). If one is set, `NO_PROXY` must cover
/// `127.0.0.1`, or requests meant for the mock go to the proxy.
pub struct MockSlackServer {
    addr: SocketAddr,
    state: Arc<MockState>,
    accept_thread: Option<JoinHandle<()>>,
}

impl MockSlackServer {
    /// Bind `127.0.0.1:0` and start serving on a background thread.
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock Slack server");
        let addr = listener.local_addr().expect("mock Slack server address");
        let state = Arc::new(MockState {
            requests: Mutex::new(Vec::new()),
            post_message_error: Mutex::new(None),
            post_message_raw_reply: Mutex::new(None),
            permalink_error: Mutex::new(None),
            next_ts: AtomicU64::new(1),
            shutdown: AtomicBool::new(false),
        });
        let thread_state = Arc::clone(&state);
        let accept_thread = std::thread::spawn(move || accept_loop(listener, thread_state));
        Self {
            addr,
            state,
            accept_thread: Some(accept_thread),
        }
    }

    /// API base URL to hand to the client, e.g. `http://127.0.0.1:54321`
    /// (no trailing slash; the client appends `/chat.postMessage`).
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// A real [`UreqSlackClient`] pointed at this server, using [`TEST_TOKEN`].
    pub fn client(&self) -> UreqSlackClient {
        UreqSlackClient::new(TEST_TOKEN, self.base_url())
    }

    /// Every request received so far, in arrival order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        lock(&self.state.requests).clone()
    }

    /// Requests received for one path, e.g. `"/chat.getPermalink"`.
    pub fn requests_to(&self, path: &str) -> Vec<RecordedRequest> {
        lock(&self.state.requests)
            .iter()
            .filter(|r| r.path == path)
            .cloned()
            .collect()
    }

    /// Number of `POST /chat.postMessage` requests received so far (i.e.
    /// send attempts), including ones answered with an error.
    pub fn post_count(&self) -> usize {
        lock(&self.state.requests)
            .iter()
            .filter(|r| r.method == "POST" && r.path == "/chat.postMessage")
            .count()
    }

    /// Make `chat.postMessage` reply `{"ok":false,"error":<error>}` from now
    /// on (`None` restores success). Failed posts get no `ts`.
    pub fn set_post_message_error(&self, error: Option<&str>) {
        *lock(&self.state.post_message_error) = error.map(str::to_string);
    }

    /// Make `chat.postMessage` reply `200 OK` with `body` verbatim from now on
    /// (`None` restores normal replies), e.g. malformed JSON. Takes precedence
    /// over [`set_post_message_error`](Self::set_post_message_error).
    pub fn set_post_message_raw_reply(&self, body: Option<&str>) {
        *lock(&self.state.post_message_raw_reply) = body.map(str::to_string);
    }

    /// Make `chat.getPermalink` reply `{"ok":false,"error":<error>}` from now
    /// on (`None` restores success), to exercise the client's fallback link.
    pub fn set_permalink_error(&self, error: Option<&str>) {
        *lock(&self.state.permalink_error) = error.map(str::to_string);
    }
}

impl Drop for MockSlackServer {
    fn drop(&mut self) {
        self.state.shutdown.store(true, Ordering::SeqCst);
        // Wake the blocking accept() so the loop sees the flag. Join only if
        // the wake-up connection succeeded; otherwise leave the thread
        // detached rather than risk blocking the test.
        if TcpStream::connect_timeout(&self.addr, Duration::from_secs(1)).is_ok() {
            if let Some(handle) = self.accept_thread.take() {
                let _ = handle.join();
            }
        }
    }
}

fn accept_loop(listener: TcpListener, state: Arc<MockState>) {
    for stream in listener.incoming() {
        if state.shutdown.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        let state = Arc::clone(&state);
        std::thread::spawn(move || {
            let _ = serve_connection(stream, &state);
        });
    }
}

/// A parsed HTTP/1.1 request.
struct RawRequest {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl RawRequest {
    /// Header value by case-insensitive name (names are stored lowercase).
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Serve exactly one request on `stream`, then close it.
fn serve_connection(stream: TcpStream, state: &MockState) -> std::io::Result<()> {
    // Bound every read/write so a misbehaving client cannot pin a thread.
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(&stream);
    let Some(raw) = read_request(&mut reader)? else {
        return Ok(()); // empty connection (e.g. the Drop wake-up)
    };

    let (path, query) = match raw.target.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (raw.target.clone(), None),
    };
    let body: Value = if raw.body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&raw.body).unwrap_or(Value::Null)
    };
    let recorded = RecordedRequest {
        method: raw.method.clone(),
        path,
        query,
        authorization: raw.header("authorization").map(str::to_string),
        content_type: raw.header("content-type").map(str::to_string),
        body,
    };
    let reply = route(&recorded, state);
    lock(&state.requests).push(recorded);

    let (status, content_type, payload) = match reply {
        Reply::Json(json) => ("200 OK", "application/json; charset=utf-8", json.to_string()),
        Reply::Raw(raw) => ("200 OK", "application/json; charset=utf-8", raw),
        Reply::NotFound => ("404 Not Found", "text/plain; charset=utf-8", "not found".to_string()),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    (&stream).write_all(response.as_bytes())?;
    (&stream).flush()
}

/// What the mock answers to one request.
enum Reply {
    /// `200 OK` with this JSON body.
    Json(Value),
    /// `200 OK` with this body verbatim (see `set_post_message_raw_reply`).
    Raw(String),
    /// `404 Not Found`.
    NotFound,
}

/// The channel ID Slack would report for a destination: IDs are echoed
/// unchanged, `#name` becomes `C` + NAME and `@name` becomes `D` + NAME
/// (uppercased, non-alphanumerics dropped), e.g. `#general` -> `CGENERAL`.
pub fn mock_channel_id(destination: &str) -> String {
    let upper = |name: &str| -> String {
        name.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_uppercase())
            .collect()
    };
    if let Some(name) = destination.strip_prefix('#') {
        format!("C{}", upper(name))
    } else if let Some(name) = destination.strip_prefix('@') {
        format!("D{}", upper(name))
    } else {
        destination.to_string()
    }
}

/// Canned Slack reply for a request.
fn route(req: &RecordedRequest, state: &MockState) -> Reply {
    if req.method != "POST" {
        return Reply::NotFound;
    }
    match req.path.as_str() {
        "/chat.postMessage" => {
            if let Some(raw) = lock(&state.post_message_raw_reply).clone() {
                return Reply::Raw(raw);
            }
            if let Some(error) = lock(&state.post_message_error).clone() {
                return Reply::Json(json!({ "ok": false, "error": error }));
            }
            let n = state.next_ts.fetch_add(1, Ordering::SeqCst);
            let ts = format!("1700000000.{n:06}");
            let channel = mock_channel_id(req.body_str("channel").unwrap_or("C0MOCK"));
            let mut message = json!({ "type": "message", "text": req.body.get("text"), "ts": ts });
            if let Some(thread_ts) = req.body.get("thread_ts") {
                message["thread_ts"] = thread_ts.clone();
            }
            Reply::Json(json!({ "ok": true, "channel": channel, "ts": ts, "message": message }))
        }
        "/chat.getPermalink" => {
            if let Some(error) = lock(&state.permalink_error).clone() {
                return Reply::Json(json!({ "ok": false, "error": error }));
            }
            let (Some(channel), Some(ts)) = (req.body_str("channel"), req.body_str("message_ts")) else {
                return Reply::Json(json!({ "ok": false, "error": "invalid_arguments" }));
            };
            let permalink = format!(
                "https://mock.slack.com/archives/{channel}/p{}",
                ts.replace('.', "")
            );
            Reply::Json(json!({ "ok": true, "channel": channel, "permalink": permalink }))
        }
        _ => Reply::NotFound,
    }
}

/// Read one HTTP/1.1 request (request line, headers, and a `Content-Length`
/// or chunked body). Returns `None` if the peer closed without sending.
fn read_request(reader: &mut impl BufRead) -> std::io::Result<Option<RawRequest>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();

    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    let mut raw = RawRequest {
        method,
        target,
        headers,
        body: Vec::new(),
    };

    let chunked = raw
        .header("transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    if chunked {
        loop {
            let mut size_line = String::new();
            reader.read_line(&mut size_line)?;
            let size_hex = size_line.trim().split(';').next().unwrap_or("");
            let size = usize::from_str_radix(size_hex, 16).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "bad chunk size")
            })?;
            if size == 0 {
                // Skip optional trailers up to the final blank line.
                loop {
                    let mut trailer = String::new();
                    if reader.read_line(&mut trailer)? == 0 || trailer.trim().is_empty() {
                        break;
                    }
                }
                break;
            }
            let mut chunk = vec![0; size];
            reader.read_exact(&mut chunk)?;
            raw.body.extend_from_slice(&chunk);
            let mut crlf = [0; 2];
            reader.read_exact(&mut crlf)?;
        }
    } else if let Some(len) = raw.header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        raw.body = vec![0; len];
        reader.read_exact(&mut raw.body)?;
    }
    Ok(Some(raw))
}

// ---------------------------------------------------------------------------
// Scripted decision provider
// ---------------------------------------------------------------------------

/// A [`DecisionProvider`] that stands in for the human.
///
/// Decisions are taken in this order:
/// 1. the first unused rule added with
///    [`when_message_contains`](Self::when_message_contains) whose needle
///    occurs in the request's message (each rule is used once), so
///    concurrent requests get decisions by content rather than arrival order;
/// 2. otherwise the next decision from the FIFO queue;
/// 3. otherwise `Err(ProviderError("no scripted decision left"))`.
///
/// The message text of every request it is asked about is recorded (even
/// when no decision is left), so tests can assert that the gateway did or did
/// not show a popup. Share it with the gateway as an `Arc` and keep a clone:
///
/// ```ignore
/// let provider = Arc::new(ScriptedProvider::new([Decision::Approve]));
/// let gateway = Gateway::new(slack, provider.clone(), dir)?;
/// // ...
/// assert_eq!(provider.call_count(), 1);
/// ```
pub struct ScriptedProvider {
    decisions: Mutex<VecDeque<Decision>>,
    rules: Mutex<Vec<(String, Decision)>>,
    seen: Mutex<Vec<String>>,
    delay: Option<Duration>,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
}

impl ScriptedProvider {
    /// A provider that returns `decisions` in FIFO order.
    pub fn new(decisions: impl IntoIterator<Item = Decision>) -> Self {
        Self {
            decisions: Mutex::new(decisions.into_iter().collect()),
            rules: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
            delay: None,
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
        }
    }

    /// Builder: answer the first request whose message contains `needle` with
    /// `decision` (checked before the FIFO queue; used once).
    pub fn when_message_contains(self, needle: impl Into<String>, decision: Decision) -> Self {
        lock(&self.rules).push((needle.into(), decision));
        self
    }

    /// Builder: sleep for `delay` before answering, like a human taking time
    /// to decide. Useful to make concurrent requests overlap.
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    /// Append a decision to the FIFO queue.
    pub fn push(&self, decision: Decision) {
        lock(&self.decisions).push_back(decision);
    }

    /// Message text of every request the provider was asked about, in call order.
    pub fn seen_messages(&self) -> Vec<String> {
        lock(&self.seen).clone()
    }

    /// How many times `request_decision` was called (i.e. popups shown).
    pub fn call_count(&self) -> usize {
        lock(&self.seen).len()
    }

    /// Scripted decisions not yet used (FIFO queue plus unused rules).
    pub fn remaining(&self) -> usize {
        lock(&self.decisions).len() + lock(&self.rules).len()
    }

    /// Highest number of `request_decision` calls that were in progress at the
    /// same time (2+ proves the gateway asked about requests concurrently).
    pub fn max_concurrent_calls(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }

    fn next_decision(&self, message: &str) -> Result<Decision, ProviderError> {
        {
            let mut rules = lock(&self.rules);
            if let Some(i) = rules.iter().position(|(needle, _)| message.contains(needle.as_str())) {
                return Ok(rules.remove(i).1);
            }
        }
        lock(&self.decisions)
            .pop_front()
            .ok_or_else(|| ProviderError("no scripted decision left".to_string()))
    }
}

impl DecisionProvider for ScriptedProvider {
    fn request_decision(&self, request: &MessageRequest) -> Result<Decision, ProviderError> {
        let message = &request.incoming.message;
        lock(&self.seen).push(message.clone());
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(now, Ordering::SeqCst);
        if let Some(delay) = self.delay {
            std::thread::sleep(delay);
        }
        let decision = self.next_decision(message);
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        decision
    }
}

// ---------------------------------------------------------------------------
// Filesystem
// ---------------------------------------------------------------------------

/// Create and return a fresh, unique directory under `std::env::temp_dir()`
/// for use as a gateway state directory. `tag` makes it recognizable
/// (non-alphanumeric characters become `-`). The directory is not removed
/// automatically.
pub fn temp_state_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let tag: String = tag
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "slack-approval-gateway-test-{tag}-{}-{nanos}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).expect("create temp state dir");
    dir
}
