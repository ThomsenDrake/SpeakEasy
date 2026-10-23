//! Desktop approval popup (egui/eframe).
//!
//! The UI is the only way a human produces a decision. Closing or ignoring a
//! popup must never authorize sending.

use crate::core::{Decision, DecisionProvider, MessageRequest, ProviderError};
use std::sync::mpsc::{channel, Receiver, Sender};

/// A pending request as handed from an MCP worker thread to the UI thread.
/// The reply half is a single-use `mpsc` sender: exactly one decision is
/// sent, then the sender is dropped. If the UI side is dropped instead
/// (app exiting), `recv` fails and the provider returns an error — never
/// an approval.
pub type PendingRequest = (MessageRequest, Sender<Result<Decision, ProviderError>>);

/// Decision provider that forwards requests to the UI thread and blocks the
/// calling MCP worker thread until the human decides (or the UI exits, which
/// returns `Err` and never approves).
pub struct UiDecisionProvider {
    tx: Sender<PendingRequest>,
}

impl UiDecisionProvider {
    /// Create the provider and the receiving end consumed by [`run_ui`].
    pub fn channel() -> (Self, Receiver<PendingRequest>) {
        unimplemented!()
    }
}

impl DecisionProvider for UiDecisionProvider {
    fn request_decision(&self, request: &MessageRequest) -> Result<Decision, ProviderError> {
        unimplemented!()
    }
}

/// Run the desktop app on the calling (main) thread until the window is
/// closed. Pending requests whose senders are dropped when this returns are
/// failed by the provider with an error — they are never approved.
pub fn run_ui(rx: Receiver<PendingRequest>) -> eframe::Result<()> {
    unimplemented!()
}
