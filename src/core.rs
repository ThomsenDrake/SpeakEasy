//! Core gateway logic: request validation, purpose fingerprinting, persisted
//! deny cooldowns, persisted approval dedupe, and decision routing.
//!
//! The public types in this module are the contract used by `slack`, `mcp`,
//! and `ui`. Behavior follows `docs/SPEC.md`.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Once, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// How long a denied purpose stays blocked, in seconds (10 minutes).
pub const COOLDOWN_SECS: u64 = 600;

/// How long sent-approval records are retained for duplicate suppression.
pub const SENT_RETENTION_SECS: u64 = 7 * 24 * 3600;

/// Similarity threshold above which two normalized messages are considered
/// the same underlying purpose for cooldown matching.
pub const SIMILARITY_THRESHOLD: f64 = 0.85;

/// Token-set (Jaccard) threshold above which two normalized messages are
/// considered the same underlying purpose.
const TOKEN_SIMILARITY_THRESHOLD: f64 = 0.7;

/// Per-side prefix length (in chars) compared by [`similarity`], so the
/// quadratic edit distance stays fast on pathological input.
const SIMILARITY_MAX_CHARS: usize = 1000;

/// Length (in chars) of the message excerpt stored with a deny cooldown.
const EXCERPT_CHARS: usize = 80;

/// Slack truncates message text beyond this many characters, so a longer
/// message could not be sent exactly as approved.
const MAX_MESSAGE_CHARS: usize = 40_000;

const COOLDOWNS_FILE: &str = "cooldowns.json";
const SENT_FILE: &str = "sent.json";

/// Advisory lock file guarding read-merge-write of the state files. Every
/// stdio MCP client spawns its own gateway process, so several processes may
/// share one state dir; the lock keeps them from clobbering each other's
/// cooldowns and sent records.
const STATE_LOCK_FILE: &str = "state.lock";

/// Advisory lock file extending `send_lock` across gateway processes, so
/// identical approvals in two processes cannot both send.
const SEND_LOCK_FILE: &str = "send.lock";

/// How long to wait for another process to release `state.lock` (held only
/// for a few file operations) before proceeding without it.
const STATE_LOCK_WAIT: Duration = Duration::from_secs(5);

/// How long to wait for another process to release `send.lock` (held across
/// its Slack call) before failing this send.
const SEND_LOCK_WAIT: Duration = Duration::from_secs(60);

const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Source of the per-process unique suffix of request IDs.
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

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
#[derive(Debug, Clone, Serialize)]
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
#[derive(Debug, Clone, Serialize)]
pub struct MessageRequest {
    pub id: String,
    pub incoming: IncomingRequest,
    /// Unix timestamp (seconds) when the gateway received the request.
    pub received_at: u64,
    /// Human-readable form of `incoming.destination` (e.g. `Drake Thomsen
    /// (@drake.thomsen)` for `U09CV6A19EH`, `#eng` for `C0123`), resolved
    /// best-effort by the Slack client. `None` when the destination is
    /// already readable (`#name`, `@name`) or cannot be resolved; the UI
    /// then shows the raw destination. Never used for sending.
    pub destination_display: Option<String>,
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

    /// Best-effort human-readable form of a destination Slack ID
    /// (`U09CV6A19EH` → `Drake Thomsen (@drake.thomsen)`, `C0123` →
    /// `#eng`), for display in the approval popup. `None` when the
    /// destination is already readable or cannot be resolved (unknown ID,
    /// missing read scopes): the UI then shows the raw destination. Never
    /// used for sending; failures must not fail the request.
    fn resolve_destination(&self, destination: &str) -> Option<String> {
        let _ = destination;
        None
    }
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
    /// Number of approved requests currently between the human's decision
    /// and the end of the send critical section. Used by
    /// [`Gateway::wait_until_no_send_in_progress`] at shutdown.
    approvals_in_flight: AtomicUsize,
    state_dir: PathBuf,
}

struct GatewayState {
    cooldowns: Vec<CooldownEntry>,
    sent: Vec<SentRecord>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct CooldownEntry {
    destination: String,
    normalized: String,
    excerpt: String,
    created_at: u64,
    expires_at: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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
        fs::create_dir_all(&state_dir)?;
        let mut state = {
            let _lock = state_file_lock(&state_dir);
            // A corrupt state file is an error rather than an empty list:
            // silently dropping persisted deny cooldowns would lift them.
            GatewayState {
                cooldowns: read_json(&state_dir.join(COOLDOWNS_FILE))?,
                sent: read_json(&state_dir.join(SENT_FILE))?,
            }
        };
        prune(&mut state, now_secs());
        Ok(Self {
            slack,
            provider,
            state: Mutex::new(state),
            send_lock: Mutex::new(()),
            approvals_in_flight: AtomicUsize::new(0),
            state_dir,
        })
    }

    /// Block until no approved send is in progress. Shutdown aid: a send
    /// holds `send_lock` for its whole critical section (dedupe re-check,
    /// Slack POST, record, persist), and `approvals_in_flight` covers the
    /// short window between the human's decision and acquiring that lock.
    /// A request still awaiting a human decision is not sending anything,
    /// so this never waits on the popup. Returns once no send can be
    /// mid-flight; callers should invoke it before exiting the process.
    pub fn wait_until_no_send_in_progress(&self) {
        while self.approvals_in_flight.load(Ordering::Relaxed) > 0
            || self.send_lock.try_lock().is_err()
        {
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Handle one agent request end to end. See `docs/SPEC.md` for the exact
    /// decision flow (cooldown check, dedupe check, provider call, send /
    /// cooldown / feedback, persistence, concurrency rules).
    pub fn handle_request(&self, req: IncomingRequest) -> RequestVerdict {
        let now = now_secs();
        let id = format!(
            "req-{now}-{}",
            NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        );
        self.handle_request_with_id(req, id, now)
    }

    /// Handle one agent request end to end with a caller-chosen request ID
    /// and receipt time. Same flow as [`Self::handle_request`]; the native
    /// demo mode uses fixed demo IDs (and receipt time) so its popup matches
    /// the reference captures. `received_at` is display-only: cooldowns and
    /// dedupe always compare against the current time.
    pub fn handle_request_with_id(
        &self,
        req: IncomingRequest,
        id: String,
        received_at: u64,
    ) -> RequestVerdict {
        if let Err(problem) = validate(&req) {
            return RequestVerdict::Error(format!("{problem}; nothing was sent"));
        }

        let now = now_secs();
        let normalized = normalize_text(&req.message);
        let hash = message_hash(&req.message, &req.destination, req.thread_ts.as_deref());

        {
            let mut state = self.lock_state();
            if let Err(e) = self.reload(&mut state, now) {
                // The send path re-checks strictly; a deny or feedback
                // needs no sent records.
                eprintln!("slack-approval-gateway: cannot reload sent records: {e}");
            }

            // 1. Cooldown check. The key is (destination, purpose) only: agent
            // identity, reason, request ID and thread are deliberately ignored
            // so reworded requests and retries from other agents still match.
            // With several matching entries the longest remaining wait wins.
            let destination = destination_key(&req.destination);
            let remaining = state
                .cooldowns
                .iter()
                .filter(|c| {
                    cooldown_active(c, now)
                        && destination_key(&c.destination) == destination
                        && same_purpose(&c.normalized, &normalized)
                })
                .map(|c| c.expires_at - now)
                .max();
            if let Some(remaining) = remaining {
                // A deny recorded after `now` was taken can be 1s "longer".
                return RequestVerdict::CooldownBlocked {
                    cooldown_remaining_secs: remaining.min(COOLDOWN_SECS),
                };
            }

            // 2. Dedupe check: an identical approved request was already sent.
            if let Some(record) = find_sent(&state.sent, &hash, &req) {
                return duplicate_verdict(record);
            }
        }

        // 3. Ask the human. The state lock is released: this may block for a
        // long time and other requests must keep flowing. The destination's
        // display name is resolved best-effort for the popup; a failure
        // changes nothing (the raw destination is shown).
        let destination_display = self.slack.resolve_destination(&req.destination);
        let msg = MessageRequest {
            id,
            incoming: req,
            received_at,
            destination_display,
        };
        let decision = self.provider.request_decision(&msg);
        let req = msg.incoming;

        // 4. Act on the decision. Anything but an explicit approval sends nothing.
        match decision {
            Ok(Decision::Approve) => self.send_approved(&req, hash),
            Ok(Decision::Deny) => self.record_denial(&req, normalized),
            Ok(Decision::Feedback(feedback)) => RequestVerdict::Feedback { feedback },
            Err(ProviderError(e)) => RequestVerdict::Error(e),
        }
    }

    /// Currently active deny cooldowns as `(destination, excerpt, remaining_secs)`.
    /// Used by the UI status line and by tests.
    pub fn active_cooldowns(&self) -> Vec<(String, String, u64)> {
        let now = now_secs();
        let state = self.lock_state();
        state
            .cooldowns
            .iter()
            .filter(|c| cooldown_active(c, now))
            .map(|c| (c.destination.clone(), c.excerpt.clone(), c.expires_at - now))
            .collect()
    }

    /// Send an approved request exactly once. `send_lock` (and `send.lock`
    /// across processes) serializes the dedupe re-check, the Slack call and
    /// the record, so two identical concurrently approved requests cannot
    /// both send. The state lock is only taken around the bookkeeping, never
    /// across the Slack network call. Fails closed when the cross-process
    /// lock cannot be taken or the sent records cannot be read; only a
    /// filesystem without locking support falls back to in-process
    /// protection.
    fn send_approved(&self, req: &IncomingRequest, hash: String) -> RequestVerdict {
        self.approvals_in_flight.fetch_add(1, Ordering::Relaxed);
        let result = self.send_approved_locked(req, hash);
        self.approvals_in_flight.fetch_sub(1, Ordering::Relaxed);
        result
    }

    fn send_approved_locked(&self, req: &IncomingRequest, hash: String) -> RequestVerdict {
        let _send_guard = self
            .send_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let _send_file_lock = match lock_file(&self.state_dir, SEND_LOCK_FILE, SEND_LOCK_WAIT) {
            Ok(lock) => lock,
            Err(e) => {
                eprintln!("slack-approval-gateway: {e}");
                return RequestVerdict::Error(
                    "the gateway's send lock is unavailable (another gateway process may still \
                     be sending); nothing was sent"
                        .into(),
                );
            }
        };

        {
            let mut state = self.lock_state();
            if let Err(e) = self.reload(&mut state, now_secs()) {
                eprintln!("slack-approval-gateway: cannot read sent records: {e}");
                return RequestVerdict::Error(
                    "the gateway cannot read its sent records, so a duplicate send cannot be \
                     ruled out; nothing was sent"
                        .into(),
                );
            }
            if let Some(record) = find_sent(&state.sent, &hash, req) {
                return duplicate_verdict(record);
            }
        }

        let sent =
            match self
                .slack
                .post_message(&req.destination, req.thread_ts.as_deref(), &req.message)
            {
                Ok(sent) => sent,
                Err(SlackError(e)) => {
                    return RequestVerdict::Error(format!("Slack send failed: {e}"))
                }
            };

        let now = now_secs();
        let mut state = self.lock_state();
        state.sent.push(SentRecord {
            message_hash: hash,
            destination: req.destination.clone(),
            thread_ts: req.thread_ts.clone(),
            ts: sent.ts.clone(),
            permalink: sent.permalink.clone(),
            sent_at: now,
        });
        // The message is already in Slack, so report the send even if the
        // record cannot be written; in-memory dedupe still applies.
        let result = self.persist(SENT_FILE, &mut state.sent, |r| sent_retained(r, now));
        if let Err(e) = result {
            eprintln!(
                "slack-approval-gateway: message sent but {SENT_FILE} could not be written \
                 (the record is lost on restart unless a later write succeeds): {e}"
            );
        }
        RequestVerdict::Sent {
            ts: sent.ts,
            permalink: sent.permalink,
            duplicate: false,
        }
    }

    /// Record a deny cooldown for the request's (destination, purpose).
    fn record_denial(&self, req: &IncomingRequest, normalized: String) -> RequestVerdict {
        // The cooldown starts at the human's decision, not at receipt: the
        // popup may have been open longer than COOLDOWN_SECS, and the agent
        // is told the full COOLDOWN_SECS remain.
        let now = now_secs();
        let mut state = self.lock_state();
        state.cooldowns.push(CooldownEntry {
            destination: req.destination.trim().to_string(),
            normalized,
            excerpt: req.message.chars().take(EXCERPT_CHARS).collect(),
            created_at: now,
            expires_at: now + COOLDOWN_SECS,
        });
        let result = self.persist(COOLDOWNS_FILE, &mut state.cooldowns, |c| {
            cooldown_active(c, now)
        });
        if let Err(e) = result {
            eprintln!(
                "slack-approval-gateway: deny cooldown is enforced but {COOLDOWNS_FILE} could not be written \
                 (it is lost on restart unless a later write succeeds): {e}"
            );
        }
        RequestVerdict::Denied {
            cooldown_remaining_secs: COOLDOWN_SECS,
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, GatewayState> {
        // A panic elsewhere must not wedge the gateway; every critical
        // section leaves the state consistent.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Merge entries written by other gateway processes sharing the state
    /// dir, then prune expired ones. A cooldowns read failure is logged and
    /// the in-memory cooldowns are used; a sent-records read failure is
    /// returned.
    fn reload(&self, state: &mut GatewayState, now: u64) -> io::Result<()> {
        let _lock = state_file_lock(&self.state_dir);
        if let Err(e) = merge_from_disk(&self.state_dir.join(COOLDOWNS_FILE), &mut state.cooldowns)
        {
            eprintln!("slack-approval-gateway: cannot reload cooldowns: {e}");
        }
        let sent = merge_from_disk(&self.state_dir.join(SENT_FILE), &mut state.sent);
        prune(state, now);
        sent
    }

    /// Atomically rewrite the state file `file` as the union of `entries` and
    /// what is on disk, keeping only entries that pass `keep`. Called with
    /// the state lock held. A file that cannot be read is not overwritten
    /// (that would drop other processes' entries) unless it is corrupt, in
    /// which case a copy is kept as `<file>.corrupt-<unix time>`.
    fn persist<T: Serialize + DeserializeOwned + PartialEq>(
        &self,
        file: &str,
        entries: &mut Vec<T>,
        keep: impl FnMut(&T) -> bool,
    ) -> io::Result<()> {
        let _lock = state_file_lock(&self.state_dir);
        let path = self.state_dir.join(file);
        let mut corrupt_copy = None;
        if let Err(e) = merge_from_disk(&path, entries) {
            if e.kind() != io::ErrorKind::InvalidData {
                return Err(e);
            }
            // Copy rather than rename: the original stays in place until
            // the rewrite below succeeds.
            let mut copy = path.as_os_str().to_owned();
            copy.push(format!(".corrupt-{}", now_secs()));
            fs::copy(&path, &copy)?;
            corrupt_copy = Some((e, PathBuf::from(copy)));
        }
        entries.retain(keep);
        write_json_atomic(&path, entries)?;
        if let Some((e, copy)) = corrupt_copy {
            eprintln!(
                "slack-approval-gateway: {e}; rewrote it and kept the corrupt file as {}",
                copy.display()
            );
        }
        Ok(())
    }
}

/// Reject requests that cannot be sent exactly as approved.
fn validate(req: &IncomingRequest) -> Result<(), String> {
    let visible = |c: char| !c.is_whitespace() && !is_invisible_or_mark(c);
    if !req.message.chars().any(visible) {
        return Err("message has no visible text".into());
    }
    if req.message.chars().count() > MAX_MESSAGE_CHARS {
        return Err(format!(
            "message is longer than Slack's {MAX_MESSAGE_CHARS}-character limit and would be truncated"
        ));
    }
    if req.destination.trim().is_empty() {
        return Err("destination must not be empty".into());
    }
    Ok(())
}

/// Destination as compared for cooldown matching: trimmed, one leading `#`
/// dropped and ASCII-lowercased, so `#eng`, `eng` and `#ENG` are one
/// destination. Sending and dedupe use the exact destination.
fn destination_key(destination: &str) -> String {
    let d = destination.trim();
    d.strip_prefix('#').unwrap_or(d).trim().to_ascii_lowercase()
}

fn cooldown_active(entry: &CooldownEntry, now: u64) -> bool {
    entry.expires_at > now
}

fn sent_retained(record: &SentRecord, now: u64) -> bool {
    now.saturating_sub(record.sent_at) <= SENT_RETENTION_SECS
}

/// Drop expired cooldowns and sent records past their retention.
fn prune(state: &mut GatewayState, now: u64) {
    state.cooldowns.retain(|c| cooldown_active(c, now));
    state.sent.retain(|r| sent_retained(r, now));
}

fn find_sent<'a>(
    sent: &'a [SentRecord],
    hash: &str,
    req: &IncomingRequest,
) -> Option<&'a SentRecord> {
    sent.iter().find(|r| {
        r.message_hash == hash && r.destination == req.destination && r.thread_ts == req.thread_ts
    })
}

fn duplicate_verdict(record: &SentRecord) -> RequestVerdict {
    RequestVerdict::Sent {
        ts: record.ts.clone(),
        permalink: record.permalink.clone(),
        duplicate: true,
    }
}

/// The cross-process state lock, best effort: on failure (logged) the caller
/// proceeds without it. Reads are safe against atomic renames and writers use
/// unique temp files, so the worst case is a lost update between processes,
/// which is better than an unpersisted deny.
fn state_file_lock(dir: &Path) -> Option<fs::File> {
    lock_file(dir, STATE_LOCK_FILE, STATE_LOCK_WAIT).unwrap_or_else(|e| {
        eprintln!("slack-approval-gateway: {e}");
        None
    })
}

/// Take the advisory cross-process lock `<dir>/<name>`, waiting at most
/// `wait`; released when the returned file is dropped. `Ok(None)` when the
/// filesystem does not support locking (logged once); the in-process
/// mutexes still apply then.
fn lock_file(dir: &Path, name: &str, wait: Duration) -> io::Result<Option<fs::File>> {
    static WARN_UNSUPPORTED: Once = Once::new();
    let path = dir.join(name);
    let in_context =
        |e: io::Error| io::Error::new(e.kind(), format!("cannot lock {}: {e}", path.display()));
    fs::create_dir_all(dir).map_err(in_context)?;
    // A read-only descriptor is enough for flock where the lock file is not
    // writable (e.g. created by another user).
    let file = match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
    {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => fs::File::open(&path),
        opened => opened,
    }
    .map_err(in_context)?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(file)),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                thread::sleep(LOCK_POLL_INTERVAL)
            }
            Err(TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("timed out after {wait:?} waiting for {}", path.display()),
                ))
            }
            Err(TryLockError::Error(e)) if e.kind() == io::ErrorKind::Interrupted => {}
            // The file is open, so anything else means this filesystem
            // cannot lock (e.g. ENOTSUP on an NFS `nolocks` mount).
            Err(TryLockError::Error(e)) => {
                WARN_UNSUPPORTED.call_once(|| {
                    eprintln!(
                        "slack-approval-gateway: {}; gateway processes sharing this state dir \
                         are not coordinated",
                        in_context(e)
                    )
                });
                return Ok(None);
            }
        }
    }
}

/// Read a JSON array state file; a missing file is an empty list.
fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<Vec<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {e}", path.display()),
            )
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(io::Error::new(e.kind(), format!("{}: {e}", path.display()))),
    }
}

/// Add on-disk entries missing from `entries` (union; entries are only ever
/// removed by pruning).
fn merge_from_disk<T: DeserializeOwned + PartialEq>(
    path: &Path,
    entries: &mut Vec<T>,
) -> io::Result<()> {
    for entry in read_json::<T>(path)? {
        if !entries.contains(&entry) {
            entries.push(entry);
        }
    }
    Ok(())
}

/// Write `value` as JSON to a temp file next to `path`, flush it to disk,
/// then rename it over `path`, so readers never see a partially written
/// file. The temp name is unique per write, so concurrent writers (e.g. two
/// processes without a working lock) never share one.
fn write_json_atomic<T: Serialize + ?Sized>(path: &Path, value: &T) -> io::Result<()> {
    static NEXT_TMP: AtomicU64 = AtomicU64::new(0);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        NEXT_TMP.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = PathBuf::from(tmp);
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Normalize free text for purpose comparison: lowercase, keep alphanumerics,
/// collapse whitespace.
pub fn normalize_text(input: &str) -> String {
    // Invisible format characters and combining marks are dropped rather
    // than turned into spaces, so interleaving them (or decomposing accents)
    // cannot split words apart. Decomposed accents still differ from
    // precomposed letters ("e" + U+0301 becomes "e", not "é").
    let mapped: String = input
        .to_lowercase()
        .chars()
        .filter(|&c| c.is_alphanumeric() || !is_invisible_or_mark(c))
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Default-ignorable format characters (zero-width, bidi controls, variation
/// selectors, tags) and the combining diacritical mark blocks.
fn is_invisible_or_mark(c: char) -> bool {
    matches!(
        u32::from(c),
        0x00AD
            | 0x0300..=0x036F
            | 0x061C
            | 0x1AB0..=0x1AFF
            | 0x180B..=0x180F
            | 0x1DC0..=0x1DFF
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0x20D0..=0x20FF
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
            | 0xFEFF
            | 0xE0000..=0xE0FFF
    )
}

/// Similarity between two already-normalized strings, `0.0..=1.0`: a
/// Levenshtein ratio over at most the first 1000 chars of each side.
pub fn similarity(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().take(SIMILARITY_MAX_CHARS).collect();
    let b: Vec<char> = b.chars().take(SIMILARITY_MAX_CHARS).collect();
    let max_len = a.len().max(b.len());
    if max_len == 0 {
        return 1.0;
    }
    1.0 - levenshtein(&a, &b) as f64 / max_len as f64
}

/// Edit distance (insert / delete / substitute, each cost 1) over chars.
fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitute = prev[j] + usize::from(ca != cb);
            cur[j + 1] = substitute.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Token-set (Jaccard) similarity between two already-normalized strings.
/// Token-set similarity between two already-normalized strings,
/// `0.0..=1.0`. Sørensen–Dice over the token sets
/// (`2 * |A ∩ B| / (|A| + |B|)`): unlike Jaccard it stays high when a
/// rewording swaps one or two words in a short message ("please review"
/// -> "kindly review"), which is exactly the retry shape a deny cooldown
/// must catch.
pub fn token_similarity(a: &str, b: &str) -> f64 {
    let a: HashSet<&str> = a.split_whitespace().collect();
    let b: HashSet<&str> = b.split_whitespace().collect();
    let total = a.len() + b.len();
    if total == 0 {
        return 1.0;
    }
    2.0 * a.intersection(&b).count() as f64 / total as f64
}

/// True when two proposed messages should be treated as the same underlying
/// purpose for cooldown matching.
pub fn same_purpose(normalized_a: &str, normalized_b: &str) -> bool {
    // Cheapest checks first; the quadratic edit distance runs last.
    normalized_a == normalized_b
        || token_similarity(normalized_a, normalized_b) >= TOKEN_SIMILARITY_THRESHOLD
        || similarity(normalized_a, normalized_b) >= SIMILARITY_THRESHOLD
}

/// Stable hash of the exact (message, destination, thread) triple. Used for
/// duplicate-send suppression. Not cryptographic; do not use for security.
pub fn message_hash(message: &str, destination: &str, thread_ts: Option<&str>) -> String {
    // 128-bit FNV-1a over a length-prefixed encoding of the triple. Hashes are
    // persisted in sent.json, so the algorithm must not change between Rust
    // releases (std's DefaultHasher makes no such guarantee). Length prefixes
    // keep field boundaries unambiguous, and None differs from Some("").
    const OFFSET_BASIS: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    fn feed(hash: &mut u128, bytes: &[u8]) {
        for &b in bytes {
            *hash ^= u128::from(b);
            *hash = hash.wrapping_mul(PRIME);
        }
    }
    fn feed_str(hash: &mut u128, s: &str) {
        feed(hash, &(s.len() as u64).to_le_bytes());
        feed(hash, s.as_bytes());
    }

    let mut hash = OFFSET_BASIS;
    feed_str(&mut hash, message);
    feed_str(&mut hash, destination);
    match thread_ts {
        None => feed(&mut hash, &[0]),
        Some(ts) => {
            feed(&mut hash, &[1]);
            feed_str(&mut hash, ts);
        }
    }
    format!("{hash:032x}")
}

/// Current unix timestamp in seconds.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::mpsc;
    use std::sync::Condvar;
    use std::thread;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(10);

    /// A unique scratch state dir, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "sag-core-{}-{name}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&dir);
            Self(dir)
        }

        fn path(&self) -> PathBuf {
            self.0.clone()
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    struct PostCall {
        destination: String,
        thread_ts: Option<String>,
        text: String,
    }

    /// Lets a test pause `MockSlack::post_message` mid-call.
    struct Gate {
        entered: Mutex<mpsc::Sender<()>>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    /// Records every `post_message` call (including failed ones).
    #[derive(Default)]
    struct MockSlack {
        calls: Mutex<Vec<PostCall>>,
        fail_with: Option<String>,
        gate: Option<Gate>,
    }

    impl MockSlack {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        fn failing(error: &str) -> Arc<Self> {
            Arc::new(Self {
                fail_with: Some(error.to_string()),
                ..Self::default()
            })
        }

        /// Returns the mock plus (entered, release) handles for the gate.
        fn gated() -> (Arc<Self>, mpsc::Receiver<()>, mpsc::Sender<()>) {
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let slack = Arc::new(Self {
                gate: Some(Gate {
                    entered: Mutex::new(entered_tx),
                    release: Mutex::new(release_rx),
                }),
                ..Self::default()
            });
            (slack, entered_rx, release_tx)
        }

        fn calls(&self) -> Vec<PostCall> {
            self.calls.lock().unwrap().clone()
        }

        fn count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl SlackClient for MockSlack {
        fn post_message(
            &self,
            destination: &str,
            thread_ts: Option<&str>,
            text: &str,
        ) -> Result<SlackSend, SlackError> {
            let n = {
                let mut calls = self.calls.lock().unwrap();
                calls.push(PostCall {
                    destination: destination.to_string(),
                    thread_ts: thread_ts.map(str::to_string),
                    text: text.to_string(),
                });
                calls.len()
            };
            if let Some(gate) = &self.gate {
                gate.entered.lock().unwrap().send(()).unwrap();
                gate.release.lock().unwrap().recv_timeout(WAIT).unwrap();
            }
            if let Some(error) = &self.fail_with {
                return Err(SlackError(error.clone()));
            }
            Ok(SlackSend {
                channel: destination.to_string(),
                ts: format!("1700000000.{n:06}"),
                permalink: format!(
                    "https://example.slack.com/archives/{destination}/p1700000000{n:06}"
                ),
            })
        }
    }

    /// Returns scripted decisions in order and records every request it saw.
    struct ScriptedProvider {
        script: Mutex<VecDeque<Result<Decision, ProviderError>>>,
        seen: Mutex<Vec<MessageRequest>>,
    }

    impl ScriptedProvider {
        fn new(script: Vec<Result<Decision, ProviderError>>) -> Arc<Self> {
            Arc::new(Self {
                script: Mutex::new(script.into()),
                seen: Mutex::new(Vec::new()),
            })
        }

        fn calls(&self) -> usize {
            self.seen.lock().unwrap().len()
        }

        fn seen(&self) -> Vec<MessageRequest> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl DecisionProvider for ScriptedProvider {
        fn request_decision(&self, request: &MessageRequest) -> Result<Decision, ProviderError> {
            self.seen.lock().unwrap().push(request.clone());
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(ProviderError("unexpected provider call".into())))
        }
    }

    /// Waits until `parties` requests are in flight, then decides each one
    /// from its message text — so concurrent requests overlap for real. The
    /// wait is bounded so a lock regression fails instead of hanging.
    struct BarrierProvider {
        parties: usize,
        arrived: Mutex<usize>,
        all_arrived: Condvar,
    }

    impl BarrierProvider {
        fn new(parties: usize) -> Arc<Self> {
            Arc::new(Self {
                parties,
                arrived: Mutex::new(0),
                all_arrived: Condvar::new(),
            })
        }

        fn calls(&self) -> usize {
            *self.arrived.lock().unwrap()
        }
    }

    impl DecisionProvider for BarrierProvider {
        fn request_decision(&self, request: &MessageRequest) -> Result<Decision, ProviderError> {
            let mut arrived = self.arrived.lock().unwrap();
            *arrived += 1;
            self.all_arrived.notify_all();
            let (_arrived, timeout) = self
                .all_arrived
                .wait_timeout_while(arrived, WAIT, |n| *n < self.parties)
                .unwrap();
            if timeout.timed_out() {
                return Err(ProviderError("barrier timed out".into()));
            }
            let message = &request.incoming.message;
            Ok(if message.contains("deny") {
                Decision::Deny
            } else if message.contains("feedback") {
                Decision::Feedback(format!("feedback for: {message}"))
            } else {
                Decision::Approve
            })
        }
    }

    /// Blocks inside `request_decision` until the test releases a decision.
    struct GatedProvider {
        entered: Mutex<mpsc::Sender<()>>,
        release: Mutex<mpsc::Receiver<Decision>>,
    }

    impl DecisionProvider for GatedProvider {
        fn request_decision(&self, _request: &MessageRequest) -> Result<Decision, ProviderError> {
            self.entered.lock().unwrap().send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(WAIT)
                .map_err(|e| ProviderError(e.to_string()))
        }
    }

    fn request(message: &str, destination: &str, thread_ts: Option<&str>) -> IncomingRequest {
        IncomingRequest {
            message: message.to_string(),
            destination: destination.to_string(),
            thread_ts: thread_ts.map(str::to_string),
            reason: "tell the team".to_string(),
            agent: "agent-a".to_string(),
            conversation: ConversationContext {
                title: "Release checklist".to_string(),
                id_or_link: "conv-123".to_string(),
                summary: "Agent finished the deploy".to_string(),
            },
        }
    }

    fn from_agent(mut req: IncomingRequest, agent: &str) -> IncomingRequest {
        req.agent = agent.to_string();
        req.reason = format!("{agent} has its own reason");
        req.conversation.id_or_link = format!("{agent}-conversation");
        req
    }

    fn gateway(dir: &TempDir, slack: &Arc<MockSlack>, provider: &Arc<ScriptedProvider>) -> Gateway {
        Gateway::new(slack.clone(), provider.clone(), dir.path()).expect("create gateway")
    }

    fn read_entries<T: DeserializeOwned>(path: &Path) -> Vec<T> {
        read_json(path).expect("valid state file")
    }

    fn assert_no_temp_files(dir: &TempDir) {
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    /// Overwrite every cooldown's expiry, in memory and on disk.
    fn set_cooldown_expiry(gw: &Gateway, expires_at: u64) {
        let mut state = gw.lock_state();
        for c in &mut state.cooldowns {
            c.expires_at = expires_at;
        }
        write_json_atomic(&gw.state_dir.join(COOLDOWNS_FILE), &state.cooldowns).unwrap();
    }

    fn sent(ts: &str, permalink: &str, duplicate: bool) -> RequestVerdict {
        RequestVerdict::Sent {
            ts: ts.to_string(),
            permalink: permalink.to_string(),
            duplicate,
        }
    }

    fn assert_blocked(verdict: &RequestVerdict, max: u64) {
        match verdict {
            RequestVerdict::CooldownBlocked {
                cooldown_remaining_secs,
            } => assert!(
                (max - 2..=max).contains(cooldown_remaining_secs),
                "remaining {cooldown_remaining_secs} not within 2s of {max}"
            ),
            other => panic!("expected CooldownBlocked, got {other:?}"),
        }
    }

    /// Handles every request on its own thread; verdicts in request order.
    /// Bounded, so a deadlock fails the test instead of hanging it.
    fn run_concurrently(gw: &Arc<Gateway>, reqs: Vec<IncomingRequest>) -> Vec<RequestVerdict> {
        let (tx, rx) = mpsc::channel();
        let n = reqs.len();
        for (i, req) in reqs.into_iter().enumerate() {
            let (gw, tx) = (Arc::clone(gw), tx.clone());
            thread::spawn(move || tx.send((i, gw.handle_request(req))).unwrap());
        }
        let mut verdicts: Vec<_> = (0..n)
            .map(|_| rx.recv_timeout(WAIT).expect("request did not finish"))
            .collect();
        verdicts.sort_by_key(|(i, _)| *i);
        verdicts.into_iter().map(|(_, v)| v).collect()
    }

    // ---- text fingerprinting ------------------------------------------------

    #[test]
    fn normalize_text_lowercases_maps_symbols_to_spaces_and_collapses() {
        assert_eq!(
            normalize_text("  Hello, World!!  Deploy #42\tnow\n"),
            "hello world deploy 42 now"
        );
        assert_eq!(normalize_text("Ünïcode—STRASSE_ok"), "ünïcode strasse ok");
        assert_eq!(normalize_text("can't-stop"), "can t stop");
        assert_eq!(normalize_text("!!! ... ???"), "");
        assert_eq!(normalize_text(""), "");
    }

    #[test]
    fn similarity_is_a_levenshtein_ratio_over_chars() {
        assert_eq!(similarity("abc", "abc"), 1.0);
        assert_eq!(similarity("", ""), 1.0);
        assert_eq!(similarity("abc", ""), 0.0);
        assert_eq!(similarity("", "abc"), 0.0);
        assert!((similarity("kitten", "sitting") - (1.0 - 3.0 / 7.0)).abs() < 1e-12);
        assert!((similarity("flaw", "lawn") - 0.5).abs() < 1e-12);
        // Chars, not bytes: one substitution of a multi-byte char.
        assert!((similarity("café", "cafe") - 0.75).abs() < 1e-12);
        assert!(
            similarity("deploy finished on main", "deploy finishd on main") >= SIMILARITY_THRESHOLD
        );
        assert!(similarity("build passed", "build failed") < SIMILARITY_THRESHOLD);
    }

    #[test]
    fn similarity_compares_only_capped_prefixes() {
        let prefix = "a".repeat(SIMILARITY_MAX_CHARS);
        let a = format!("{prefix}{}", "x".repeat(50_000));
        let b = format!("{prefix}{}", "y".repeat(50_000));
        let started = std::time::Instant::now();
        assert_eq!(similarity(&a, &b), 1.0);
        assert!(started.elapsed() < Duration::from_secs(5));
        // Below the cap, every char counts.
        let short_a = format!("{}x", "a".repeat(9));
        let short_b = format!("{}y", "a".repeat(9));
        assert!((similarity(&short_a, &short_b) - 0.9).abs() < 1e-12);
    }

    #[test]
    fn token_similarity_is_sorensen_dice_over_token_sets() {
        assert_eq!(token_similarity("a b c", "b c d"), 2.0 / 3.0);
        assert_eq!(token_similarity("a a b", "b a"), 1.0);
        assert_eq!(token_similarity("a b", "c d"), 0.0);
        assert_eq!(token_similarity("", ""), 1.0);
        assert_eq!(token_similarity("a", ""), 0.0);
        // A one-word rewording of a short message stays above the threshold.
        assert!(token_similarity("deploy finished please review",
                                "deploy finished kindly review") >= TOKEN_SIMILARITY_THRESHOLD);
    }

    #[test]
    fn same_purpose_matches_equal_reworded_and_typo_variants_only() {
        let n = normalize_text;
        assert!(same_purpose("", ""));
        assert!(same_purpose(
            &n("Deploy of API v2 finished successfully."),
            &n("Successfully finished: deploy of api v2!")
        ));
        assert!(same_purpose(
            &n("The nightly build failed on main"),
            &n("the nightly build has failed on main")
        ));
        assert!(same_purpose(
            &n("Deploying release 4.2 to production now"),
            &n("Deploying relase 4.2 to prodution now")
        ));
        assert!(!same_purpose(
            &n("Lunch is at noon"),
            &n("The nightly build failed on main")
        ));
        assert!(!same_purpose(&n("build passed"), &n("build failed")));
    }

    #[test]
    fn message_hash_is_stable_and_covers_the_exact_triple() {
        let h = message_hash("Deploy done", "#deploys", Some("1700000000.000100"));
        assert_eq!(
            h,
            message_hash("Deploy done", "#deploys", Some("1700000000.000100"))
        );
        assert_eq!(h.len(), 32);
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        // Pinned: hashes are persisted, so the algorithm must never drift.
        assert_eq!(
            message_hash("", "", None),
            "f5a535e74106dd2ef3685728151008bf"
        );
        assert_eq!(h, "298db774625dce1194002d88f3ecb5a7");

        let others = [
            message_hash("Deploy done!", "#deploys", Some("1700000000.000100")),
            message_hash("deploy done", "#deploys", Some("1700000000.000100")),
            message_hash("Deploy done", "#deploys ", Some("1700000000.000100")),
            message_hash("Deploy done", "#general", Some("1700000000.000100")),
            message_hash("Deploy done", "#deploys", Some("1700000000.000200")),
            message_hash("Deploy done", "#deploys", None),
        ];
        for other in &others {
            assert_ne!(&h, other);
        }
        assert_ne!(
            message_hash("m", "d", None),
            message_hash("m", "d", Some(""))
        );
        assert_ne!(message_hash("ab", "c", None), message_hash("a", "bc", None));
    }

    // ---- approve / dedupe -----------------------------------------------------

    #[test]
    fn approve_sends_exact_text_to_exact_destination_and_thread() {
        let dir = TempDir::new("approve");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Approve)]);
        let gw = gateway(&dir, &slack, &provider);
        let message = "Deploy of *api* v2 finished ✅\n  (exact   spacing kept) ";
        let req = request(message, "#deploys", Some("1700000000.000100"));

        let before = now_secs();
        let verdict = gw.handle_request(req.clone());

        assert_eq!(
            verdict,
            sent(
                "1700000000.000001",
                "https://example.slack.com/archives/#deploys/p1700000000000001",
                false
            )
        );
        assert_eq!(
            slack.calls(),
            vec![PostCall {
                destination: "#deploys".into(),
                thread_ts: Some("1700000000.000100".into()),
                text: message.into(),
            }]
        );
        // The human saw the exact request with a gateway-assigned ID.
        let seen = provider.seen();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].id.starts_with("req-"), "id {}", seen[0].id);
        assert!(seen[0].received_at >= before && seen[0].received_at <= now_secs());
        assert_eq!(seen[0].incoming.message, message);
        assert_eq!(seen[0].incoming.destination, "#deploys");
        assert_eq!(
            seen[0].incoming.thread_ts.as_deref(),
            Some("1700000000.000100")
        );
        assert_eq!(seen[0].incoming.agent, "agent-a");
        assert_eq!(seen[0].incoming.reason, "tell the team");
        assert_eq!(seen[0].incoming.conversation.id_or_link, "conv-123");
        // Persisted atomically.
        let records: Vec<SentRecord> = read_entries(&dir.file(SENT_FILE));
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].message_hash,
            message_hash(message, "#deploys", Some("1700000000.000100"))
        );
        assert_eq!(records[0].destination, "#deploys");
        assert_eq!(records[0].ts, "1700000000.000001");
        assert_no_temp_files(&dir);
        assert!(gw.active_cooldowns().is_empty());
    }

    #[test]
    fn approve_without_thread_passes_none() {
        let dir = TempDir::new("approve-nothread");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Approve)]);
        let gw = gateway(&dir, &slack, &provider);

        gw.handle_request(request("hello", "@alice", None));

        assert_eq!(
            slack.calls(),
            vec![PostCall {
                destination: "@alice".into(),
                thread_ts: None,
                text: "hello".into(),
            }]
        );
    }

    #[test]
    fn identical_retry_returns_cached_duplicate_without_provider_or_slack() {
        let dir = TempDir::new("dedupe");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Approve)]);
        let gw = gateway(&dir, &slack, &provider);
        let req = request("Release 4.2 is live", "#announce", Some("1.2"));

        let first = gw.handle_request(req.clone());
        let retry = gw.handle_request(req.clone());
        // Another agent, reason and conversation, same exact triple.
        let other_agent = gw.handle_request(from_agent(req, "agent-b"));

        let (ts, permalink) = match &first {
            RequestVerdict::Sent {
                ts,
                permalink,
                duplicate: false,
            } => (ts.clone(), permalink.clone()),
            other => panic!("expected a fresh send, got {other:?}"),
        };
        assert_eq!(retry, sent(&ts, &permalink, true));
        assert_eq!(other_agent, sent(&ts, &permalink, true));
        assert_eq!(slack.count(), 1);
        assert_eq!(provider.calls(), 1);
    }

    #[test]
    fn any_change_to_message_destination_or_thread_needs_fresh_approval() {
        let dir = TempDir::new("fresh");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new((0..4).map(|_| Ok(Decision::Approve)).collect());
        let gw = gateway(&dir, &slack, &provider);

        let variants = [
            request("Release 4.2 is live", "#announce", None),
            request("Release 4.2 is live!", "#announce", None),
            request("Release 4.2 is live", "#general", None),
            request("Release 4.2 is live", "#announce", Some("1.2")),
        ];
        for (i, req) in variants.iter().enumerate() {
            match gw.handle_request(req.clone()) {
                RequestVerdict::Sent {
                    duplicate: false, ..
                } => {}
                other => panic!("variant {i}: expected a fresh send, got {other:?}"),
            }
            assert_eq!(provider.calls(), i + 1);
        }
        let expected: Vec<PostCall> = variants
            .iter()
            .map(|r| PostCall {
                destination: r.destination.clone(),
                thread_ts: r.thread_ts.clone(),
                text: r.message.clone(),
            })
            .collect();
        assert_eq!(slack.calls(), expected);
    }

    #[test]
    fn concurrent_identical_approvals_send_exactly_once() {
        let dir = TempDir::new("concurrent-same");
        let slack = MockSlack::new();
        let provider = BarrierProvider::new(2);
        let gw = Arc::new(Gateway::new(slack.clone(), provider.clone(), dir.path()).unwrap());
        let req = request("Release 4.2 is live", "#announce", None);

        let verdicts = run_concurrently(&gw, vec![req.clone(), req]);

        assert_eq!(provider.calls(), 2, "both requests reached the human");
        assert_eq!(slack.count(), 1, "identical approvals must not double-send");
        let fresh: Vec<_> = verdicts
            .iter()
            .filter(|v| {
                matches!(
                    v,
                    RequestVerdict::Sent {
                        duplicate: false,
                        ..
                    }
                )
            })
            .collect();
        let dupes: Vec<_> = verdicts
            .iter()
            .filter(|v| {
                matches!(
                    v,
                    RequestVerdict::Sent {
                        duplicate: true,
                        ..
                    }
                )
            })
            .collect();
        assert_eq!((fresh.len(), dupes.len()), (1, 1), "{verdicts:?}");
        match (fresh[0], dupes[0]) {
            (
                RequestVerdict::Sent {
                    ts: a,
                    permalink: pa,
                    ..
                },
                RequestVerdict::Sent {
                    ts: b,
                    permalink: pb,
                    ..
                },
            ) => assert_eq!((a, pa), (b, pb)),
            _ => unreachable!(),
        }
    }

    #[test]
    fn concurrent_distinct_requests_get_their_own_decisions() {
        let dir = TempDir::new("concurrent-mixed");
        let slack = MockSlack::new();
        let provider = BarrierProvider::new(3);
        let gw = Arc::new(Gateway::new(slack.clone(), provider.clone(), dir.path()).unwrap());
        let messages = [
            "please approve the release notes for 4.2",
            "please deny this lunch poll announcement",
            "please send feedback about the outage summary",
        ];

        let verdicts = run_concurrently(
            &gw,
            messages
                .iter()
                .map(|m| request(m, "#general", None))
                .collect(),
        );

        assert!(
            matches!(
                verdicts[0],
                RequestVerdict::Sent {
                    duplicate: false,
                    ..
                }
            ),
            "{verdicts:?}"
        );
        assert_eq!(
            verdicts[1],
            RequestVerdict::Denied {
                cooldown_remaining_secs: COOLDOWN_SECS
            }
        );
        assert_eq!(
            verdicts[2],
            RequestVerdict::Feedback {
                feedback: format!("feedback for: {}", messages[2])
            }
        );
        assert_eq!(
            slack.calls(),
            vec![PostCall {
                destination: "#general".into(),
                thread_ts: None,
                text: messages[0].into(),
            }]
        );
        let cooldowns = gw.active_cooldowns();
        assert_eq!(cooldowns.len(), 1);
        assert_eq!(cooldowns[0].1, messages[1]);
    }

    // ---- deny / cooldowns -----------------------------------------------------

    #[test]
    fn deny_sends_nothing_and_sets_a_persisted_cooldown() {
        let dir = TempDir::new("deny");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Deny)]);
        let gw = gateway(&dir, &slack, &provider);
        let message = format!("Heads up: {}", "é".repeat(100));

        let before = now_secs();
        let verdict = gw.handle_request(request(&message, "  #general ", None));

        assert_eq!(
            verdict,
            RequestVerdict::Denied {
                cooldown_remaining_secs: COOLDOWN_SECS
            }
        );
        assert_eq!(slack.count(), 0);
        let excerpt: String = message.chars().take(80).collect();
        let active = gw.active_cooldowns();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].0, "#general");
        assert_eq!(active[0].1, excerpt);
        assert!(
            (COOLDOWN_SECS - 2..=COOLDOWN_SECS).contains(&active[0].2),
            "{active:?}"
        );

        let entries: Vec<CooldownEntry> = read_entries(&dir.file(COOLDOWNS_FILE));
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.destination, "#general");
        assert_eq!(entry.normalized, normalize_text(&message));
        assert_eq!(entry.excerpt, excerpt);
        assert!(entry.created_at >= before && entry.created_at <= now_secs());
        assert_eq!(entry.expires_at, entry.created_at + COOLDOWN_SECS);
        assert_no_temp_files(&dir);
        assert!(!dir.file(SENT_FILE).exists());
    }

    #[test]
    fn reworded_retry_from_another_agent_is_blocked_without_a_popup() {
        let dir = TempDir::new("reworded");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Deny)]);
        let gw = gateway(&dir, &slack, &provider);
        gw.handle_request(request(
            "Heads up: the nightly build failed on main, investigating now",
            "#eng",
            None,
        ));

        let reworded = from_agent(
            request(
                "heads-up — Nightly build FAILED on main; investigating now!!",
                " #eng",
                None,
            ),
            "agent-b",
        );
        assert_blocked(&gw.handle_request(reworded), COOLDOWN_SECS);
        // Identical retry and a different thread in the same destination too.
        assert_blocked(
            &gw.handle_request(request(
                "Heads up: the nightly build failed on main, investigating now",
                "#eng",
                Some("1700000000.000100"),
            )),
            COOLDOWN_SECS,
        );
        assert_eq!(provider.calls(), 1, "no popup while the cooldown is active");
        assert_eq!(slack.count(), 0);
    }

    #[test]
    fn same_message_to_a_different_destination_is_not_blocked() {
        let dir = TempDir::new("other-dest");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Deny), Ok(Decision::Approve)]);
        let gw = gateway(&dir, &slack, &provider);
        let message = "The nightly build failed on main";
        gw.handle_request(request(message, "#eng", None));

        let verdict = gw.handle_request(request(message, "#random", None));

        assert!(
            matches!(
                verdict,
                RequestVerdict::Sent {
                    duplicate: false,
                    ..
                }
            ),
            "{verdict:?}"
        );
        assert_eq!(provider.calls(), 2);
        assert_eq!(slack.calls()[0].destination, "#random");
    }

    #[test]
    fn unrelated_message_to_the_same_destination_is_not_blocked() {
        let dir = TempDir::new("unrelated");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![
            Ok(Decision::Deny),
            Ok(Decision::Feedback("later".into())),
        ]);
        let gw = gateway(&dir, &slack, &provider);
        gw.handle_request(request("The nightly build failed on main", "#eng", None));

        let verdict = gw.handle_request(request("Lunch is at noon in the big room", "#eng", None));

        assert_eq!(
            verdict,
            RequestVerdict::Feedback {
                feedback: "later".into()
            }
        );
        assert_eq!(provider.calls(), 2);
    }

    #[test]
    fn blocked_request_reports_the_remaining_cooldown() {
        let dir = TempDir::new("remaining");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Deny)]);
        let gw = gateway(&dir, &slack, &provider);
        let req = request("The nightly build failed on main", "#eng", None);
        gw.handle_request(req.clone());

        set_cooldown_expiry(&gw, now_secs() + 42);

        assert_blocked(&gw.handle_request(req), 42);
        assert_eq!(provider.calls(), 1);
    }

    #[test]
    fn expired_cooldown_no_longer_blocks() {
        let dir = TempDir::new("expired-runtime");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Deny), Ok(Decision::Approve)]);
        let gw = gateway(&dir, &slack, &provider);
        let req = request("The nightly build failed on main", "#eng", None);
        gw.handle_request(req.clone());

        set_cooldown_expiry(&gw, now_secs());

        assert!(gw.active_cooldowns().is_empty());
        assert!(matches!(
            gw.handle_request(req),
            RequestVerdict::Sent {
                duplicate: false,
                ..
            }
        ));
        assert_eq!(provider.calls(), 2);
        assert_eq!(slack.count(), 1);
    }

    #[test]
    fn expired_entries_are_pruned_on_load() {
        let dir = TempDir::new("expired-load");
        fs::create_dir_all(dir.path()).unwrap();
        let now = now_secs();
        let message = "The nightly build failed on main";
        let cooldowns = vec![
            CooldownEntry {
                destination: "#eng".into(),
                normalized: normalize_text(message),
                excerpt: message.into(),
                created_at: now - COOLDOWN_SECS - 5,
                expires_at: now - 5,
            },
            CooldownEntry {
                destination: "#ops".into(),
                normalized: normalize_text(message),
                excerpt: message.into(),
                created_at: now - 100,
                expires_at: now + 500,
            },
        ];
        write_json_atomic(&dir.file(COOLDOWNS_FILE), &cooldowns).unwrap();
        let record = |sent_at, ts: &str| SentRecord {
            message_hash: message_hash(message, &format!("#{ts}"), None),
            destination: format!("#{ts}"),
            thread_ts: None,
            ts: ts.into(),
            permalink: format!("https://example.slack.com/archives/{ts}"),
            sent_at,
        };
        let records = vec![
            record(now - SENT_RETENTION_SECS - 10, "old"),
            record(now - 10, "recent"),
        ];
        write_json_atomic(&dir.file(SENT_FILE), &records).unwrap();
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Approve)]);
        let gw = gateway(&dir, &slack, &provider);

        let active = gw.active_cooldowns();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].0, "#ops");
        assert!((498..=500).contains(&active[0].2));
        assert_eq!(gw.lock_state().sent, vec![records[1].clone()]);

        // The expired cooldown and the stale sent record no longer apply...
        assert!(matches!(
            gw.handle_request(request(message, "#old", None)),
            RequestVerdict::Sent {
                duplicate: false,
                ..
            }
        ));
        // ...while the live ones still do.
        assert_eq!(
            gw.handle_request(request(message, "#recent", None)),
            sent("recent", "https://example.slack.com/archives/recent", true)
        );
        assert_blocked(&gw.handle_request(request(message, "#ops", None)), 500);
        assert!(matches!(
            gw.handle_request(request(message, "#eng", None)),
            RequestVerdict::Error(_)
        ));
        assert_eq!(
            provider.calls(),
            2,
            "#old and #eng reached the human; #eng hit an empty script"
        );
    }

    #[test]
    fn cooldowns_and_sent_records_survive_a_restart() {
        let dir = TempDir::new("restart");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Deny), Ok(Decision::Approve)]);
        let denied = request("The nightly build failed on main", "#eng", None);
        let approved = request("Release 4.2 is live", "#announce", None);
        let first_send = {
            let gw = gateway(&dir, &slack, &provider);
            gw.handle_request(denied.clone());
            gw.handle_request(approved.clone())
        };

        let provider2 = ScriptedProvider::new(vec![]);
        let gw2 = gateway(&dir, &slack, &provider2);

        assert_blocked(
            &gw2.handle_request(from_agent(
                request("the nightly build has failed on main", "#eng", None),
                "agent-c",
            )),
            COOLDOWN_SECS,
        );
        let RequestVerdict::Sent { ts, permalink, .. } = first_send else {
            panic!("expected a send, got {first_send:?}");
        };
        assert_eq!(gw2.handle_request(approved), sent(&ts, &permalink, true));
        assert_eq!(provider2.calls(), 0);
        assert_eq!(slack.count(), 1);
        assert_eq!(gw2.active_cooldowns().len(), 1);
    }

    #[test]
    fn gateways_sharing_a_state_dir_see_and_keep_each_others_state() {
        let dir = TempDir::new("shared");
        let slack = MockSlack::new();
        let provider_a = ScriptedProvider::new(vec![Ok(Decision::Deny), Ok(Decision::Approve)]);
        let provider_b = ScriptedProvider::new(vec![Ok(Decision::Deny)]);
        // Both load an empty dir, as two gateway processes started together would.
        let gw_a = gateway(&dir, &slack, &provider_a);
        let gw_b = gateway(&dir, &slack, &provider_b);

        gw_a.handle_request(request("The nightly build failed on main", "#eng", None));
        assert_blocked(
            &gw_b.handle_request(request(
                "the nightly build has failed on main",
                "#eng",
                None,
            )),
            COOLDOWN_SECS,
        );
        gw_b.handle_request(request("Lunch is at noon in the big room", "#eng", None));
        let release = request("Release 4.2 is live", "#announce", None);
        let fresh = gw_a.handle_request(release.clone());
        assert!(matches!(
            &fresh,
            RequestVerdict::Sent {
                duplicate: false,
                ..
            }
        ));
        assert!(matches!(
            gw_b.handle_request(release),
            RequestVerdict::Sent {
                duplicate: true,
                ..
            }
        ));

        assert_eq!(provider_a.calls(), 2);
        assert_eq!(provider_b.calls(), 1);
        assert_eq!(slack.count(), 1);
        // B's write did not clobber A's cooldown on disk.
        let on_disk: Vec<CooldownEntry> = read_entries(&dir.file(COOLDOWNS_FILE));
        assert_eq!(on_disk.len(), 2, "{on_disk:?}");
        let gw_c = gateway(&dir, &slack, &ScriptedProvider::new(vec![]));
        assert_eq!(gw_c.active_cooldowns().len(), 2);
    }

    // ---- feedback / errors ----------------------------------------------------

    #[test]
    fn feedback_returns_text_and_sends_nothing() {
        let dir = TempDir::new("feedback");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![
            Ok(Decision::Feedback("Mention the ticket number".into())),
            Ok(Decision::Approve),
        ]);
        let gw = gateway(&dir, &slack, &provider);

        let verdict = gw.handle_request(request("Release 4.2 is live", "#announce", None));

        assert_eq!(
            verdict,
            RequestVerdict::Feedback {
                feedback: "Mention the ticket number".into()
            }
        );
        assert_eq!(slack.count(), 0);
        assert!(gw.active_cooldowns().is_empty());
        // The revised request is a new request needing a new approval.
        let revised = gw.handle_request(request("Release 4.2 is live (REL-42)", "#announce", None));
        assert!(matches!(
            revised,
            RequestVerdict::Sent {
                duplicate: false,
                ..
            }
        ));
        assert_eq!(provider.calls(), 2);
        assert_eq!(slack.calls()[0].text, "Release 4.2 is live (REL-42)");
    }

    #[test]
    fn provider_error_is_an_error_and_never_sends() {
        let dir = TempDir::new("provider-error");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![
            Err(ProviderError(
                "gateway UI is not available; no decision was made".into(),
            )),
            Ok(Decision::Approve),
        ]);
        let gw = gateway(&dir, &slack, &provider);
        let req = request("Release 4.2 is live", "#announce", None);

        let verdict = gw.handle_request(req.clone());

        assert_eq!(
            verdict,
            RequestVerdict::Error("gateway UI is not available; no decision was made".into())
        );
        assert_eq!(slack.count(), 0);
        assert!(gw.active_cooldowns().is_empty());
        assert!(!dir.file(SENT_FILE).exists());
        assert!(!dir.file(COOLDOWNS_FILE).exists());
        // Nothing was recorded, so a retry asks the human again.
        assert!(matches!(
            gw.handle_request(req),
            RequestVerdict::Sent {
                duplicate: false,
                ..
            }
        ));
        assert_eq!(provider.calls(), 2);
    }

    #[test]
    fn slack_error_is_an_error_and_records_nothing() {
        let dir = TempDir::new("slack-error");
        let slack = MockSlack::failing("channel_not_found");
        let provider = ScriptedProvider::new(vec![Ok(Decision::Approve), Ok(Decision::Approve)]);
        let gw = gateway(&dir, &slack, &provider);
        let req = request("Release 4.2 is live", "#nope", None);

        match gw.handle_request(req.clone()) {
            RequestVerdict::Error(e) => assert!(e.contains("channel_not_found"), "{e}"),
            other => panic!("expected Error, got {other:?}"),
        }
        assert!(gw.lock_state().sent.is_empty());
        assert!(!dir.file(SENT_FILE).exists());
        assert!(matches!(gw.handle_request(req), RequestVerdict::Error(_)));
        assert_eq!(
            provider.calls(),
            2,
            "a failed send is not cached as a success"
        );
        assert_eq!(slack.count(), 2);
    }

    #[test]
    fn blank_message_or_destination_is_rejected_without_a_popup() {
        let dir = TempDir::new("blank");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![]);
        let gw = gateway(&dir, &slack, &provider);

        assert!(matches!(
            gw.handle_request(request(" \n\t", "#eng", None)),
            RequestVerdict::Error(_)
        ));
        assert!(matches!(
            gw.handle_request(request("hello", "  ", None)),
            RequestVerdict::Error(_)
        ));
        assert_eq!(provider.calls(), 0);
        assert_eq!(slack.count(), 0);
    }

    #[test]
    fn request_ids_are_unique() {
        let dir = TempDir::new("ids");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![
            Ok(Decision::Feedback("a".into())),
            Ok(Decision::Feedback("b".into())),
        ]);
        let gw = gateway(&dir, &slack, &provider);
        let req = request("Release 4.2 is live", "#announce", None);

        gw.handle_request(req.clone());
        gw.handle_request(req);

        let seen = provider.seen();
        assert_ne!(seen[0].id, seen[1].id);
    }

    // ---- state dir / persistence ----------------------------------------------

    #[test]
    fn new_creates_a_missing_state_dir() {
        let dir = TempDir::new("mkdir");
        let nested = dir.path().join("a").join("b");
        Gateway::new(
            MockSlack::new(),
            ScriptedProvider::new(vec![]),
            nested.clone(),
        )
        .unwrap();
        assert!(nested.is_dir());
    }

    #[test]
    fn corrupt_state_file_fails_creation() {
        let dir = TempDir::new("corrupt");
        fs::create_dir_all(dir.path()).unwrap();
        fs::write(dir.file(COOLDOWNS_FILE), "not json").unwrap();

        let err = Gateway::new(MockSlack::new(), ScriptedProvider::new(vec![]), dir.path())
            .err()
            .expect("corrupt cooldowns must not be silently dropped");

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains(COOLDOWNS_FILE), "{err}");
    }

    // ---- lock discipline ------------------------------------------------------

    /// Runs `probe` on another thread and fails if it does not finish in time
    /// (it would hang if the state lock were held by the paused request).
    fn assert_completes<T: Send + 'static>(probe: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(probe());
        });
        rx.recv_timeout(WAIT)
            .expect("state lock must not be held across this call")
    }

    fn write_active_cooldown(dir: &TempDir, destination: &str, message: &str) {
        fs::create_dir_all(dir.path()).unwrap();
        let now = now_secs();
        let entry = CooldownEntry {
            destination: destination.into(),
            normalized: normalize_text(message),
            excerpt: message.into(),
            created_at: now,
            expires_at: now + COOLDOWN_SECS,
        };
        write_json_atomic(&dir.file(COOLDOWNS_FILE), &[entry]).unwrap();
    }

    #[test]
    fn state_lock_is_not_held_while_waiting_for_the_human() {
        let dir = TempDir::new("lock-provider");
        write_active_cooldown(&dir, "#blocked", "spam spam spam");
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let provider = Arc::new(GatedProvider {
            entered: Mutex::new(entered_tx),
            release: Mutex::new(release_rx),
        });
        let slack = MockSlack::new();
        let gw = Arc::new(Gateway::new(slack.clone(), provider, dir.path()).unwrap());

        let pending = {
            let gw = Arc::clone(&gw);
            thread::spawn(move || {
                gw.handle_request(request("Release 4.2 is live", "#announce", None))
            })
        };
        entered_rx.recv_timeout(WAIT).unwrap();

        let probe_gw = Arc::clone(&gw);
        let (cooldowns, blocked) = assert_completes(move || {
            (
                probe_gw.active_cooldowns(),
                probe_gw.handle_request(request("spam spam spam", "#blocked", None)),
            )
        });
        assert_eq!(cooldowns.len(), 1);
        assert_blocked(&blocked, COOLDOWN_SECS);

        release_tx.send(Decision::Approve).unwrap();
        assert!(matches!(
            pending.join().unwrap(),
            RequestVerdict::Sent {
                duplicate: false,
                ..
            }
        ));
        assert_eq!(slack.count(), 1);
    }

    #[test]
    fn state_lock_is_not_held_during_the_slack_call() {
        let dir = TempDir::new("lock-slack");
        write_active_cooldown(&dir, "#blocked", "spam spam spam");
        let (slack, entered_rx, release_tx) = MockSlack::gated();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Approve)]);
        let gw = Arc::new(Gateway::new(slack.clone(), provider, dir.path()).unwrap());

        let pending = {
            let gw = Arc::clone(&gw);
            thread::spawn(move || {
                gw.handle_request(request("Release 4.2 is live", "#announce", None))
            })
        };
        entered_rx.recv_timeout(WAIT).unwrap();

        let probe_gw = Arc::clone(&gw);
        let (cooldowns, blocked) = assert_completes(move || {
            (
                probe_gw.active_cooldowns(),
                probe_gw.handle_request(request("spam spam spam", "#blocked", None)),
            )
        });
        assert_eq!(cooldowns.len(), 1);
        assert_blocked(&blocked, COOLDOWN_SECS);

        release_tx.send(()).unwrap();
        assert!(matches!(
            pending.join().unwrap(),
            RequestVerdict::Sent {
                duplicate: false,
                ..
            }
        ));
    }

    // ---- send path exclusivity ------------------------------------------------

    /// Handles `req` on its own thread; the verdict arrives on the receiver.
    fn spawn_request(gw: &Arc<Gateway>, req: IncomingRequest) -> mpsc::Receiver<RequestVerdict> {
        let (tx, rx) = mpsc::channel();
        let gw = Arc::clone(gw);
        thread::spawn(move || tx.send(gw.handle_request(req)).unwrap());
        rx
    }

    fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + WAIT;
        while !condition() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn identical_approval_waits_for_the_in_flight_send_and_is_a_duplicate() {
        let dir = TempDir::new("send-lock");
        let (slack, entered_rx, release_tx) = MockSlack::gated();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Approve), Ok(Decision::Approve)]);
        let gw = Arc::new(Gateway::new(slack.clone(), provider.clone(), dir.path()).unwrap());
        let req = request("Release 4.2 is live", "#announce", None);

        let first = spawn_request(&gw, req.clone());
        entered_rx.recv_timeout(WAIT).unwrap(); // in post_message, not yet recorded
        let second = spawn_request(&gw, req);
        wait_until("the second approval", || provider.calls() == 2);

        assert!(
            entered_rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "an identical approval reached Slack while the first send was in flight"
        );
        release_tx.send(()).unwrap();
        let first = first.recv_timeout(WAIT).unwrap();
        let RequestVerdict::Sent {
            ts,
            permalink,
            duplicate: false,
        } = &first
        else {
            panic!("expected a fresh send, got {first:?}");
        };
        assert_eq!(
            second.recv_timeout(WAIT).unwrap(),
            sent(ts, permalink, true)
        );
        assert_eq!(slack.count(), 1);
    }

    #[test]
    fn identical_approvals_in_two_gateways_on_one_state_dir_send_once() {
        // Two Gateway instances stand in for two gateway processes: each has
        // its own send_lock, so only the cross-process lock can serialize them.
        let dir = TempDir::new("send-lock-shared");
        let (slack, entered_rx, release_tx) = MockSlack::gated();
        let provider_a = ScriptedProvider::new(vec![Ok(Decision::Approve)]);
        let provider_b = ScriptedProvider::new(vec![Ok(Decision::Approve)]);
        let gw_a = Arc::new(Gateway::new(slack.clone(), provider_a, dir.path()).unwrap());
        let gw_b = Arc::new(Gateway::new(slack.clone(), provider_b.clone(), dir.path()).unwrap());
        let req = request("Release 4.2 is live", "#announce", None);

        let first = spawn_request(&gw_a, req.clone());
        entered_rx.recv_timeout(WAIT).unwrap();
        let second = spawn_request(&gw_b, req);
        wait_until("the second approval", || provider_b.calls() == 1);

        assert!(
            entered_rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "the other gateway sent while the first send was in flight"
        );
        release_tx.send(()).unwrap();
        let first = first.recv_timeout(WAIT).unwrap();
        let RequestVerdict::Sent {
            ts,
            permalink,
            duplicate: false,
        } = &first
        else {
            panic!("expected a fresh send, got {first:?}");
        };
        assert_eq!(
            second.recv_timeout(WAIT).unwrap(),
            sent(ts, permalink, true)
        );
        assert_eq!(slack.count(), 1);
    }

    #[test]
    fn send_fails_closed_when_the_send_lock_cannot_be_taken() {
        let dir = TempDir::new("send-lock-broken");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Approve), Ok(Decision::Approve)]);
        let gw = gateway(&dir, &slack, &provider);
        let req = request("Release 4.2 is live", "#announce", None);
        fs::create_dir(dir.file(SEND_LOCK_FILE)).unwrap();

        match gw.handle_request(req.clone()) {
            RequestVerdict::Error(e) => assert!(e.contains("nothing was sent"), "{e}"),
            other => panic!("expected Error, got {other:?}"),
        }
        assert_eq!(slack.count(), 0);

        fs::remove_dir(dir.file(SEND_LOCK_FILE)).unwrap();
        assert!(matches!(
            gw.handle_request(req),
            RequestVerdict::Sent {
                duplicate: false,
                ..
            }
        ));
        assert_eq!(slack.count(), 1);
    }

    // ---- persistence failures ---------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn sent_record_that_fails_to_persist_still_dedupes_in_memory() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("sent-unwritable");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Approve)]);
        let gw = gateway(&dir, &slack, &provider);
        fs::write(dir.file(SEND_LOCK_FILE), "").unwrap();
        // A read-only state dir makes the atomic write fail.
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
        let writable = fs::write(dir.file("probe"), "").is_ok();
        let req = request("Release 4.2 is live", "#announce", None);

        let first = gw.handle_request(req.clone());
        let retry = gw.handle_request(req);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();

        let RequestVerdict::Sent {
            ts,
            permalink,
            duplicate: false,
        } = &first
        else {
            panic!("the message was sent, so the verdict must say so: {first:?}");
        };
        if !writable {
            assert!(!dir.file(SENT_FILE).exists());
        }
        assert_eq!(retry, sent(ts, permalink, true));
        assert_eq!(slack.count(), 1);
        assert_eq!(provider.calls(), 1);
    }

    #[test]
    fn deny_is_persisted_even_when_the_state_lock_is_unavailable() {
        let dir = TempDir::new("state-lock-broken");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Deny)]);
        let gw = gateway(&dir, &slack, &provider);
        fs::remove_file(dir.file(STATE_LOCK_FILE)).unwrap();
        fs::create_dir(dir.file(STATE_LOCK_FILE)).unwrap();
        let message = "The nightly build failed on main";

        let verdict = gw.handle_request(request(message, "#eng", None));

        assert_eq!(
            verdict,
            RequestVerdict::Denied {
                cooldown_remaining_secs: COOLDOWN_SECS
            }
        );
        let on_disk: Vec<CooldownEntry> = read_entries(&dir.file(COOLDOWNS_FILE));
        assert_eq!(on_disk.len(), 1);
        assert_blocked(
            &gw.handle_request(request(message, "#eng", None)),
            COOLDOWN_SECS,
        );
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_state_file_is_not_overwritten() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("unreadable");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new((0..3).map(|_| Ok(Decision::Deny)).collect());
        let gw = gateway(&dir, &slack, &provider);
        let path = dir.file(COOLDOWNS_FILE);
        gw.handle_request(request("The nightly build failed on main", "#eng", None));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&path).is_ok() {
            return; // running as root: permissions cannot make the file unreadable
        }

        // Another process's entries may be in the unreadable file: keep it.
        let verdict = gw.handle_request(request("Lunch is at noon in the big room", "#eng", None));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        assert_eq!(
            verdict,
            RequestVerdict::Denied {
                cooldown_remaining_secs: COOLDOWN_SECS
            }
        );
        assert_eq!(read_entries::<CooldownEntry>(&path).len(), 1);
        assert_eq!(gw.active_cooldowns().len(), 2, "still enforced in memory");
        // The next successful write includes the entry that failed to persist.
        gw.handle_request(request("Quarterly planning starts on Monday", "#eng", None));
        assert_eq!(read_entries::<CooldownEntry>(&path).len(), 3);
    }

    #[test]
    fn corrupt_state_file_found_at_runtime_is_copied_aside_and_rewritten() {
        let dir = TempDir::new("corrupt-runtime");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Deny), Ok(Decision::Deny)]);
        let gw = gateway(&dir, &slack, &provider);
        gw.handle_request(request("The nightly build failed on main", "#eng", None));
        fs::write(dir.file(COOLDOWNS_FILE), "{not json").unwrap();

        gw.handle_request(request("Lunch is at noon in the big room", "#eng", None));

        assert_eq!(
            read_entries::<CooldownEntry>(&dir.file(COOLDOWNS_FILE)).len(),
            2
        );
        let copies: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().contains("cooldowns.json.corrupt-"))
            .collect();
        assert_eq!(copies.len(), 1, "{copies:?}");
        assert_eq!(fs::read_to_string(&copies[0]).unwrap(), "{not json");
        let restarted = gateway(&dir, &slack, &ScriptedProvider::new(vec![]));
        assert_eq!(restarted.active_cooldowns().len(), 2);
    }

    // ---- cooldown matching hardening --------------------------------------------

    #[test]
    fn longest_matching_cooldown_is_reported() {
        let dir = TempDir::new("longest");
        fs::create_dir_all(dir.path()).unwrap();
        let now = now_secs();
        let entry = |message: &str, expires_at| CooldownEntry {
            destination: "#eng".into(),
            normalized: normalize_text(message),
            excerpt: message.into(),
            created_at: now,
            expires_at,
        };
        let entries = [
            entry("The nightly build failed on main", now + 42),
            entry("the nightly build has failed on main", now + 300),
        ];
        write_json_atomic(&dir.file(COOLDOWNS_FILE), &entries).unwrap();
        let provider = ScriptedProvider::new(vec![]);
        let gw = gateway(&dir, &MockSlack::new(), &provider);

        let verdict = gw.handle_request(request("The nightly build failed on main!", "#eng", None));

        assert_blocked(&verdict, 300);
        assert_eq!(provider.calls(), 0);
    }

    /// Denies only after the clock has moved past the request's receipt.
    struct SlowDenyProvider;

    impl DecisionProvider for SlowDenyProvider {
        fn request_decision(&self, request: &MessageRequest) -> Result<Decision, ProviderError> {
            while now_secs() <= request.received_at {
                thread::sleep(Duration::from_millis(20));
            }
            Ok(Decision::Deny)
        }
    }

    #[test]
    fn cooldown_starts_at_the_decision_not_at_receipt() {
        let dir = TempDir::new("decision-time");
        let gw = Gateway::new(MockSlack::new(), Arc::new(SlowDenyProvider), dir.path()).unwrap();
        let before = now_secs();

        let verdict = gw.handle_request(request("The nightly build failed on main", "#eng", None));

        assert_eq!(
            verdict,
            RequestVerdict::Denied {
                cooldown_remaining_secs: COOLDOWN_SECS
            }
        );
        let entries: Vec<CooldownEntry> = read_entries(&dir.file(COOLDOWNS_FILE));
        assert!(entries[0].created_at > before, "{entries:?}");
        assert_eq!(entries[0].expires_at, entries[0].created_at + COOLDOWN_SECS);
    }

    #[test]
    fn normalize_text_drops_invisible_characters_and_combining_marks() {
        assert_eq!(normalize_text("fa\u{200B}il\u{200D}ed"), "failed");
        assert_eq!(
            normalize_text("don\u{00AD}e \u{202E}now\u{202C}\u{FEFF}"),
            "done now"
        );
        assert_eq!(
            normalize_text("cafe\u{301} cr\u{0065}\u{300}me"),
            "cafe creme"
        );
        assert_eq!(normalize_text("İSTANBUL office"), "istanbul office");
        // Alphabetic combining marks are letters and stay.
        assert_eq!(normalize_text("\u{3B1}\u{345}"), "\u{3B1}\u{345}");
    }

    #[test]
    fn invisible_characters_or_decomposed_accents_do_not_split_words() {
        let dir = TempDir::new("invisible");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Deny), Ok(Decision::Deny)]);
        let gw = gateway(&dir, &slack, &provider);
        let english = "Heads up: the nightly build failed on main, investigating now";
        let french = "R\u{E9}union d'\u{E9}quipe d\u{E9}plac\u{E9}e \u{E0} 15h demain";
        gw.handle_request(request(english, "#eng", None));
        gw.handle_request(request(french, "#eng", None));

        let interleaved: String = english.chars().flat_map(|c| [c, '\u{200B}']).collect();
        let decomposed = french
            .replace('\u{E9}', "e\u{301}")
            .replace('\u{E0}', "a\u{300}");
        assert_blocked(
            &gw.handle_request(request(&interleaved, "#eng", None)),
            COOLDOWN_SECS,
        );
        assert_blocked(
            &gw.handle_request(request(&decomposed, "#eng", None)),
            COOLDOWN_SECS,
        );
        assert_eq!(provider.calls(), 2);
    }

    #[test]
    fn messages_without_letters_or_digits_share_one_purpose() {
        // SPEC-literal: they all normalize to "", so one denial blocks every
        // symbol-only message to that destination (over-blocking, never a
        // send). Messages with no visible text at all are rejected.
        let dir = TempDir::new("symbols");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Deny)]);
        let gw = gateway(&dir, &slack, &provider);
        gw.handle_request(request("\u{1F6A8}\u{1F6A8}\u{1F6A8}", "#eng", None));

        for retry in ["\u{1F6A8}", "\u{1F680} !!!", "???"] {
            assert_blocked(
                &gw.handle_request(request(retry, "#eng", None)),
                COOLDOWN_SECS,
            );
        }
        assert!(matches!(
            gw.handle_request(request("\u{200B}\u{FEFF} ", "#eng", None)),
            RequestVerdict::Error(_)
        ));
        assert_eq!(provider.calls(), 1);
    }

    #[test]
    fn spellings_of_one_destination_share_a_cooldown() {
        assert_eq!(destination_key("  #ENG "), "eng");
        assert_eq!(destination_key("eng"), "eng");
        assert_eq!(destination_key("@Alice"), "@alice");
        let dir = TempDir::new("dest-spelling");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![
            Ok(Decision::Deny),
            Ok(Decision::Feedback("a".into())),
            Ok(Decision::Feedback("b".into())),
        ]);
        let gw = gateway(&dir, &slack, &provider);
        let message = "The nightly build failed on main";
        gw.handle_request(request(message, "#eng", None));

        for destination in ["#ENG", "eng", " #eng "] {
            assert_blocked(
                &gw.handle_request(request(message, destination, None)),
                COOLDOWN_SECS,
            );
        }
        assert!(matches!(
            gw.handle_request(request(message, "#eng-alerts", None)),
            RequestVerdict::Feedback { .. }
        ));
        assert!(matches!(
            gw.handle_request(request(message, "@eng", None)),
            RequestVerdict::Feedback { .. }
        ));
        assert_eq!(
            gw.active_cooldowns()[0].0,
            "#eng",
            "the stored destination is not rewritten"
        );
    }

    #[test]
    fn over_long_message_is_rejected_without_a_popup() {
        let dir = TempDir::new("too-long");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Feedback("ok".into()))]);
        let gw = gateway(&dir, &slack, &provider);

        match gw.handle_request(request(&"a".repeat(MAX_MESSAGE_CHARS + 1), "#eng", None)) {
            RequestVerdict::Error(e) => assert!(e.contains("40000"), "{e}"),
            other => panic!("expected Error, got {other:?}"),
        }
        assert_eq!(provider.calls(), 0);
        // The limit counts characters, not bytes.
        assert_eq!(
            gw.handle_request(request(&"\u{E9}".repeat(MAX_MESSAGE_CHARS), "#eng", None)),
            RequestVerdict::Feedback {
                feedback: "ok".into()
            }
        );
        assert_eq!(slack.count(), 0);
    }

    // ---- second review round ----------------------------------------------------

    #[test]
    fn cooldown_check_comes_before_dedupe() {
        let dir = TempDir::new("cooldown-first");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Approve), Ok(Decision::Deny)]);
        let gw = gateway(&dir, &slack, &provider);
        let original = request("The nightly build failed on main", "#eng", None);
        gw.handle_request(original.clone());
        gw.handle_request(request(
            "the nightly build has failed on main!",
            "#eng",
            None,
        ));

        assert_blocked(&gw.handle_request(original), COOLDOWN_SECS);
        assert_eq!(provider.calls(), 2);
        assert_eq!(slack.count(), 1);
    }

    #[test]
    fn corrupt_sent_records_fail_creation() {
        let dir = TempDir::new("corrupt-sent");
        fs::create_dir_all(dir.path()).unwrap();
        fs::write(dir.file(SENT_FILE), "[{").unwrap();

        let err = Gateway::new(MockSlack::new(), ScriptedProvider::new(vec![]), dir.path())
            .err()
            .expect("corrupt sent records must not be silently dropped");

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains(SENT_FILE), "{err}");
    }

    #[test]
    fn stale_sent_records_are_dropped_from_disk_on_the_next_write() {
        let dir = TempDir::new("stale-sent");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Approve)]);
        let gw = gateway(&dir, &slack, &provider);
        let stale = SentRecord {
            message_hash: message_hash("old news", "#eng", None),
            destination: "#eng".into(),
            thread_ts: None,
            ts: "1.1".into(),
            permalink: "https://example.slack.com/archives/eng/p11".into(),
            sent_at: now_secs() - SENT_RETENTION_SECS - 1,
        };
        write_json_atomic(&dir.file(SENT_FILE), &[stale]).unwrap();

        gw.handle_request(request("Release 4.2 is live", "#announce", None));

        let on_disk: Vec<SentRecord> = read_entries(&dir.file(SENT_FILE));
        assert_eq!(on_disk.len(), 1);
        assert_eq!(on_disk[0].destination, "#announce");
    }

    #[test]
    fn unreadable_sent_records_fail_the_send_closed() {
        let dir = TempDir::new("sent-unreadable");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Approve), Ok(Decision::Deny)]);
        let gw = gateway(&dir, &slack, &provider);
        fs::create_dir(dir.file(SENT_FILE)).unwrap(); // reading it fails

        match gw.handle_request(request("Release 4.2 is live", "#announce", None)) {
            RequestVerdict::Error(e) => assert!(e.contains("nothing was sent"), "{e}"),
            other => panic!("expected Error, got {other:?}"),
        }
        assert_eq!(slack.count(), 0);
        // Denials do not depend on the sent records.
        assert_eq!(
            gw.handle_request(request("The nightly build failed on main", "#eng", None)),
            RequestVerdict::Denied {
                cooldown_remaining_secs: COOLDOWN_SECS
            }
        );
    }

    #[test]
    fn lock_wait_is_bounded() {
        let dir = TempDir::new("lock-timeout");
        let holder = lock_file(&dir.path(), "x.lock", WAIT)
            .unwrap()
            .expect("lock");

        let path = dir.path();
        let err = assert_completes(move || {
            lock_file(&path, "x.lock", Duration::from_millis(100)).map(|_| ())
        })
        .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        drop(holder);
        assert!(lock_file(&dir.path(), "x.lock", WAIT).unwrap().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn read_only_lock_files_still_lock() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("readonly-locks");
        fs::create_dir_all(dir.path()).unwrap();
        for name in [STATE_LOCK_FILE, SEND_LOCK_FILE] {
            fs::write(dir.file(name), "").unwrap();
            fs::set_permissions(dir.file(name), fs::Permissions::from_mode(0o444)).unwrap();
        }
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Approve)]);
        let gw = gateway(&dir, &slack, &provider);

        assert!(matches!(
            gw.handle_request(request("Release 4.2 is live", "#announce", None)),
            RequestVerdict::Sent {
                duplicate: false,
                ..
            }
        ));
        assert_eq!(slack.count(), 1);
    }

    #[test]
    fn state_dir_removed_at_runtime_is_recreated() {
        let dir = TempDir::new("dir-removed");
        let slack = MockSlack::new();
        let provider = ScriptedProvider::new(vec![Ok(Decision::Approve), Ok(Decision::Deny)]);
        let gw = gateway(&dir, &slack, &provider);
        fs::remove_dir_all(dir.path()).unwrap();

        assert!(matches!(
            gw.handle_request(request("Release 4.2 is live", "#announce", None)),
            RequestVerdict::Sent {
                duplicate: false,
                ..
            }
        ));
        gw.handle_request(request("The nightly build failed on main", "#eng", None));

        assert_eq!(read_entries::<SentRecord>(&dir.file(SENT_FILE)).len(), 1);
        assert_eq!(
            read_entries::<CooldownEntry>(&dir.file(COOLDOWNS_FILE)).len(),
            1
        );
    }

    #[test]
    fn concurrent_writers_never_publish_a_torn_file() {
        let dir = TempDir::new("torn");
        fs::create_dir_all(dir.path()).unwrap();
        let path = dir.file(COOLDOWNS_FILE);
        write_json_atomic(&path, &Vec::<u64>::new()).unwrap();
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let writers: Vec<_> = (0..4u64)
            .map(|w| {
                let path = path.clone();
                thread::spawn(move || {
                    for i in 0..100u64 {
                        // Different lengths, so interleaved writes would tear.
                        let value: Vec<u64> = (0..(w * 37 + i) % 50).collect();
                        write_json_atomic(&path, &value).unwrap();
                    }
                })
            })
            .collect();
        let reader = {
            let (path, done) = (path.clone(), Arc::clone(&done));
            thread::spawn(move || {
                let mut reads = 0;
                while !done.load(Ordering::SeqCst) {
                    read_json::<u64>(&path).expect("a reader saw a torn file");
                    reads += 1;
                }
                reads
            })
        };
        for w in writers {
            w.join().unwrap();
        }
        done.store(true, Ordering::SeqCst);

        assert!(reader.join().unwrap() > 0);
        assert_no_temp_files(&dir);
    }
}
