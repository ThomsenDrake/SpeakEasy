//! Integration tests for `KorotovskyClient` against a mock of Korotovsky's
//! Slack MCP server (`scripts/mock_korotovsky.py`, MCP over stdio), plus the
//! korotovsky transport-selection and credential handling in `from_env`.

mod common;

use common::temp_state_dir;
use slack_approval_gateway::core::SlackClient;
use slack_approval_gateway::slack;
use slack_approval_gateway::slack::SlackTransport;
use slack_approval_gateway::slack_stdio::KorotovskyClient;
use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const TEST_BOT_TOKEN: &str = "xoxb-korotovsky-test-token";

/// The mock server script, addressed absolutely.
fn mock_script() -> String {
    format!("{}/scripts/mock_korotovsky.py", env!("CARGO_MANIFEST_DIR"))
}

/// A `SLACK_STDIO_COMMAND` that runs the mock with a per-test log file. The
/// `env KEY=VALUE …` prefix keeps every knob off the process environment, so
/// tests never race on env vars.
fn mock_command(log: &Path, extra: &[(&str, &str)]) -> String {
    let mut parts = vec![
        "env".to_string(),
        format!("MOCK_KOROTOVSKY_LOG={}", log.display()),
    ];
    for (key, value) in extra {
        parts.push(format!("{key}={value}"));
    }
    parts.push("python3".to_string());
    parts.push(mock_script());
    parts.join(" ")
}

/// A client driving the mock, with the send-tool handshake already done.
fn mock_client(dir: &Path, extra: &[(&str, &str)]) -> KorotovskyClient {
    let command = mock_command(&dir.join("mock.log"), extra);
    KorotovskyClient::new(
        Some(TEST_BOT_TOKEN.to_string()),
        None,
        &command,
        "text/plain",
        dir,
    )
    .expect("the handshake with the mock must succeed")
}

/// Every `{"tool": …, "arguments": …}` line the mock recorded.
fn recorded_calls(dir: &Path) -> Vec<Value> {
    let log = dir.join("mock.log");
    let Ok(contents) = std::fs::read_to_string(&log) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry.get("tool").is_some())
        .collect()
}

/// The `{"env": …}` line the mock recorded at startup (the last one — every
/// client construction spawns a fresh mock appending to the same log).
fn recorded_env(dir: &Path) -> Value {
    let log = dir.join("mock.log");
    let contents = std::fs::read_to_string(&log).expect("the mock must record its env");
    let entry = contents
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry.get("env").is_some())
        .last()
        .expect("the mock records its env at startup");
    entry.get("env").cloned().unwrap_or(Value::Null)
}

#[test]
fn handshake_and_post_message_send_the_exact_text_to_the_exact_destination() {
    let dir = temp_state_dir("stdio-send");
    let client = mock_client(&dir, &[]);

    let text = "Deploy *v1.2.3* finished :tada:\n  second line — ünïcödé & \"quoted\"  ";
    let send = client
        .post_message("C0123SMOKE", None, text)
        .expect("post_message");

    // The server got the exact text, the exact destination and text/plain
    // (no markdown interpretation of the approved text).
    let calls = recorded_calls(&dir);
    assert_eq!(calls.len(), 1, "recorded calls: {calls:?}");
    assert_eq!(calls[0].get("tool").and_then(Value::as_str), Some("conversations_add_message"));
    let args = calls[0].get("arguments").cloned().unwrap_or(Value::Null);
    assert_eq!(args.get("channel_id").and_then(Value::as_str), Some("C0123SMOKE"));
    assert_eq!(args.get("text").and_then(Value::as_str), Some(text));
    assert_eq!(args.get("content_type").and_then(Value::as_str), Some("text/plain"));
    assert!(args.get("thread_ts").is_none(), "no thread without a thread: {args}");

    // The reply is plain text; the ts is parsed out and the permalink built.
    assert_eq!(send.channel, "C0123SMOKE");
    assert_eq!(send.ts, "1700000000.000100");
    assert!(send.permalink.starts_with("https://"), "{}", send.permalink);
    assert!(
        send.permalink.ends_with("/archives/C0123SMOKE/p1700000000000100"),
        "{}",
        send.permalink
    );
}

#[test]
fn post_message_in_a_thread_passes_thread_ts() {
    let dir = temp_state_dir("stdio-thread");
    let client = mock_client(&dir, &[]);

    let send = client
        .post_message("D0456SMOKE", Some("1699999999.000200"), "thread reply")
        .expect("post_message");

    let calls = recorded_calls(&dir);
    let args = calls[0].get("arguments").cloned().unwrap_or(Value::Null);
    assert_eq!(args.get("channel_id").and_then(Value::as_str), Some("D0456SMOKE"));
    assert_eq!(args.get("thread_ts").and_then(Value::as_str), Some("1699999999.000200"));
    assert_eq!(send.ts, "1700000000.000100");
    assert!(send.permalink.ends_with("/archives/D0456SMOKE/p1700000000000100"));
}

#[test]
fn post_message_surfaces_tool_errors() {
    let dir = temp_state_dir("stdio-tool-error");
    let client = mock_client(&dir, &[("MOCK_KOROTOVSKY_TOOL_ERROR", "1")]);

    let err = client.post_message("C0123SMOKE", None, "boom").unwrap_err();
    assert!(err.0.contains("conversations_add_message"), "{}", err.0);
    assert!(err.0.contains("channel_not_found"), "{}", err.0);
}

#[test]
fn stdout_noise_is_skipped() {
    let dir = temp_state_dir("stdio-noise");
    // The real server prints a non-JSON line to stdout on a bad log level;
    // the client must skip anything that is not one JSON-RPC message.
    let client = mock_client(&dir, &[("MOCK_KOROTOVSKY_NOISE", "1")]);
    client.post_message("C0123SMOKE", None, "hi").expect("post_message");
    assert_eq!(recorded_calls(&dir).len(), 1);
}

#[test]
fn startup_fails_when_the_send_tool_is_not_registered() {
    let dir = temp_state_dir("stdio-no-tool");
    let command = mock_command(
        &dir.join("mock.log"),
        &[("MOCK_KOROTOVSKY_NO_ADD_MESSAGE", "1")],
    );
    let err = KorotovskyClient::new(
        Some(TEST_BOT_TOKEN.to_string()),
        None,
        &command,
        "text/plain",
        &dir,
    )
    .expect_err("the handshake must fail when the send tool is missing");
    assert!(err.0.contains("no send-message tool"), "{}", err.0);
    assert!(err.0.contains("SLACK_MCP_ADD_MESSAGE_TOOL"), "{}", err.0);
}

#[test]
fn startup_fails_with_the_server_stderr_when_the_server_exits() {
    let dir = temp_state_dir("stdio-exit");
    let command = mock_command(&dir.join("mock.log"), &[("MOCK_KOROTOVSKY_EXIT", "1")]);
    let err = KorotovskyClient::new(
        Some(TEST_BOT_TOKEN.to_string()),
        None,
        &command,
        "text/plain",
        &dir,
    )
    .expect_err("the handshake must fail when the server exits");
    assert!(err.0.contains("handshake failed"), "{}", err.0);
    assert!(err.0.contains("invalid token, refusing to start"), "{}", err.0);
}

#[test]
fn startup_fails_for_a_missing_server_command() {
    let dir = temp_state_dir("stdio-missing");
    let err = KorotovskyClient::new(
        Some(TEST_BOT_TOKEN.to_string()),
        None,
        "definitely-not-a-real-slack-mcp-server-binary",
        "text/plain",
        &dir,
    )
    .expect_err("a missing server must fail at startup");
    assert!(err.0.contains("cannot start"), "{}", err.0);
    assert!(err.0.contains("SLACK_STDIO_COMMAND"), "{}", err.0);
}

#[test]
fn resolve_destination_names_users_and_channels_from_csv() {
    let dir = temp_state_dir("stdio-resolve");
    let client = mock_client(&dir, &[]);

    assert_eq!(
        client.resolve_destination("U0123SMOKE").as_deref(),
        Some("Drake Thomsen (@drake.thomsen)")
    );
    assert_eq!(client.resolve_destination("U0456SMOKE").as_deref(), Some("Jane Doe (@jane.doe)"));
    assert_eq!(client.resolve_destination("C0123SMOKE").as_deref(), Some("#eng"));
    // Quoted CSV fields (a comma inside the channel name).
    assert_eq!(
        client.resolve_destination("C0456SMOKE").as_deref(),
        Some("#team, chat")
    );
    // DM IDs, #names and @names are already readable or unresolvable.
    assert_eq!(client.resolve_destination("D0123SMOKE"), None);
    assert_eq!(client.resolve_destination("#eng"), None);
    assert_eq!(client.resolve_destination("@drake.thomsen"), None);
}

#[test]
fn dropping_the_client_kills_the_server_process() {
    let dir = temp_state_dir("stdio-drop");
    let pidfile = dir.join("mock.pid");
    let client = mock_client(&dir, &[("MOCK_KOROTOVSKY_PIDFILE", pidfile.to_str().unwrap())]);
    let pid: u32 = std::fs::read_to_string(&pidfile)
        .expect("the mock writes its pidfile at startup")
        .trim()
        .parse()
        .expect("a pid");

    let alive = || {
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    assert!(alive(), "the mock must be running while the client lives");

    drop(client);
    let deadline = Instant::now() + Duration::from_secs(5);
    while alive() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(), "dropping the client must kill the server process");
}

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

/// Env vars that steer transport selection and `KorotovskyClient::from_env`.
/// `SLACK_MCP_*` variables are set in some developer shells and would be
/// inherited by `cargo test`, so every env test removes them under the guard.
const STDIO_ENV: &[&str] = &[
    "SLACK_TRANSPORT",
    "SLACK_BOT_TOKEN",
    "SLACK_USER_TOKEN",
    "SLACK_STDIO_COMMAND",
    "SLACK_STDIO_CONTENT_TYPE",
    "SLACK_STDIO_TOOL",
    "SLACK_STDIO_CHANNEL_ARG",
    "SLACK_STDIO_TEXT_ARG",
    "SLACK_STDIO_THREAD_ARG",
    "SLACK_WORKSPACE_DOMAIN",
    "SLACK_MCP_XOXB_TOKEN",
    "SLACK_MCP_XOXP_TOKEN",
    "SLACK_MCP_XOXC_TOKEN",
    "SLACK_MCP_ADD_MESSAGE_TOOL",
    "SLACK_MCP_LOG_LEVEL",
    "SLACK_MCP_USERS_CACHE",
    "SLACK_MCP_CHANNELS_CACHE",
];

/// The only test in this binary that touches environment variables, so all
/// env-dependent checks live here and run sequentially.
#[test]
fn from_env_credentials_transport_resolution_and_server_env() {
    let _guard = EnvGuard::save(STDIO_ENV);
    for name in STDIO_ENV {
        std::env::remove_var(name);
    }
    let dir = temp_state_dir("stdio-env");
    let log: PathBuf = dir.join("mock.log");

    // --- transport resolution
    assert_eq!(
        slack::resolve_transport(&dir).unwrap(),
        SlackTransport::SlackMcp
    );
    std::env::set_var("SLACK_TRANSPORT", "korotovsky");
    assert_eq!(
        slack::resolve_transport(&dir).unwrap(),
        SlackTransport::Korotovsky
    );
    std::env::set_var("SLACK_TRANSPORT", "slack-mcp-stdio");
    assert_eq!(
        slack::resolve_transport(&dir).unwrap(),
        SlackTransport::Korotovsky
    );
    std::env::set_var("SLACK_TRANSPORT", "stdio");
    assert_eq!(
        slack::resolve_transport(&dir).unwrap(),
        SlackTransport::Korotovsky
    );
    std::env::remove_var("SLACK_TRANSPORT");
    std::fs::write(dir.join("slack_transport"), "korotovsky\n").unwrap();
    assert_eq!(
        slack::resolve_transport(&dir).unwrap(),
        SlackTransport::Korotovsky
    );
    std::env::set_var("SLACK_TRANSPORT", "carrier-pigeon");
    let err = slack::resolve_transport(&dir).expect_err("unknown transport must be an error");
    assert!(err.0.contains("carrier-pigeon"), "{}", err.0);
    assert!(err.0.contains("korotovsky"), "{}", err.0);
    std::env::remove_var("SLACK_TRANSPORT");

    // --- credentials: nothing anywhere is an error naming the files
    let err = KorotovskyClient::from_env(&dir).expect_err("no credential must be an error");
    assert!(err.0.contains("slack_bot_token"), "{}", err.0);
    assert!(err.0.contains("slack_user_token"), "{}", err.0);

    // The server's own env vars count as a credential (the child inherits
    // them; the gateway needs no copy).
    std::env::set_var("SLACK_MCP_XOXC_TOKEN", "xoxc-inherited");
    let command = mock_command(&log, &[]);
    std::env::set_var("SLACK_STDIO_COMMAND", &command);
    KorotovskyClient::from_env(&dir).expect("an inherited server token must be enough");
    std::env::remove_var("SLACK_MCP_XOXC_TOKEN");

    // A bot token file is picked up; an env var beats the file.
    std::fs::write(dir.join("slack_bot_token"), "xoxb-from-file\n").unwrap();
    KorotovskyClient::from_env(&dir).expect("the bot token file must be read");
    std::env::set_var("SLACK_BOT_TOKEN", "xoxb-from-env");
    KorotovskyClient::from_env(&dir).expect("the env token must be read");
    std::env::remove_var("SLACK_BOT_TOKEN");

    // A user token file works on its own; a token with whitespace is rejected.
    std::fs::remove_file(dir.join("slack_bot_token")).unwrap();
    std::fs::write(dir.join("slack_user_token"), "xoxp-from-file\n").unwrap();
    KorotovskyClient::from_env(&dir).expect("the user token file must be read");
    std::fs::write(dir.join("slack_bot_token"), "two tokens\n").unwrap();
    let err = KorotovskyClient::from_env(&dir).expect_err("whitespace must be an error");
    assert!(err.0.contains("single token"), "{}", err.0);
    std::fs::remove_file(dir.join("slack_bot_token")).unwrap();

    // A bad content type is rejected.
    std::env::set_var("SLACK_STDIO_CONTENT_TYPE", "text/html");
    let err = KorotovskyClient::from_env(&dir).expect_err("a bad content type must be an error");
    assert!(err.0.contains("text/plain"), "{}", err.0);
    std::env::remove_var("SLACK_STDIO_CONTENT_TYPE");

    // --- the server env the gateway sets (set-if-absent defaults)
    let _client = KorotovskyClient::from_env(&dir).expect("from_env must build the client");
    let env = recorded_env(&dir);
    assert_eq!(
        env.get("SLACK_MCP_XOXP_TOKEN").and_then(Value::as_str),
        Some("xoxp-from-file"),
        "the gateway passes its user token to the child"
    );
    assert_eq!(
        env.get("SLACK_MCP_ADD_MESSAGE_TOOL").and_then(Value::as_str),
        Some("true"),
        "the send tool must be enabled"
    );
    assert_eq!(
        env.get("SLACK_MCP_LOG_LEVEL").and_then(Value::as_str),
        Some("error")
    );
    let users_cache = env
        .get("SLACK_MCP_USERS_CACHE")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        users_cache.ends_with("korotovsky_users_cache.json"),
        "{users_cache}"
    );
    assert!(
        users_cache.starts_with(dir.to_str().unwrap()),
        "the cache stays in the state dir: {users_cache}"
    );

    // The operator's own values win over the gateway defaults.
    std::env::set_var("SLACK_MCP_ADD_MESSAGE_TOOL", "C0123SMOKE");
    std::env::set_var("SLACK_MCP_LOG_LEVEL", "debug");
    let client = KorotovskyClient::from_env(&dir).expect("from_env must build the client");
    drop(client);
    let env = recorded_env(&dir);
    assert_eq!(
        env.get("SLACK_MCP_ADD_MESSAGE_TOOL").and_then(Value::as_str),
        Some("C0123SMOKE")
    );
    assert_eq!(env.get("SLACK_MCP_LOG_LEVEL").and_then(Value::as_str), Some("debug"));
    std::env::remove_var("SLACK_MCP_ADD_MESSAGE_TOOL");
    std::env::remove_var("SLACK_MCP_LOG_LEVEL");

    // --- the server command: env var > <state_dir>/slack_stdio_command file
    // (the env var is set above; drop it so the file takes over, then put it
    // back for the sections below)
    std::env::remove_var("SLACK_STDIO_COMMAND");
    let file_command = mock_command(&log, &[]);
    std::fs::write(dir.join("slack_stdio_command"), format!("{file_command}\n")).unwrap();
    let client = KorotovskyClient::from_env(&dir).expect("the command file must be read");
    drop(client);
    std::env::set_var("SLACK_STDIO_COMMAND", &command);

    // --- the workspace domain for fallback permalinks: the env var wins
    // over the <state_dir>/slack_workspace_domain file (the mock's reply has
    // no permalink, so the gateway builds one from this domain)
    std::fs::write(dir.join("slack_workspace_domain"), "filedomain\n").unwrap();
    let client = KorotovskyClient::from_env(&dir).expect("from_env must build the client");
    let send = client
        .post_message("C0123SMOKE", None, "permalink from the domain file")
        .expect("post_message");
    assert!(
        send.permalink.starts_with("https://filedomain.slack.com/archives/"),
        "{}",
        send.permalink
    );
    drop(client);
    std::env::set_var("SLACK_WORKSPACE_DOMAIN", "envdomain");
    let client = KorotovskyClient::from_env(&dir).expect("from_env must build the client");
    let send = client
        .post_message("C0123SMOKE", None, "permalink from the env domain")
        .expect("post_message");
    assert!(
        send.permalink.starts_with("https://envdomain.slack.com/archives/"),
        "{}",
        send.permalink
    );
    drop(client);
    std::env::remove_var("SLACK_WORKSPACE_DOMAIN");

    // --- build_client wires the korotovsky transport end to end
    std::env::set_var("SLACK_TRANSPORT", "korotovsky");
    let client = slack::build_client(&dir).expect("build_client must build the korotovsky client");
    let send = client
        .post_message("C0123SMOKE", None, "via build_client")
        .expect("post_message through the trait object");
    assert_eq!(send.ts, "1700000000.000100");
    std::env::remove_var("SLACK_TRANSPORT");
}
