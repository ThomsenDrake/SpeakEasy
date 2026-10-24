//! Integration tests for `MistralConnectorSlackClient` against the local
//! mock Mistral platform API, plus transport selection in `slack`.

mod common;

use common::{temp_state_dir, MockMistralServer, TEST_MISTRAL_KEY};
use serde_json::json;
use slack_approval_gateway::core::SlackClient;
use slack_approval_gateway::mistral_slack::MistralConnectorSlackClient;
use slack_approval_gateway::slack::{resolve_transport, SlackTransport};
use std::ffi::OsString;

#[test]
fn post_message_discovers_send_tool_and_sends_exact_arguments_with_bearer_key() {
    let mock = MockMistralServer::start();
    let text = "Deploy *v1.2.3* finished :tada:\n  second line — ünïcödé & \"quoted\"  ";

    let sent = mock
        .client()
        .post_message("#general", Some("1699999999.000200"), text)
        .expect("post_message should succeed");

    // Discovery first, then the tool call.
    let requests = mock.requests();
    let paths: Vec<&str> = requests.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "/v1/connectors/slack/tools",
            "/v1/connectors/slack/tools/slack_send_message/call"
        ]
    );
    let discovery = &requests[0];
    assert_eq!(discovery.method, "GET");
    assert_eq!(
        discovery.authorization.as_deref(),
        Some("Bearer mistral-test-key")
    );
    assert!(
        discovery.query.as_deref().unwrap_or("").contains("page_size=100"),
        "query was {:?}",
        discovery.query
    );

    // Whole-body equality: exact values and no extra keys.
    let call = &requests[1];
    assert_eq!(call.method, "POST");
    assert_eq!(call.authorization.as_deref(), Some("Bearer mistral-test-key"));
    assert!(
        call.content_type
            .as_deref()
            .unwrap_or("")
            .starts_with("application/json"),
        "content type was {:?}",
        call.content_type
    );
    assert_eq!(
        call.body,
        json!({ "arguments": { "channel": "#general", "text": text, "thread_ts": "1699999999.000200" } })
    );
    assert_eq!(call.body_str("text"), None, "text lives under arguments");
    assert_eq!(
        call.body.pointer("/arguments/text").and_then(|v| v.as_str()),
        Some(text),
        "text must be sent verbatim"
    );

    // The outcome comes from the structured content.
    assert_eq!(sent.channel, "#general");
    assert_eq!(sent.ts, "1700000000.000100");
    assert_eq!(
        sent.permalink,
        "https://mock.slack.com/archives/CGENERAL/p1700000000000100"
    );
}

#[test]
fn thread_ts_argument_is_absent_when_none() {
    let mock = MockMistralServer::start();

    mock.client()
        .post_message("C0123ABC", None, "hello")
        .expect("post_message should succeed");

    let calls = mock.tool_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].body,
        json!({ "arguments": { "channel": "C0123ABC", "text": "hello" } })
    );
}

#[test]
fn pinned_tool_skips_discovery() {
    let mock = MockMistralServer::start();
    let client = mock
        .client()
        .with_connector("slack")
        .with_tool(Some("slack_send_message".to_string()));

    client
        .post_message("#general", None, "hi")
        .expect("post_message should succeed");

    let requests = mock.requests();
    assert_eq!(requests.len(), 1, "no tool-list request when the tool is pinned");
    assert_eq!(
        requests[0].path,
        "/v1/connectors/slack/tools/slack_send_message/call"
    );
}

#[test]
fn custom_connector_and_argument_names_are_used() {
    let mock = MockMistralServer::start();
    let client = mock
        .client()
        .with_connector("019cfb41-5e8a-7350-b162-b2b80604fe5a")
        .with_tool(Some("send_slack_message".to_string()))
        .with_arguments("channel_id", "message", "parent_ts");

    client
        .post_message("C0123ABC", Some("1699999999.000200"), "hello")
        .expect("post_message should succeed");

    let calls = mock.tool_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].path,
        "/v1/connectors/019cfb41-5e8a-7350-b162-b2b80604fe5a/tools/send_slack_message/call"
    );
    assert_eq!(
        calls[0].body,
        json!({ "arguments": {
            "channel_id": "C0123ABC", "message": "hello", "parent_ts": "1699999999.000200"
        } })
    );
}

#[test]
fn permalink_comes_from_text_and_ts_from_the_link_when_structured_content_is_absent() {
    let mock = MockMistralServer::start();
    mock.set_tool_reply(Some(json!({
        "content": [{ "type": "text",
            "text": "Posted to #general: https://acme.slack.com/archives/C1/p1700000000000100 — done" }]
    })));

    let sent = mock
        .client()
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
    let mock = MockMistralServer::start();
    mock.set_tool_reply(Some(json!({
        "content": [{ "type": "text", "text": "ok" }],
        "metadata": { "mcp_meta": { "structuredContent": { "ts": "1700000000.000042" } } }
    })));

    let sent = mock
        .client()
        .post_message("C0123ABC", None, "hello")
        .expect("post_message should succeed");

    assert_eq!(sent.ts, "1700000000.000042");
    assert_eq!(
        sent.permalink,
        "https://workspace.slack.com/archives/C0123ABC/p1700000000000042"
    );
}

#[test]
fn tool_error_metadata_maps_to_slack_error_with_the_tool_text() {
    let mock = MockMistralServer::start();
    mock.set_tool_reply(Some(json!({
        "content": [{ "type": "text", "text": "channel_not_found: no such channel" }],
        "metadata": { "mcp_meta": { "isError": true } }
    })));

    let err = mock
        .client()
        .post_message("C_NOPE", None, "hello")
        .expect_err("isError metadata must be an error");

    assert!(
        err.0.contains("channel_not_found: no such channel"),
        "error was {:?}",
        err.0
    );
    assert!(!err.0.contains(TEST_MISTRAL_KEY), "the key must never appear in errors");
}

#[test]
fn http_error_maps_to_slack_error_with_detail_and_never_the_key() {
    let mock = MockMistralServer::start();
    mock.set_call_error(Some((
        401,
        "No credentials found for connector 'slack'. Please authenticate.",
    )));

    let err = mock
        .client()
        .post_message("#general", None, "hello")
        .expect_err("HTTP 401 must be an error");

    assert!(err.0.contains("401"), "error was {:?}", err.0);
    assert!(
        err.0.contains("No credentials found for connector 'slack'"),
        "error was {:?}",
        err.0
    );
    assert!(!err.0.contains(TEST_MISTRAL_KEY), "the key must never appear in errors");
    // Discovery succeeded; only the tool call was rejected.
    assert_eq!(mock.tool_calls().len(), 1);
}

#[test]
fn discovery_without_a_send_tool_lists_the_available_names() {
    let mock = MockMistralServer::start();
    mock.set_tools(json!([
        { "name": "slack_search_messages" },
        { "name": "slack_list_channels" }
    ])
    .as_array()
    .unwrap()
    .clone());

    let err = mock
        .client()
        .post_message("#general", None, "hello")
        .expect_err("no send tool in the catalog must be an error");

    assert!(err.0.contains("no send-message tool"), "error was {:?}", err.0);
    assert!(err.0.contains("slack_search_messages"), "error was {:?}", err.0);
    assert!(err.0.contains("MISTRAL_SLACK_TOOL"), "error was {:?}", err.0);
    assert!(mock.tool_calls().is_empty(), "no tool call without a tool");
}

#[test]
fn discovery_is_cached_across_calls() {
    let mock = MockMistralServer::start();
    let client = mock.client();

    client.post_message("C1", None, "one").expect("first send");
    client.post_message("C2", None, "two").expect("second send");

    let listings = mock.requests_to("/v1/connectors/slack/tools");
    assert_eq!(listings.len(), 1, "the tool list is fetched once per client");
    assert_eq!(mock.tool_calls().len(), 2);
}

#[test]
fn transport_error_maps_to_slack_error() {
    // Grab a free port, then close it so the connection is refused.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let client = MistralConnectorSlackClient::new(TEST_MISTRAL_KEY, format!("http://{addr}"));

    let err = client
        .post_message("C1", None, "hello")
        .expect_err("connection refused must be an error");

    assert!(err.0.starts_with("Mistral API"), "error was {:?}", err.0);
    assert!(!err.0.contains(TEST_MISTRAL_KEY), "the key must never appear in errors");
}

#[test]
fn new_strips_trailing_slash_and_debug_redacts_key() {
    let mock = MockMistralServer::start();
    let client = MistralConnectorSlackClient::new(TEST_MISTRAL_KEY, format!("{}/", mock.base_url()));

    client.post_message("C1", None, "hi").expect("post_message should succeed");

    assert_eq!(
        mock.requests()[0].path,
        "/v1/connectors/slack/tools",
        "the trailing slash must be stripped"
    );
    let debug = format!("{client:?}");
    assert!(!debug.contains(TEST_MISTRAL_KEY), "Debug output leaked the key: {debug}");
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

/// Env vars that steer `MistralConnectorSlackClient::from_env` and transport
/// selection. `MISTRAL_API_KEY` is set in the developer's shell and would be
/// inherited by `cargo test`, so every env test removes it under the guard.
const MISTRAL_ENV: &[&str] = &[
    "MISTRAL_API_KEY",
    "MISTRAL_API_BASE",
    "MISTRAL_SLACK_CONNECTOR",
    "MISTRAL_SLACK_TOOL",
    "MISTRAL_SLACK_CHANNEL_ARG",
    "MISTRAL_SLACK_TEXT_ARG",
    "MISTRAL_SLACK_THREAD_ARG",
    "SLACK_TRANSPORT",
];

/// The only test in this binary that touches environment variables, so all
/// env-dependent checks live here and run sequentially.
#[test]
fn from_env_reads_key_file_and_arg_overrides() {
    let _guard = EnvGuard::save(MISTRAL_ENV);
    let mock = MockMistralServer::start();
    for name in MISTRAL_ENV {
        std::env::remove_var(name);
    }
    std::env::set_var("MISTRAL_API_BASE", mock.base_url());

    // No env key and no key file: error naming both places.
    let dir = temp_state_dir("mistral-from-env");
    let err = MistralConnectorSlackClient::from_env(&dir)
        .expect_err("no key anywhere must be an error");
    assert!(err.0.contains("MISTRAL_API_KEY"), "error was {:?}", err.0);
    assert!(err.0.contains("mistral_api_key"), "error was {:?}", err.0);

    // Blank key file: error.
    let key_path = dir.join("mistral_api_key");
    std::fs::write(&key_path, "  \n").unwrap();
    assert!(MistralConnectorSlackClient::from_env(&dir).is_err(), "blank key file must be an error");

    // Key file, trimmed of surrounding whitespace; arg-name overrides apply.
    std::fs::write(&key_path, "  mistral-from-file \n\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    std::env::set_var("MISTRAL_SLACK_CHANNEL_ARG", "channel_id");
    std::env::set_var("MISTRAL_SLACK_TEXT_ARG", "message");
    let client = MistralConnectorSlackClient::from_env(&dir).expect("key file should be used");
    client
        .post_message("C1", Some("1699999999.000200"), "from file")
        .expect("post_message should succeed");
    let call = &mock.tool_calls()[0];
    assert_eq!(call.authorization.as_deref(), Some("Bearer mistral-from-file"));
    assert_eq!(
        call.body,
        json!({ "arguments": {
            "channel_id": "C1", "message": "from file", "thread_ts": "1699999999.000200"
        } })
    );

    // MISTRAL_API_KEY takes precedence over the file.
    std::env::set_var("MISTRAL_API_KEY", "mistral-from-env");
    let client = MistralConnectorSlackClient::from_env(&dir).expect("env key should be used");
    client.post_message("C2", None, "from env").expect("post_message should succeed");
    let last = mock.tool_calls().pop().unwrap();
    assert_eq!(last.authorization.as_deref(), Some("Bearer mistral-from-env"));

    // A key with whitespace is rejected.
    std::env::set_var("MISTRAL_API_KEY", "two tokens");
    let err = MistralConnectorSlackClient::from_env(&dir)
        .expect_err("a key with spaces must be an error");
    assert!(err.0.contains("single token"), "error was {:?}", err.0);
}

#[test]
fn resolve_transport_prefers_env_then_file_then_default() {
    let _guard = EnvGuard::save(&["SLACK_TRANSPORT"]);
    std::env::remove_var("SLACK_TRANSPORT");
    let dir = temp_state_dir("transport");

    // Default: direct.
    assert_eq!(resolve_transport(&dir).unwrap(), SlackTransport::Direct);

    // File: one line, trimmed.
    std::fs::write(dir.join("slack_transport"), "mistral-connector\n").unwrap();
    assert_eq!(resolve_transport(&dir).unwrap(), SlackTransport::MistralConnector);

    // Env beats the file; aliases accepted.
    std::env::set_var("SLACK_TRANSPORT", "direct");
    assert_eq!(resolve_transport(&dir).unwrap(), SlackTransport::Direct);
    std::env::set_var("SLACK_TRANSPORT", "connector");
    assert_eq!(resolve_transport(&dir).unwrap(), SlackTransport::MistralConnector);
    std::env::set_var("SLACK_TRANSPORT", "mistral_connector");
    assert_eq!(resolve_transport(&dir).unwrap(), SlackTransport::MistralConnector);

    // Unknown value: error naming the accepted values.
    std::env::set_var("SLACK_TRANSPORT", "carrier-pigeon");
    let err = resolve_transport(&dir).expect_err("unknown transport must be an error");
    assert!(err.0.contains("carrier-pigeon"), "error was {:?}", err.0);
    assert!(err.0.contains("mistral-connector"), "error was {:?}", err.0);
}
