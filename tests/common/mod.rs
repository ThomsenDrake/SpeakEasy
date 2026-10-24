//! Shared helpers for the integration tests.
//!
//! - [`MockSlackServer`]: a local stand-in for the Slack Web API. It records
//!   every request and replies with canned JSON.
//! - [`MockMistralServer`]: a local stand-in for the Mistral platform API
//!   (connector tool listing and tool calls). Same recording contract.
//! - [`ScriptedProvider`]: a [`DecisionProvider`] that replays scripted
//!   human decisions and records what it was asked about.
//! - [`temp_state_dir`]: a fresh, unique gateway state directory.
//!
//! Each test binary compiles this module separately (`mod common;`), so not
//! every binary uses every helper.
#![allow(dead_code)]

use serde_json::{json, Value};
use slack_approval_gateway::core::{Decision, DecisionProvider, MessageRequest, ProviderError};
use slack_approval_gateway::mistral_slack::MistralConnectorSlackClient;
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

/// Mistral API key used by [`MockMistralServer::client`]. Requests made with
/// it carry `Authorization: Bearer mistral-test-key`.
pub const TEST_MISTRAL_KEY: &str = "mistral-test-key";

/// Lock a mutex, ignoring poisoning (a panicking test thread must not cascade
/// into unrelated assertions).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Routing and recording interface shared by the mock API servers
/// ([`MockSlackServer`], [`MockMistralServer`]).
trait MockApi: Send + Sync {
    /// The canned reply for one request (computed before recording).
    fn route(&self, req: &RecordedRequest) -> Reply;
    /// Record a request (after routing, before the reply is written, so it
    /// is visible as soon as the client call returns).
    fn record(&self, req: RecordedRequest);
    /// True when the accept loop should stop.
    fn shutdown_requested(&self) -> bool;
    /// Signal shutdown (called by the server handle's `Drop`).
    fn stop(&self);
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
        let api: Arc<dyn MockApi> = state.clone();
        let accept_thread = std::thread::spawn(move || accept_loop(listener, api));
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
        self.state.stop();
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

impl MockApi for MockState {
    fn route(&self, req: &RecordedRequest) -> Reply {
        route_slack(req, self)
    }
    fn record(&self, req: RecordedRequest) {
        lock(&self.requests).push(req);
    }
    fn shutdown_requested(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }
    fn stop(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

fn accept_loop(listener: TcpListener, api: Arc<dyn MockApi>) {
    for stream in listener.incoming() {
        if api.shutdown_requested() {
            break;
        }
        let Ok(stream) = stream else { continue };
        let api = Arc::clone(&api);
        std::thread::spawn(move || {
            let _ = serve_connection(stream, &*api);
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
fn serve_connection(stream: TcpStream, api: &dyn MockApi) -> std::io::Result<()> {
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
    let reply = api.route(&recorded);
    api.record(recorded);

    let (status, content_type, payload) = match reply {
        Reply::Json(json) => (
            "200 OK".to_string(),
            "application/json; charset=utf-8".to_string(),
            json.to_string(),
        ),
        Reply::Raw(raw) => (
            "200 OK".to_string(),
            "application/json; charset=utf-8".to_string(),
            raw,
        ),
        Reply::Status(code, body) => (
            format!("{} {}", code, reason_phrase(code)),
            "application/json; charset=utf-8".to_string(),
            body,
        ),
        Reply::NotFound => (
            "404 Not Found".to_string(),
            "text/plain; charset=utf-8".to_string(),
            "not found".to_string(),
        ),
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
    /// A non-200 status code with this JSON body (e.g. a forced API error).
    Status(u16, String),
    /// `404 Not Found`.
    NotFound,
}

/// Reason phrase for the status codes the mocks produce.
fn reason_phrase(code: u16) -> &'static str {
    match code {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Error",
    }
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
fn route_slack(req: &RecordedRequest, state: &MockState) -> Reply {
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
// Mock Mistral platform API (Slack connector)
// ---------------------------------------------------------------------------

/// State shared between the mock Mistral API server handle and its threads.
struct MockMistralState {
    requests: Mutex<Vec<RecordedRequest>>,
    /// The connector's tool catalog, served by `GET /v1/connectors/*/tools`.
    tools: Mutex<Vec<Value>>,
    /// Exact JSON reply for tool calls when set (`None` = the default reply).
    tool_reply: Mutex<Option<Value>>,
    /// Forced tool-call failure: HTTP status + `{"detail": …}` body.
    call_error: Mutex<Option<(u16, String)>>,
    shutdown: AtomicBool,
}

/// A mock Mistral platform API on `127.0.0.1:<random port>`.
///
/// Routes (anything else gets `404 Not Found`):
/// - `GET /v1/connectors/{connector}/tools` replies with the tool catalog as
///   a bare JSON array (what the real API returns), settable with
///   [`set_tools`](Self::set_tools). The default catalog lists
///   `slack_search_messages`, `slack_list_channels` and `slack_send_message`.
/// - `POST /v1/connectors/{connector}/tools/{tool}/call` replies with an MCP
///   CallToolResult-shaped JSON document: a text block containing a Slack
///   permalink plus `metadata.mcp_meta.structuredContent` with
///   `{ok, channel, ts, permalink}`. Replace it with
///   [`set_tool_reply`](Self::set_tool_reply), or force a non-200 status with
///   `{"detail": …}` via [`set_call_error`](Self::set_call_error).
///
/// Like [`MockSlackServer`], every request is recorded (including its
/// `Authorization` header and parsed JSON body), each connection is served on
/// its own thread, and dropping the server stops the accept loop.
pub struct MockMistralServer {
    addr: SocketAddr,
    state: Arc<MockMistralState>,
    accept_thread: Option<JoinHandle<()>>,
}

impl MockMistralServer {
    /// Bind `127.0.0.1:0` and start serving on a background thread.
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock Mistral server");
        let addr = listener.local_addr().expect("mock Mistral server address");
        let state = Arc::new(MockMistralState {
            requests: Mutex::new(Vec::new()),
            tools: Mutex::new(default_mistral_tools()),
            tool_reply: Mutex::new(None),
            call_error: Mutex::new(None),
            shutdown: AtomicBool::new(false),
        });
        let api: Arc<dyn MockApi> = state.clone();
        let accept_thread = std::thread::spawn(move || accept_loop(listener, api));
        Self {
            addr,
            state,
            accept_thread: Some(accept_thread),
        }
    }

    /// API base URL to hand to the client, e.g. `http://127.0.0.1:54321`
    /// (no trailing slash; the client appends `/v1/connectors/...`).
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// A real [`MistralConnectorSlackClient`] pointed at this server, using
    /// [`TEST_MISTRAL_KEY`].
    pub fn client(&self) -> MistralConnectorSlackClient {
        MistralConnectorSlackClient::new(TEST_MISTRAL_KEY, self.base_url())
    }

    /// Every request received so far, in arrival order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        lock(&self.state.requests).clone()
    }

    /// Requests received for one path prefix, e.g. `"/v1/connectors/slack/tools"`.
    pub fn requests_to(&self, path: &str) -> Vec<RecordedRequest> {
        lock(&self.state.requests)
            .iter()
            .filter(|r| r.path == path)
            .cloned()
            .collect()
    }

    /// Recorded tool calls (`POST /v1/connectors/{c}/tools/{t}/call`), in
    /// arrival order.
    pub fn tool_calls(&self) -> Vec<RecordedRequest> {
        lock(&self.state.requests)
            .iter()
            .filter(|r| r.method == "POST" && r.path.ends_with("/call"))
            .cloned()
            .collect()
    }

    /// Replace the connector's tool catalog.
    pub fn set_tools(&self, tools: Vec<Value>) {
        *lock(&self.state.tools) = tools;
    }

    /// Serve this exact JSON document for every tool call (`None` restores
    /// the default reply).
    pub fn set_tool_reply(&self, reply: Option<Value>) {
        *lock(&self.state.tool_reply) = reply;
    }

    /// Make tool calls fail with an HTTP status and a `{"detail": …}` body
    /// (`None` restores success).
    pub fn set_call_error(&self, error: Option<(u16, &str)>) {
        *lock(&self.state.call_error) = error.map(|(code, detail)| (code, detail.to_string()));
    }
}

impl Drop for MockMistralServer {
    fn drop(&mut self) {
        self.state.stop();
        if TcpStream::connect_timeout(&self.addr, Duration::from_secs(1)).is_ok() {
            if let Some(handle) = self.accept_thread.take() {
                let _ = handle.join();
            }
        }
    }
}

impl MockApi for MockMistralState {
    fn route(&self, req: &RecordedRequest) -> Reply {
        route_mistral(req, self)
    }
    fn record(&self, req: RecordedRequest) {
        lock(&self.requests).push(req);
    }
    fn shutdown_requested(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }
    fn stop(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

/// The default connector tool catalog: two read tools plus the send tool
/// with `channel`/`text`/`thread_ts` arguments.
fn default_mistral_tools() -> Vec<Value> {
    json!([
        { "name": "slack_search_messages", "inputSchema": { "type": "object" } },
        { "name": "slack_list_channels", "inputSchema": { "type": "object" } },
        { "name": "slack_send_message", "inputSchema": { "type": "object", "properties": {
            "channel": { "type": "string" }, "text": { "type": "string" },
            "thread_ts": { "type": "string" }
        } } }
    ])
    .as_array()
    .expect("default tool catalog is an array")
    .clone()
}

/// The default tool-call reply: an MCP CallToolResult with the permalink in
/// both the text block and the structured content.
fn default_tool_reply() -> Value {
    json!({
        "content": [{ "type": "text",
            "text": "Message sent to #general: https://mock.slack.com/archives/CGENERAL/p1700000000000100" }],
        "metadata": { "mcp_meta": { "structuredContent": {
            "ok": true, "channel": "CGENERAL", "ts": "1700000000.000100",
            "permalink": "https://mock.slack.com/archives/CGENERAL/p1700000000000100"
        } } }
    })
}

/// Canned Mistral API reply for a request. Paths look like
/// `/v1/connectors/{connector}/tools` (list) or
/// `/v1/connectors/{connector}/tools/{tool}/call` (invoke).
fn route_mistral(req: &RecordedRequest, state: &MockMistralState) -> Reply {
    let segments: Vec<&str> = req.path.split('/').filter(|s| !s.is_empty()).collect();
    let is_tools = segments.len() == 4
        && segments[0] == "v1"
        && segments[1] == "connectors"
        && segments[3] == "tools";
    let is_call = segments.len() == 6
        && segments[0] == "v1"
        && segments[1] == "connectors"
        && segments[3] == "tools"
        && segments[5] == "call";
    if is_tools && req.method == "GET" {
        // The real API answers with a bare array of tools.
        return Reply::Json(Value::Array(lock(&state.tools).clone()));
    }
    if is_call && req.method == "POST" {
        if let Some((code, detail)) = lock(&state.call_error).clone() {
            return Reply::Status(code, json!({ "detail": detail }).to_string());
        }
        if let Some(reply) = lock(&state.tool_reply).clone() {
            return Reply::Json(reply);
        }
        return Reply::Json(default_tool_reply());
    }
    Reply::NotFound
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
