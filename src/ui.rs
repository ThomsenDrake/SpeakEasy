//! Desktop approval popup (egui/eframe).
//!
//! The UI is the only way a human produces a decision. Closing or ignoring a
//! popup must never authorize sending.

use crate::core::{
    now_secs, Decision, DecisionProvider, Gateway, MessageRequest, ProviderError, COOLDOWN_SECS,
};
use eframe::egui;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
        let (tx, rx) = channel();
        (Self { tx }, rx)
    }
}

impl DecisionProvider for UiDecisionProvider {
    fn request_decision(&self, request: &MessageRequest) -> Result<Decision, ProviderError> {
        track_in_flight();
        let unavailable =
            || ProviderError("gateway UI is not available; no decision was made".into());
        // Single-use reply channel: the UI sends at most one decision for this
        // request, and a dropped sender (UI gone) fails instead of approving.
        let (reply_tx, reply_rx) = channel();
        self.tx
            .send((request.clone(), reply_tx))
            .map_err(|_| unavailable())?;
        reply_rx.recv().map_err(|_| unavailable())?
    }
}

/// Requests whose MCP worker thread may still be busy with them: waiting for
/// a decision, then sending to Slack or recording the denial, then answering
/// the agent. The app waits for these on exit, because `main` ends the
/// process as soon as [`run_ui`] returns.
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// This worker thread's in-flight marker. It is released when the thread
    /// exits (the MCP server handles each tool call on its own thread) or
    /// when the thread requests its next decision.
    static IN_FLIGHT_MARKER: RefCell<Option<InFlightMarker>> = const { RefCell::new(None) };
}

struct InFlightMarker;

impl Drop for InFlightMarker {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

fn track_in_flight() {
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    // Replacing this thread's previous marker releases it.
    IN_FLIGHT_MARKER.with(|marker| *marker.borrow_mut() = Some(InFlightMarker));
}

/// Wait until no request is in flight. Give up only once `idle_limit` passes
/// without any request finishing: the gateway sends approved messages one at
/// a time, so each queued send gets a full budget after the previous one.
fn wait_for_in_flight(idle_limit: Duration) {
    let mut lowest = IN_FLIGHT.load(Ordering::SeqCst);
    let mut deadline = Instant::now() + idle_limit;
    while lowest > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        let now_in_flight = IN_FLIGHT.load(Ordering::SeqCst);
        if now_in_flight < lowest {
            lowest = now_in_flight;
            deadline = Instant::now() + idle_limit;
        }
    }
    let left = IN_FLIGHT.load(Ordering::SeqCst);
    if left > 0 {
        eprintln!(
            "slack-approval-gateway: exiting with {left} request(s) still in progress; \
             an approved message may reach Slack without being recorded or reported"
        );
    }
}

const WINDOW_TITLE: &str = "Slack Approval Gateway";

/// How often the UI wakes up to pick up new requests and refresh countdowns.
/// The provider cannot reach the egui context, so the UI polls the channel.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// The action buttons stay disabled this long after a popup is first shown
/// and after the window regains focus, so a double-click on one popup's
/// button cannot decide the next queued request, and the click that brings
/// the window forward cannot press a button.
const ARM_DELAY: Duration = Duration::from_millis(800);

/// Minimum interval between cooldown snapshots.
const COOLDOWN_REFRESH: Duration = Duration::from_secs(1);

/// At most this many cooldowns are listed in the status area.
const MAX_LISTED_COOLDOWNS: usize = 3;

/// The feedback editor scrolls beyond this height, so the actions, status
/// and caption below it stay in the window.
const FEEDBACK_MAX_HEIGHT: f32 = 96.0;

/// On exit, give up waiting for in-flight requests after this long without
/// one finishing. Undecided requests fail at once; this covers one approved
/// send: two state-lock waits (5 s each) and both Slack calls timing out
/// (15 s each), plus margin.
const IN_FLIGHT_WAIT: Duration = Duration::from_secs(45);

/// Run the desktop app on the calling (main) thread until the window is
/// closed. Pending requests whose senders are dropped when this returns are
/// failed by the provider with an error — they are never approved.
///
/// The status area lists the cooldowns persisted in the gateway's state dir
/// (resolved like `main.rs` does) plus denials made in this window;
/// [`run_ui_with_cooldowns`] reads them from the gateway instead.
pub fn run_ui(rx: Receiver<PendingRequest>) -> eframe::Result<()> {
    run_app(
        rx,
        CooldownSource::StateFile(state_dir().join("cooldowns.json")),
    )
}

/// Like [`run_ui`], but the status area shows the gateway's active deny
/// cooldowns.
pub fn run_ui_with_cooldowns(
    rx: Receiver<PendingRequest>,
    gateway: Arc<Gateway>,
) -> eframe::Result<()> {
    run_app(rx, CooldownSource::Gateway(gateway))
}

fn run_app(rx: Receiver<PendingRequest>, cooldowns: CooldownSource) -> eframe::Result<()> {
    let app = ApprovalApp::new(rx, cooldowns);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(WINDOW_TITLE)
            .with_inner_size([640.0, 620.0])
            .with_min_inner_size([460.0, 460.0])
            .with_resizable(true),
        ..Default::default()
    };
    eframe::run_native(WINDOW_TITLE, options, Box::new(|_cc| Ok(Box::new(app))))
}

/// The gateway's state directory, resolved exactly as `main.rs` does.
fn state_dir() -> PathBuf {
    if let Ok(d) = std::env::var("SLACK_GATEWAY_STATE_DIR") {
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".slack-approval-gateway")
}

/// The request currently shown, with its reply channel and per-popup state.
struct Popup {
    request: MessageRequest,
    reply: Sender<Result<Decision, ProviderError>>,
    /// When the actions started arming: the first frame this popup was
    /// rendered in a focused window. `None` until then.
    armed_since: Option<Instant>,
    feedback_open: bool,
    /// Move keyboard focus to the feedback editor on the next frame.
    focus_feedback: bool,
    feedback: String,
}

impl Popup {
    fn new((request, reply): PendingRequest) -> Self {
        Self {
            request,
            reply,
            armed_since: None,
            feedback_open: false,
            focus_feedback: false,
            feedback: String::new(),
        }
    }
}

/// Cooldowns as `(destination, excerpt, expires_at)`.
type CooldownList = Vec<(String, String, u64)>;

enum CooldownSource {
    /// The gateway's in-memory cooldowns.
    Gateway(Arc<Gateway>),
    /// The gateway's persisted `cooldowns.json`.
    StateFile(PathBuf),
}

struct Cooldowns {
    source: CooldownSource,
    /// Last snapshot read from `source`.
    snapshot: CooldownList,
    /// Denials made in this window, shown until `source` reflects them.
    local: CooldownList,
    fetched_at: Option<Instant>,
}

impl Cooldowns {
    fn new(source: CooldownSource) -> Self {
        Self {
            source,
            snapshot: Vec::new(),
            local: Vec::new(),
            fetched_at: None,
        }
    }

    fn refresh(&mut self) {
        if self
            .fetched_at
            .is_some_and(|t| t.elapsed() < COOLDOWN_REFRESH)
        {
            return;
        }
        self.fetched_at = Some(Instant::now());
        let now = now_secs();
        match &self.source {
            CooldownSource::Gateway(gateway) => {
                self.snapshot = gateway
                    .active_cooldowns()
                    .into_iter()
                    .map(|(dest, excerpt, remaining)| (dest, excerpt, now + remaining))
                    .collect();
            }
            CooldownSource::StateFile(path) => {
                // On a read or parse error, keep showing the last snapshot.
                if let Some(list) = read_cooldowns_file(path) {
                    self.snapshot = list;
                }
            }
        }
        self.local.retain(|(_, _, expires_at)| *expires_at > now);
    }

    fn record_denial(&mut self, request: &MessageRequest) {
        // Same key and excerpt as the gateway's own cooldown entry.
        let incoming = &request.incoming;
        self.local.push((
            incoming.destination.trim().to_string(),
            incoming.message.chars().take(80).collect(),
            now_secs() + COOLDOWN_SECS,
        ));
    }

    /// Active entries as `(destination, excerpt, remaining_secs)`, newest first.
    fn active(&self) -> Vec<(&str, &str, u64)> {
        let now = now_secs();
        let mut active: Vec<(&str, &str, u64)> = Vec::new();
        for (dest, excerpt, expires_at) in self.snapshot.iter().chain(&self.local) {
            if *expires_at <= now {
                continue;
            }
            let remaining = expires_at - now;
            match active
                .iter_mut()
                .find(|(d, e, _)| *d == dest.as_str() && *e == excerpt.as_str())
            {
                Some(entry) => entry.2 = entry.2.max(remaining),
                None => active.push((dest, excerpt, remaining)),
            }
        }
        active.sort_by_key(|entry| std::cmp::Reverse(entry.2));
        active
    }
}

/// Active cooldowns from the gateway's `cooldowns.json`; `None` if the file
/// cannot be read or parsed.
fn read_cooldowns_file(path: &Path) -> Option<CooldownList> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(Vec::new()),
        Err(_) => return None,
    };
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&bytes).ok()?;
    let now = now_secs();
    Some(
        entries
            .iter()
            .filter_map(|entry| {
                Some((
                    entry.get("destination")?.as_str()?.to_string(),
                    entry.get("excerpt")?.as_str()?.to_string(),
                    entry.get("expires_at")?.as_u64()?,
                ))
            })
            .filter(|(_, _, expires_at)| *expires_at > now)
            .collect(),
    )
}

struct ApprovalApp {
    rx: Receiver<PendingRequest>,
    /// Requests waiting behind `current`, in arrival order.
    queue: VecDeque<PendingRequest>,
    current: Option<Popup>,
    cooldowns: Cooldowns,
    shown_title: String,
    shut_down: bool,
}

impl ApprovalApp {
    fn new(rx: Receiver<PendingRequest>, cooldowns: CooldownSource) -> Self {
        Self {
            rx,
            queue: VecDeque::new(),
            current: None,
            cooldowns: Cooldowns::new(cooldowns),
            shown_title: WINDOW_TITLE.to_string(),
            shut_down: false,
        }
    }

    fn pending_count(&self) -> usize {
        self.queue.len() + usize::from(self.current.is_some())
    }

    /// Move newly arrived requests into the queue. Returns true if any arrived.
    fn drain_incoming(&mut self) -> bool {
        let mut arrived = false;
        while let Ok(pending) = self.rx.try_recv() {
            self.queue.push_back(pending);
            arrived = true;
        }
        arrived
    }

    fn advance(&mut self) {
        if self.current.is_none() {
            self.current = self.queue.pop_front().map(Popup::new);
        }
    }

    /// Deliver the human's decision for the current popup exactly once.
    fn decide(&mut self, decision: Decision) {
        // `take` makes a second click on the same popup a no-op.
        let Some(popup) = self.current.take() else {
            return;
        };
        if matches!(decision, Decision::Deny) {
            self.cooldowns.record_denial(&popup.request);
        }
        // The requester may already be gone; nothing else to do then.
        let _ = popup.reply.send(Ok(decision));
        self.advance();
    }

    /// Fail every undecided request (never approving it), then give the
    /// worker threads time to finish sends already approved and to answer
    /// their agents before the process exits. Runs on every exit path:
    /// window close and Cmd-Q both call `on_exit` and then drop the app.
    fn shut_down(&mut self) {
        if std::mem::replace(&mut self.shut_down, true) {
            return;
        }
        // Close the intake first, so later requests fail at once instead of
        // queueing. Dropping the receiver drops the requests buffered in it.
        drop(std::mem::replace(
            &mut self.rx,
            channel::<PendingRequest>().1,
        ));
        // Dropping the reply senders makes the blocked provider calls fail.
        self.current = None;
        self.queue.clear();
        wait_for_in_flight(IN_FLIGHT_WAIT);
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match self.pending_count() {
            0 => WINDOW_TITLE.to_string(),
            n => format!("{WINDOW_TITLE} — {n} pending"),
        };
        if title != self.shown_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.shown_title = title;
        }
    }

    fn show_status(&self, ui: &mut egui::Ui) {
        let waiting = match self.pending_count() {
            0 => "0 request(s) waiting".to_string(),
            1 => "1 request(s) waiting (this one)".to_string(),
            n => format!("{n} request(s) waiting (this one + {} queued)", n - 1),
        };
        let active = self.cooldowns.active();
        ui.horizontal_wrapped(|ui| {
            ui.strong(waiting);
            ui.label("·");
            ui.label(format!("{} active cooldown(s)", active.len()));
        });
        for (destination, excerpt, remaining) in active.iter().take(MAX_LISTED_COOLDOWNS) {
            let line = format!(
                "Blocked: {destination} — \"{}\" — {} left",
                one_line(excerpt),
                format_duration(*remaining)
            );
            ui.add(egui::Label::new(egui::RichText::new(&line).small()).truncate())
                .on_hover_text(line);
        }
        if active.len() > MAX_LISTED_COOLDOWNS {
            ui.label(
                egui::RichText::new(format!("…and {} more", active.len() - MAX_LISTED_COOLDOWNS))
                    .small(),
            );
        }
    }
}

impl eframe::App for ApprovalApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.drain_incoming() {
            // Restore a minimized window only on macOS, where that does not
            // activate the app; on X11 and Windows it also takes keyboard
            // focus from whatever the user is typing in.
            #[cfg(target_os = "macos")]
            if ctx.input(|i| i.viewport().minimized) == Some(true) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            }
            // Bounce the Dock icon (flash the taskbar entry) until noticed.
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Critical,
            ));
        }
        self.advance();
        self.cooldowns.refresh();
        self.update_title(ctx);
        ctx.request_repaint_after(POLL_INTERVAL);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut decision = None;

        egui::Panel::bottom("actions_and_status").show(ui, |ui| {
            ui.add_space(6.0);
            if let Some(popup) = self.current.as_mut() {
                let focused = ui.input(|i| i.viewport().focused).unwrap_or(true);
                if !focused {
                    popup.armed_since = None;
                }
                let armed = focused && {
                    let since = *popup.armed_since.get_or_insert_with(Instant::now);
                    let elapsed = since.elapsed();
                    if elapsed < ARM_DELAY {
                        ui.ctx().request_repaint_after(ARM_DELAY - elapsed);
                    }
                    elapsed >= ARM_DELAY
                };
                // Per-request widget IDs: keyboard focus on one popup's
                // button cannot carry over to the next popup's button.
                ui.push_id(popup.request.id.clone(), |ui| {
                    show_destination_summary(ui, &popup.request);
                    decision = show_actions(ui, popup, armed);
                    if !focused {
                        ui.label(
                            egui::RichText::new("Click this window to enable the actions.")
                                .small()
                                .weak(),
                        );
                    }
                });
                ui.separator();
            }
            self.show_status(ui);
            ui.label(
                egui::RichText::new(
                    "Closing this window never approves a message. If the app quits, pending \
                     requests fail and the agent is told nothing was sent.",
                )
                .small()
                .weak(),
            );
            ui.add_space(4.0);
        });

        // Keyed per request, so every popup opens scrolled to its top.
        let scroll_id = self.current.as_ref().map(|p| p.request.id.clone());
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("request", scroll_id))
                .auto_shrink([false, false])
                .show(ui, |ui| match &self.current {
                    Some(popup) => {
                        ui.push_id(popup.request.id.as_str(), |ui| {
                            show_request(ui, &popup.request)
                        });
                    }
                    None => {
                        ui.add_space(24.0);
                        ui.vertical_centered(|ui| {
                            ui.heading("No pending requests");
                            ui.label(
                                "Slack message requests from agents appear here for your \
                                 approval. Nothing is sent to Slack without it.",
                            );
                        });
                    }
                });
        });

        if let Some(decision) = decision {
            self.decide(decision);
            self.update_title(ui.ctx());
            ui.ctx().request_repaint();
        }
    }

    fn on_exit(&mut self) {
        self.shut_down();
    }
}

impl Drop for ApprovalApp {
    fn drop(&mut self) {
        self.shut_down();
    }
}

/// One line next to the actions naming where Approve posts, so a long message
/// cannot push the destination out of view.
fn show_destination_summary(ui: &mut egui::Ui, request: &MessageRequest) {
    let incoming = &request.incoming;
    let target = match &incoming.thread_ts {
        Some(ts) => format!("{} (reply in Slack thread {ts})", incoming.destination),
        None => format!("{} (new top-level message)", incoming.destination),
    };
    ui.horizontal(|ui| {
        ui.strong("Approve sends to:");
        ui.add(egui::Label::new(egui::RichText::new(&target).monospace()).truncate())
            .on_hover_text(&target);
    });
    hidden_chars_warning(ui, "The destination", &incoming.destination);
    if let Some(ts) = &incoming.thread_ts {
        hidden_chars_warning(ui, "The thread timestamp", ts);
    }
}

/// A button for the decision row. It reacts to the pointer (and assistive
/// technology) only: it never takes keyboard focus, so stray Tab and
/// Enter/Space keystrokes meant for another app cannot press it.
fn decision_button<'a>(text: impl Into<egui::WidgetText>) -> egui::Button<'a> {
    egui::Button::new(text).sense(egui::Sense::CLICK)
}

/// The Approve / Deny / Give feedback row and the feedback editor.
fn show_actions(ui: &mut egui::Ui, popup: &mut Popup, armed: bool) -> Option<Decision> {
    let mut decision = None;
    ui.horizontal(|ui| {
        let approve = decision_button(egui::RichText::new("Approve").strong());
        if ui
            .add_enabled(armed, approve)
            .on_hover_text("Send exactly this message to exactly this destination, once.")
            .clicked()
        {
            decision = Some(Decision::Approve);
        }
        if ui
            .add_enabled(armed, decision_button("Deny"))
            .on_hover_text(format!(
                "Send nothing, and block requests with the same purpose to this destination \
                 for {} minutes.",
                COOLDOWN_SECS / 60
            ))
            .clicked()
        {
            decision = Some(Decision::Deny);
        }
        let feedback = decision_button("Give feedback").selected(popup.feedback_open);
        if ui
            .add_enabled(armed, feedback)
            .on_hover_text("Send nothing; return your feedback to the requesting agent.")
            .clicked()
        {
            popup.feedback_open = !popup.feedback_open;
            popup.focus_feedback = popup.feedback_open;
        }
    });
    if popup.feedback_open {
        ui.add_space(4.0);
        ui.label("Feedback for the agent (nothing is sent to Slack):");
        let edit = egui::ScrollArea::vertical()
            .id_salt("feedback_scroll")
            .max_height(FEEDBACK_MAX_HEIGHT)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut popup.feedback)
                        .id_salt("feedback")
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .hint_text("What should the agent change?"),
                )
            })
            .inner;
        if std::mem::take(&mut popup.focus_feedback) {
            edit.request_focus();
        }
        let has_text = !popup.feedback.trim().is_empty();
        if ui
            .add_enabled(armed && has_text, decision_button("Submit feedback"))
            .on_disabled_hover_text("Enter feedback first.")
            .clicked()
        {
            decision = Some(Decision::Feedback(popup.feedback.trim().to_string()));
        }
    }
    decision
}

/// The read-only details of one request, in clearly separated sections.
fn show_request(ui: &mut egui::Ui, request: &MessageRequest) {
    let incoming = &request.incoming;

    ui.heading("Slack message approval request");
    egui::Grid::new("header")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            ui.label("Request ID:");
            selectable_mono(ui, &request.id);
            ui.end_row();
            ui.label("Requesting agent:");
            ui.add(egui::Label::new(&incoming.agent).selectable(true).wrap());
            ui.end_row();
            ui.label("Received:");
            ui.label(format_received(request.received_at, now_secs()));
            ui.end_row();
        });

    section(
        ui,
        "Proposed message (exact text)",
        &format!(
            "Sent verbatim if approved ({} characters).",
            incoming.message.chars().count()
        ),
        |ui| {
            hidden_chars_warning(ui, "The message", &incoming.message);
            let mut text = incoming.message.as_str();
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .id_salt("message")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(1),
            );
        },
    );

    section(
        ui,
        "Destination (Slack)",
        "Where the message is posted in Slack. This is NOT the agent conversation below.",
        |ui| {
            hidden_chars_warning(ui, "The destination", &incoming.destination);
            egui::Grid::new("destination")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Channel or user:");
                    selectable_mono(ui, &incoming.destination);
                    ui.end_row();
                    ui.label("Slack thread:");
                    match &incoming.thread_ts {
                        Some(ts) => {
                            ui.horizontal_wrapped(|ui| {
                                ui.label("reply in thread");
                                selectable_mono(ui, ts);
                            });
                        }
                        None => {
                            ui.label("none (new top-level message)");
                        }
                    }
                    ui.end_row();
                });
            if let Some(ts) = &incoming.thread_ts {
                hidden_chars_warning(ui, "The thread timestamp", ts);
            }
        },
    );

    section(ui, "Reason", "Why the agent wants to send this.", |ui| {
        ui.add(egui::Label::new(&incoming.reason).selectable(true).wrap());
    });

    let conversation = &incoming.conversation;
    section(
        ui,
        "Originating agent conversation (context only — not a Slack thread)",
        "Where the request came from. Nothing is posted here.",
        |ui| {
            egui::Grid::new("conversation")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Title:");
                    ui.add(
                        egui::Label::new(&conversation.title)
                            .selectable(true)
                            .wrap(),
                    );
                    ui.end_row();
                    ui.label("ID or link:");
                    ui.horizontal_wrapped(|ui| {
                        selectable_mono(ui, &conversation.id_or_link);
                        let link = conversation.id_or_link.trim();
                        if link.starts_with("https://") || link.starts_with("http://") {
                            ui.hyperlink_to("open", link);
                        }
                    });
                    ui.end_row();
                });
            ui.label("Summary:");
            ui.add(
                egui::Label::new(&conversation.summary)
                    .selectable(true)
                    .wrap(),
            );
        },
    );
}

fn section(
    ui: &mut egui::Ui,
    title: &str,
    caption: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(8.0);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(egui::RichText::new(title).strong().size(15.0));
        ui.label(egui::RichText::new(caption).small().weak());
        ui.add_space(4.0);
        add_contents(ui);
    });
}

fn selectable_mono(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(egui::RichText::new(text).monospace())
            .selectable(true)
            .wrap(),
    );
}

/// Warn about characters that render invisibly or reorder text, so the human
/// is not approving something other than what they see.
fn hidden_chars_warning(ui: &mut egui::Ui, what: &str, text: &str) {
    const MAX_LISTED: usize = 6;
    let hidden = hidden_chars(text);
    if hidden.is_empty() {
        return;
    }
    let mut distinct: Vec<char> = Vec::new();
    for &c in &hidden {
        if !distinct.contains(&c) {
            distinct.push(c);
        }
    }
    let mut listed: Vec<String> = distinct
        .iter()
        .take(MAX_LISTED)
        .map(|&c| format!("U+{:04X}", c as u32))
        .collect();
    if distinct.len() > MAX_LISTED {
        listed.push(format!("and {} more kinds", distinct.len() - MAX_LISTED));
    }
    ui.colored_label(
        ui.visuals().warn_fg_color,
        format!(
            "Warning: {what} contains {} invisible or text-direction character(s): {}",
            hidden.len(),
            listed.join(", ")
        ),
    );
}

/// The invisible or text-reordering characters in `text`, in order. Parts of
/// ordinary emoji sequences (presentation selectors, zero-width joiners,
/// well-formed subdivision flags) are not reported, so the warning stays
/// meaningful.
fn hidden_chars(text: &str) -> Vec<char> {
    let chars: Vec<char> = text.chars().collect();

    // Tags of a well-formed subdivision flag: black flag, 2-7 tag digits or
    // lowercase tag letters, cancel tag (e.g. the flag of England). Any other
    // tag character is reported: tags can smuggle invisible ASCII text.
    let mut flag_tag = vec![false; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\u{1F3F4}' {
            let spec = chars[i + 1..]
                .iter()
                .take_while(|c| matches!(c, '\u{E0030}'..='\u{E0039}' | '\u{E0061}'..='\u{E007A}'))
                .count();
            let end = i + 1 + spec;
            if (2..=7).contains(&spec) && chars.get(end) == Some(&'\u{E007F}') {
                flag_tag[i + 1..=end].fill(true);
                i = end;
            }
        }
        i += 1;
    }

    let mut hidden = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).map(|j| chars[j]);
        let next = chars.get(i + 1).copied();
        let emoji_part = match c {
            // Presentation selector after a pictograph, or in a keycap.
            '\u{FE0E}' | '\u{FE0F}' => {
                prev.is_some_and(is_pictographic)
                    || (prev.is_some_and(|p| matches!(p, '0'..='9' | '#' | '*'))
                        && next == Some('\u{20E3}'))
            }
            // Zero-width joiner between two pictographs.
            '\u{200D}' => {
                prev.is_some_and(|p| is_pictographic(p) || p == '\u{FE0F}')
                    && next.is_some_and(is_pictographic)
            }
            '\u{E0020}'..='\u{E007F}' => flag_tag[i],
            _ => false,
        };
        if is_hidden_char(c) && !emoji_part {
            hidden.push(c);
        }
    }
    hidden
}

/// Rough stand-in for Unicode's `Extended_Pictographic`: the code points that
/// start or join emoji sequences.
fn is_pictographic(c: char) -> bool {
    matches!(
        c,
        '\u{00A9}'
            | '\u{00AE}'
            | '\u{203C}'
            | '\u{2049}'
            | '\u{2122}'
            | '\u{2139}'
            | '\u{2194}'..='\u{2BFF}'
            | '\u{3030}'
            | '\u{303D}'
            | '\u{3297}'
            | '\u{3299}'
            | '\u{1F000}'..='\u{1FAFF}'
    )
}

fn is_hidden_char(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{115F}'
                | '\u{1160}'
                | '\u{17B4}'
                | '\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF0}'..='\u{FFFB}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0000}'..='\u{E0FFF}'
        )
}

/// Collapse a cooldown excerpt onto one status line.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn format_duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// `received_at` as a UTC date-time plus its age, e.g.
/// `2026-10-23 14:05:09 UTC (2m 03s ago)`.
fn format_received(received_at: u64, now: u64) -> String {
    let days = received_at / 86_400;
    let secs_of_day = received_at % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC ({} ago)",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
        format_duration(now.saturating_sub(received_at))
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian `(year, month, day)`
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}
