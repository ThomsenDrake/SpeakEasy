//! End-to-end tests for the pre-tool hook mode: the real binary run as
//! `--hook <agent>` (agent hook payload on stdin) -> the instance socket ->
//! `speakeasy/approve` -> `Gateway::handle_approval` (scripted human
//! decisions). SpeakEasy has no Slack connection here (`hooks-only`): the
//! agent's own tool would send.

mod common;

use common::ScriptedProvider;
use serde_json::{json, Value};
use slack_approval_gateway::core::{Decision, DecisionProvider, Gateway};
use slack_approval_gateway::instance;
use slack_approval_gateway::slack::NoSlackClient;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_slack-approval-gateway");

/// A short state dir (the socket path must fit in `sun_path`).
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = PathBuf::from(format!(
            "/tmp/sag-hook-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Run an instance for `dir` in this process (it lives until the test ends).
fn instance(dir: &Path, provider: Arc<dyn DecisionProvider>) -> instance::InstanceGuard {
    // (Tests run the hook with SLACK_GATEWAY_STATE_DIR; the installer pins
    // --state-dir instead, tested in `the_pinned_state_dir_wins`.)
    let (guard, listener) = instance::acquire(dir).expect("become the instance");
    let gateway = Gateway::new(Arc::new(NoSlackClient), provider, dir.to_path_buf()).unwrap();
    instance::serve_connections(Arc::new(gateway), listener);
    guard
}

struct Answer {
    stdout: String,
    stderr: String,
    code: i32,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_str(self.stdout.trim()).unwrap_or_else(|e| {
            panic!("stdout is not JSON ({e}): {:?}", self.stdout)
        })
    }

    /// The deny reason of a Claude-style answer.
    fn claude_deny_reason(&self) -> String {
        assert_eq!(self.code, 2, "a deny exits 2: {:?} {:?}", self.stdout, self.stderr);
        let out = self.json();
        assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        out["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .to_string()
    }
}

fn hook(dir: &Path, agent: &str, wait: &str, payload: &Value) -> Answer {
    hook_env(dir, agent, wait, payload, &[])
}

fn hook_env(dir: &Path, agent: &str, wait: &str, payload: &Value, env: &[(&str, &str)]) -> Answer {
    let mut child = Command::new(BIN)
        .args(["--hook", agent, "--wait", wait])
        .env("SLACK_GATEWAY_STATE_DIR", dir)
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run the hook");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    Answer {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code().unwrap_or(-1),
    }
}

fn send(message: &str) -> Value {
    json!({
        "session_id": "sess-1",
        "cwd": "/work/repo",
        "hook_event_name": "PreToolUse",
        "tool_name": "mcp__plugin_slack_slack__slack_send_message",
        "tool_input": { "channel_id": "C0123", "message": message, "thread_ts": "1700000000.000100" },
    })
}

#[test]
fn approval_lets_the_exact_call_run() {
    let dir = Dir::new();
    let provider = Arc::new(ScriptedProvider::new([Decision::Approve]));
    let _instance = instance(&dir.0, provider.clone());

    let a = hook(&dir.0, "claude", "60", &send("Deploy *done* :tada:"));
    assert_eq!(a.code, 0, "{a:?}", a = (&a.stdout, &a.stderr));
    assert_eq!(a.json()["hookSpecificOutput"]["permissionDecision"], "allow");

    let seen = provider.requests();
    assert_eq!(seen.len(), 1);
    let r = &seen[0].incoming;
    assert_eq!(r.message, "Deploy *done* :tada:");
    assert_eq!(r.destination, "C0123");
    assert_eq!(r.thread_ts.as_deref(), Some("1700000000.000100"));
    assert_eq!(r.agent, "Claude Code");
    assert_eq!(r.conversation.id_or_link, "sess-1");
    assert!(r.reason.contains("slack_send_message"), "{}", r.reason);
}

#[test]
fn deny_blocks_and_a_reworded_retry_is_blocked_without_asking() {
    let dir = Dir::new();
    let provider = Arc::new(ScriptedProvider::new([Decision::Deny]));
    let _instance = instance(&dir.0, provider.clone());

    let reason = hook(&dir.0, "claude", "60", &send("Please merge PR 42 today")).claude_deny_reason();
    assert!(reason.contains("denied"), "{reason}");
    assert!(reason.contains("10 minutes"), "{reason}");

    let reason = hook(&dir.0, "claude", "60", &send("please merge PR #42 today!")).claude_deny_reason();
    assert!(reason.contains("recently denied"), "{reason}");
    assert_eq!(provider.call_count(), 1, "the retry must not reach the human");
}

#[test]
fn feedback_is_returned_to_the_model() {
    let dir = Dir::new();
    let provider = Arc::new(ScriptedProvider::new([Decision::Feedback(
        "Mention the rollback plan".into(),
    )]));
    let _instance = instance(&dir.0, provider.clone());
    let reason = hook(&dir.0, "codex", "60", &send("Shipping now")).claude_deny_reason();
    assert!(reason.contains("Mention the rollback plan"), "{reason}");
}

#[test]
fn other_tools_and_slack_reads_get_no_opinion_and_never_reach_the_human() {
    let dir = Dir::new();
    let provider = Arc::new(ScriptedProvider::new([]));
    let _instance = instance(&dir.0, provider.clone());
    for payload in [
        json!({"tool_name": "Bash", "tool_input": {"command": "echo slack"}}),
        json!({"tool_name": "mcp__plugin_slack_slack__slack_read_channel", "tool_input": {"channel_id": "C1"}}),
        json!({"tool_name": "mcp__slack-approval-gateway__request_slack_message", "tool_input": {"message": "x"}}),
    ] {
        let a = hook(&dir.0, "claude", "60", &payload);
        assert_eq!((a.code, a.stdout.as_str()), (0, ""), "{payload}");
    }
    assert_eq!(provider.call_count(), 0);
}

#[test]
fn no_running_instance_blocks_slack_writes() {
    let dir = Dir::new();
    let reason = hook(&dir.0, "claude", "60", &send("hi")).claude_deny_reason();
    assert!(reason.contains("not running"), "{reason}");
}

#[test]
fn agent_formats_for_vibe_and_goose() {
    let dir = Dir::new();
    let provider = Arc::new(ScriptedProvider::new([Decision::Deny, Decision::Approve]));
    let _instance = instance(&dir.0, provider.clone());

    let vibe_call = json!({"tool_name": "mcp_slack.slack_send_message", "args": {"channel_id": "C9", "message": "first"}});
    let a = hook(&dir.0, "vibe", "60", &vibe_call);
    assert_eq!(a.code, 0, "Vibe denies with JSON on exit 0");
    assert_eq!(a.json()["decision"], "deny");

    let goose_call = json!({"tool_name": "slack__slack_send_message", "tool_input": {"channel_id": "C9", "message": "an unrelated second message"}});
    let a = hook(&dir.0, "goose", "60", &goose_call);
    assert_eq!((a.code, a.stdout.as_str()), (0, ""), "goose allows with empty output");
}

#[test]
fn no_decision_in_time_blocks_before_the_wait_runs_out() {
    let dir = Dir::new();
    // A human who takes far longer than the hook may wait.
    let provider = Arc::new(ScriptedProvider::new([Decision::Approve]).with_delay(Duration::from_secs(60)));
    let _instance = instance(&dir.0, provider.clone());
    let started = Instant::now();
    let reason = hook(&dir.0, "claude", "10", &send("slow")).claude_deny_reason();
    let took = started.elapsed();
    assert!(reason.contains("in time"), "{reason}");
    assert!(took < Duration::from_secs(12), "answered after {took:?}");
}

#[test]
fn two_hooks_stopping_the_same_call_share_one_window() {
    let dir = Dir::new();
    // Slow enough that both hooks are waiting at the same time.
    let provider = Arc::new(
        ScriptedProvider::new([Decision::Approve]).with_delay(Duration::from_millis(800)),
    );
    let _instance = instance(&dir.0, provider.clone());

    // Cursor runs its own hook and the Claude Code hook it imports.
    let mut claude = send("one call, two hooks");
    claude["tool_use_id"] = json!("toolu_1");
    claude["tool_name"] = json!("MCP:slack_send_message");
    let cursor = json!({
        "hook_event_name": "beforeMCPExecution",
        "tool_name": "slack_send_message",
        "tool_input": claude["tool_input"].to_string(),
        "mcp_server_name": "slack",
        "session_id": "sess-1",
        "tool_use_id": "toolu_1",
    });
    let d = dir.0.clone();
    // Cursor runs the imported hook with its own environment.
    let first = std::thread::spawn(move || {
        hook_env(&d, "claude", "60", &claude, &[("CURSOR_VERSION", "2.0")])
    });
    let second = hook(&dir.0, "cursor", "60", &cursor);
    let first = first.join().unwrap();

    assert_eq!(provider.call_count(), 1, "one window for one call");
    assert_eq!(first.json()["hookSpecificOutput"]["permissionDecision"], "allow");
    assert_eq!((second.code, second.json()["permission"].clone()), (0, json!("allow")));
}

#[test]
fn cursor_gets_its_own_deny_shape() {
    let dir = Dir::new();
    let provider = Arc::new(ScriptedProvider::new([Decision::Deny]));
    let _instance = instance(&dir.0, provider.clone());
    let cursor = json!({
        "tool_name": "slack_send_message",
        "tool_input": "{\"channel_id\":\"C1\",\"message\":\"cursor send\"}",
        "mcp_server_name": "slack",
    });
    let a = hook(&dir.0, "cursor", "60", &cursor);
    assert_eq!(a.code, 0);
    assert_eq!(a.json()["permission"], "deny");
    assert!(a.json()["agent_message"].as_str().unwrap().contains("denied"));
    assert_eq!(provider.requests()[0].incoming.message, "cursor send");
}

/// Waits for the requester to go away, like the GUI provider, and records
/// when it was asked and when it saw the cancel.
struct WaitsForCancel {
    asked: std::sync::atomic::AtomicBool,
    cancelled_at: std::sync::Mutex<Option<Instant>>,
}

impl DecisionProvider for WaitsForCancel {
    fn request_decision(
        &self,
        _: &slack_approval_gateway::core::MessageRequest,
    ) -> Result<Decision, slack_approval_gateway::core::ProviderError> {
        unreachable!("hooks ask with a deadline")
    }

    fn request_decision_until(
        &self,
        _: &slack_approval_gateway::core::MessageRequest,
        deadline: Instant,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Decision, slack_approval_gateway::core::ProviderError> {
        self.asked.store(true, Ordering::SeqCst);
        while Instant::now() < deadline {
            if cancel.load(Ordering::Relaxed) {
                *self.cancelled_at.lock().unwrap() = Some(Instant::now());
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(slack_approval_gateway::core::ProviderError("withdrawn".into()))
    }
}

#[test]
fn a_killed_hook_withdraws_its_request() {
    let dir = Dir::new();
    let provider = Arc::new(WaitsForCancel {
        asked: Default::default(),
        cancelled_at: Default::default(),
    });
    let _instance = instance(&dir.0, provider.clone());
    let mut child = Command::new(BIN)
        .args(["--hook", "pi", "--wait", "120"])
        .env("SLACK_GATEWAY_STATE_DIR", &dir.0)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(send("then the user pressed Esc").to_string().as_bytes())
        .unwrap();
    // (The first launch of a freshly built binary can take seconds on macOS.)
    let started = Instant::now();
    while !provider.asked.load(Ordering::SeqCst) {
        assert!(started.elapsed() < Duration::from_secs(30), "the hook never asked");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(provider.cancelled_at.lock().unwrap().is_none(), "still waiting for the human");
    let killed = Instant::now();
    child.kill().unwrap();
    let _ = child.wait();
    for _ in 0..150 {
        if let Some(at) = *provider.cancelled_at.lock().unwrap() {
            assert!(at.duration_since(killed) < Duration::from_secs(3));
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("the request was not withdrawn after its hook died");
}


#[test]
fn the_pinned_state_dir_wins_over_the_environment() {
    let dir = Dir::new();
    let decoy = Dir::new();
    let provider = Arc::new(ScriptedProvider::new([Decision::Approve]));
    let _instance = instance(&dir.0, provider.clone());
    // A project could point SLACK_GATEWAY_STATE_DIR somewhere else; the
    // installer's --state-dir still reaches the real instance.
    let mut child = Command::new(BIN)
        .args(["--hook", "claude", "--wait", "60", "--state-dir"])
        .arg(&dir.0)
        .env("SLACK_GATEWAY_STATE_DIR", &decoy.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(send("pinned").to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(provider.call_count(), 1);
}

#[test]
fn ok_code_replaces_exit_zero_only() {
    let dir = Dir::new();
    let run = |payload: Value| {
        let mut child = Command::new(BIN)
            .args(["--hook", "claude", "--wait", "60", "--ok-code", "10", "--state-dir"])
            .arg(&dir.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
        child.wait_with_output().unwrap().status.code()
    };
    // Not a Slack write: proceed, as 10.
    assert_eq!(run(json!({"tool_name": "Bash", "tool_input": {}})), Some(10));
    // A Slack write with no SpeakEasy running: a block stays 2.
    assert_eq!(run(send("blocked")), Some(2));
}
