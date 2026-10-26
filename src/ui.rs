//! Desktop approval UI — a Rust-native popup (egui/eframe, no browser
//! engine) with a 1:1 recreation of the former React + Once UI + Motion
//! Primitives interface.
//!
//! This module is the bridge between the gateway's MCP worker threads and
//! the native UI event loop. It replaces the old Tauri webview transport
//! (events + commands) with typed Rust calls:
//!
//! - A worker thread calls [`GuiDecisionProvider::request_decision`], which
//!   registers a reply channel and queues the request in the [`UiState`]
//!   outbox, then reveals the window through the installed [`UiHooks`].
//!   Requests that arrive before the UI is running stay queued in the outbox
//!   and are delivered on the UI's first drain.
//! - The UI event loop drains the outbox ([`UiState::drain_outbox`]) and shows
//!   one request at a time. When the human decides, the UI calls
//!   [`UiState::submit`], which resolves exactly one blocked worker. The first
//!   decision wins: repeats and unknown/stale IDs are rejected, so a repeated
//!   click cannot double-send and a stale ID cannot decide another request.
//! - A rejected submission is reported to the UI as an error; the request is
//!   *retained* so the human can try again (the old webview handler wrongly
//!   advanced the queue even when submission failed).
//! - Closing the window only hides it (the gateway keeps serving MCP); the
//!   tray icon or a Dock click reopens it. Quitting ([`UiState::quit`]) drops
//!   every pending reply sender, so blocked `request_decision` calls return
//!   `Err` — unresolved requests fail and are never approved. The process
//!   then waits for any already-approved send to finish and be recorded
//!   (see `main.rs`).

pub mod anim;
pub mod app;
pub mod components;
pub mod demo;
pub mod icons;
pub mod shaders;
pub mod sys;
pub mod text;
pub mod theme;

use crate::core::{Decision, DecisionProvider, Gateway, MessageRequest, ProviderError};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// Why a decision submission was rejected by the bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitError {
    /// The request ID is unknown or was already decided (stale/duplicate).
    UnknownOrStale,
    /// The gateway is quitting; nothing is accepted anymore.
    Quitting,
}

impl std::fmt::Display for SubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubmitError::UnknownOrStale => write!(f, "unknown or already-decided request"),
            SubmitError::Quitting => write!(f, "the gateway is quitting"),
        }
    }
}

impl std::error::Error for SubmitError {}

/// Callbacks into the native UI event loop, installed by the GUI once it is
/// running. They are called from gateway worker threads while the bridge
/// lock is held, so they must be cheap and must never call back into the
/// bridge (that would deadlock).
#[derive(Default)]
pub struct UiHooks {
    /// A request arrived: bring the (hidden) window to the front.
    pub reveal: Option<Box<dyn Fn() + Send + Sync>>,
    /// Wake the event loop so the outbox drain is seen promptly.
    pub notify: Option<Box<dyn Fn() + Send + Sync>>,
    /// The queue drained: hide the window (never quits the gateway).
    pub hide: Option<Box<dyn Fn() + Send + Sync>>,
    /// Quit the gateway (tray Quit).
    pub quit: Option<Box<dyn Fn() + Send + Sync>>,
}

struct BridgeInner {
    /// Reply channels of requests waiting for a human decision.
    pending: HashMap<String, Sender<Result<Decision, ProviderError>>>,
    /// Requests delivered by workers but not yet pulled by the UI.
    outbox: Vec<MessageRequest>,
    /// IDs of requests the UI already pulled that were withdrawn since
    /// (their deadline passed); the UI drops them from its queue.
    withdrawn: Vec<String>,
    /// Set by `quit`: no new requests are accepted, submissions fail.
    quitting: bool,
}

/// Shared UI state: the pending-request map, the outbox, the hooks into the
/// UI event loop, and the gateway (for the cooldown status line).
pub struct UiState {
    inner: Mutex<BridgeInner>,
    hooks: Mutex<UiHooks>,
    gateway: Mutex<Option<Arc<Gateway>>>,
    /// Monotonic counter of requests ever enqueued (diagnostics/tests).
    enqueued: AtomicUsize,
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

impl UiState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BridgeInner {
                pending: HashMap::new(),
                outbox: Vec::new(),
                withdrawn: Vec::new(),
                quitting: false,
            }),
            hooks: Mutex::new(UiHooks::default()),
            gateway: Mutex::new(None),
            enqueued: AtomicUsize::new(0),
        }
    }

    pub fn set_gateway(&self, gateway: Arc<Gateway>) {
        *self.gateway.lock().unwrap_or_else(PoisonError::into_inner) = Some(gateway);
    }

    /// Install the UI event-loop hooks. Called once by the GUI when its
    /// `egui::Context` exists. Installing hooks does not itself reveal the
    /// window; requests already in the outbox are revealed by the UI on its
    /// first frame.
    pub fn install_hooks(&self, hooks: UiHooks) {
        *self.hooks.lock().unwrap_or_else(PoisonError::into_inner) = hooks;
    }

    fn lock_inner(&self) -> MutexGuard<'_, BridgeInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_hooks(&self) -> MutexGuard<'_, UiHooks> {
        self.hooks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Gateway worker side: register the reply channel and queue the request
    /// for the UI, then reveal the window. The reveal happens while the
    /// bridge lock is held, so it cannot interleave with a concurrent
    /// `hide_if_drained` (which checks the outbox under the same lock).
    fn enqueue(
        &self,
        id: String,
        tx: Sender<Result<Decision, ProviderError>>,
        request: MessageRequest,
    ) -> Result<(), ProviderError> {
        let mut inner = self.lock_inner();
        if inner.quitting {
            return Err(ProviderError(
                "gateway UI is quitting; no decision was made".into(),
            ));
        }
        eprintln!("slack-approval-gateway: queueing request {} for the UI", request.id);
        inner.pending.insert(id, tx);
        inner.outbox.push(request);
        self.enqueued.fetch_add(1, Ordering::Relaxed);
        let hooks = self.lock_hooks();
        if let Some(reveal) = hooks.reveal.as_ref() {
            reveal();
        }
        if let Some(notify) = hooks.notify.as_ref() {
            notify();
        }
        Ok(())
    }

    /// UI side: take the requests that arrived since the last call, in
    /// arrival order.
    pub fn drain_outbox(&self) -> Vec<MessageRequest> {
        let mut inner = self.lock_inner();
        if inner.outbox.is_empty() {
            return Vec::new();
        }
        std::mem::take(&mut inner.outbox)
    }

    /// Gateway worker side: withdraw an undecided request (its deadline
    /// passed), so no decision can be made for it any more. Returns false
    /// when the request was already decided (or the gateway is quitting):
    /// the decision is then waiting on the worker's channel.
    fn withdraw(&self, request_id: &str) -> bool {
        let mut inner = self.lock_inner();
        if inner.pending.remove(request_id).is_none() {
            return false;
        }
        eprintln!("slack-approval-gateway: request {request_id} expired; withdrawing it");
        inner.outbox.retain(|r| r.id != request_id);
        // Tell the UI either way: it drops the request if it already pulled
        // it, and hides a window that was revealed for nothing.
        inner.withdrawn.push(request_id.to_string());
        if let Some(notify) = self.lock_hooks().notify.as_ref() {
            notify();
        }
        true
    }

    /// UI side: take the IDs of requests withdrawn since the last call.
    pub fn drain_withdrawn(&self) -> Vec<String> {
        std::mem::take(&mut self.lock_inner().withdrawn)
    }

    /// UI side: record the human's decision. The first decision wins; repeat
    /// submissions and unknown/stale IDs are rejected (`Err`) so the UI can
    /// keep the request and show an error instead of advancing the queue.
    pub fn submit(&self, request_id: &str, decision: Decision) -> Result<(), SubmitError> {
        let mut inner = self.lock_inner();
        if inner.quitting {
            return Err(SubmitError::Quitting);
        }
        eprintln!(
            "slack-approval-gateway: submit_decision {request_id} found={}",
            inner.pending.contains_key(request_id)
        );
        match inner.pending.remove(request_id) {
            Some(tx) => {
                let _ = tx.send(Ok(decision));
                Ok(())
            }
            None => Err(SubmitError::UnknownOrStale),
        }
    }

    /// UI side: hide the window once the queue has drained. The outbox check
    /// and the hide hook run under the bridge lock, so a request that arrives
    /// concurrently either lands in the outbox before the check (no hide) or
    /// reveals the window after the hide — never a hidden window with a
    /// pending request.
    pub fn hide_if_drained(&self) {
        let inner = self.lock_inner();
        if !inner.outbox.is_empty() {
            return;
        }
        let hooks = self.lock_hooks();
        if let Some(hide) = hooks.hide.as_ref() {
            eprintln!("slack-approval-gateway: queue drained; hiding window");
            hide();
        }
    }

    /// Quit the gateway: fail every unresolved request (its worker returns
    /// `Err`, which is never an approval) and ask the UI event loop to close.
    /// An already-accepted decision is unaffected: the gateway's send
    /// coordination in `main.rs` lets it finish and be recorded.
    pub fn quit(&self) {
        let mut inner = self.lock_inner();
        if inner.quitting {
            return;
        }
        eprintln!("slack-approval-gateway: quitting; failing {} pending requests", inner.pending.len());
        inner.quitting = true;
        // Dropping the senders wakes every blocked worker with `Err`.
        inner.pending.clear();
        inner.outbox.clear();
        let hooks = self.lock_hooks();
        if let Some(quit) = hooks.quit.as_ref() {
            quit();
        }
    }

    /// Currently active deny cooldowns as `(destination, excerpt, remaining_secs)`.
    pub fn active_cooldowns(&self) -> Vec<(String, String, u64)> {
        self.gateway
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|g| g.active_cooldowns())
            .unwrap_or_default()
    }

    /// Requests waiting for a human decision (pending map + outbox).
    pub fn pending_count(&self) -> usize {
        let inner = self.lock_inner();
        inner.pending.len() + inner.outbox.len()
    }

    /// Total requests ever enqueued (diagnostics).
    pub fn enqueued_total(&self) -> usize {
        self.enqueued.load(Ordering::Relaxed)
    }
}

/// Decision provider that forwards requests to the native UI and blocks the
/// calling MCP worker thread until the human decides. If the gateway quits,
/// the pending sender is dropped and the request fails — it is never approved.
pub struct GuiDecisionProvider {
    state: Arc<UiState>,
}

impl GuiDecisionProvider {
    pub fn new(state: Arc<UiState>) -> Self {
        Self { state }
    }
}

impl DecisionProvider for GuiDecisionProvider {
    fn request_decision(&self, request: &MessageRequest) -> Result<Decision, ProviderError> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.state.enqueue(request.id.clone(), tx, request.clone())?;
        rx.recv().unwrap_or_else(|_| Err(ui_unavailable()))
    }

    fn request_decision_until(
        &self,
        request: &MessageRequest,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Result<Decision, ProviderError> {
        if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
            return Err(ProviderError(
                "the hook that asked stopped waiting before the request was shown".into(),
            ));
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.state.enqueue(request.id.clone(), tx, request.clone())?;
        // Wake up regularly to notice a requester that went away.
        const POLL: Duration = Duration::from_millis(200);
        let why = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break "no one answered the approval request in time";
            }
            if cancel.load(Ordering::Relaxed) {
                break "the hook that asked stopped waiting";
            }
            match rx.recv_timeout(left.min(POLL)) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Disconnected) => return Err(ui_unavailable()),
                Err(RecvTimeoutError::Timeout) => {}
            }
        };
        if self.state.withdraw(&request.id) {
            Err(ProviderError(format!(
                "{why}; the request was withdrawn and no decision was made"
            )))
        } else {
            // Decided (or quitting) just as it was withdrawn.
            rx.recv().unwrap_or_else(|_| Err(ui_unavailable()))
        }
    }
}

fn ui_unavailable() -> ProviderError {
    ProviderError("gateway UI is not available; no decision was made".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{ConversationContext, IncomingRequest, SlackClient, SlackSend, SlackError};
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(10);

    fn sample_request(id: &str) -> MessageRequest {
        MessageRequest {
            id: id.to_string(),
            incoming: IncomingRequest {
                message: format!("hello from {id}"),
                destination: "#eng".to_string(),
                thread_ts: None,
                reason: "test".to_string(),
                agent: "test-agent".to_string(),
                conversation: ConversationContext {
                    title: "t".to_string(),
                    id_or_link: "l".to_string(),
                    summary: "s".to_string(),
                },
            },
            received_at: 0,
            destination_display: None,
        }
    }

    /// Build hooks that append their name to a shared log, for race assertions.
    fn logged_hooks(log: &Arc<Mutex<Vec<&'static str>>>) -> UiHooks {
        let mk = |name: &'static str| {
            let log = Arc::clone(log);
            Box::new(move || log.lock().unwrap().push(name)) as Box<dyn Fn() + Send + Sync>
        };
        UiHooks {
            reveal: Some(mk("reveal")),
            notify: Some(mk("notify")),
            hide: Some(mk("hide")),
            quit: Some(mk("quit")),
        }
    }

    fn spawn_request(
        provider: &GuiDecisionProvider,
        request: MessageRequest,
    ) -> std::thread::JoinHandle<Result<Decision, ProviderError>> {
        let provider = provider.clone_state();
        std::thread::spawn(move || provider.request_decision(&request))
    }

    impl GuiDecisionProvider {
        fn clone_state(&self) -> GuiDecisionProvider {
            GuiDecisionProvider::new(Arc::clone(&self.state))
        }
    }

    fn wait_for_outbox(state: &UiState, n: usize) {
        for _ in 0..200 {
            if state.drain_outbox_len() == n {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("outbox never reached {n} (has {})", state.drain_outbox_len());
    }

    impl UiState {
        fn drain_outbox_len(&self) -> usize {
            self.lock_inner().outbox.len()
        }
    }

    #[test]
    fn an_undecided_request_is_withdrawn_at_its_deadline() {
        let state = Arc::new(UiState::new());
        let provider = GuiDecisionProvider::new(Arc::clone(&state));

        // Still in the outbox at the deadline: it simply disappears.
        let deadline = Instant::now() + Duration::from_millis(50);
        let result =
            provider.request_decision_until(&sample_request("req-w1"), deadline, &AtomicBool::new(false));
        assert!(result.is_err(), "an expired request is never a decision");
        assert!(state.drain_outbox().is_empty());
        assert_eq!(state.drain_withdrawn(), vec!["req-w1".to_string()]);
        assert_eq!(state.pending_count(), 0);

        // Already pulled by the UI: the UI is told to drop it, and a late
        // click is rejected.
        let worker = {
            let provider = provider.clone_state();
            std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_millis(300);
                provider.request_decision_until(&sample_request("req-w2"), deadline, &AtomicBool::new(false))
            })
        };
        wait_for_outbox(&state, 1);
        assert_eq!(state.drain_outbox().len(), 1);
        assert!(worker.join().unwrap().is_err());
        assert_eq!(state.drain_withdrawn(), vec!["req-w2".to_string()]);
        assert_eq!(
            state.submit("req-w2", Decision::Approve),
            Err(SubmitError::UnknownOrStale)
        );
    }

    #[test]
    fn a_request_whose_requester_left_is_withdrawn_early() {
        let state = Arc::new(UiState::new());
        let provider = GuiDecisionProvider::new(Arc::clone(&state));
        let cancel = Arc::new(AtomicBool::new(false));
        let worker = {
            let provider = provider.clone_state();
            let cancel = Arc::clone(&cancel);
            std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(60);
                provider.request_decision_until(&sample_request("req-c1"), deadline, &cancel)
            })
        };
        wait_for_outbox(&state, 1);
        state.drain_outbox();
        let started = Instant::now();
        cancel.store(true, Ordering::Relaxed);
        let result = worker.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(result.is_err());
        assert_eq!(state.drain_withdrawn(), vec!["req-c1".to_string()]);
    }

    #[test]
    fn a_decision_before_the_deadline_is_returned() {
        let state = Arc::new(UiState::new());
        let provider = GuiDecisionProvider::new(Arc::clone(&state));
        let worker = {
            let provider = provider.clone_state();
            std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(30);
                provider.request_decision_until(&sample_request("req-d1"), deadline, &AtomicBool::new(false))
            })
        };
        wait_for_outbox(&state, 1);
        state.drain_outbox();
        state.submit("req-d1", Decision::Approve).unwrap();
        assert!(matches!(worker.join().unwrap(), Ok(Decision::Approve)));
        assert!(state.drain_withdrawn().is_empty());
    }

    #[test]
    fn request_is_queued_without_a_ui_and_resolved_by_submit() {
        let state = Arc::new(UiState::new());
        let provider = GuiDecisionProvider::new(Arc::clone(&state));
        let request = sample_request("req-1");

        let handle = spawn_request(&provider, request);

        // No UI: the request waits in the outbox and the provider blocks.
        wait_for_outbox(&state, 1);
        assert!(!handle.is_finished());

        state.submit("req-1", Decision::Approve).unwrap();
        let decision = handle.join().unwrap().unwrap();
        assert!(matches!(decision, Decision::Approve));
    }

    #[test]
    fn first_decision_wins_and_repeats_are_ignored() {
        let state = Arc::new(UiState::new());
        let provider = GuiDecisionProvider::new(Arc::clone(&state));
        let handle = spawn_request(&provider, sample_request("req-2"));
        wait_for_outbox(&state, 1);

        state.submit("req-2", Decision::Deny).unwrap();
        // Repeat and unknown submissions are rejected, never delivered.
        assert_eq!(
            state.submit("req-2", Decision::Approve),
            Err(SubmitError::UnknownOrStale)
        );
        assert_eq!(
            state.submit("req-unknown", Decision::Approve),
            Err(SubmitError::UnknownOrStale)
        );

        let decision = handle.join().unwrap().unwrap();
        assert!(matches!(decision, Decision::Deny));
    }

    #[test]
    fn failed_submission_retains_the_request() {
        let state = Arc::new(UiState::new());
        let provider = GuiDecisionProvider::new(Arc::clone(&state));
        let handle = spawn_request(&provider, sample_request("req-stale"));
        wait_for_outbox(&state, 1);

        // A stale/unknown submission must not consume the request: the real
        // one still resolves it afterwards.
        assert_eq!(
            state.submit("req-other", Decision::Approve),
            Err(SubmitError::UnknownOrStale)
        );
        assert!(!handle.is_finished());
        state.submit("req-stale", Decision::Feedback("try again".into())).unwrap();
        let decision = handle.join().unwrap().unwrap();
        match decision {
            Decision::Feedback(f) => assert_eq!(f, "try again"),
            other => panic!("expected feedback, got {other:?}"),
        }
    }

    #[test]
    fn requests_arriving_before_ui_init_are_delivered_on_first_drain() {
        let state = Arc::new(UiState::new());
        let provider = GuiDecisionProvider::new(Arc::clone(&state));
        // No hooks installed at all: the workers only queue.
        // Queue the two requests in a deterministic order: two racing threads
        // would make the arrival order (and the FIFO assertion below) flaky.
        let h1 = spawn_request(&provider, sample_request("req-a"));
        wait_for_outbox(&state, 1);
        let h2 = spawn_request(&provider, sample_request("req-b"));
        wait_for_outbox(&state, 2);

        // The UI starts later and drains the outbox in arrival order.
        let drained = state.drain_outbox();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].id, "req-a");
        assert_eq!(drained[1].id, "req-b");
        assert!(state.drain_outbox().is_empty());

        state.submit("req-b", Decision::Deny).unwrap();
        state.submit("req-a", Decision::Approve).unwrap();
        assert!(matches!(h1.join().unwrap().unwrap(), Decision::Approve));
        assert!(matches!(h2.join().unwrap().unwrap(), Decision::Deny));
    }

    #[test]
    fn concurrent_requests_keep_their_ids_and_decision_recipients() {
        let state = Arc::new(UiState::new());
        let provider = GuiDecisionProvider::new(Arc::clone(&state));
        let n = 8;
        let mut handles = Vec::new();
        for i in 0..n {
            handles.push(spawn_request(&provider, sample_request(&format!("req-{i}"))));
        }
        wait_for_outbox(&state, n);
        let drained = state.drain_outbox();
        assert_eq!(drained.len(), n);

        // Decide in a shuffled order; every worker must get its own decision.
        let mut order: Vec<usize> = (0..n).collect();
        order.reverse();
        for &i in &order {
            let decision = if i % 3 == 0 {
                Decision::Approve
            } else if i % 3 == 1 {
                Decision::Deny
            } else {
                Decision::Feedback(format!("fb-{i}"))
            };
            state.submit(&format!("req-{i}"), decision).unwrap();
        }
        for (i, handle) in handles.into_iter().enumerate() {
            let decision = handle.join().unwrap().unwrap();
            match (i % 3, decision) {
                (0, Decision::Approve) => {}
                (1, Decision::Deny) => {}
                (2, Decision::Feedback(f)) => assert_eq!(f, format!("fb-{i}")),
                (m, other) => panic!("request {i} (mod {m}): wrong decision {other:?}"),
            }
        }
    }

    #[test]
    fn hide_only_fires_when_the_outbox_is_empty_and_never_races_a_new_request() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(UiState::new());
        state.install_hooks(logged_hooks(&log));
        let provider = GuiDecisionProvider::new(Arc::clone(&state));

        // 1. A queued (not yet drained) request blocks the hide. (The enqueue
        //    itself fired reveal+notify; only `hide` must be absent.)
        let handle = spawn_request(&provider, sample_request("req-h1"));
        wait_for_outbox(&state, 1);
        state.hide_if_drained();
        assert!(
            !log.lock().unwrap().contains(&"hide"),
            "hide must not fire with a queued request"
        );

        // 2. Once the UI drained and decided it, the hide fires.
        assert_eq!(state.drain_outbox().len(), 1);
        state.submit("req-h1", Decision::Approve).unwrap();
        handle.join().unwrap().unwrap();
        log.lock().unwrap().clear();
        state.hide_if_drained();
        assert_eq!(*log.lock().unwrap(), vec!["hide"]);

        // 3. A hide racing a new request: the reveal must come after the hide
        //    (the bridge lock serializes check+hook against push+reveal).
        log.lock().unwrap().clear();
        let state2 = Arc::clone(&state);
        let hider = std::thread::spawn(move || state2.hide_if_drained());
        let provider2 = GuiDecisionProvider::new(Arc::clone(&state));
        let spawner = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1));
            provider2.request_decision(&sample_request("req-h2"))
        });
        hider.join().unwrap();
        // Wait until the racing request is actually queued, then decide it.
        for _ in 0..200 {
            if state.enqueued_total() >= 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        state.submit("req-h2", Decision::Deny).unwrap();
        let decision = spawner.join().unwrap();
        let calls = log.lock().unwrap().clone();
        // Either the enqueue landed first (outbox non-empty → no hide) or the
        // hide landed first and the reveal followed it.
        if calls.contains(&"hide") {
            let hide_pos = calls.iter().position(|&c| c == "hide").unwrap();
            let reveal_pos = calls.iter().position(|&c| c == "reveal").unwrap();
            assert!(reveal_pos > hide_pos, "reveal must follow hide: {calls:?}");
        }
        assert!(matches!(decision.unwrap(), Decision::Deny));
    }

    #[test]
    fn quit_fails_unresolved_requests_and_blocks_new_ones() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(UiState::new());
        state.install_hooks(logged_hooks(&log));
        let provider = GuiDecisionProvider::new(Arc::clone(&state));

        let h1 = spawn_request(&provider, sample_request("req-q1"));
        let h2 = spawn_request(&provider, sample_request("req-q2"));
        wait_for_outbox(&state, 2);
        assert_eq!(state.drain_outbox().len(), 2);
        // The enqueues fired reveal+notify; only the quit hook is asserted.
        log.lock().unwrap().clear();

        state.quit();
        assert_eq!(*log.lock().unwrap(), vec!["quit"]);

        // Both blocked workers fail — never approved.
        for handle in [h1, h2] {
            match handle.join().unwrap() {
                Err(ProviderError(e)) => assert!(e.contains("not available"), "{e}"),
                Ok(d) => panic!("quit must never approve, got {d:?}"),
            }
        }
        // New requests are rejected immediately.
        let late = spawn_request(&provider, sample_request("req-q3"));
        match late.join().unwrap() {
            Err(ProviderError(e)) => assert!(e.contains("quitting"), "{e}"),
            Ok(d) => panic!("late request must fail, got {d:?}"),
        }
        // Submissions after quit fail.
        assert_eq!(state.submit("req-q1", Decision::Approve), Err(SubmitError::Quitting));
        // Quitting twice is a no-op (no second quit hook call).
        state.quit();
        assert_eq!(*log.lock().unwrap(), vec!["quit"]);
    }

    #[test]
    fn quit_lets_an_already_accepted_approval_proceed_to_the_send() {
        // End-to-end with a real gateway: the human approves, the gateway
        // starts the (mock) Slack send, and a concurrent quit must not cut the
        // accepted decision short — the worker keeps `Ok(Approve)` and the
        // gateway's send coordination can wait for the send to finish.
        let dir = std::env::temp_dir().join(format!(
            "sag-ui-quit-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let slack = Arc::new(GatedSlack::default());
        let state = Arc::new(UiState::new());
        let provider = Arc::new(GuiDecisionProvider::new(Arc::clone(&state)));
        let gateway = Arc::new(
            Gateway::new(slack.clone(), provider.clone(), dir.clone()).expect("gateway"),
        );
        state.set_gateway(Arc::clone(&gateway));

        let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
        *slack.entered.lock().unwrap() = Some(entered_tx);
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        *slack.release.lock().unwrap() = Some(release_rx);

        let gw = Arc::clone(&gateway);
        let worker = std::thread::spawn(move || {
            gw.handle_request(IncomingRequest {
                message: "send me".to_string(),
                destination: "#eng".to_string(),
                thread_ts: None,
                reason: "test".to_string(),
                agent: "agent".to_string(),
                conversation: ConversationContext {
                    title: "t".to_string(),
                    id_or_link: "l".to_string(),
                    summary: "s".to_string(),
                },
            })
        });

        // Wait until the request is queued for the UI, then approve it. The
        // gateway assigns the request ID, so read it from the drained request.
        wait_for_outbox(&state, 1);
        let drained = state.drain_outbox();
        assert_eq!(drained.len(), 1);
        let request_id = drained[0].id.clone();
        state.submit(&request_id, Decision::Approve).unwrap();

        // The gateway is now inside the (gated) Slack send.
        entered_rx.recv_timeout(WAIT).expect("send started");

        // Quit while the approved send is in flight: the decision was already
        // accepted, so the worker must still see the send's outcome.
        state.quit();

        // Let the send finish; the approved-send coordination must observe it.
        release_tx.send(()).unwrap();
        gateway.wait_until_no_send_in_progress();

        let verdict = worker.join().unwrap();
        match verdict {
            crate::core::RequestVerdict::Sent { duplicate: false, .. } => {}
            other => panic!("expected the approved send to complete, got {other:?}"),
        }
        assert_eq!(slack.calls(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    /// Mock Slack client whose `post_message` blocks until released, and that
    /// records calls. Never touches the network.
    #[derive(Default)]
    struct GatedSlack {
        calls: AtomicUsize,
        entered: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>,
        release: std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    }

    impl GatedSlack {
        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }
    }

    impl SlackClient for GatedSlack {
        fn post_message(
            &self,
            destination: &str,
            _thread_ts: Option<&str>,
            _text: &str,
        ) -> Result<SlackSend, SlackError> {
            let n = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(tx) = self.entered.lock().unwrap().as_ref() {
                let _ = tx.send(());
            }
            if let Some(rx) = self.release.lock().unwrap().as_ref() {
                let _ = rx.recv_timeout(WAIT);
            }
            Ok(SlackSend {
                channel: destination.to_string(),
                ts: format!("1700000000.{n:06}"),
                permalink: format!("https://example.slack.com/archives/{destination}/p{n:06}"),
            })
        }
    }

    #[test]
    fn feedback_and_deny_decisions_reach_the_worker_verbatim() {
        let state = Arc::new(UiState::new());
        let provider = GuiDecisionProvider::new(Arc::clone(&state));
        let h1 = spawn_request(&provider, sample_request("req-fb"));
        let h2 = spawn_request(&provider, sample_request("req-dn"));
        wait_for_outbox(&state, 2);
        state.drain_outbox();

        state.submit("req-fb", Decision::Feedback("  padded  ".into())).unwrap();
        state.submit("req-dn", Decision::Deny).unwrap();
        match h1.join().unwrap().unwrap() {
            Decision::Feedback(f) => assert_eq!(f, "  padded  "),
            other => panic!("expected feedback, got {other:?}"),
        }
        assert!(matches!(h2.join().unwrap().unwrap(), Decision::Deny));
    }
}
