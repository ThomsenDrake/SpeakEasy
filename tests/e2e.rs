//! End-to-end tests: JSON-RPC over a Unix socket pair -> `mcp::run` ->
//! `Gateway` (scripted human decisions) -> real `UreqSlackClient` -> mock
//! Slack Web API.

mod common;

use common::{mock_channel_id, temp_state_dir, MockSlackServer, ScriptedProvider, TEST_TOKEN};
use serde_json::{json, Value};
use slack_approval_gateway::core::{
    Decision, DecisionProvider, Gateway, MessageRequest, ProviderError, COOLDOWN_SECS,
};
use slack_approval_gateway::mcp::{self, TOOL_NAME};
use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::ops::Deref;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Upper bound on waiting for any single response, so a broken server fails
/// the test instead of hanging it.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

const COOLDOWN_NOTE: &str =
    "a similar request was denied recently; nothing was sent and no approval prompt was shown";

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Start a mock Slack API. If a proxy is configured, first make sure
/// requests to the mock bypass it (the client reads the proxy variables when
/// it is built). Every test calls this before building any client, so the
/// one-time environment change happens before anything reads it.
fn mock_slack() -> MockSlackServer {
    static BYPASS_PROXY: Once = Once::new();
    BYPASS_PROXY.call_once(|| {
        let proxy_vars = [
            "ALL_PROXY",
            "all_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
        ];
        if proxy_vars.iter().any(|v| std::env::var_os(v).is_some()) {
            for name in ["NO_PROXY", "no_proxy"] {
                let value = match std::env::var(name) {
                    Ok(v) if !v.trim().is_empty() => format!("{v},127.0.0.1,localhost"),
                    _ => "127.0.0.1,localhost".to_string(),
                };
                std::env::set_var(name, value);
            }
        }
    });
    MockSlackServer::start()
}

/// A gateway that sends through a real client pointed at `mock`.
fn gateway(
    mock: &MockSlackServer,
    provider: Arc<dyn DecisionProvider>,
    dir: &Path,
) -> Arc<Gateway> {
    let gateway =
        Gateway::new(Arc::new(mock.client()), provider, dir.to_path_buf()).expect("create gateway");
    Arc::new(gateway)
}

/// A fresh gateway state directory, removed when the test ends.
struct StateDir(PathBuf);

impl StateDir {
    fn new(tag: &str) -> Self {
        Self(temp_state_dir(tag))
    }
}

impl Deref for StateDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Valid `request_slack_message` arguments.
fn args(message: &str, destination: &str, thread_ts: Option<&str>) -> Value {
    let mut args = json!({
        "message": message,
        "destination": destination,
        "reason": "Tell the team about the release",
        "agent": "release-agent",
        "conversation": {
            "title": "Ship v2.4.0",
            "id_or_link": "https://agents.example.com/conversations/abc123",
            "summary": "Cut and deployed release v2.4.0; the user asked to announce it.",
        },
    });
    if let Some(ts) = thread_ts {
        args["thread_ts"] = json!(ts);
    }
    args
}

/// The status object of a `tools/call` result, after checking that the text
/// content and `structuredContent` carry the same object and that `isError`
/// is set exactly for `error`.
fn tool_payload(result: &Value) -> Value {
    let content = result["content"]
        .as_array()
        .expect("result.content is an array");
    assert_eq!(content.len(), 1, "one content item expected: {result}");
    assert_eq!(content[0]["type"], "text", "{result}");
    let text = content[0]["text"].as_str().expect("text content");
    let payload: Value = serde_json::from_str(text).expect("text content is a JSON object");
    assert_eq!(
        result["structuredContent"], payload,
        "structuredContent mirrors the text: {result}"
    );
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    assert_eq!(
        is_error,
        payload["status"] == "error",
        "isError only for errors: {result}"
    );
    payload
}

/// The test's end of an in-process MCP connection; `mcp::run` serves the
/// other end on a background thread.
struct McpClient {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    server: Option<JoinHandle<std::io::Result<()>>>,
    next_id: u64,
}

impl McpClient {
    fn start(gateway: Arc<Gateway>) -> Self {
        let (client, server) = UnixStream::pair().expect("socket pair");
        let server_input = BufReader::new(server.try_clone().expect("clone server end"));
        let handle = std::thread::spawn(move || mcp::run(gateway, server_input, server));
        client
            .set_read_timeout(Some(RESPONSE_TIMEOUT))
            .expect("set read timeout");
        let reader = BufReader::new(client.try_clone().expect("clone client end"));
        Self {
            writer: client,
            reader,
            server: Some(handle),
            next_id: 1,
        }
    }

    fn send_raw(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).expect("write to server");
        self.writer.flush().expect("flush to server");
    }

    fn send(&mut self, message: &Value) {
        let mut line = message.to_string().into_bytes();
        line.push(b'\n');
        self.send_raw(&line);
    }

    /// Send a request without waiting for its response; returns its id.
    fn send_request(&mut self, method: &str, params: Value) -> Value {
        let id = json!(self.next_id);
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        id
    }

    /// Read the next response line.
    fn recv(&mut self) -> Value {
        let mut line = String::new();
        let read = self
            .reader
            .read_line(&mut line)
            .expect("read a response line (timed out?)");
        assert!(read > 0, "server closed the stream instead of responding");
        // Some responses are huge; keep failure messages readable.
        let excerpt: String = line.chars().take(300).collect();
        assert!(
            line.ends_with('\n'),
            "responses are newline-terminated: {excerpt:?}"
        );
        let response: Value = serde_json::from_str(&line)
            .unwrap_or_else(|e| panic!("response is not JSON ({e}): {excerpt:?}"));
        assert_eq!(response["jsonrpc"], "2.0", "{response}");
        response
    }

    /// Send a request and return its response (which must carry its id).
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.send_request(method, params);
        let response = self.recv();
        assert_eq!(
            response["id"], id,
            "response to the wrong request: {response}"
        );
        response
    }

    /// Send a request that must succeed and return its `result`.
    fn result(&mut self, method: &str, params: Value) -> Value {
        let response = self.request(method, params);
        assert!(
            response.get("error").is_none(),
            "unexpected error: {response}"
        );
        response["result"].clone()
    }

    /// Send a request that must fail and return its `error` object.
    fn error(&mut self, method: &str, params: Value) -> Value {
        let response = self.request(method, params);
        assert!(
            response.get("result").is_none(),
            "expected an error: {response}"
        );
        response["error"].clone()
    }

    fn initialize(&mut self) -> Value {
        let result = self.result(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "e2e-test", "version": "0" },
            }),
        );
        self.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        result
    }

    /// Call `request_slack_message` and return the status object.
    fn call_tool(&mut self, arguments: Value) -> Value {
        let result = self.result(
            "tools/call",
            json!({ "name": TOOL_NAME, "arguments": arguments }),
        );
        tool_payload(&result)
    }

    /// Close the input (EOF for the server) and check that the server wrote
    /// nothing more, closed its end, and `mcp::run` returned `Ok(())`.
    fn finish(mut self) {
        self.writer
            .shutdown(Shutdown::Write)
            .expect("shut down client writes");
        let mut rest = String::new();
        self.reader
            .read_to_string(&mut rest)
            .expect("server closes its end after EOF");
        assert!(
            rest.is_empty(),
            "unexpected output after the last response: {rest:?}"
        );
        let handle = self.server.take().expect("server thread");
        let deadline = Instant::now() + RESPONSE_TIMEOUT;
        while !handle.is_finished() {
            assert!(
                Instant::now() < deadline,
                "mcp::run did not return after EOF"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        handle
            .join()
            .expect("mcp::run must not panic")
            .expect("mcp::run returns Ok(()) on EOF");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn initialize_and_tools_list_describe_the_single_tool() {
    let mock = mock_slack();
    let provider = Arc::new(ScriptedProvider::new([]));
    let dir = StateDir::new("e2e-initialize");
    let mut client = McpClient::start(gateway(&mock, provider.clone(), &dir));

    // The client's protocol version is echoed back.
    let init = client.result(
        "initialize",
        json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "e2e-test", "version": "0" },
        }),
    );
    assert_eq!(init["protocolVersion"], "2025-03-26");
    assert_eq!(
        init["capabilities"],
        json!({ "tools": { "listChanged": false } })
    );
    assert_eq!(init["serverInfo"]["name"], "slack-approval-gateway");
    assert_eq!(init["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));

    // Notifications get no response: the next line answers the ping.
    client.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    assert_eq!(client.result("ping", json!({})), json!({}));

    // Without a requested version, the server's default is used.
    let init = client.result("initialize", json!({ "capabilities": {} }));
    assert_eq!(init["protocolVersion"], "2025-06-18");

    let list = client.result("tools/list", json!({}));
    let tools = list["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 1, "exactly one tool: {list}");
    let tool = &tools[0];
    assert_eq!(tool["name"], TOOL_NAME);
    assert_eq!(tool["name"], "request_slack_message");
    let description = tool["description"].as_str().expect("description");
    assert!(description.contains("approv"), "{description}");
    assert!(description.contains("10 minutes"), "{description}");

    let schema = &tool["inputSchema"];
    assert_eq!(schema["type"], "object");
    let required: BTreeSet<&str> = schema["required"]
        .as_array()
        .expect("required array")
        .iter()
        .map(|v| v.as_str().expect("required entries are strings"))
        .collect();
    assert_eq!(
        required,
        BTreeSet::from(["message", "destination", "reason", "agent", "conversation"])
    );
    let properties = schema["properties"].as_object().expect("properties");
    let names: BTreeSet<&str> = properties.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        BTreeSet::from([
            "message",
            "destination",
            "thread_ts",
            "reason",
            "agent",
            "conversation"
        ])
    );
    for field in ["message", "destination", "thread_ts", "reason", "agent"] {
        assert_eq!(properties[field]["type"], "string", "{field}");
        assert!(
            properties[field]["description"].is_string(),
            "{field} is described"
        );
    }

    let conversation = &properties["conversation"];
    assert_eq!(conversation["type"], "object");
    assert_eq!(
        conversation["required"],
        json!(["title", "id_or_link", "summary"])
    );
    for field in ["title", "id_or_link", "summary"] {
        assert_eq!(
            conversation["properties"][field]["type"], "string",
            "conversation.{field}"
        );
    }

    client.finish();
    assert_eq!(provider.call_count(), 0);
    assert!(mock.requests().is_empty());
}

#[test]
fn approve_sends_exact_message_to_exact_destination_and_thread() {
    let mock = mock_slack();
    let provider = Arc::new(ScriptedProvider::new([Decision::Approve]));
    let dir = StateDir::new("e2e-approve");
    let mut client = McpClient::start(gateway(&mock, provider.clone(), &dir));
    client.initialize();

    // Leading/trailing whitespace, newlines, mrkdwn, and non-ASCII all
    // survive verbatim.
    let message = "  Release *v2.4.0* is live :rocket:\n\n• <https://example.com/notes|Notes> — ünïcödé \"quoted\" & <@U123>  ";
    let payload = client.call_tool(args(message, "#deploys", Some("1699999999.000200")));

    let channel = mock_channel_id("#deploys");
    assert_eq!(channel, "CDEPLOYS");
    assert_eq!(
        payload,
        json!({
            "status": "sent",
            "ts": "1700000000.000001",
            "permalink": format!("https://mock.slack.com/archives/{channel}/p1700000000000001"),
            "duplicate": false,
        })
    );

    let posts = mock.requests_to("/chat.postMessage");
    assert_eq!(posts.len(), 1);
    assert_eq!(
        posts[0].authorization.as_deref(),
        Some(format!("Bearer {TEST_TOKEN}").as_str())
    );
    assert_eq!(
        posts[0].body,
        json!({ "channel": "#deploys", "text": message, "thread_ts": "1699999999.000200" })
    );
    let permalinks = mock.requests_to("/chat.getPermalink");
    assert_eq!(permalinks.len(), 1);
    assert_eq!(
        permalinks[0].body_str("message_ts"),
        Some("1700000000.000001")
    );
    assert_eq!(provider.seen_messages(), vec![message.to_string()]);

    client.finish();
}

#[test]
fn deny_blocks_reworded_retry_to_same_destination_without_prompt() {
    let mock = mock_slack();
    let provider = Arc::new(ScriptedProvider::new([Decision::Deny]));
    let dir = StateDir::new("e2e-deny");
    let mut client = McpClient::start(gateway(&mock, provider.clone(), &dir));
    client.initialize();

    let original =
        "Heads up: the staging database will be down for maintenance tonight from 10pm to 11pm.";
    let payload = client.call_tool(args(original, "#eng", None));
    assert_eq!(
        payload,
        json!({ "status": "denied", "cooldown_remaining_secs": 600 })
    );
    assert_eq!(COOLDOWN_SECS, 600);

    // Reworded, from another agent with another reason and conversation.
    let reworded = "Heads up! The staging database will be down for maintenance tonight, from 10pm until 11pm.";
    let mut retry = args(reworded, "#eng", None);
    retry["agent"] = json!("another-agent");
    retry["reason"] = json!("Different justification for the same announcement");
    retry["conversation"]["id_or_link"] = json!("conv-999");
    let payload = client.call_tool(retry);
    assert_eq!(payload["status"], "cooldown_blocked", "{payload}");
    assert_eq!(payload["note"], COOLDOWN_NOTE);
    let remaining = payload["cooldown_remaining_secs"]
        .as_u64()
        .expect("remaining secs");
    assert!((590..=600).contains(&remaining), "remaining {remaining}");
    assert_eq!(payload.as_object().unwrap().len(), 3, "{payload}");

    // No popup for the blocked retry, and nothing reached Slack.
    assert_eq!(provider.call_count(), 1);
    assert_eq!(provider.seen_messages(), vec![original.to_string()]);
    assert_eq!(mock.post_count(), 0);

    // The cooldown is per destination: the same text elsewhere is asked about.
    provider.push(Decision::Feedback("Not to #random either.".into()));
    let payload = client.call_tool(args(original, "#random", None));
    assert_eq!(
        payload,
        json!({ "status": "feedback", "feedback": "Not to #random either." })
    );
    assert_eq!(provider.call_count(), 2);
    assert!(mock.requests().is_empty());

    client.finish();
}

#[test]
fn feedback_is_returned_without_sending_and_revision_needs_new_approval() {
    let mock = mock_slack();
    let feedback = "Please drop the customer's name and link the incident ticket instead.";
    let provider = Arc::new(ScriptedProvider::new([Decision::Feedback(feedback.into())]));
    let dir = StateDir::new("e2e-feedback");
    let mut client = McpClient::start(gateway(&mock, provider.clone(), &dir));
    client.initialize();

    let draft = "Acme Corp hit the checkout outage at 14:05 UTC; we are rolling back now.";
    let payload = client.call_tool(args(draft, "#incidents", None));
    assert_eq!(
        payload,
        json!({ "status": "feedback", "feedback": feedback })
    );
    assert_eq!(mock.post_count(), 0);
    assert!(
        mock.requests().is_empty(),
        "no Slack call at all: {:?}",
        mock.requests()
    );

    // Feedback starts no cooldown; the revision is a new request that needs
    // (and gets) its own approval.
    provider.push(Decision::Approve);
    let revised =
        "A customer hit the checkout outage at 14:05 UTC (INC-4211); we are rolling back now.";
    let payload = client.call_tool(args(revised, "#incidents", None));
    assert_eq!(payload["status"], "sent", "{payload}");
    assert_eq!(payload["duplicate"], false);
    assert_eq!(
        provider.seen_messages(),
        vec![draft.to_string(), revised.to_string()]
    );
    let posts = mock.requests_to("/chat.postMessage");
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].body_str("text"), Some(revised));

    client.finish();
}

#[test]
fn identical_retry_after_approval_is_a_duplicate_and_not_resent() {
    let mock = mock_slack();
    let provider = Arc::new(ScriptedProvider::new([Decision::Approve]));
    let dir = StateDir::new("e2e-duplicate");
    let mut client = McpClient::start(gateway(&mock, provider.clone(), &dir));
    client.initialize();

    let message = "Weekly metrics are posted in the dashboard.";
    let first = client.call_tool(args(message, "C0METRICS", Some("1700000500.000100")));
    assert_eq!(first["status"], "sent", "{first}");
    assert_eq!(first["duplicate"], false);
    assert_eq!(mock.post_count(), 1);

    // The identical request (even from another agent) returns the cached
    // result: no popup, no second post.
    let mut again = args(message, "C0METRICS", Some("1700000500.000100"));
    again["agent"] = json!("retrying-agent");
    let second = client.call_tool(again);
    assert_eq!(
        second,
        json!({
            "status": "sent",
            "ts": first["ts"],
            "permalink": first["permalink"],
            "duplicate": true,
        })
    );
    assert_eq!(mock.post_count(), 1);
    assert_eq!(provider.call_count(), 1);

    // Approval covers only that exact request: another thread is a new
    // request that needs a fresh decision (none is scripted, so it fails
    // and nothing is sent).
    let other_thread = client.call_tool(args(message, "C0METRICS", Some("1700000600.000100")));
    assert_eq!(other_thread["status"], "error", "{other_thread}");
    assert!(other_thread["error"].is_string(), "{other_thread}");
    assert_eq!(provider.call_count(), 2);
    assert_eq!(mock.post_count(), 1);

    client.finish();
}

#[test]
fn concurrent_requests_get_their_own_decisions() {
    let mock = mock_slack();
    let provider = Arc::new(
        ScriptedProvider::new([])
            .when_message_contains("ALPHA", Decision::Approve)
            .when_message_contains(
                "BRAVO",
                Decision::Feedback("BRAVO needs a softer tone".into()),
            )
            .when_message_contains("CHARLIE", Decision::Deny)
            .with_delay(Duration::from_millis(500)),
    );
    let dir = StateDir::new("e2e-concurrent");
    let mut client = McpClient::start(gateway(&mock, provider.clone(), &dir));
    client.initialize();

    let alpha = "ALPHA: the new onboarding flow ships Monday.";
    let bravo = "BRAVO: whoever broke the build, fix it now.";
    let charlie = "CHARLIE: lunch-and-learn moved to Thursday.";
    let alpha_id = client.send_request(
        "tools/call",
        json!({ "name": TOOL_NAME, "arguments": args(alpha, "#alpha", None) }),
    );
    let bravo_id = client.send_request(
        "tools/call",
        json!({ "name": TOOL_NAME, "arguments": args(bravo, "#bravo", None) }),
    );
    let charlie_id = client.send_request("tools/call", json!({ "name": TOOL_NAME, "arguments": args(charlie, "#charlie", Some("1700000700.000100")) }));

    // Pending decisions do not block the read loop: the ping is answered
    // while all three requests wait for the (slow) human.
    let ping_id = client.send_request("ping", json!({}));
    let first = client.recv();
    assert_eq!(first["id"], ping_id, "ping answered first: {first}");
    assert_eq!(first["result"], json!({}));

    let mut by_id = HashMap::new();
    for _ in 0..3 {
        let response = client.recv();
        assert!(response.get("error").is_none(), "{response}");
        let id = response["id"].as_u64().expect("numeric id");
        assert!(
            by_id
                .insert(id, tool_payload(&response["result"]))
                .is_none(),
            "one response per id"
        );
    }
    let payload = |id: &Value| by_id[&id.as_u64().unwrap()].clone();

    let alpha_payload = payload(&alpha_id);
    assert_eq!(alpha_payload["status"], "sent", "{alpha_payload}");
    assert_eq!(alpha_payload["duplicate"], false);
    assert_eq!(alpha_payload["ts"], "1700000000.000001");
    assert_eq!(
        alpha_payload["permalink"],
        format!(
            "https://mock.slack.com/archives/{}/p1700000000000001",
            mock_channel_id("#alpha")
        )
    );
    assert_eq!(
        payload(&bravo_id),
        json!({ "status": "feedback", "feedback": "BRAVO needs a softer tone" })
    );
    assert_eq!(
        payload(&charlie_id),
        json!({ "status": "denied", "cooldown_remaining_secs": 600 })
    );

    assert!(
        provider.max_concurrent_calls() >= 2,
        "decisions were requested concurrently (max {})",
        provider.max_concurrent_calls()
    );
    assert_eq!(provider.call_count(), 3);
    assert_eq!(
        provider.remaining(),
        0,
        "each rule was used by its own request"
    );

    // Only the approved request reached Slack, exactly as requested.
    let posts = mock.requests_to("/chat.postMessage");
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].body, json!({ "channel": "#alpha", "text": alpha }));

    client.finish();
}

#[test]
fn deny_cooldown_survives_restart() {
    let mock = mock_slack();
    let dir = StateDir::new("e2e-restart-cooldown");
    let denied = "Reminder: please submit your expense reports by Friday.";

    let first_provider = Arc::new(ScriptedProvider::new([Decision::Deny]));
    let mut client = McpClient::start(gateway(&mock, first_provider.clone(), &dir));
    client.initialize();
    let payload = client.call_tool(args(denied, "#finance", None));
    assert_eq!(
        payload,
        json!({ "status": "denied", "cooldown_remaining_secs": 600 })
    );
    client.finish();
    assert!(dir.join("cooldowns.json").exists(), "cooldown persisted");

    // Simulated restart: a new gateway (and provider) on the same state dir.
    let provider = Arc::new(ScriptedProvider::new([]));
    let mut client = McpClient::start(gateway(&mock, provider.clone(), &dir));
    client.initialize();
    let mut retry = args(
        "Reminder: please submit your expense reports by this Friday!",
        "#finance",
        None,
    );
    retry["agent"] = json!("restarted-agent");
    let payload = client.call_tool(retry);
    assert_eq!(payload["status"], "cooldown_blocked", "{payload}");
    assert_eq!(payload["note"], COOLDOWN_NOTE);
    let remaining = payload["cooldown_remaining_secs"]
        .as_u64()
        .expect("remaining secs");
    assert!((590..=600).contains(&remaining), "remaining {remaining}");

    assert_eq!(provider.call_count(), 0, "no popup after restart");
    assert_eq!(mock.post_count(), 0);
    client.finish();
}

#[test]
fn approved_send_is_deduplicated_after_restart() {
    let mock = mock_slack();
    let dir = StateDir::new("e2e-restart-sent");
    let message = "Release notes for v2.4.0 are up.";

    let first_provider = Arc::new(ScriptedProvider::new([Decision::Approve]));
    let mut client = McpClient::start(gateway(&mock, first_provider.clone(), &dir));
    let first = client.call_tool(args(message, "#releases", None));
    assert_eq!(first["status"], "sent", "{first}");
    client.finish();

    let provider = Arc::new(ScriptedProvider::new([]));
    let mut client = McpClient::start(gateway(&mock, provider.clone(), &dir));
    let again = client.call_tool(args(message, "#releases", None));
    assert_eq!(
        again,
        json!({ "status": "sent", "ts": first["ts"], "permalink": first["permalink"], "duplicate": true })
    );
    assert_eq!(provider.call_count(), 0);
    assert_eq!(mock.post_count(), 1);
    client.finish();
}

#[test]
fn slack_failure_is_a_tool_error_and_records_nothing() {
    let mock = mock_slack();
    let provider = Arc::new(ScriptedProvider::new([
        Decision::Approve,
        Decision::Approve,
    ]));
    let dir = StateDir::new("e2e-slack-error");
    let mut client = McpClient::start(gateway(&mock, provider.clone(), &dir));
    client.initialize();

    mock.set_post_message_error(Some("channel_not_found"));
    let message = "Standup moved to 10:30 today.";
    let result = client.result(
        "tools/call",
        json!({ "name": TOOL_NAME, "arguments": args(message, "#nope", None) }),
    );
    assert_eq!(result["isError"], true, "{result}");
    let payload = tool_payload(&result);
    assert_eq!(payload["status"], "error");
    let error = payload["error"].as_str().expect("error text");
    assert!(error.contains("channel_not_found"), "{error}");

    // Nothing was recorded as sent: a retry asks again and sends.
    mock.set_post_message_error(None);
    let payload = client.call_tool(args(message, "#nope", None));
    assert_eq!(payload["status"], "sent", "{payload}");
    assert_eq!(payload["duplicate"], false);
    assert_eq!(provider.call_count(), 2);
    assert_eq!(mock.post_count(), 2);

    client.finish();
}

#[test]
fn invalid_params_and_protocol_errors() {
    let mock = mock_slack();
    let provider = Arc::new(ScriptedProvider::new([]));
    let dir = StateDir::new("e2e-invalid");
    let mut client = McpClient::start(gateway(&mock, provider.clone(), &dir));
    client.initialize();

    let call = |arguments: Value| json!({ "name": TOOL_NAME, "arguments": arguments });
    let message = "Valid message text.";

    // Missing required argument.
    let mut missing_reason = args(message, "#general", None);
    missing_reason.as_object_mut().unwrap().remove("reason");
    let error = client.error("tools/call", call(missing_reason));
    assert_eq!(error["code"], -32602, "{error}");
    assert!(
        error["message"].as_str().unwrap().contains("reason"),
        "{error}"
    );

    // Unknown tool.
    let error = client.error(
        "tools/call",
        json!({ "name": "send_slack_message", "arguments": args(message, "#general", None) }),
    );
    assert_eq!(error["code"], -32602, "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("send_slack_message"),
        "{error}"
    );

    // Wrongly typed and malformed arguments.
    let mut wrong_type = args(message, "#general", None);
    wrong_type["thread_ts"] = json!(1700000000.0001);
    let mut missing_summary = args(message, "#general", None);
    missing_summary["conversation"]
        .as_object_mut()
        .unwrap()
        .remove("summary");
    let mut conversation_string = args(message, "#general", None);
    conversation_string["conversation"] = json!("a conversation");
    let mut message_number = args(message, "#general", None);
    message_number["message"] = json!(42);
    let mut blank_agent = args(message, "#general", None);
    blank_agent["agent"] = json!("   ");
    let mut link_as_thread = args(message, "#general", None);
    link_as_thread["thread_ts"] = json!("https://acme.slack.com/archives/C1/p1700000000000100");
    let mut misspelled_field = args(message, "#general", None);
    misspelled_field["thread"] = json!("1700000000.000100");
    let cases = [
        (wrong_type, "thread_ts"),
        (missing_summary, "conversation.summary"),
        (conversation_string, "conversation"),
        (message_number, "message"),
        (blank_agent, "agent"),
        (link_as_thread, "thread_ts"),
        (misspelled_field, "thread"),
        (json!("not an object"), "arguments"),
    ];
    for (arguments, mentioned) in cases {
        let error = client.error("tools/call", call(arguments.clone()));
        assert_eq!(error["code"], -32602, "{arguments}: {error}");
        let text = error["message"].as_str().unwrap();
        assert!(text.contains(mentioned), "{arguments}: {error}");
    }
    let error = client.error("tools/call", json!({ "name": TOOL_NAME }));
    assert_eq!(error["code"], -32602, "{error}");

    // Unknown method.
    let error = client.error("resources/list", json!({}));
    assert_eq!(error["code"], -32601, "{error}");

    // Malformed JSON and invalid UTF-8: parse errors with a null id.
    client.send_raw(b"{\"jsonrpc\":\"2.0\",\"id\":77,\"method\":\n");
    let response = client.recv();
    assert_eq!(response["id"], Value::Null, "{response}");
    assert_eq!(response["error"]["code"], -32700, "{response}");
    client.send_raw(b"\xff\xfe\n");
    let response = client.recv();
    assert_eq!(response["id"], Value::Null, "{response}");
    assert_eq!(response["error"]["code"], -32700, "{response}");

    // Not an object (batches are not supported), and a request without id.
    client.send_raw(b"[{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"ping\"}]\n");
    let response = client.recv();
    assert_eq!(response["id"], Value::Null, "{response}");
    assert_eq!(response["error"]["code"], -32600, "{response}");
    client.send(&json!({ "jsonrpc": "2.0", "method": "tools/list" }));
    let response = client.recv();
    assert_eq!(response["id"], Value::Null, "{response}");
    assert_eq!(response["error"]["code"], -32600, "{response}");

    // Unknown notifications and blank lines get no response; string ids are
    // echoed verbatim.
    client.send(&json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 1 } }));
    client.send_raw(b"\n  \r\n");
    client.send(&json!({ "jsonrpc": "2.0", "id": "ping-1", "method": "ping" }));
    let response = client.recv();
    assert_eq!(
        response,
        json!({ "jsonrpc": "2.0", "id": "ping-1", "result": {} })
    );

    // None of that reached the human or Slack, and the server still works.
    assert_eq!(provider.call_count(), 0);
    assert!(mock.requests().is_empty());
    provider.push(Decision::Feedback("ok".into()));
    let payload = client.call_tool(args(message, "#general", None));
    assert_eq!(payload, json!({ "status": "feedback", "feedback": "ok" }));
    assert_eq!(provider.call_count(), 1);

    client.finish();
}

/// Records every request it is asked about and answers with feedback, so
/// nothing is ever sent.
struct RecordingProvider {
    seen: Mutex<Vec<MessageRequest>>,
}

impl DecisionProvider for RecordingProvider {
    fn request_decision(&self, request: &MessageRequest) -> Result<Decision, ProviderError> {
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(request.clone());
        Ok(Decision::Feedback("recorded".into()))
    }
}

#[test]
fn every_argument_reaches_the_human_verbatim() {
    let mock = mock_slack();
    let provider = Arc::new(RecordingProvider {
        seen: Mutex::new(Vec::new()),
    });
    let dir = StateDir::new("e2e-fields");
    let mut client = McpClient::start(gateway(&mock, provider.clone(), &dir));
    client.initialize();

    let message = "  Exact text\nwith a second line  ";
    let reason = "  Dana asked for the rollout status ";
    let summary = "Phase 2 finished; Dana wants a heads-up.\nNo customer impact.";
    let arguments = json!({
        "message": message,
        "destination": "@dana",
        "thread_ts": "1700000800.000300",
        "reason": reason,
        "agent": "rollout-agent/7",
        "conversation": {
            "title": "Rollout — phase 2",
            "id_or_link": "https://agents.example.com/c/42",
            "summary": summary,
        },
    });
    let payload = client.call_tool(arguments.clone());
    assert_eq!(
        payload,
        json!({ "status": "feedback", "feedback": "recorded" })
    );

    // An omitted or null `thread_ts` both mean "no thread".
    let mut omitted = arguments.clone();
    omitted.as_object_mut().unwrap().remove("thread_ts");
    client.call_tool(omitted);
    let mut null_thread = arguments.clone();
    null_thread["thread_ts"] = Value::Null;
    client.call_tool(null_thread);

    let seen = provider.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 3);
    let first = &seen[0].incoming;
    assert_eq!(first.message, message);
    assert_eq!(first.destination, "@dana");
    assert_eq!(first.thread_ts.as_deref(), Some("1700000800.000300"));
    assert_eq!(first.reason, reason);
    assert_eq!(first.agent, "rollout-agent/7");
    assert_eq!(first.conversation.title, "Rollout — phase 2");
    assert_eq!(
        first.conversation.id_or_link,
        "https://agents.example.com/c/42"
    );
    assert_eq!(first.conversation.summary, summary);
    assert_eq!(seen[1].incoming.thread_ts, None);
    assert_eq!(seen[2].incoming.thread_ts, None);
    let ids: BTreeSet<&str> = seen.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids.len(), 3, "each request gets its own ID");
    assert!(mock.requests().is_empty());

    client.finish();
}

/// Panics instead of deciding, standing in for a bug in the gateway.
struct PanickingProvider;

impl DecisionProvider for PanickingProvider {
    fn request_decision(&self, _request: &MessageRequest) -> Result<Decision, ProviderError> {
        panic!("simulated gateway bug");
    }
}

#[test]
fn panic_while_handling_a_call_is_a_tool_error_and_the_server_keeps_running() {
    let mock = mock_slack();
    let dir = StateDir::new("e2e-panic");
    let mut client = McpClient::start(gateway(&mock, Arc::new(PanickingProvider), &dir));
    client.initialize();

    let result = client.result(
        "tools/call",
        json!({ "name": TOOL_NAME, "arguments": args("Panic test.", "#general", None) }),
    );
    assert_eq!(result["isError"], true, "{result}");
    let payload = tool_payload(&result);
    assert_eq!(payload["status"], "error");
    let error = payload["error"].as_str().expect("error text");
    assert!(error.contains("internal gateway error"), "{error}");

    assert_eq!(client.result("ping", json!({})), json!({}));
    assert_eq!(mock.post_count(), 0);
    client.finish();
}

#[test]
fn concurrent_large_responses_are_written_whole() {
    const CALLS: usize = 16;
    let mock = mock_slack();
    // Each response line is far larger than the socket buffer, so it takes
    // many writes; workers that did not serialize whole lines would
    // interleave them.
    let feedback = |i: usize| {
        format!(
            "{i:02}:{}",
            char::from(b'a' + i as u8).to_string().repeat(64 * 1024)
        )
    };
    let mut scripted = ScriptedProvider::new([]);
    for i in 0..CALLS {
        scripted = scripted
            .when_message_contains(format!("[REQ-{i:02}]"), Decision::Feedback(feedback(i)));
    }
    let provider = Arc::new(scripted.with_delay(Duration::from_millis(200)));
    let dir = StateDir::new("e2e-large");
    let mut client = McpClient::start(gateway(&mock, provider.clone(), &dir));
    client.initialize();

    let mut expected = HashMap::new();
    for i in 0..CALLS {
        let message = format!("[REQ-{i:02}] nightly report is ready");
        let id = client.send_request(
            "tools/call",
            json!({ "name": TOOL_NAME, "arguments": args(&message, "#reports", None) }),
        );
        expected.insert(id.as_u64().unwrap(), feedback(i));
    }
    for _ in 0..CALLS {
        // `recv` fails on any line that is not one whole JSON message.
        let response = client.recv();
        let id = response["id"].as_u64().expect("numeric id");
        let want = expected.remove(&id).expect("one response per request");
        let payload = tool_payload(&response["result"]);
        assert!(
            payload == json!({ "status": "feedback", "feedback": want }),
            "response {id} does not carry its own feedback"
        );
    }
    assert!(provider.max_concurrent_calls() >= 2);
    assert!(mock.requests().is_empty());
    client.finish();
}
