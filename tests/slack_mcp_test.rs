//! Integration tests for `SlackMcpClient` against the local mock Slack MCP
//! server (MCP Streamable HTTP), plus credential handling in `from_env`.

mod common;

use common::{temp_state_dir, MockSlackMcpServer, TEST_SLACK_MCP_TOKEN};
use serde_json::{json, Value};
use slack_approval_gateway::core::SlackClient;
use slack_approval_gateway::slack;
use slack_approval_gateway::slack_mcp::SlackMcpClient;
use std::ffi::OsString;
use std::sync::Arc;

/// A client pointed at `mock`, with the handshake traffic cleared from the
/// mock's recording (the handshake is construction, not test traffic).
fn fresh_client(mock: &MockSlackMcpServer) -> SlackMcpClient {
    let client = mock.client();
    mock.clear_handshake();
    client
}

#[test]
fn handshake_runs_at_construction_and_the_session_id_is_sent_back() {
    let mock = MockSlackMcpServer::start();

    let client = SlackMcpClient::new(TEST_SLACK_MCP_TOKEN, mock.mcp_url())
        .expect("the handshake with the mock must succeed");

    // The handshake is exactly initialize + notifications/initialized, with
    // the bearer token on both.
    assert_eq!(mock.methods(), ["initialize", "notifications/initialized"]);
    for request in mock.requests() {
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/mcp");
        assert_eq!(
            request.authorization.as_deref(),
            Some("Bearer xoxp-slack-mcp-test-token")
        );
        let accept = request.header("accept").unwrap_or("");
        assert!(accept.contains("application/json"), "accept was {accept:?}");
        assert!(
            accept.contains("text/event-stream"),
            "accept was {accept:?}"
        );
    }

    // Later requests carry the session id from the initialize reply and the
    // negotiated protocol version.
    client.post_message("#general", None, "hi").expect("post_message");
    let call = &mock.calls_to("slack_send_message")[0];
    assert_eq!(call.header("mcp-session-id"), Some("mock-session-1"));
    assert_eq!(call.header("mcp-protocol-version"), Some("2025-06-18"));
}

#[test]
fn post_message_discovers_send_tool_and_sends_exact_arguments_with_bearer_token() {
    let mock = MockSlackMcpServer::start();
    let client = fresh_client(&mock);
    let text = "Deploy *v1.2.3* finished :tada:\n  second line — ünïcödé & \"quoted\"  ";

    let sent = client
        .post_message("#general", Some("1699999999.000200"), text)
        .expect("post_message should succeed");

    // Discovery first, then the tool call.
    assert_eq!(mock.methods(), ["tools/list", "tools/call"]);
    let discovery = &mock.requests()[0];
    assert_eq!(discovery.body["method"], "tools/list");
    assert_eq!(
        discovery.authorization.as_deref(),
        Some("Bearer xoxp-slack-mcp-test-token")
    );
    assert!(
        discovery
            .content_type
            .as_deref()
            .unwrap_or("")
            .starts_with("application/json"),
        "content type was {:?}",
        discovery.content_type
    );

    // Whole-params equality: exact values, schema-detected argument names
    // (channel_id/message/thread_ts), and no extra keys.
    let call = &mock.calls_to("slack_send_message")[0];
    assert_eq!(call.method, "POST");
    assert_eq!(call.path, "/mcp");
    assert_eq!(
        call.authorization.as_deref(),
        Some("Bearer xoxp-slack-mcp-test-token")
    );
    assert_eq!(call.body["jsonrpc"], "2.0");
    assert_eq!(call.body["method"], "tools/call");
    assert_eq!(
        call.body["params"],
        json!({ "name": "slack_send_message", "arguments": {
            "channel_id": "#general", "message": text, "thread_ts": "1699999999.000200"
        } })
    );
    assert_eq!(
        call.body
            .pointer("/params/arguments/message")
            .and_then(Value::as_str),
        Some(text),
        "text must be sent verbatim"
    );

    // The outcome comes from the JSON text block; the channel is the
    // resolved one from message_context, like the other transports.
    assert_eq!(sent.channel, "CGENERAL");
    assert_eq!(sent.ts, "1700000000.000001");
    assert_eq!(
        sent.permalink,
        "https://mock.slack.com/archives/CGENERAL/p1700000000000001"
    );
}

#[test]
fn thread_ts_argument_is_absent_when_none() {
    let mock = MockSlackMcpServer::start();
    let client = fresh_client(&mock);

    client
        .post_message("C0123ABC", None, "hello")
        .expect("post_message should succeed");

    let calls = mock.calls_to("slack_send_message");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].body["params"]["arguments"],
        json!({ "channel_id": "C0123ABC", "message": "hello" })
    );
    assert!(
        calls[0].body["params"]["arguments"]
            .as_object()
            .expect("arguments object")
            .get("thread_ts")
            .is_none(),
        "no thread key when there is no thread"
    );
}

#[test]
fn pinned_tool_skips_discovery_and_uses_legacy_argument_names() {
    let mock = MockSlackMcpServer::start();
    let client = mock
        .client()
        .with_tool(Some("slack_send_message".to_string()));
    mock.clear_handshake();

    client
        .post_message("#general", None, "hi")
        .expect("post_message should succeed");

    // No tools/list: the pinned tool is called directly, and without a
    // schema the legacy channel/text names are used.
    assert_eq!(mock.methods(), ["tools/call"]);
    assert_eq!(
        mock.calls_to("slack_send_message")[0].body["params"]["arguments"],
        json!({ "channel": "#general", "text": "hi" })
    );
}

#[test]
fn custom_argument_name_overrides_are_used() {
    let mock = MockSlackMcpServer::start();
    let client = mock
        .client()
        .with_tool(Some("send_slack_message".to_string()))
        .with_arguments(Some("channel_id"), Some("message"), Some("parent_ts"));
    mock.clear_handshake();

    client
        .post_message("C0123ABC", Some("1699999999.000200"), "hello")
        .expect("post_message should succeed");

    let calls = mock.calls_to("send_slack_message");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].body["params"]["arguments"],
        json!({ "channel_id": "C0123ABC", "message": "hello", "parent_ts": "1699999999.000200" })
    );
}

#[test]
fn argument_names_adapt_to_a_differently_shaped_schema() {
    let mock = MockSlackMcpServer::start();
    // A server whose send tool declares channel/content/parent_ts.
    mock.set_tools(
        json!([
            { "name": "post_message", "inputSchema": { "type": "object", "properties": {
                "channel": { "type": "string" },
                "content": { "type": "string" },
                "parent_ts": { "type": "string" }
            } } }
        ])
        .as_array()
        .unwrap()
        .clone(),
    );
    let client = fresh_client(&mock);

    client
        .post_message("#general", Some("1699999999.000200"), "hello")
        .expect("post_message should succeed");

    let calls = mock.calls_to("post_message");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].body["params"]["name"],
        "post_message",
        "a name containing post+message is a send tool"
    );
    assert_eq!(
        calls[0].body["params"]["arguments"],
        json!({ "channel": "#general", "content": "hello", "parent_ts": "1699999999.000200" })
    );
}

#[test]
fn thread_reply_fails_when_the_tool_declares_no_thread_argument() {
    let mock = MockSlackMcpServer::start();
    mock.set_tools(
        json!([
            { "name": "slack_send_message", "inputSchema": { "type": "object", "properties": {
                "channel_id": { "type": "string" }, "message": { "type": "string" }
            }, "required": ["channel_id", "message"] } }
        ])
        .as_array()
        .unwrap()
        .clone(),
    );
    let client = fresh_client(&mock);

    let err = client
        .post_message("#general", Some("1699999999.000200"), "hello")
        .expect_err("a thread reply the tool cannot express must be an error");

    assert!(err.0.contains("no thread argument"), "error was {:?}", err.0);
    assert!(
        mock.calls_to("slack_send_message").is_empty(),
        "nothing must be sent"
    );
}

#[test]
fn permalink_comes_from_text_and_ts_from_the_link_when_structured_content_is_absent() {
    let mock = MockSlackMcpServer::start();
    mock.set_tool_reply(Some(json!({
        "content": [{ "type": "text",
            "text": "Posted to #general: https://acme.slack.com/archives/C1/p1700000000000100 — done" }]
    })));
    let client = fresh_client(&mock);

    let sent = client
        .post_message("#general", None, "hello")
        .expect("post_message should succeed");

    assert_eq!(sent.ts, "1700000000.000100");
    assert_eq!(
        sent.permalink,
        "https://acme.slack.com/archives/C1/p1700000000000100"
    );
}

#[test]
fn missing_outcome_falls_back_to_constructed_permalink() {
    let mock = MockSlackMcpServer::start();
    mock.set_tool_reply(Some(json!({
        "content": [{ "type": "text", "text": "ok" }],
        "structuredContent": { "ts": "1700000000.000042" }
    })));
    let client = fresh_client(&mock);

    let sent = client
        .post_message("C0123ABC", None, "hello")
        .expect("post_message should succeed");

    assert_eq!(sent.ts, "1700000000.000042");
    // The workspace domain comes from SLACK_WORKSPACE_DOMAIN, which the env
    // test below changes, so only the domain-independent parts are checked
    // here.
    assert!(sent.permalink.starts_with("https://"), "{}", sent.permalink);
    assert!(
        sent.permalink
            .ends_with(".slack.com/archives/C0123ABC/p1700000000000042"),
        "{}",
        sent.permalink
    );
}

#[test]
fn tool_is_error_maps_to_slack_error_with_the_tool_text() {
    let mock = MockSlackMcpServer::start();
    mock.set_tool_reply(Some(json!({
        "content": [{ "type": "text", "text": "channel_not_found: no such channel" }],
        "isError": true
    })));
    let client = fresh_client(&mock);

    let err = client
        .post_message("C_NOPE", None, "hello")
        .expect_err("isError must be an error");

    assert!(
        err.0.contains("channel_not_found: no such channel"),
        "error was {:?}",
        err.0
    );
    assert!(
        err.0.contains("slack_send_message"),
        "the failing tool is named: {:?}",
        err.0
    );
    assert!(
        !err.0.contains(TEST_SLACK_MCP_TOKEN),
        "the token must never appear in errors"
    );
}

#[test]
fn send_tool_error_maps_to_slack_error() {
    let mock = MockSlackMcpServer::start();
    mock.set_send_error(Some("channel_not_found"));
    let client = fresh_client(&mock);

    let err = client
        .post_message("C_DOES_NOT_EXIST", None, "hello")
        .expect_err("a failed send must be an error");

    assert!(err.0.contains("channel_not_found"), "error was {:?}", err.0);
    assert_eq!(mock.send_count(), 1);
}

#[test]
fn http_error_maps_to_slack_error_with_detail_and_never_the_token() {
    let mock = MockSlackMcpServer::start();
    let client = fresh_client(&mock);
    mock.set_call_error(Some((
        401,
        "No credentials found. Please authenticate with Slack.",
    )));

    let err = client
        .post_message("#general", None, "hello")
        .expect_err("HTTP 401 must be an error");

    assert!(err.0.contains("401"), "error was {:?}", err.0);
    assert!(
        err.0.contains("No credentials found. Please authenticate with Slack."),
        "error was {:?}",
        err.0
    );
    assert!(
        !err.0.contains(TEST_SLACK_MCP_TOKEN),
        "the token must never appear in errors"
    );
    // Discovery itself was rejected, so no tool was called.
    assert!(mock.calls_to("slack_send_message").is_empty());
}

#[test]
fn handshake_failure_is_a_startup_error_and_never_the_token() {
    let mock = MockSlackMcpServer::start();
    mock.set_call_error(Some((401, "invalid_token")));

    let err = SlackMcpClient::new(TEST_SLACK_MCP_TOKEN, mock.mcp_url())
        .expect_err("a rejected handshake must fail construction");

    assert!(err.0.contains("initialize"), "error was {:?}", err.0);
    assert!(err.0.contains("401"), "error was {:?}", err.0);
    assert!(err.0.contains("invalid_token"), "error was {:?}", err.0);
    assert!(
        !err.0.contains(TEST_SLACK_MCP_TOKEN),
        "the token must never appear in errors"
    );
}

#[test]
fn discovery_without_a_send_tool_lists_the_available_names() {
    let mock = MockSlackMcpServer::start();
    mock.set_tools(
        json!([
            { "name": "slack_search_messages" },
            { "name": "slack_list_channels" }
        ])
        .as_array()
        .unwrap()
        .clone(),
    );
    let client = fresh_client(&mock);

    let err = client
        .post_message("#general", None, "hello")
        .expect_err("no send tool in the catalog must be an error");

    assert!(err.0.contains("no send-message tool"), "error was {:?}", err.0);
    assert!(err.0.contains("slack_search_messages"), "error was {:?}", err.0);
    assert!(err.0.contains("SLACK_MCP_TOOL"), "error was {:?}", err.0);
    assert!(
        mock.calls_to("slack_send_message").is_empty(),
        "no tool call without a tool"
    );
}

#[test]
fn discovery_is_cached_across_calls() {
    let mock = MockSlackMcpServer::start();
    let client = fresh_client(&mock);

    client.post_message("C1", None, "one").expect("first send");
    client.post_message("C2", None, "two").expect("second send");

    assert_eq!(
        mock.methods(),
        ["tools/list", "tools/call", "tools/call"],
        "the tool list is fetched once per client"
    );
    assert_eq!(mock.send_count(), 2);
}

#[test]
fn transport_error_fails_the_handshake() {
    // Grab a free port, then close it so the connection is refused.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let err = SlackMcpClient::new(TEST_SLACK_MCP_TOKEN, format!("http://{addr}/mcp"))
        .expect_err("connection refused must fail the handshake");

    assert!(
        err.0.starts_with("Slack MCP initialize"),
        "error was {:?}",
        err.0
    );
    assert!(
        !err.0.contains(TEST_SLACK_MCP_TOKEN),
        "the token must never appear in errors"
    );
}

#[test]
fn new_strips_trailing_slash_and_debug_redacts_token() {
    let mock = MockSlackMcpServer::start();
    let client = SlackMcpClient::new(TEST_SLACK_MCP_TOKEN, format!("{}/", mock.mcp_url()))
        .expect("handshake");

    client.post_message("C1", None, "hi").expect("post_message");

    assert_eq!(
        mock.requests()[0].path,
        "/mcp",
        "the trailing slash must be stripped"
    );
    let debug = format!("{client:?}");
    assert!(
        !debug.contains(TEST_SLACK_MCP_TOKEN),
        "Debug output leaked the token: {debug}"
    );
}

#[test]
fn sse_replies_are_parsed() {
    let mock = MockSlackMcpServer::start();
    let client = fresh_client(&mock);
    mock.set_sse(true);

    let sent = client
        .post_message("#general", None, "hello via sse")
        .expect("an SSE reply must parse");

    assert_eq!(sent.channel, "CGENERAL");
    assert_eq!(sent.ts, "1700000000.000001");
    assert_eq!(
        sent.permalink,
        "https://mock.slack.com/archives/CGENERAL/p1700000000000001"
    );
    assert_eq!(mock.send_count(), 1);
}

#[test]
fn concurrent_posts_each_get_their_own_result() {
    let mock = MockSlackMcpServer::start();
    let client = Arc::new(mock.client());

    let handles: Vec<_> = (0..8)
        .map(|i| {
            let client = Arc::clone(&client);
            std::thread::spawn(move || {
                let text = format!("message number {i}");
                let sent = client
                    .post_message(&format!("C{i}"), None, &text)
                    .expect("post_message should succeed");
                (i, sent)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    assert_eq!(mock.send_count(), 8);
    let posts = mock.calls_to("slack_send_message");
    for (i, sent) in &results {
        assert_eq!(sent.channel, format!("C{i}"));
        assert_eq!(
            sent.permalink,
            format!("https://mock.slack.com/archives/C{i}/p{}", sent.ts.replace('.', ""))
        );
        let req = posts
            .iter()
            .find(|r| {
                r.body
                    .pointer("/params/arguments/channel_id")
                    .and_then(Value::as_str)
                    == Some(format!("C{i}").as_str())
            })
            .expect("a recorded send for this channel");
        assert_eq!(
            req.body
                .pointer("/params/arguments/message")
                .and_then(Value::as_str),
            Some(format!("message number {i}").as_str())
        );
    }
    let mut ts: Vec<_> = results.iter().map(|(_, s)| s.ts.clone()).collect();
    ts.sort();
    ts.dedup();
    assert_eq!(ts.len(), 8, "every send must get a distinct ts");
}

#[test]
fn resolve_destination_resolves_user_and_channel_ids() {
    let mock = MockSlackMcpServer::start();
    let client = fresh_client(&mock);

    // User ID -> "Real Name (@username)" via the nested user object.
    assert_eq!(
        client.resolve_destination("U0DRAKE"),
        Some("Drake Thomsen (@drake.thomsen)".to_string())
    );
    let user_calls = mock.calls_to("slack_read_user_profile");
    assert_eq!(user_calls.len(), 1);
    assert_eq!(
        user_calls[0].body["params"]["arguments"],
        json!({ "user_id": "U0DRAKE" })
    );
    assert_eq!(
        user_calls[0].authorization.as_deref(),
        Some("Bearer xoxp-slack-mcp-test-token")
    );

    // Channel ID -> "#name" via the channel object.
    assert_eq!(client.resolve_destination("C0ENG"), Some("#eng".to_string()));
    let channel_calls = mock.calls_to("slack_read_channel");
    assert_eq!(channel_calls.len(), 1);
    assert_eq!(
        channel_calls[0].body["params"]["arguments"],
        json!({ "channel_id": "C0ENG" })
    );

    // A flat JSON user object resolves the same way.
    mock.set_tool_reply_for(
        "slack_read_user_profile",
        json!({ "content": [{ "type": "text", "text": serde_json::to_string(&json!({
            "real_name": "Drake Thomsen", "name": "drake.thomsen",
            "profile": { "display_name": "Drake" }
        })).unwrap() }] }),
    );
    assert_eq!(
        client.resolve_destination("U0DRAKE"),
        Some("Drake Thomsen (@drake.thomsen)".to_string())
    );

    // `Key: value` text lines resolve the same way.
    mock.set_tool_reply_for(
        "slack_read_user_profile",
        json!({ "content": [{ "type": "text", "text":
            "User ID: U0DRAKE\nUsername: drake.thomsen\nDisplay Name: Drake\nReal Name: Drake Thomsen\n" }] }),
    );
    assert_eq!(
        client.resolve_destination("U0DRAKE"),
        Some("Drake Thomsen (@drake.thomsen)".to_string())
    );

    // A channels array (the list tool's shape) resolves the channel too.
    mock.set_tool_reply_for(
        "slack_read_channel",
        json!({ "content": [{ "type": "text", "text": serde_json::to_string(&json!({
            "channels": [{ "id": "C0RAND", "name": "random" }]
        })).unwrap() }] }),
    );
    assert_eq!(
        client.resolve_destination("C0RAND"),
        Some("#random".to_string())
    );

    // `Channel: #name (ID)` text resolves the channel too.
    mock.set_tool_reply_for(
        "slack_read_channel",
        json!({ "content": [{ "type": "text", "text":
            "Channel: #eng (C0ENG)\n\n=== Message from someone ===" }] }),
    );
    assert_eq!(client.resolve_destination("C0ENG"), Some("#eng".to_string()));
}

#[test]
fn resolve_destination_returns_none_for_names_dm_ids_and_failures() {
    let mock = MockSlackMcpServer::start();
    let client = fresh_client(&mock);

    // Already-readable destinations and DM IDs need no lookup.
    assert_eq!(client.resolve_destination("#eng"), None);
    assert_eq!(client.resolve_destination("@drake"), None);
    assert_eq!(client.resolve_destination("D0DRAKE"), None);
    assert!(
        mock.requests().is_empty(),
        "no lookup for #/@/D destinations"
    );

    // Unknown IDs fall back to the raw destination.
    assert_eq!(client.resolve_destination("U0NOPE"), None);
    assert_eq!(client.resolve_destination("C0NOPE"), None);

    // Lookup failures fall back to the raw destination.
    mock.set_call_error(Some((500, "boom")));
    assert_eq!(client.resolve_destination("U0DRAKE"), None);
    assert_eq!(client.resolve_destination("C0ENG"), None);
}

/// Checks the ~30s request timeout against a server that never replies.
/// Slow, so ignored by default: `cargo test --test slack_mcp_test -- --ignored`.
#[test]
#[ignore = "takes about 30 seconds"]
fn unresponsive_server_times_out() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    // Accept and hold connections open without replying.
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().flatten() {
            held.push(stream);
        }
    });

    let started = std::time::Instant::now();
    let err = SlackMcpClient::new(TEST_SLACK_MCP_TOKEN, format!("http://{addr}/mcp"))
        .expect_err("an unresponsive server must time out the handshake");

    let elapsed = started.elapsed();
    assert!(
        elapsed.as_secs() >= 28 && elapsed.as_secs() <= 60,
        "took {elapsed:?}"
    );
    assert!(err.0.contains("no reply within 30s"), "error was {:?}", err.0);
}

/// Restores the given environment variables when dropped (even on panic).
struct EnvGuard(Vec<(&'static str, Option<OsString>)>);

impl EnvGuard {
    fn save(names: &[&'static str]) -> Self {
        Self(names.iter().map(|n| (*n, std::env::var_os(n))).collect())
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (name, value) in &self.0 {
            match value {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
    }
}

/// Env vars that steer `SlackMcpClient::from_env` and transport selection.
/// `SLACK_MCP_TOKEN` is not set in the developer's shell, but the Mistral
/// vars are guarded too: the `build_client` dispatch checks below must see
/// the mistral-connector branch fail for lack of a key.
const SLACK_MCP_ENV: &[&str] = &[
    "SLACK_MCP_TOKEN",
    "SLACK_MCP_URL",
    "SLACK_MCP_TOOL",
    "SLACK_MCP_CHANNEL_ARG",
    "SLACK_MCP_TEXT_ARG",
    "SLACK_MCP_THREAD_ARG",
    "SLACK_WORKSPACE_DOMAIN",
    "SLACK_TRANSPORT",
    "MISTRAL_API_KEY",
    "MISTRAL_API_BASE",
];

/// The only test in this binary that touches environment variables, so all
/// env-dependent checks live here and run sequentially.
#[test]
fn from_env_reads_token_file_url_and_arg_overrides() {
    let _guard = EnvGuard::save(SLACK_MCP_ENV);
    let mock = MockSlackMcpServer::start();
    for name in SLACK_MCP_ENV {
        std::env::remove_var(name);
    }
    std::env::set_var("SLACK_MCP_URL", mock.mcp_url());

    // No env token and no token file: error naming both places.
    let dir = temp_state_dir("slack-mcp-from-env");
    let err = SlackMcpClient::from_env(&dir).expect_err("no token anywhere must be an error");
    assert!(err.0.contains("SLACK_MCP_TOKEN"), "error was {:?}", err.0);
    assert!(err.0.contains("slack_mcp_token"), "error was {:?}", err.0);

    // Blank token file: error.
    let token_path = dir.join("slack_mcp_token");
    std::fs::write(&token_path, "  \n").unwrap();
    assert!(
        SlackMcpClient::from_env(&dir).is_err(),
        "blank token file must be an error"
    );

    // Token file, trimmed of surrounding whitespace; the arg-name overrides
    // apply over a differently-shaped schema.
    mock.set_tools(
        json!([
            { "name": "post_message", "inputSchema": { "type": "object", "properties": {
                "channel": { "type": "string" }, "content": { "type": "string" },
                "parent_ts": { "type": "string" }
            } } }
        ])
        .as_array()
        .unwrap()
        .clone(),
    );
    std::fs::write(&token_path, "  xoxp-from-file \n\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    std::env::set_var("SLACK_MCP_CHANNEL_ARG", "channel_id");
    std::env::set_var("SLACK_MCP_TEXT_ARG", "message");
    std::env::set_var("SLACK_MCP_THREAD_ARG", "thread_ts");
    let client = SlackMcpClient::from_env(&dir).expect("token file should be used");
    client
        .post_message("C1", Some("1699999999.000200"), "from file")
        .expect("post_message should succeed");
    let call = &mock.calls_to("post_message")[0];
    assert_eq!(call.authorization.as_deref(), Some("Bearer xoxp-from-file"));
    assert_eq!(
        call.body["params"]["arguments"],
        json!({ "channel_id": "C1", "message": "from file", "thread_ts": "1699999999.000200" })
    );

    // SLACK_MCP_TOKEN takes precedence over the file.
    std::env::set_var("SLACK_MCP_TOKEN", "xoxp-from-env");
    let client = SlackMcpClient::from_env(&dir).expect("env token should be used");
    client
        .post_message("C2", None, "from env")
        .expect("post_message should succeed");
    let last = mock.calls_to("post_message").pop().unwrap();
    assert_eq!(last.authorization.as_deref(), Some("Bearer xoxp-from-env"));

    // A token with whitespace is rejected.
    std::env::set_var("SLACK_MCP_TOKEN", "two tokens");
    let err = SlackMcpClient::from_env(&dir)
        .expect_err("a token with spaces must be an error");
    assert!(err.0.contains("single token"), "error was {:?}", err.0);
    std::env::remove_var("SLACK_MCP_TOKEN");
    for name in [
        "SLACK_MCP_CHANNEL_ARG",
        "SLACK_MCP_TEXT_ARG",
        "SLACK_MCP_THREAD_ARG",
    ] {
        std::env::remove_var(name);
    }

    // Fallback permalink uses SLACK_WORKSPACE_DOMAIN, else "workspace".
    mock.set_tool_reply(Some(json!({
        "content": [{ "type": "text", "text": "ok" }],
        "structuredContent": { "ts": "1700000000.000042" }
    })));
    std::env::set_var("SLACK_WORKSPACE_DOMAIN", "acme");
    let sent = client.post_message("C3", None, "domain").unwrap();
    assert_eq!(
        sent.permalink,
        format!("https://acme.slack.com/archives/C3/p{}", sent.ts.replace('.', ""))
    );
    std::env::remove_var("SLACK_WORKSPACE_DOMAIN");
    let sent = client.post_message("C4", None, "no domain").unwrap();
    assert_eq!(
        sent.permalink,
        format!("https://workspace.slack.com/archives/C4/p{}", sent.ts.replace('.', ""))
    );

    // build_client dispatches on the selected transport.
    std::fs::remove_file(&token_path).unwrap();
    let err = slack::build_client(&dir)
        .map(|_| ())
        .expect_err("no MCP token must be an error");
    assert!(err.0.contains("SLACK_MCP_TOKEN"), "error was {:?}", err.0);
    std::env::set_var("SLACK_TRANSPORT", "mistral-connector");
    let err = slack::build_client(&dir)
        .map(|_| ())
        .expect_err("no Mistral key must be an error");
    assert!(err.0.contains("MISTRAL_API_KEY"), "error was {:?}", err.0);
    std::env::remove_var("SLACK_TRANSPORT");
    std::env::set_var("SLACK_MCP_TOKEN", "xoxp-from-env");
    let client = slack::build_client(&dir).expect("the default transport builds an MCP client");
    client
        .post_message("C5", None, "via build_client")
        .expect("send through the built client");
    let last = mock.calls_to("post_message").pop().unwrap();
    assert_eq!(last.authorization.as_deref(), Some("Bearer xoxp-from-env"));
}
