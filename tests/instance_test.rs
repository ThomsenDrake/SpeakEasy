//! Integration tests for single-instance coordination (`src/instance.rs`):
//! MCP sessions served over the instance's Unix socket — the exact path
//! every bridged agent process takes.

mod common;

use common::{MockSlackServer, ScriptedProvider};
use serde_json::{json, Value};
use slack_approval_gateway::core::{Decision, Gateway};
use slack_approval_gateway::instance;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);

/// Start a mock Slack API, making sure requests to it bypass any proxy.
fn mock_slack() -> MockSlackServer {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        for name in ["NO_PROXY", "no_proxy"] {
            let value = match std::env::var(name) {
                Ok(v) if !v.trim().is_empty() => format!("{v},127.0.0.1,localhost"),
                _ => "127.0.0.1,localhost".to_string(),
            };
            std::env::set_var(name, value);
        }
    });
    MockSlackServer::start()
}

/// A short state dir (Unix socket paths are limited to ~104 bytes, so the
/// per-user temp dir is too long).
fn state_dir(tag: &str) -> PathBuf {
    let dir = PathBuf::from(format!(
        "/tmp/sag-sock-{}-{}-{}",
        tag,
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A connected "bridge": the client end of a socket connection to the
/// instance, exactly what a spawned agent process gets from
/// `instance::connect_existing`.
struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Client {
    fn connect(dir: &PathBuf) -> Self {
        let stream = instance::connect_existing(dir).expect("connect to instance");
        stream
            .set_read_timeout(Some(RESPONSE_TIMEOUT))
            .expect("set read timeout");
        let writer = stream.try_clone().expect("clone stream");
        Self {
            writer,
            reader: BufReader::new(stream),
        }
    }

    fn send(&mut self, message: Value) {
        let mut line = serde_json::to_vec(&message).expect("serialize");
        line.push(b'\n');
        self.writer.write_all(&line).expect("write request");
        self.writer.flush().expect("flush request");
    }

    fn recv(&mut self) -> Value {
        let mut line = Vec::new();
        self.reader
            .read_until(b'\n', &mut line)
            .expect("read response");
        assert!(!line.is_empty(), "instance closed the connection");
        serde_json::from_slice(&line).expect("parse response")
    }

    fn rpc(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        self.recv()
    }

    fn initialize(&mut self, id: u64) {
        let response = self.rpc(
            id,
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "socket-test", "version": "0"},
            }),
        );
        assert_eq!(
            response["result"]["serverInfo"]["name"],
            "slack-approval-gateway"
        );
    }

    fn request_message(&mut self, id: u64, message: &str) -> Value {
        let response = self.rpc(
            id,
            "tools/call",
            json!({
                "name": "request_slack_message",
                "arguments": {
                    "message": message,
                    "destination": "#general",
                    "reason": "socket test",
                    "agent": "socket-test",
                    "conversation": {
                        "title": "Socket test",
                        "id_or_link": "sock-1",
                        "summary": "Exercising the instance socket.",
                    },
                },
            }),
        );
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .expect("text content");
        serde_json::from_str(text).expect("parse verdict")
    }
}

/// Start an instance (gateway + socket listener) in `dir`.
fn start_instance(
    mock: &MockSlackServer,
    provider: Arc<dyn slack_approval_gateway::core::DecisionProvider>,
    dir: &PathBuf,
) -> (instance::InstanceGuard, Arc<Gateway>) {
    let gateway = Arc::new(
        Gateway::new(Arc::new(mock.client()), provider, dir.clone()).expect("create gateway"),
    );
    let (guard, listener) = instance::acquire(dir).expect("acquire instance");
    instance::serve_connections(Arc::clone(&gateway), listener);
    (guard, gateway)
}

#[test]
fn socket_serves_a_full_mcp_session() {
    let mock = mock_slack();
    let dir = state_dir("session");
    let provider = Arc::new(ScriptedProvider::new([Decision::Approve]));
    let (_guard, _gateway) = start_instance(&mock, provider.clone(), &dir);

    let mut client = Client::connect(&dir);
    client.initialize(1);
    let verdict = client.request_message(2, "hello over the socket");
    assert_eq!(verdict["status"], "sent");
    assert_eq!(verdict["duplicate"], false);
    assert_eq!(mock.post_count(), 1);
    let posts = mock.requests_to("/chat.postMessage");
    assert_eq!(posts[0].body_str("text"), Some("hello over the socket"));
    assert_eq!(posts[0].body_str("channel"), Some("#general"));
}

#[test]
fn two_socket_clients_share_one_gateway() {
    let mock = mock_slack();
    let dir = state_dir("shared");
    let provider = Arc::new(ScriptedProvider::new([Decision::Approve]));
    let (_guard, _gateway) = start_instance(&mock, provider.clone(), &dir);

    let mut alice = Client::connect(&dir);
    let mut bob = Client::connect(&dir);
    alice.initialize(1);
    bob.initialize(2);

    // Alice's request is approved and sent.
    let verdict = alice.request_message(3, "shared gateway alpha");
    assert_eq!(verdict["status"], "sent");
    assert_eq!(mock.post_count(), 1);

    // Bob retries the identical request over his own connection: the shared
    // gateway dedupes it — one popup, one send.
    let verdict = bob.request_message(4, "shared gateway alpha");
    assert_eq!(verdict["status"], "sent");
    assert_eq!(verdict["duplicate"], true);
    assert_eq!(mock.post_count(), 1);
    assert_eq!(provider.call_count(), 1);
}

#[test]
fn deny_cooldown_blocks_across_socket_clients() {
    let mock = mock_slack();
    let dir = state_dir("cooldown");
    let provider = Arc::new(ScriptedProvider::new([Decision::Deny]));
    let (_guard, _gateway) = start_instance(&mock, provider.clone(), &dir);

    let mut alice = Client::connect(&dir);
    let mut bob = Client::connect(&dir);
    alice.initialize(1);
    bob.initialize(2);

    let verdict = alice.request_message(3, "Deploy finished, please review");
    assert_eq!(verdict["status"], "denied");
    assert_eq!(verdict["cooldown_remaining_secs"], 600);

    // A reworded retry from a different client is blocked without a popup.
    let verdict = bob.request_message(4, "Deploy finished, kindly review");
    assert_eq!(verdict["status"], "cooldown_blocked");
    assert!(verdict["cooldown_remaining_secs"].as_u64().unwrap() <= 600);
    assert_eq!(provider.call_count(), 1);
    assert_eq!(mock.post_count(), 0);
}
