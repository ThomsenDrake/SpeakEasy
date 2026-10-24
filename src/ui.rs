//! Desktop approval UI transport (Tauri webview).
//!
//! The human-facing popup is the webview frontend in `ui/` (React + the
//! Once UI design system + Motion Primitives). This module is the bridge: it
//! receives decision requests from MCP worker threads, forwards them to the
//! webview as `approval-request` events, and resolves them when the webview
//! invokes `submit_decision`. Closing the window only hides it — the
//! gateway process keeps serving MCP (it is a long-running single instance;
//! see `src/instance.rs`); the tray icon or a Dock click reopens the window,
//! and only the tray's Quit item stops the app. Quitting drops the pending
//! reply senders, so blocked `request_decision` calls return `Err` — they
//! are never approved.

use crate::core::{Decision, DecisionProvider, Gateway, MessageRequest, ProviderError};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc::Sender, Arc, Mutex, PoisonError};
use tauri::{AppHandle, Emitter, Manager, State};

/// The human's decision as submitted by the webview
/// (`{"type":"approve"}` / `{"type":"deny"}` / `{"type":"feedback","feedback":"…"}`).
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum DecisionPayload {
    Approve,
    Deny,
    Feedback { feedback: String },
}

impl From<DecisionPayload> for Decision {
    fn from(payload: DecisionPayload) -> Self {
        match payload {
            DecisionPayload::Approve => Decision::Approve,
            DecisionPayload::Deny => Decision::Deny,
            DecisionPayload::Feedback { feedback } => Decision::Feedback(feedback),
        }
    }
}

/// Shared UI state: pending requests, the outbox for a not-yet-ready webview,
/// the app handle for emitting events, and the gateway (for the cooldown
/// status line).
pub struct UiState {
    pending: Mutex<HashMap<String, Sender<Result<Decision, ProviderError>>>>,
    outbox: Mutex<Vec<MessageRequest>>,
    webview_ready: AtomicBool,
    app: Mutex<Option<AppHandle>>,
    gateway: Mutex<Option<Arc<Gateway>>>,
}

impl UiState {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            outbox: Mutex::new(Vec::new()),
            webview_ready: AtomicBool::new(false),
            app: Mutex::new(None),
            gateway: Mutex::new(None),
        }
    }

    pub fn set_app(&self, app: AppHandle) {
        *self.app.lock().unwrap_or_else(PoisonError::into_inner) = Some(app);
    }

    pub fn set_gateway(&self, gateway: Arc<Gateway>) {
        *self.gateway.lock().unwrap_or_else(PoisonError::into_inner) = Some(gateway);
    }

    fn register(&self, id: String, tx: Sender<Result<Decision, ProviderError>>) {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, tx);
    }

    /// Bring the (hidden) approval window to the front. The window exists
    /// from app start; a request arriving at any time must surface it.
    fn reveal_window(&self) {
        let app = self.app.lock().unwrap_or_else(PoisonError::into_inner);
        match app.as_ref() {
            Some(app) => match app.get_webview_window("main") {
                Some(window) => {
                    let was_visible = window.is_visible().unwrap_or(false);
                    let show = window.show();
                    let focus = window.set_focus();
                    let now_visible = window.is_visible().unwrap_or(false);
                    eprintln!(
                        "slack-approval-gateway: reveal window: was_visible={was_visible} \
                         show={show:?} focus={focus:?} now_visible={now_visible}"
                    );
                }
                None => eprintln!(
                    "slack-approval-gateway: reveal window: no window with label 'main'"
                ),
            },
            None => eprintln!("slack-approval-gateway: reveal window: no app handle yet"),
        }
    }

    /// Deliver a request to the webview, or queue it until the webview is ready.
    fn deliver(&self, request: &MessageRequest) {
        if self.webview_ready.load(Ordering::Relaxed) {
            // Scope the app-handle borrow: reveal_window locks it again.
            let delivered = {
                let app = self.app.lock().unwrap_or_else(PoisonError::into_inner);
                match app.as_ref() {
                    Some(app) => {
                        eprintln!("slack-approval-gateway: emitting approval-request {}", request.id);
                        match app.emit("approval-request", request) {
                            Ok(()) => true,
                            Err(e) => {
                                eprintln!("slack-approval-gateway: cannot deliver request to UI: {e}");
                                false
                            }
                        }
                    }
                    None => false,
                }
            };
            if delivered {
                self.reveal_window();
                return;
            }
        }
        eprintln!("slack-approval-gateway: webview not ready; queueing request {}", request.id);
        self.outbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        self.reveal_window();
    }

    /// The webview signals it is listening; flush queued requests.
    fn mark_webview_ready(&self) {
        eprintln!("slack-approval-gateway: webview ready");
        self.webview_ready.store(true, Ordering::Relaxed);
        // Scope the app-handle borrow: reveal_window locks it again.
        let flushed_any = {
            let app = self.app.lock().unwrap_or_else(PoisonError::into_inner);
            match app.as_ref() {
                Some(app) => {
                    let queued: Vec<MessageRequest> = std::mem::take(
                        &mut *self.outbox.lock().unwrap_or_else(PoisonError::into_inner),
                    );
                    for request in &queued {
                        eprintln!("slack-approval-gateway: flushing queued request {}", request.id);
                        if let Err(e) = app.emit("approval-request", request) {
                            eprintln!("slack-approval-gateway: cannot deliver request to UI: {e}");
                        }
                    }
                    !queued.is_empty()
                }
                None => false,
            }
        };
        if flushed_any {
            self.reveal_window();
        }
    }

    /// Record the human's decision. The first decision wins; repeats and
    /// unknown request IDs are ignored (repeated clicks cannot double-send).
    fn submit(&self, request_id: &str, decision: Decision) {
        let tx = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(request_id);
        eprintln!("slack-approval-gateway: submit_decision {request_id} found={}", tx.is_some());
        if let Some(tx) = tx {
            let _ = tx.send(Ok(decision));
        }
    }

    fn active_cooldowns(&self) -> Vec<(String, String, u64)> {
        self.gateway
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|g| g.active_cooldowns())
            .unwrap_or_default()
    }

    #[cfg(test)]
    fn outbox_len(&self) -> usize {
        self.outbox.lock().unwrap_or_else(PoisonError::into_inner).len()
    }
}

/// Decision provider that forwards requests to the webview and blocks the
/// calling MCP worker thread until the human decides. If the app exits, the
/// pending sender is dropped and the request fails — it is never approved.
pub struct TauriDecisionProvider {
    state: Arc<UiState>,
}

impl TauriDecisionProvider {
    pub fn new(state: Arc<UiState>) -> Self {
        Self { state }
    }
}

impl DecisionProvider for TauriDecisionProvider {
    fn request_decision(&self, request: &MessageRequest) -> Result<Decision, ProviderError> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.state.register(request.id.clone(), tx);
        self.state.deliver(request);
        match rx.recv() {
            Ok(result) => result,
            Err(_) => Err(ProviderError(
                "gateway UI is not available; no decision was made".into(),
            )),
        }
    }
}

#[tauri::command]
pub fn submit_decision(state: State<'_, Arc<UiState>>, request_id: String, decision: DecisionPayload) {
    state.submit(&request_id, decision.into());
}

#[tauri::command]
pub fn ui_ready(state: State<'_, Arc<UiState>>) {
    state.mark_webview_ready();
}

#[tauri::command]
pub fn get_active_cooldowns(state: State<'_, Arc<UiState>>) -> Vec<(String, String, u64)> {
    state.active_cooldowns()
}

/// Hide the approval window (the webview calls this once its queue has
/// drained after a decision). Hiding is not quitting: the gateway keeps
/// serving MCP, and the next request (or the tray icon) shows it again.
#[tauri::command]
pub fn hide_window(window: tauri::Window) {
    let _ = window.hide();
}

/// Quit the gateway (tray menu / webview). Pending requests fail in the
/// provider — nothing pending is ever approved.
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{ConversationContext, IncomingRequest};

    fn sample_request(id: &str) -> MessageRequest {
        MessageRequest {
            id: id.to_string(),
            incoming: IncomingRequest {
                message: "hello".to_string(),
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
        }
    }

    #[test]
    fn request_is_queued_without_a_webview_and_resolved_by_submit() {
        let state = Arc::new(UiState::new());
        let provider = TauriDecisionProvider::new(Arc::clone(&state));
        let request = sample_request("req-1");

        let handle = std::thread::spawn({
            let provider = provider;
            let request = request.clone();
            move || provider.request_decision(&request)
        });

        // No webview: the request waits in the outbox and the provider blocks.
        for _ in 0..100 {
            if state.outbox_len() == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(state.outbox_len(), 1);
        assert!(!handle.is_finished());

        state.submit("req-1", Decision::Approve);
        let decision = handle.join().unwrap().unwrap();
        assert!(matches!(decision, Decision::Approve));
    }

    #[test]
    fn first_decision_wins_and_repeats_are_ignored() {
        let state = Arc::new(UiState::new());
        let provider = TauriDecisionProvider::new(Arc::clone(&state));
        let request = sample_request("req-2");

        let handle = std::thread::spawn({
            let provider = provider;
            let request = request.clone();
            move || provider.request_decision(&request)
        });
        for _ in 0..100 {
            if state.outbox_len() == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        state.submit("req-2", Decision::Deny);
        state.submit("req-2", Decision::Approve); // repeat: ignored
        state.submit("req-unknown", Decision::Approve); // unknown: ignored

        let decision = handle.join().unwrap().unwrap();
        assert!(matches!(decision, Decision::Deny));
    }

    #[test]
    fn feedback_payload_maps_to_feedback_decision() {
        let payload: DecisionPayload =
            serde_json::from_str(r#"{"type":"feedback","feedback":"change it"}"#).unwrap();
        let decision: Decision = payload.into();
        match decision {
            Decision::Feedback(f) => assert_eq!(f, "change it"),
            other => panic!("expected feedback, got {other:?}"),
        }
    }
}
