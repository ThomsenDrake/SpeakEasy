//! Shared helpers for the integration tests.
//!
//! - [`MockSlackMcpServer`]: a local stand-in for the official Slack MCP
//!   server (MCP Streamable HTTP: JSON-RPC POSTs, JSON or SSE replies). It
//!   records every request and replies with canned JSON-RPC results.
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
use slack_approval_gateway::slack_mcp::SlackMcpClient;
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Slack user OAuth token used by [`MockSlackMcpServer::client`]. Requests
/// made with it carry `Authorization: Bearer xoxp-slack-mcp-test-token`.
pub const TEST_SLACK_MCP_TOKEN: &str = "xoxp-slack-mcp-test-token";

/// Mistral API key used by [`MockMistralServer::client`]. Requests made with
/// it carry `Authorization: Bearer mistral-test-key`.
pub const TEST_MISTRAL_KEY: &str = "mistral-test-key";

/// Lock a mutex, ignoring poisoning (a panicking test thread must not cascade
/// into unrelated assertions).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Routing and recording interface shared by the mock API servers
/// ([`MockSlackMcpServer`], [`MockMistralServer`]).
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
// Mock Slack MCP server (the official Slack MCP endpoint, locally)
// ---------------------------------------------------------------------------

/// One HTTP request received by a mock server.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    /// HTTP method, e.g. `POST`.
    pub method: String,
    /// Request path without the query string, e.g. `/mcp`.
    pub path: String,
    /// Raw query string without the leading `?`, if any.
    pub query: Option<String>,
    /// Value of the `Authorization` header, if present.
    pub authorization: Option<String>,
    /// Value of the `Content-Type` header, if present.
    pub content_type: Option<String>,
    /// Every request header, with lowercased names.
    pub headers: Vec<(String, String)>,
    /// Parsed JSON body; `Value::Null` if the body was empty or not JSON.
    pub body: Value,
}

impl RecordedRequest {
    /// A string field of the JSON body, e.g. `req.body_str("text")`.
    pub fn body_str(&self, key: &str) -> Option<&str> {
        self.body.get(key).and_then(Value::as_str)
    }

    /// A request header value by lowercased name, e.g.
    /// `req.header("mcp-session-id")`.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// State shared between the mock Slack MCP server handle and its threads.
struct MockSlackMcpState {
    requests: Mutex<Vec<RecordedRequest>>,
    /// The server's tool catalog, served by `tools/list`.
    tools: Mutex<Vec<Value>>,
    /// Exact CallToolResult for every tool call when set (`None` = the
    /// default replies).
    tool_reply: Mutex<Option<Value>>,
    /// Per-tool-name CallToolResult overrides (checked before `tool_reply`).
    tool_replies: Mutex<HashMap<String, Value>>,
    /// Forced failure for every request: HTTP status + `{"detail": …}` body.
    call_error: Mutex<Option<(u16, String)>>,
    /// When set, the send tool replies `isError` with this text.
    send_error: Mutex<Option<String>>,
    /// Serve tool-call replies as an SSE stream (`data:` lines).
    sse: AtomicBool,
    /// Counter for generated message timestamps.
    next_ts: AtomicU64,
    shutdown: AtomicBool,
}

/// A mock Slack MCP server on `127.0.0.1:<random port>`, speaking MCP
/// Streamable HTTP: one JSON-RPC message per `POST /mcp`, with plain-JSON
/// replies (or SSE streams after [`set_sse`](Self::set_sse)).
///
/// Routes (anything else gets `404 Not Found`):
/// - `initialize` replies with the echoed protocol version, empty
///   capabilities and server info, plus an `Mcp-Session-Id` response header
///   (which the client must send back on later requests);
/// - `notifications/initialized` replies `202 Accepted` with no body;
/// - `tools/list` replies with the tool catalog, settable with
///   [`set_tools`](Self::set_tools). The default catalog mirrors the
///   official Slack MCP server's list and order: `slack_search_messages`,
///   `slack_search_users`, `slack_search_channels`, `slack_read_channel`,
///   `slack_read_user_profile`, `slack_list_user_channels` and
///   `slack_send_message` (declaring `channel_id`/`message`/`thread_ts`);
/// - `tools/call` replies with an MCP CallToolResult. `slack_send_message`
///   gets `{"message_link": …, "message_context": {…}}` as a JSON string in
///   a text block, with a fresh `ts` per send (`N` = 1, 2, 3, … in
///   `1700000000.00000N`) and the channel ID Slack would report for the
///   requested destination (see [`mock_channel_id`]); the lookup tools get
///   canned `U0DRAKE`/`C0ENG`/`C0RAND` replies (`slack_read_user_profile` a
///   nested user object, `slack_list_user_channels` a channels array,
///   `slack_read_channel` a channel object), and the search tools fail
///   loudly — they must never be called. Replace any of it with
///   [`set_tool_reply`](Self::set_tool_reply) or
///   [`set_tool_reply_for`](Self::set_tool_reply_for), make the send tool
///   fail with [`set_send_error`](Self::set_send_error), or fail every
///   request with an HTTP status via [`set_call_error`](Self::set_call_error).
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
pub struct MockSlackMcpServer {
    addr: SocketAddr,
    state: Arc<MockSlackMcpState>,
    accept_thread: Option<JoinHandle<()>>,
}

impl MockSlackMcpServer {
    /// Bind `127.0.0.1:0` and start serving on a background thread.
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock Slack MCP server");
        let addr = listener.local_addr().expect("mock Slack MCP server address");
        let state = Arc::new(MockSlackMcpState {
            requests: Mutex::new(Vec::new()),
            tools: Mutex::new(default_mcp_tools()),
            tool_reply: Mutex::new(None),
            tool_replies: Mutex::new(HashMap::new()),
            call_error: Mutex::new(None),
            send_error: Mutex::new(None),
            sse: AtomicBool::new(false),
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

    /// The MCP endpoint URL to hand to the client, e.g.
    /// `http://127.0.0.1:54321/mcp`.
    pub fn mcp_url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    /// A real [`SlackMcpClient`] pointed at this server, using
    /// [`TEST_SLACK_MCP_TOKEN`]. The MCP handshake runs here, so a broken
    /// mock fails the test with the handshake error.
    pub fn client(&self) -> SlackMcpClient {
        SlackMcpClient::new(TEST_SLACK_MCP_TOKEN, self.mcp_url())
            .expect("the MCP handshake with the mock must succeed")
    }

    /// Every request received so far, in arrival order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        lock(&self.state.requests).clone()
    }

    /// Forget the recorded MCP handshake requests (`initialize` +
    /// `notifications/initialized`), keeping everything else. Building a
    /// client runs the handshake, so tests clear it to see only their own
    /// traffic — including traffic from an earlier gateway in the same test
    /// (the restart tests build a second gateway on the same mock).
    pub fn clear_handshake(&self) {
        lock(&self.state.requests).retain(|r| {
            let method = r.body.get("method").and_then(Value::as_str);
            method != Some("initialize") && method != Some("notifications/initialized")
        });
    }

    /// The JSON-RPC method of each recorded request, in arrival order.
    pub fn methods(&self) -> Vec<String> {
        lock(&self.state.requests)
            .iter()
            .map(|r| {
                r.body
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            })
            .collect()
    }

    /// Recorded `tools/call` requests for one tool, in arrival order.
    pub fn calls_to(&self, tool: &str) -> Vec<RecordedRequest> {
        lock(&self.state.requests)
            .iter()
            .filter(|r| r.body.pointer("/params/name").and_then(Value::as_str) == Some(tool))
            .cloned()
            .collect()
    }

    /// Number of send-tool calls received so far (i.e. send attempts),
    /// including ones answered with an error.
    pub fn send_count(&self) -> usize {
        lock(&self.state.requests)
            .iter()
            .filter(|r| {
                r.body.get("method").and_then(Value::as_str) == Some("tools/call")
                    && r.body
                        .pointer("/params/name")
                        .and_then(Value::as_str)
                        .is_some_and(is_send_tool)
            })
            .count()
    }

    /// Replace the server's tool catalog (served by `tools/list`).
    pub fn set_tools(&self, tools: Vec<Value>) {
        *lock(&self.state.tools) = tools;
    }

    /// Serve this exact CallToolResult for every tool call (`None` restores
    /// the default replies).
    pub fn set_tool_reply(&self, reply: Option<Value>) {
        *lock(&self.state.tool_reply) = reply;
    }

    /// Serve this exact CallToolResult for one tool's calls only (takes
    /// precedence over [`set_tool_reply`](Self::set_tool_reply)); used to
    /// give the read tools (`slack_read_user_profile`, …) their own replies.
    pub fn set_tool_reply_for(&self, tool: &str, reply: Value) {
        lock(&self.state.tool_replies).insert(tool.to_string(), reply);
    }

    /// Make the send tool reply `isError: true` with this text from now on
    /// (`None` restores success).
    pub fn set_send_error(&self, error: Option<&str>) {
        *lock(&self.state.send_error) = error.map(str::to_string);
    }

    /// Make every request fail with an HTTP status and a `{"detail": …}` body
    /// (`None` restores success).
    pub fn set_call_error(&self, error: Option<(u16, &str)>) {
        *lock(&self.state.call_error) = error.map(|(code, detail)| (code, detail.to_string()));
    }

    /// Serve tool-call replies as an SSE stream (`data:` lines) instead of
    /// plain JSON, exercising the client's SSE reply handling.
    pub fn set_sse(&self, sse: bool) {
        self.state.sse.store(sse, Ordering::SeqCst);
    }
}

impl Drop for MockSlackMcpServer {
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

impl MockApi for MockSlackMcpState {
    fn route(&self, req: &RecordedRequest) -> Reply {
        route_slack_mcp(req, self)
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
        headers: raw.headers.clone(),
        body,
    };
    let reply = api.route(&recorded);
    api.record(recorded);

    let (status, content_type, payload, extra_headers) = match reply {
        Reply::Json(json) => (
            "200 OK".to_string(),
            "application/json; charset=utf-8".to_string(),
            json.to_string(),
            Vec::new(),
        ),
        Reply::Raw(raw) => (
            "200 OK".to_string(),
            "application/json; charset=utf-8".to_string(),
            raw,
            Vec::new(),
        ),
        Reply::Status(code, body) => (
            format!("{} {}", code, reason_phrase(code)),
            "application/json; charset=utf-8".to_string(),
            body,
            Vec::new(),
        ),
        Reply::NotFound => (
            "404 Not Found".to_string(),
            "text/plain; charset=utf-8".to_string(),
            "not found".to_string(),
            Vec::new(),
        ),
        // A JSON reply with extra response headers (the MCP handshake's
        // `Mcp-Session-Id`).
        Reply::JsonWithHeaders(json, headers) => (
            "200 OK".to_string(),
            "application/json; charset=utf-8".to_string(),
            json.to_string(),
            headers,
        ),
        // An SSE stream of `data:` lines (MCP Streamable HTTP replies).
        Reply::Sse(body) => (
            "200 OK".to_string(),
            "text/event-stream".to_string(),
            body,
            Vec::new(),
        ),
    };
    let mut response = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n");
    for (name, value) in &extra_headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    ));
    (&stream).write_all(response.as_bytes())?;
    (&stream).flush()
}

/// What a mock answers to one request.
enum Reply {
    /// `200 OK` with this JSON body.
    Json(Value),
    /// `200 OK` with this body verbatim (raw replies, e.g. malformed JSON).
    Raw(String),
    /// A status code with this body: a non-200 error (e.g. a forced API
    /// error), or `202 Accepted` with an empty body (MCP notifications).
    Status(u16, String),
    /// `404 Not Found`.
    NotFound,
    /// `200 OK` with this JSON body plus extra response headers.
    JsonWithHeaders(Value, Vec<(String, String)>),
    /// `200 OK` with this `text/event-stream` body verbatim.
    Sse(String),
}

/// Reason phrase for the status codes the mocks produce.
fn reason_phrase(code: u16) -> &'static str {
    match code {
        202 => "Accepted",
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

/// The default tool catalog, shaped like the official Slack MCP server's
/// `tools/list` — including its order, where the search/read tools come
/// before the profile/list lookups — plus `slack_send_message` declaring
/// `channel_id`/`message`/`thread_ts`. The lookup tools declare the
/// official schemas: `slack_read_user_profile` takes `user_id`;
/// `slack_list_user_channels` takes `types`/`name_prefix`/`limit`/`cursor`/
/// `format`/`team_id` and no channel ID.
fn default_mcp_tools() -> Vec<Value> {
    json!([
        { "name": "slack_search_messages", "inputSchema": { "type": "object", "properties": {
            "query": { "type": "string" } } } },
        { "name": "slack_search_users", "inputSchema": { "type": "object", "properties": {
            "query": { "type": "string" } }, "required": ["query"] } },
        { "name": "slack_search_channels", "inputSchema": { "type": "object", "properties": {
            "query": { "type": "string" } }, "required": ["query"] } },
        { "name": "slack_read_channel", "inputSchema": { "type": "object", "properties": {
            "channel_id": { "type": "string" } }, "required": ["channel_id"] } },
        { "name": "slack_read_user_profile", "inputSchema": { "type": "object", "properties": {
            "user_id": { "type": "string" } } } },
        { "name": "slack_list_user_channels", "inputSchema": { "type": "object", "properties": {
            "types": { "type": "string" },
            "name_prefix": { "type": "string" },
            "exclude_archived": { "type": "boolean" },
            "limit": { "type": "integer" },
            "cursor": { "type": "string" },
            "format": { "type": "string" },
            "team_id": { "type": "string" }
        } } },
        { "name": "slack_send_message", "inputSchema": { "type": "object", "properties": {
            "channel_id": { "type": "string" },
            "message": { "type": "string" },
            "thread_ts": { "type": "string" }
        }, "required": ["channel_id", "message"] } }
    ])
    .as_array()
    .expect("default tool catalog is an array")
    .clone()
}

/// A CallToolResult carrying `payload` as a JSON string inside a text block
/// (the way the Slack tools wrap their results).
fn json_text_reply(payload: &Value) -> Value {
    json!({ "content": [{ "type": "text", "text": payload.to_string() }] })
}

/// The default `slack_send_message` CallToolResult: a fresh `ts` per send and
/// the channel ID Slack would report for the requested destination.
fn default_send_reply(state: &MockSlackMcpState, arguments: &Value) -> Value {
    let n = state.next_ts.fetch_add(1, Ordering::SeqCst);
    let ts = format!("1700000000.{n:06}");
    let channel = mock_channel_id(
        arguments
            .get("channel_id")
            .and_then(Value::as_str)
            .unwrap_or("C0MOCK"),
    );
    let link = format!(
        "https://mock.slack.com/archives/{channel}/p{}",
        ts.replace('.', "")
    );
    json_text_reply(&json!({
        "message_link": link,
        "message_context": { "message_ts": ts, "channel_id": channel }
    }))
}

/// True when a tool name looks like "send a message" (the same predicate
/// the client's discovery uses: `slack_send_message`, `post_message`, …).
fn is_send_tool(tool: &str) -> bool {
    (tool.contains("send") || tool.contains("post")) && tool.contains("message")
}

/// The default CallToolResult for one tool call.
fn default_tool_result(state: &MockSlackMcpState, tool: &str, arguments: &Value) -> Value {
    if is_send_tool(tool) {
        return match lock(&state.send_error).clone() {
            Some(error) => {
                json!({ "content": [{ "type": "text", "text": error }], "isError": true })
            }
            None => default_send_reply(state, arguments),
        };
    }
    // The official channel-lookup tool: a channels array (IDs + names).
    if tool == "slack_list_user_channels" {
        return json_text_reply(&json!({ "channels": [
            { "id": "C0ENG", "name": "eng" },
            { "id": "C0RAND", "name": "random" }
        ] }));
    }
    // The official user-lookup tool: a nested user object (the same shape
    // for a loosely-matching user tool that is not a channel tool).
    if tool == "slack_read_user_profile"
        || (tool.contains("user") && !tool.contains("search") && !tool.contains("channel"))
    {
        return match arguments.get("user_id").and_then(Value::as_str) {
            Some("U0DRAKE") => json_text_reply(&json!({ "user": {
                "real_name": "Drake Thomsen", "name": "drake.thomsen",
                "profile": { "display_name": "Drake" }
            } })),
            _ => json!({ "content": [{ "type": "text", "text": "user_not_found" }], "isError": true }),
        };
    }
    // A channel read by ID: a channel object (the same shape for a
    // loosely-matching channel tool).
    if tool == "slack_read_channel" || tool.contains("channel") {
        return match arguments.get("channel_id").and_then(Value::as_str) {
            Some("C0ENG") => {
                json_text_reply(&json!({ "channel": { "id": "C0ENG", "name": "eng" } }))
            }
            Some("C0RAND") => {
                json_text_reply(&json!({ "channel": { "id": "C0RAND", "name": "random" } }))
            }
            _ => json!({ "content": [{ "type": "text", "text": "channel_not_found" }], "isError": true }),
        };
    }
    // The search tools are decoys: the client must prefer the exact lookup
    // tool names, so a search call means that preference failed.
    if tool.contains("search") {
        return json!({ "content": [{
            "type": "text", "text": format!("unexpected search tool call: {tool}")
        }], "isError": true });
    }
    json!({ "content": [{ "type": "text", "text": format!("unknown tool: {tool}") }], "isError": true })
}

/// Canned Slack MCP reply for one JSON-RPC request (`POST /mcp`).
fn route_slack_mcp(req: &RecordedRequest, state: &MockSlackMcpState) -> Reply {
    if req.method != "POST" || req.path != "/mcp" {
        return Reply::NotFound;
    }
    if let Some((code, detail)) = lock(&state.call_error).clone() {
        return Reply::Status(code, json!({ "detail": detail }).to_string());
    }
    let id = req.body.get("id").cloned().unwrap_or(Value::Null);
    let method = req.body.get("method").and_then(Value::as_str).unwrap_or("");
    let rpc = |result: Value| json!({ "jsonrpc": "2.0", "id": id, "result": result });
    match method {
        "initialize" => Reply::JsonWithHeaders(
            rpc(json!({
                "protocolVersion": req
                    .body
                    .pointer("/params/protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or("2025-06-18"),
                "capabilities": {},
                "serverInfo": { "name": "mock-slack-mcp", "version": "0" }
            })),
            vec![("Mcp-Session-Id".to_string(), "mock-session-1".to_string())],
        ),
        // The handshake completes with a notification; the server accepts it
        // and answers nothing.
        "notifications/initialized" => Reply::Status(202, String::new()),
        "tools/list" => Reply::Json(rpc(json!({ "tools": lock(&state.tools).clone() }))),
        "tools/call" => {
            let tool = req
                .body
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let arguments = req
                .body
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let result = if let Some(reply) = lock(&state.tool_replies).get(tool).cloned() {
                reply
            } else if let Some(reply) = lock(&state.tool_reply).clone() {
                reply
            } else {
                default_tool_result(state, tool, &arguments)
            };
            let reply = Reply::Json(rpc(result));
            if state.sse.load(Ordering::SeqCst) {
                // The same message, framed as an SSE `data:` line.
                match reply {
                    Reply::Json(body) => Reply::Sse(format!("data: {body}\n\n")),
                    other => other,
                }
            } else {
                reply
            }
        }
        _ => Reply::Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("method not found: {method}") }
        })),
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
    /// Per-tool-name reply overrides (checked before `tool_reply`).
    tool_replies: Mutex<HashMap<String, Value>>,
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
            tool_replies: Mutex::new(HashMap::new()),
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

    /// Serve this exact JSON document for one tool's calls only (takes
    /// precedence over [`set_tool_reply`](Self::set_tool_reply)); used to
    /// give the read tools (`slack_read_user_profile`, …) their own replies.
    pub fn set_tool_reply_for(&self, tool: &str, reply: Value) {
        lock(&self.state.tool_replies).insert(tool.to_string(), reply);
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

/// The default connector tool catalog, shaped like the real `slack`
/// connector: two read tools plus `slack_send_message` declaring
/// `channel_id`/`message`/`thread_ts` (and extra optional properties).
fn default_mistral_tools() -> Vec<Value> {
    json!([
        { "name": "slack_search_messages", "inputSchema": { "type": "object" } },
        { "name": "slack_list_channels", "inputSchema": { "type": "object" } },
        { "name": "slack_send_message", "inputSchema": { "type": "object", "properties": {
            "channel_id": { "type": "string", "description": "Search all channels" },
            "message": { "type": "string", "description": "Add a message" },
            "thread_ts": { "type": "string", "description": "Provide another message's ts value to make this message a reply" },
            "reply_broadcast": { "type": "boolean", "description": "Also send to conversation" }
        }, "required": ["channel_id", "message"] } }
    ])
    .as_array()
    .expect("default tool catalog is an array")
    .clone()
}

/// The default tool-call reply, shaped like the real slack connector's: a
/// JSON string inside a text block (no MCP structured content).
fn default_tool_reply() -> Value {
    json!({
        "content": [{ "type": "text", "text":
            "{\"message_link\":\"https://mock.slack.com/archives/CGENERAL/p1700000000000100\",\"message_context\":{\"message_ts\":\"1700000000.000100\",\"channel_id\":\"CGENERAL\"}}"
        }]
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
        // segments = [v1, connectors, <connector>, tools, <tool>, call]
        if let Some(tool) = segments.get(4) {
            if let Some(reply) = lock(&state.tool_replies).get(*tool).cloned() {
                return Reply::Json(reply);
            }
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
    requests: Mutex<Vec<MessageRequest>>,
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
            requests: Mutex::new(Vec::new()),
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

    /// Every request the provider was asked about, in call order (includes
    /// the gateway-assigned ID and the resolved destination display name).
    pub fn requests(&self) -> Vec<MessageRequest> {
        lock(&self.requests).clone()
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
        lock(&self.requests).push(request.clone());
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
