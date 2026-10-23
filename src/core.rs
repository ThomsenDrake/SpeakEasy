//! Core gateway logic: request validation, purpose fingerprinting, persisted
//! deny cooldowns, persisted approval dedupe, and decision routing.
//!
//! The public types in this module are the contract used by `slack`, `mcp`,
//! and `ui`. Implement the `unimplemented!()` bodies per `docs/SPEC.md`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// How long a denied purpose stays blocked, in seconds (10 minutes).
pub const COOLDOWN_SECS: u64 = 600;

/// How long sent-approval records are retained for duplicate suppression.
pub const SENT_RETENTION_SECS: u64 = 7 * 24 * 3600;

/// Similarity threshold above which two normalized messages are considered
/// the same underlying purpose for cooldown matching.
pub const SIMILARITY_THRESHOLD: f64 = 0.85;

/// Context from the originating agent conversation (NOT a Slack thread).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationContext {
    /// Title of the originating agent conversation.
    pub title: String,
    /// ID or link of the originating agent conversation.
    pub id_or_link: String,
    /// Brief relevant summary of the originating agent conversation.
    pub summary: String,
}

/// A message request as submitted by an agent (before the gateway assigns an ID).
#[derive(Debug, Clone)]
pub struct IncomingRequest {
    /// The exact proposed message text. Sent verbatim on approval.
    pub message: String,
    /// Intended Slack recipient: channel (`#name` or ID) or user (`@name` or ID).
    pub destination: String,
    /// Destination Slack thread timestamp, if the message is a thread reply.
    pub thread_ts: Option<String>,
    /// Agent-supplied reason for sending.
    pub reason: String,
    /// Identity of the requesting agent.
    pub agent: String,
    /// Context from the originating agent conversation.
    pub conversation: ConversationContext,
}

/// A message request with its gateway-assigned ID, shown to the human.
#[derive(Debug, Clone)]
pub struct MessageRequest {
    pub id: String,
    pub incoming: IncomingRequest,
    /// Unix timestamp (seconds) when the gateway received the request.
    pub received_at: u64,
}

/// The human's decision on a pending request.
#[derive(Debug, Clone)]
pub enum Decision {
    /// Send the exact message to the exact destination, once.
    Approve,
    /// Send nothing; block the same purpose for [`COOLDOWN_SECS`].
    Deny,
    /// Send nothing; return the feedback text to the requesting agent.
    Feedback(String),
}

/// The decision provider failed or was withdrawn (e.g. the app is exiting).
/// This must never be treated as approval.
#[derive(Debug)]
pub struct ProviderError(pub String);

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ProviderError {}

/// Receives requests and produces human decisions. Implementations must be
/// thread-safe: `request_decision` may be called concurrently from multiple
/// MCP worker threads and must block until a decision is available. Exactly
/// one decision must be returned per request; a repeated decision for the
/// same request must be ignored by the caller.
pub trait DecisionProvider: Send + Sync {
    fn request_decision(&self, request: &MessageRequest) -> Result<Decision, ProviderError>;
}

/// A successfully sent Slack message.
#[derive(Debug, Clone)]
pub struct SlackSend {
    pub channel: String,
    pub ts: String,
    pub permalink: String,
}

#[derive(Debug)]
pub struct SlackError(pub String);

impl std::fmt::Display for SlackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SlackError {}

/// The exclusive path for sending Slack messages. The gateway is the only
/// component holding Slack credentials; agents never see this trait's
/// implementation or the token.
pub trait SlackClient: Send + Sync {
    fn post_message(
        &self,
        destination: &str,
        thread_ts: Option<&str>,
        text: &str,
    ) -> Result<SlackSend, SlackError>;
}

/// The outcome of handling one agent request, serialized back to the agent.
#[derive(Debug, Clone, PartialEq)]
pub enum RequestVerdict {
    /// The exact message was sent to the exact destination. `duplicate` is
    /// true when this is a retry of an already-approved identical request:
    /// the cached result is returned and nothing is sent again.
    Sent {
        ts: String,
        permalink: String,
        duplicate: bool,
    },
    /// The human denied the request. Nothing was sent; the same purpose is
    /// blocked for `cooldown_remaining_secs` (initially [`COOLDOWN_SECS`]).
    Denied { cooldown_remaining_secs: u64 },
    /// The human gave feedback. Nothing was sent; the agent receives the text.
    Feedback { feedback: String },
    /// A matching deny cooldown is active. No popup was shown and nothing was
    /// sent; the agent receives the remaining cooldown.
    CooldownBlocked { cooldown_remaining_secs: u64 },
    /// Something failed (Slack error, provider error, I/O error). Nothing was
    /// sent unless a `Sent` record already existed.
    Error(String),
}

/// The approval gateway. Holds the Slack client, the decision provider, and
/// the persisted state (deny cooldowns and sent-approval records).
pub struct Gateway {
    slack: Arc<dyn SlackClient>,
    provider: Arc<dyn DecisionProvider>,
    state: Mutex<GatewayState>,
    /// Serializes the check-and-send critical section so two identical
    /// concurrently approved requests cannot both send.
    send_lock: Mutex<()>,
    state_dir: PathBuf,
}

struct GatewayState {
    cooldowns: Vec<CooldownEntry>,
    sent: Vec<SentRecord>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct CooldownEntry {
    destination: String,
    normalized: String,
    excerpt: String,
    created_at: u64,
    expires_at: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SentRecord {
    message_hash: String,
    destination: String,
    thread_ts: Option<String>,
    ts: String,
    permalink: String,
    sent_at: u64,
}

impl Gateway {
    /// Create a gateway, loading persisted cooldowns and sent records from
    /// `state_dir` (creating the directory if needed) and pruning expired
    /// entries.
    pub fn new(
        slack: Arc<dyn SlackClient>,
        provider: Arc<dyn DecisionProvider>,
        state_dir: PathBuf,
    ) -> std::io::Result<Self> {
        unimplemented!()
    }

    /// Handle one agent request end to end. See `docs/SPEC.md` for the exact
    /// decision flow (cooldown check, dedupe check, provider call, send /
    /// cooldown / feedback, persistence, concurrency rules).
    pub fn handle_request(&self, req: IncomingRequest) -> RequestVerdict {
        unimplemented!()
    }

    /// Currently active deny cooldowns as `(destination, excerpt, remaining_secs)`.
    /// Used by the UI status line and by tests.
    pub fn active_cooldowns(&self) -> Vec<(String, String, u64)> {
        unimplemented!()
    }
}

/// Normalize free text for purpose comparison: lowercase, keep alphanumerics,
/// collapse whitespace.
pub fn normalize_text(input: &str) -> String {
    unimplemented!()
}

/// Similarity between two already-normalized strings, `0.0..=1.0`.
/// Implement as a Levenshtein-based ratio (cap input length for performance).
pub fn similarity(a: &str, b: &str) -> f64 {
    unimplemented!()
}

/// Token-set (Jaccard) similarity between two already-normalized strings.
pub fn token_similarity(a: &str, b: &str) -> f64 {
    unimplemented!()
}

/// True when two proposed messages should be treated as the same underlying
/// purpose for cooldown matching.
pub fn same_purpose(normalized_a: &str, normalized_b: &str) -> bool {
    unimplemented!()
}

/// Stable hash of the exact (message, destination, thread) triple. Used for
/// duplicate-send suppression. Not cryptographic; do not use for security.
pub fn message_hash(message: &str, destination: &str, thread_ts: Option<&str>) -> String {
    unimplemented!()
}

/// Current unix timestamp in seconds.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
