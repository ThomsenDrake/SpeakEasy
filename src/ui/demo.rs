//! Demo mode (`--demo`): synthetic fixtures for visually inspecting the
//! native popup without a running gateway.
//!
//! The demo is fully isolated from production: it uses a throwaway state dir
//! (never `~/.slack-approval-gateway`), a [`DemoSlack`] client that records
//! sends in memory and never touches the network, no instance lock, no Unix
//! socket, no MCP stdio, and no real Slack credentials. Requests go through
//! the real [`Gateway`] (so cooldown/dedupe/decision plumbing is exercised),
//! but with fixed demo IDs and a fixed receipt time so the popup matches the
//! reference captures.
//!
//! Environment knobs (all optional):
//!
//! - `SLACK_GATEWAY_DEMO_SEED`: `empty` (default `single`), `single_cooldown`
//!   (or `cooldown`), `multi`, `long`, `unresolved`.
//! - `SLACK_GATEWAY_DEMO_STICKY_DECIDING=1`: freeze the popup in the
//!   "submitting" state after a decision (all controls disabled).
//! - `SLACK_GATEWAY_DEMO_FAIL_SUBMIT=1`: make every decision submission fail
//!   (the request is retained and the submission-error line is shown).
//! - `SLACK_GATEWAY_DEMO_STATE_DIR`: reuse a state dir instead of a fresh
//!   temp dir (still never the production dir unless explicitly set).
//! - `SLACK_GATEWAY_DEMO_SIZE=WxH`: override the initial window size
//!   (logical px, clamped to the 520×560 minimum).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::core::{
    normalize_text, ConversationContext, Gateway, IncomingRequest, MessageRequest, SlackClient,
    SlackError, SlackSend, COOLDOWN_SECS,
};

/// Fixed receipt time for demo requests (2024-10-22 16:00:00 UTC), so the
/// "received …" line matches the reference captures regardless of the clock.
pub const DEMO_RECEIVED_AT: u64 = 1729612800;

fn conversation(title: &str, id_or_link: &str, summary: &str) -> ConversationContext {
    ConversationContext {
        title: title.to_string(),
        id_or_link: id_or_link.to_string(),
        summary: summary.to_string(),
    }
}

fn request(id: &str, incoming: IncomingRequest, destination_display: Option<&str>) -> MessageRequest {
    MessageRequest {
        id: id.to_string(),
        incoming,
        received_at: DEMO_RECEIVED_AT,
        destination_display: destination_display.map(str::to_string),
    }
}

fn incoming(
    message: &str,
    destination: &str,
    thread_ts: Option<&str>,
    reason: &str,
    agent: &str,
    conversation: ConversationContext,
) -> IncomingRequest {
    IncomingRequest {
        message: message.to_string(),
        destination: destination.to_string(),
        thread_ts: thread_ts.map(str::to_string),
        reason: reason.to_string(),
        agent: agent.to_string(),
        conversation,
    }
}

/// The reference's demo request (id `demo-0001`).
pub fn demo_request() -> MessageRequest {
    request(
        "demo-0001",
        incoming(
            "Heads-up: gateway v0.3 finished deploying — 12/12 checks green. Rolling out to the remaining workspaces now.",
            "U09CV6A19EH",
            Some("1729612800.123456"),
            "The team asked for deploy notifications; the canary passed and the runbook says to announce.",
            "claude-code",
            conversation(
                "Ship gateway v0.3",
                "https://app.superset.sh/workspaces/speakeasy/tasks/deploy-gateway-v03",
                "Agent finished the release checklist (build, tests, smoke test, changelog) and asks to announce the deploy to the release manager.",
            ),
        ),
        Some("Drake Thomsen (@drake.thomsen)"),
    )
}

/// A long, multiline request with an unbreakable token (id `demo-0002`).
pub fn long_request() -> MessageRequest {
    request(
        "demo-0002",
        incoming(
            "Deploy of release 4.2.1 to production finished successfully.\n\n• api: 12/12 checks green ✅\n• web: 12/12 checks green ✅\n• db migration 042 applied\n\nA very long unbreakable token: eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U\n\nRollout continues to the remaining workspaces.",
            "C0123456789ABCDEF",
            None,
            "The release manager asked for a detailed postmortem-style summary of the deploy, including the full checklist output and the artifact digest, so the whole channel can see exactly what shipped.",
            "codex-cli-agent-with-a-very-long-name",
            conversation(
                "Release 4.2.1 production deploy — postmortem summary thread",
                "https://app.superset.sh/workspaces/speakeasy/tasks/release-4-2-1-prod-deploy-postmortem-summary-042",
                "Agent finished the release checklist (build, tests, smoke test, changelog) and asks to announce the deploy to the release manager. This summary is intentionally long so the panel wraps over several lines and exercises the scroll behavior of the popup when the content is tall.",
            ),
        ),
        Some("#eng-deploys (very-long-channel-name-for-wrapping)"),
    )
}

/// A short request (id `demo-0003`).
pub fn third_request() -> MessageRequest {
    request(
        "demo-0003",
        incoming(
            "Lunch poll: pizza at noon?",
            "#general",
            None,
            "Team lunch coordination.",
            "goose",
            conversation("Lunch", "conv-42", "Deciding lunch."),
        ),
        None,
    )
}

/// The demo request with an unresolvable destination (raw ID shown).
pub fn unresolved_request() -> MessageRequest {
    let mut req = demo_request();
    req.id = "demo-0001".to_string();
    req.incoming.destination = "U09UNKNOWN99".to_string();
    req.incoming.thread_ts = None;
    req.destination_display = None;
    req
}

/// Which fixture set `--demo` seeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoSeed {
    Empty,
    Single,
    SingleCooldown,
    Multi,
    Long,
    Unresolved,
}

impl DemoSeed {
    pub fn from_env() -> Self {
        match std::env::var("SLACK_GATEWAY_DEMO_SEED").as_deref() {
            Ok("empty") => Self::Empty,
            Ok("single_cooldown") | Ok("cooldown") => Self::SingleCooldown,
            Ok("multi") => Self::Multi,
            Ok("long") => Self::Long,
            Ok("unresolved") => Self::Unresolved,
            _ => Self::Single,
        }
    }

    /// The requests to queue, in order.
    pub fn requests(self) -> Vec<MessageRequest> {
        match self {
            Self::Empty => Vec::new(),
            Self::Single | Self::SingleCooldown => vec![demo_request()],
            Self::Multi => vec![demo_request(), long_request(), third_request()],
            Self::Long => vec![long_request()],
            Self::Unresolved => vec![unresolved_request()],
        }
    }

    /// Active deny cooldowns to seed, as `(destination, excerpt, remaining_secs)`.
    pub fn cooldowns(self) -> Vec<(String, String, u64)> {
        match self {
            Self::SingleCooldown => vec![(
                "#eng-deploys".to_string(),
                "Heads-up: gateway v0.3…".to_string(),
                443,
            )],
            Self::Multi => vec![
                (
                    "#eng-deploys".to_string(),
                    "Heads-up: gateway v0.3…".to_string(),
                    443,
                ),
                ("#general".to_string(), "Lunch poll: pizza at noon?".to_string(), 42),
            ],
            _ => Vec::new(),
        }
    }
}

/// `SLACK_GATEWAY_DEMO_SIZE=WxH`: override the demo window's initial inner
/// size (logical px, clamped to the 520×560 minimum) — for inspecting the
/// responsive layout at other window sizes.
pub fn demo_window_size() -> Option<[f32; 2]> {
    let v = std::env::var("SLACK_GATEWAY_DEMO_SIZE").ok()?;
    let (w, h) = v.split_once('x')?;
    let w: f32 = w.trim().parse().ok()?;
    let h: f32 = h.trim().parse().ok()?;
    Some([w.max(520.0), h.max(560.0)])
}

/// Freeze the popup in the "submitting" state after a decision.
pub fn demo_sticky_deciding() -> bool {
    std::env::var("SLACK_GATEWAY_DEMO_STICKY_DECIDING")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

/// Make every decision submission fail (submission-error state).
pub fn demo_fail_submit() -> bool {
    std::env::var("SLACK_GATEWAY_DEMO_FAIL_SUBMIT")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

/// A Slack client that records sends in memory. It never touches the
/// network; approvals "succeed" with a synthetic permalink.
#[derive(Default)]
pub struct DemoSlack {
    /// `(destination, thread_ts, text)` of every send, in order.
    pub sent: Mutex<Vec<(String, Option<String>, String)>>,
}

impl DemoSlack {
    pub fn sends(&self) -> Vec<(String, Option<String>, String)> {
        self.sent.lock().unwrap().clone()
    }
}

impl SlackClient for DemoSlack {
    fn post_message(
        &self,
        destination: &str,
        thread_ts: Option<&str>,
        text: &str,
    ) -> Result<SlackSend, SlackError> {
        let n = {
            let mut sent = self.sent.lock().unwrap();
            sent.push((
                destination.to_string(),
                thread_ts.map(str::to_string),
                text.to_string(),
            ));
            sent.len()
        };
        Ok(SlackSend {
            channel: destination.to_string(),
            ts: format!("1700000000.{n:06}"),
            permalink: format!("https://demo.slack.com/archives/{destination}/p{n:06}"),
        })
    }

    fn resolve_destination(&self, destination: &str) -> Option<String> {
        match destination {
            "U09CV6A19EH" => Some("Drake Thomsen (@drake.thomsen)".to_string()),
            "C0123456789ABCDEF" => {
                Some("#eng-deploys (very-long-channel-name-for-wrapping)".to_string())
            }
            _ => None,
        }
    }
}

/// The demo's state dir: `SLACK_GATEWAY_DEMO_STATE_DIR` if set, else a fresh
/// temp dir. Never the production state dir.
pub fn demo_state_dir() -> PathBuf {
    if let Ok(d) = std::env::var("SLACK_GATEWAY_DEMO_STATE_DIR") {
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    let unique = format!(
        "sag-demo-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    );
    std::env::temp_dir().join(unique)
}

/// Write the seed cooldowns into `dir/cooldowns.json` (the gateway's on-disk
/// format) and an empty `sent.json`, before [`Gateway::new`] reads them.
///
/// The seeded entries' `normalized` purpose deliberately matches no demo
/// request: the reference harness seeded the popup directly, while the demo
/// runs the requests through the real gateway — a matching cooldown would
/// block the request instead of displaying alongside it.
pub fn seed_state(dir: &Path, cooldowns: &[(String, String, u64)]) {
    std::fs::create_dir_all(dir).expect("demo: cannot create state dir");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let entries: Vec<serde_json::Value> = cooldowns
        .iter()
        .map(|(destination, excerpt, remaining)| {
            serde_json::json!({
                "destination": destination,
                "normalized": normalize_text(&format!("{excerpt} — denied earlier")),
                "excerpt": excerpt,
                "created_at": now.saturating_sub(COOLDOWN_SECS.saturating_sub(*remaining)),
                "expires_at": now.saturating_add(*remaining),
            })
        })
        .collect();
    std::fs::write(
        dir.join("cooldowns.json"),
        serde_json::to_string_pretty(&entries).unwrap(),
    )
    .expect("demo: cannot write cooldowns.json");
    std::fs::write(dir.join("sent.json"), "[]").expect("demo: cannot write sent.json");
}

/// Spawn the demo requests through the real gateway (fixed IDs and receipt
/// time). Each call runs on its own worker thread and blocks until the human
/// decides — exactly like an MCP worker.
pub fn spawn_demo_requests(gateway: &Arc<Gateway>, requests: &[MessageRequest]) {
    for req in requests {
        let gateway = Arc::clone(gateway);
        let incoming = req.incoming.clone();
        let id = req.id.clone();
        std::thread::spawn(move || {
            let verdict = gateway.handle_request_with_id(incoming, id, DEMO_RECEIVED_AT);
            eprintln!("slack-approval-gateway: demo request finished: {verdict:?}");
        });
    }
}
