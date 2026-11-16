//! The native approval popup: the eframe application and its desktop
//! lifecycle (tray icon, menus, window show/hide/quit, clipboard, keyboard).
//!
//! The layout engine and the painting of every region live in
//! [`crate::ui::app::paint`]; this module owns the app state, the event-loop
//! integration (`eframe::App`), and the macOS desktop integration.

pub mod paint;
mod shortcuts;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use eframe::egui;
use egui::{Context, Event, Id, Key, Pos2, Rect, ViewportCommand, Visuals};
use epaint::text::Galley;

use crate::core::{Decision, MessageRequest};
use crate::ui::anim::{AnimatedNumber, SegmentState, TextEffect};
use crate::ui::app::shortcuts::{HeldKeys, KeyAction};
use crate::ui::components::{self, FocusId, FocusTracker, TitleSegment};
use crate::ui::shaders::Effects;
use crate::ui::sys;
use crate::ui::text::TextSpec;
use crate::ui::theme::{Metrics, Palette, Theme};
use crate::ui::UiState;

// ---------------------------------------------------------------------------
// Copy (verbatim from the reference UI)
// ---------------------------------------------------------------------------

pub(crate) const APP_TITLE: &str = "SpeakEasy";
pub(crate) const HEADING: &str = "Message approval request";
pub(crate) const LABEL_MESSAGE: &str = "Proposed message (exact text)";
pub(crate) const LABEL_DESTINATION: &str = "Destination (Slack)";
pub(crate) const HINT_DESTINATION: &str =
    "Where the message would be sent — this is not the agent conversation.";
pub(crate) const LABEL_REASON: &str = "Reason";
pub(crate) const LABEL_CONVERSATION: &str = "Originating agent conversation";
pub(crate) const HINT_CONVERSATION: &str = "Context only — not a Slack thread.";
pub(crate) const TEXTAREA_LABEL: &str = "Feedback for the requesting agent";
pub(crate) const TEXTAREA_PLACEHOLDER: &str = "Tell the agent what to change…";
pub(crate) const BTN_SUBMIT_FEEDBACK: &str = "Submit feedback";
pub(crate) const BTN_GIVE_FEEDBACK: &str = "Give feedback";
pub(crate) const BTN_DENY: &str = "Deny";
pub(crate) const BTN_APPROVE: &str = "Approve";
pub(crate) const SUBMIT_ERROR: &str = "The decision could not be submitted — the gateway did not accept it. Nothing was sent; try again.";
pub(crate) const EMPTY_TITLE: &str = "Waiting for agent requests";
pub(crate) const EMPTY_SUB: &str =
    "When an agent asks to send a Slack message, it appears here for your approval.";
pub(crate) const FOOTER_TEXT: &str = "Closing this window hides it and never approves a message; SpeakEasy keeps running (menu-bar icon to reopen or quit). If SpeakEasy quits, pending requests fail and the agent is told nothing was sent.";
pub(crate) const COOLDOWN_PREFIX: &str = "Deny cooldowns active: ";

// ---------------------------------------------------------------------------
// Geometry constants (resolved reference layout, 720×780 @ root 15px)
//
// Once UI is rem-based with a responsive root font size (15px at viewport
// widths ≤768px, 16px above), so every spacing/radius/typography constant
// below is the *root-15* base: the paint pass multiplies them by
// `Metrics::s` (see `paint.rs`). Absolute values (1px borders, the wheel
// line step) are not scaled.
// ---------------------------------------------------------------------------

/// Header: padding space-16/space-24, content row, 1px bottom border.
pub(crate) const HEADER_PAD_X: f32 = 22.5;
pub(crate) const HEADER_PAD_Y: f32 = 15.0;
/// Main: padding space-24; the content column is `maxWidth="l"` centered.
pub(crate) const MAIN_PAD: f32 = 22.5;
/// Footer: 1px top border, padding space-12/space-24, gap space-4.
pub(crate) const FOOTER_PAD_X: f32 = 22.5;
pub(crate) const FOOTER_PAD_Y: f32 = 11.25;
pub(crate) const FOOTER_GAP: f32 = 3.75;
/// Card: padding space-24, gap space-20, radius-l.
pub(crate) const CARD_PAD: f32 = 22.5;
pub(crate) const CARD_GAP: f32 = 18.75;
/// Sections: label + space-8 + content (+ space-8 + hint).
pub(crate) const SECTION_GAP: f32 = 7.5;
/// Actions column: gap space-12; feedback column gap space-8.
pub(crate) const ACTIONS_GAP: f32 = 11.25;
/// Message/conversation panels: padding space-12 / space-16, radius-m.
pub(crate) const MSG_PANEL_PAD: f32 = 11.25;
pub(crate) const CONV_PANEL_PAD: f32 = 15.0;
/// Empty-state card: padding space-48, gap space-12.
pub(crate) const EMPTY_PAD: f32 = 45.0;
pub(crate) const EMPTY_GAP: f32 = 11.25;
/// The wheel/keyboard line-mode scroll step (Chrome: 40px per line —
/// absolute, not rem-based).
pub(crate) const LINE_STEP: f32 = 40.0;
/// The page-mode keyboard scroll overlap (Chrome keeps ~2 lines visible).
pub(crate) const PAGE_OVERLAP: f32 = 2.0 * LINE_STEP;
/// The scrollbar (Once UI `scrollbar-minimal`): an overlay thumb,
/// `--webkit-scrollbar { width: var(--static-space-4) }` (3.75px at root 15
/// — rem-based; `Metrics::scrollbar_w`).
pub(crate) const SCROLLBAR_THUMB_ALPHA: f32 = 0.3;
pub(crate) const SCROLLBAR_THUMB_ALPHA_HOVER: f32 = 0.5;
pub(crate) const SCROLLBAR_MIN_THUMB: f32 = 26.0;
pub(crate) const SCROLLBAR_IDLE_HIDE: f64 = 0.5;

/// Demo-mode flags (mirrors of the reference capture harness's `__SAG__`).
#[derive(Debug, Clone, Copy, Default)]
pub struct DemoFlags {
    pub sticky_deciding: bool,
    pub fail_submit: bool,
}

/// What a key extracted in `raw_input_hook` (so egui never sees it) does,
/// applied in order in `App::logic` (see the Keyboard section). Focus moves
/// (Tab, Escape leaving the textarea) are applied by the hook itself, so
/// each key is judged with the focus the keys before it left.
enum PendingKey {
    /// Enter/Escape decided `request`, the front card when it was pressed
    /// (applied only while that card is still in front).
    Decide { decision: Decision, request: String },
    /// Enter or Space activated the focused `button` on `request` (applied
    /// only while that card is still in front).
    Activate { button: FocusId, request: String },
    /// Scroll the main area by this many points (arrow keys).
    Scroll(f32),
    /// Scroll by ±one page (Space / Shift+Space, like the browser).
    Page(f32),
    /// Scroll to an absolute offset (Home/End); `f32::MAX` means bottom.
    ScrollTo(f32),
}

/// What `extract_keys` carries from one event of a batch to the next.
struct KeyBatch {
    /// The tracked front card when the batch was read.
    front: Option<String>,
    /// An earlier event in the batch has an effect that only the logic or
    /// paint pass applies (a queued decision or activation, a click): later
    /// keys are judged as not armed.
    unsettled: bool,
    /// The OS key-downs of the gated keys not yet matched with a press
    /// (`sys::take_shortcut_key_downs`).
    os_repeats: Vec<(Key, bool)>,
}

/// Per-frame prepared title-segment paint data.
pub(crate) struct TitleSegPaint {
    pub x: f32,
    pub galley: Arc<Galley>,
    /// `None` once the segment has fully revealed (paint the plain galley).
    pub state: Option<SegmentState>,
    pub tex: Option<paint::TitleTex>,
}

/// The overlay scrollbar's transient state.
struct ScrollbarState {
    fade: components::Tween,
    hover: components::Tween,
    last_activity: f64,
}

impl ScrollbarState {
    fn new() -> Self {
        Self {
            fade: components::Tween::new(0.0),
            hover: components::Tween::new(0.0),
            last_activity: f64::NEG_INFINITY,
        }
    }

    /// A scroll happened: show the thumb and restart the idle-hide timer.
    fn activity(&mut self, now: f64) {
        self.fade.set(1.0, now);
        self.last_activity = now;
    }

    fn update(&mut self, now: f64) {
        if now - self.last_activity > SCROLLBAR_IDLE_HIDE {
            self.fade.set(0.0, now);
        }
    }

    fn fading(&self, now: f64) -> bool {
        !self.fade.at_target(now) || !self.hover.at_target(now)
    }
}

// ---------------------------------------------------------------------------
// The app
// ---------------------------------------------------------------------------

pub struct ApprovalApp {
    state: Arc<UiState>,
    effects: Arc<Effects>,
    theme: Theme,
    pal: Palette,
    reduce_motion: bool,
    is_demo: bool,
    demo: DemoFlags,

    // Queue and per-request UI state.
    queue: VecDeque<MessageRequest>,
    cooldowns: Vec<(String, String, u64)>,
    deciding: bool,
    submit_error: bool,
    feedback_open: bool,
    feedback_text: String,
    focus: FocusTracker,
    current_id: Option<String>,
    /// Input time when the current request became the front card. A click
    /// whose press started earlier, or less than `shortcuts::CLICK_ARM`
    /// later, was aimed at a previous card (one that was withdrawn under
    /// the pointer) and is ignored.
    front_since: f64,
    /// The earliest time the latest primary pointer press can have gone
    /// down: the time of the input batch before the one that brought it.
    pressed_at: f64,
    /// Input time of the current input batch, and of the one before it: a
    /// key or button press in the current batch went down after
    /// `prev_batch_at`, which is all that is known of its time.
    batch_at: f64,
    prev_batch_at: f64,
    /// The next time the keys or the clicks arm, for which a frame is
    /// requested (`schedule_arming`): a press counts as armed only when the
    /// batch before the one that brings it was taken after arming.
    wake_at: Option<f64>,
    /// Input time when new requests brought the window forward (`reveal`
    /// from `sync_queue`; not the user's own Dock or menu-bar reveal). A
    /// click or key that was meant for the app the window came in front of
    /// must not decide the front card.
    revealed_at: f64,
    /// Input time when the user re-established an orphaned focus (Tab,
    /// Escape or a click; see `orphaned_focus`): the keys and clicks arm
    /// again from then.
    refocused_at: f64,
    /// egui's focus is on a widget that is none of the card's controls (an
    /// assistive technology moved it there): Enter and Escape do nothing.
    foreign_focus: bool,
    /// Input time since when the window has been on screen with the
    /// keyboard focus (`None`: hidden or unfocused). With `front_since`, it
    /// arms the Enter/Escape shortcuts (see `shortcuts`).
    focused_since: Option<f64>,
    /// When Enter/Escape/Space went down (key-repeat and pre-arm press
    /// detection).
    held_keys: HeldKeys,
    /// Input time of the latest text input or key press other than
    /// Enter/Escape/Tab: typing re-arms the keys (see `shortcuts`).
    last_typed_at: f64,
    /// Input time since when the window has been on screen (`None`:
    /// hidden, minimized or fully covered). With `front_since`, it arms
    /// clicks on the decision buttons (`shortcuts::click_armed`).
    on_screen_since: Option<f64>,
    /// The front card was withdrawn while one of its controls (the
    /// feedback textarea, or a button focused with Tab or a click) had the
    /// focus. The keys the user is pressing were meant for that control, so
    /// Enter and Escape decide nothing, and no button activates, until the
    /// user re-establishes the focus: Tab/Shift+Tab, a click in the window,
    /// or a plain Escape (which only clears this, as it would only have left
    /// the field). That key or click decides nothing, and the keys and
    /// clicks arm again from it (`refocused_at`). See
    /// `shortcuts::while_orphaned`.
    orphaned_focus: bool,
    ta_id: Id,

    // Animation clock. The clock only advances while the window is actually
    // on screen (a hidden/minimized/occluded webview throttles rAF in the
    // reference, so animations freeze until the window is really shown).
    clock: Instant,
    frozen_now: f64,
    last_paint_real: Option<f64>,
    frame_now: f64,
    window_visible: bool,

    // Effects.
    title_fx: TextEffect,
    title_segs: Vec<TitleSegment>,
    title_shared: Vec<Option<paint::TitleTex>>,
    title_ppp: f32,
    /// The spec `title_segs` was laid out with (re-laid-out when the
    /// responsive metrics change the title's size).
    title_spec: TextSpec,
    badge: AnimatedNumber,
    /// Whether the pending-count badge is currently mounted. The reference
    /// renders the Tag only while `queue.length > 0`, so it mounts (showing
    /// the count immediately) when the queue first becomes non-empty and
    /// unmounts when it drains.
    badge_mounted: bool,

    // Layout memory (produced by the paint pass, consumed by logic).
    main_rect: Rect,
    content_height: f32,
    max_scroll: f32,
    scroll: ScrollbarState,
    scroll_offset: f32,

    // Desktop integration.
    #[cfg(target_os = "macos")]
    tray: Option<tray_icon::TrayIcon>,
    #[cfg(target_os = "macos")]
    app_menu: Option<tray_icon::menu::Menu>,
    #[cfg(target_os = "macos")]
    dock_observer: Option<sys::DockReopenObserver>,
    #[cfg(target_os = "macos")]
    key_down_monitor: Option<sys::KeyDownMonitor>,
    ctx_slot: Arc<Mutex<Option<Context>>>,
    quit_requested: Arc<AtomicBool>,
    pending_inject: Arc<Mutex<Vec<Event>>>,

    // Per-frame plumbing.
    events: Vec<components::AppEvent>,
    pending_keys: Vec<PendingKey>,
    initialized: bool,
    title_started: bool,
    window_title: Option<String>,
    hover_repaint_until: f64,
    last_pointer: Option<Pos2>,
}

impl ApprovalApp {
    pub fn new(
        state: Arc<UiState>,
        ctx_slot: Arc<Mutex<Option<Context>>>,
        theme: Theme,
        reduce_motion: bool,
        is_demo: bool,
        demo: DemoFlags,
    ) -> Self {
        let pal = Palette::for_theme(theme);
        let effects = Arc::new(Effects::new());
        Self {
            state,
            effects,
            theme,
            pal,
            reduce_motion,
            is_demo,
            demo,
            queue: VecDeque::new(),
            cooldowns: Vec::new(),
            deciding: false,
            submit_error: false,
            feedback_open: false,
            feedback_text: String::new(),
            focus: FocusTracker::default(),
            current_id: None,
            front_since: 0.0,
            pressed_at: f64::NEG_INFINITY,
            batch_at: f64::NEG_INFINITY,
            prev_batch_at: f64::NEG_INFINITY,
            wake_at: None,
            revealed_at: f64::NEG_INFINITY,
            refocused_at: f64::NEG_INFINITY,
            foreign_focus: false,
            focused_since: None,
            held_keys: HeldKeys::default(),
            last_typed_at: f64::NEG_INFINITY,
            on_screen_since: None,
            orphaned_focus: false,
            ta_id: Id::unique("feedback"),
            clock: Instant::now(),
            frozen_now: 0.0,
            last_paint_real: None,
            frame_now: 0.0,
            window_visible: false,
            title_fx: TextEffect::new(APP_TITLE),
            title_segs: Vec::new(),
            title_shared: Vec::new(),
            title_ppp: 0.0,
            title_spec: TextSpec::new(0.0, 0.0, 0.0, 0.0),
            badge: AnimatedNumber::new(0, 0.0),
            badge_mounted: false,
            main_rect: Rect::ZERO,
            content_height: 0.0,
            max_scroll: 0.0,
            scroll: ScrollbarState::new(),
            scroll_offset: 0.0,
            #[cfg(target_os = "macos")]
            tray: None,
            #[cfg(target_os = "macos")]
            app_menu: None,
            #[cfg(target_os = "macos")]
            dock_observer: None,
            #[cfg(target_os = "macos")]
            key_down_monitor: None,
            ctx_slot,
            quit_requested: Arc::new(AtomicBool::new(false)),
            pending_inject: Arc::new(Mutex::new(Vec::new())),
            events: Vec::new(),
            pending_keys: Vec::new(),
            initialized: false,
            title_started: false,
            window_title: None,
            hover_repaint_until: 0.0,
            last_pointer: None,
        }
    }

    /// The animation clock: advances only while the window is painted. While
    /// the window is hidden the clock freezes, and on reveal it resumes from
    /// the frozen value (it must not jump to the wall-clock elapsed time —
    /// that would fast-forward every animation started before the reveal,
    /// e.g. the title reveal, which the reference plays on first reveal).
    fn anim_now(&mut self, ctx: &Context) -> f64 {
        let vp = ctx.input(|i| i.viewport().clone());
        // "Painted" means the window is actually on screen. The app's own
        // `window_visible` flag is not enough: eframe keeps running
        // (throttled) updates for windows the OS has not mapped yet, and a
        // hidden or fully occluded webview throttles rAF in the reference,
        // so the clock must stay frozen until the window is really shown
        // (and while it is minimized or fully covered by another window).
        let painted = self.window_visible
            && !vp.minimized.unwrap_or(false)
            && !vp.occluded.unwrap_or(false)
            && sys::window_on_screen();
        let real = self.clock.elapsed().as_secs_f64();
        if painted {
            if let Some(last) = self.last_paint_real {
                self.frozen_now += real - last;
            }
            self.last_paint_real = Some(real);
            self.frozen_now
        } else {
            self.last_paint_real = None;
            self.frozen_now
        }
    }

    fn reveal(&mut self, ctx: &Context) {
        self.window_visible = true;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        ctx.request_repaint();
    }

    fn hide(&mut self, ctx: &Context) {
        self.window_visible = false;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        ctx.request_repaint();
    }

    fn apply_visuals(&self, ctx: &Context) {
        use egui::{Stroke, style::TextCursorStyle};
        let mut visuals = if self.theme.is_dark() {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        // Keep the text rasterization options identical to the ones
        // `install_fonts` set before the first frame. `set_visuals` replaces
        // the whole `Visuals` (including `text_options`); if these drifted,
        // egui would rebuild the font atlas on the next pass, and galleys the
        // app keeps across frames (the header title's segments) would be left
        // with stale atlas UVs and render as garbled fragments.
        crate::ui::text::apply_text_options(&mut visuals);
        // macOS system text selection (translucent); selected glyphs keep
        // their own color (Chrome behavior).
        visuals.selection.bg_fill = self.pal.selection;
        visuals.selection.stroke = Stroke::new(1.0, self.pal.text_strong);
        visuals.text_cursor = TextCursorStyle {
            stroke: Stroke::new(1.0, self.pal.text_strong),
            ..Default::default()
        };
        ctx.set_visuals(visuals);
    }

    fn refresh_cooldowns(&mut self) {
        self.cooldowns = self.state.active_cooldowns();
    }

    /// Take the requests that arrived from the bridge (before the UI
    /// existed, or between passes) and drop the ones withdrawn after their
    /// deadline (an agent hook stopped waiting: no decision can be made for
    /// them any more).
    fn sync_queue(&mut self, ctx: &Context) {
        let incoming = self.state.drain_outbox();
        if !incoming.is_empty() {
            for req in incoming {
                self.queue.push_back(req);
            }
            // The window comes forward (and takes the focus) over whatever
            // the user was pointing at or typing in, even when it was
            // already partly on screen: a click or key aimed there must not
            // decide the front card (`shortcuts::click_armed`, `armed_at`).
            self.revealed_at = ctx.input(|i| i.time);
            self.reveal(ctx);
            ctx.request_repaint();
        }

        let withdrawn = self.state.drain_withdrawn();
        if !withdrawn.is_empty() {
            self.queue.retain(|r| !withdrawn.contains(&r.id));
            // The front card left under a focused control (`track_current`
            // resets the focus for the next card): what Enter and Escape
            // would now do is not what the user is pressing them for. Only
            // a withdrawal does this; a decision by the user advances the
            // queue without it.
            let front_withdrawn = self
                .current_id
                .as_ref()
                .is_some_and(|id| withdrawn.contains(id));
            if front_withdrawn && self.focus.focused.is_some() && !self.queue.is_empty() {
                self.orphaned_focus = true;
            }
            // (The reveal hook shows the window without going through
            // `reveal`, so `window_visible` cannot tell whether it is up.)
            if self.queue.is_empty() {
                self.state.hide_if_drained();
                self.hide(ctx);
            }
            ctx.request_repaint();
        }
    }

    /// Reset the per-request UI state when the current request changes:
    /// feedback text and visibility, focus and transient submission state
    /// all belong to exactly one request.
    fn track_current(&mut self, ctx: &Context) {
        let current_id = self.queue.front().map(|r| r.id.clone());
        if current_id == self.current_id {
            return;
        }
        self.current_id = current_id;
        self.front_since = ctx.input(|i| i.time);
        self.feedback_open = false;
        self.feedback_text.clear();
        self.submit_error = false;
        self.deciding = false;
        self.focus = FocusTracker::default();
        // egui's focus too: the buttons keep their ids from card to card,
        // so a focus left on one would otherwise be the new card's (and the
        // keys follow egui's focus; see `adopt_egui_focus`).
        self.sync_egui_focus(ctx);
        if let Some(req) = self.queue.front() {
            self.ta_id = Id::unique("feedback").with(&req.id);
        } else {
            // No card is left for an orphaned key to decide.
            self.orphaned_focus = false;
        }
        // The card height may have changed: clamp (never reset) the scroll.
        self.scroll_offset = self.scroll_offset.clamp(0.0, self.max_scroll);
    }

    /// The egui id of a card control.
    fn control_id(&self, control: FocusId) -> Id {
        match control {
            FocusId::Textarea => self.ta_id,
            button => button_id(button),
        }
    }

    /// The card control egui widget `id` is, if it is one.
    fn control_of(&self, id: Id) -> Option<FocusId> {
        CONTROLS.into_iter().find(|&f| self.control_id(f) == id)
    }

    /// The card control an AccessKit node is, if it is one (node ids are
    /// derived from egui ids).
    fn control_of_node(&self, node: egui::accesskit::NodeId) -> Option<FocusId> {
        CONTROLS
            .into_iter()
            .find(|&f| self.control_id(f).accesskit_id() == node)
    }

    /// Give egui's focus to the tracker's control; with none, egui's focus
    /// goes nowhere (whatever widget had it).
    fn sync_egui_focus(&self, ctx: &Context) {
        let target = self.focus.focused.map(|f| self.control_id(f));
        ctx.memory_mut(|m| match target {
            Some(id) => m.request_focus(id),
            None => {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            }
        });
    }

    /// The keys act on the control egui's focus is on (`focused`): the
    /// paint pass moves it to the textarea on a click, and VoiceOver or
    /// Full Keyboard Access move it to any control (AccessKit focus
    /// requests). With no egui focus, the tracker's own focus stands.
    fn adopt_egui_focus(&mut self, focused: Option<Id>) {
        self.foreign_focus = false;
        if let Some(id) = focused {
            self.follow_focus(self.control_of(id));
        }
    }

    /// egui's focus is (or is about to be) on `control`, or, with `None`,
    /// on a widget that is none of the card's controls. A button focused
    /// that way is focused with the keyboard (its ring shows, and Enter or
    /// Space presses it). On any other widget, Enter and Escape do nothing
    /// (`foreign_focus`) and no control has the focus.
    fn follow_focus(&mut self, control: Option<FocusId>) {
        self.foreign_focus = control.is_none();
        match control {
            Some(FocusId::Textarea) => {
                if self.focus.focused != Some(FocusId::Textarea) {
                    let keyboard = self.focus.keyboard;
                    self.focus.focus(FocusId::Textarea, keyboard);
                }
            }
            Some(button) => self.focus.focus(button, true),
            None => self.focus.blur(),
        }
    }

    fn scroll_by(&mut self, ctx: &Context, dy: f32, now: f64) {
        let new = (self.scroll_offset + dy).clamp(0.0, self.max_scroll);
        if (new - self.scroll_offset).abs() > 1e-6 {
            self.scroll_offset = new;
            self.scroll.activity(now);
            ctx.request_repaint();
        }
    }

    /// Remember when the primary pointer button went down (see `decide`):
    /// at the earliest, when the batch before this one was taken.
    fn track_press(&mut self, ctx: &Context) {
        if ctx.input(|i| i.pointer.primary_pressed()) {
            self.pressed_at = self.prev_batch_at;
        }
    }

    /// The human decided. Mirrors the reference's `decide` with the required
    /// correction: the queue advances only after an accepted decision; a
    /// rejected submission retains the request and shows an error.
    fn decide(&mut self, ctx: &Context, decision: Decision) {
        let Some(current) = self.queue.front() else {
            return;
        };
        // Only the tracked current card: one that took the front in this
        // pass (a key decided or a hook withdrew the card before it) is
        // never decided before `track_current` has reset its per-request
        // state and stamped `front_since`.
        if self.deciding || self.current_id.as_ref() != Some(&current.id) {
            return;
        }
        // A pointer press that began before this card appeared belongs to
        // the card that was there (withdrawn or decided since): never let it
        // decide this one. A press in the very pass that made it the front
        // card was not aimed at it either (it had not been painted yet).
        let stale_click =
            ctx.input(|i| i.pointer.primary_released()) && self.pressed_at <= self.front_since;
        if stale_click {
            return;
        }
        self.deciding = true;
        self.submit_error = false;
        let id = current.id.clone();
        let result = if self.demo.fail_submit {
            // Demo: exercise the real rejection path (unknown/stale ID).
            self.state.submit("__demo_fail__", decision)
        } else {
            self.state.submit(&id, decision)
        };
        match result {
            Ok(()) => {
                if self.demo.sticky_deciding {
                    // Demo: freeze in the submitting state.
                    ctx.request_repaint();
                    return;
                }
                self.queue.pop_front();
                self.deciding = false;
                if self.queue.is_empty() {
                    self.state.hide_if_drained();
                    self.hide(ctx);
                }
                self.refresh_cooldowns();
                ctx.request_repaint();
            }
            Err(e) => {
                eprintln!("slack-approval-gateway: decision for {id} was not accepted: {e}");
                self.deciding = false;
                self.submit_error = true;
                ctx.request_repaint();
            }
        }
    }

    /// A click on Approve, Deny or Submit feedback: decide, unless the press
    /// came too soon to be aimed at this card (`shortcuts::click_armed`).
    /// Returns false for such an ignored click (the caller drops the focus
    /// it gave the button).
    pub(crate) fn decide_click(&mut self, ctx: &Context, decision: Decision) -> bool {
        if !shortcuts::click_armed(
            self.pressed_at,
            self.front_since,
            self.on_screen_since,
            self.restarted_at(),
        ) {
            return false;
        }
        self.decide(ctx, decision);
        true
    }

    /// The latest event that restarts both arming delays: new requests
    /// brought the window forward, or the user re-established an orphaned
    /// focus.
    fn restarted_at(&self) -> f64 {
        self.revealed_at.max(self.refocused_at)
    }

    /// When the keys arm for the tracked front card (`shortcuts::armed_at`).
    fn keys_armed_at(&self) -> Option<f64> {
        let front = self
            .queue
            .front()
            .filter(|r| self.current_id.as_ref() == Some(&r.id))
            .map(|_| self.front_since);
        shortcuts::armed_at(
            front,
            self.focused_since,
            self.last_typed_at,
            self.restarted_at(),
            self.deciding,
        )
    }

    /// A press counts as armed only when the batch before the one that
    /// brings it was taken after arming. Ask for a frame at the next time
    /// the keys or the clicks arm, so that such a batch exists when the
    /// user acts (with no input, eframe runs no frame).
    fn schedule_arming(&mut self, ctx: &Context) {
        let now = ctx.input(|i| i.time);
        let clicks = self.queue.front().map(|_| {
            shortcuts::clicks_armed_at(self.front_since, self.on_screen_since, self.restarted_at())
        });
        self.wake_at = [self.keys_armed_at(), clicks]
            .into_iter()
            .flatten()
            .filter(|&t| t > now)
            .min_by(f64::total_cmp);
        if let Some(at) = self.wake_at {
            // A little after: the frame must not start before arming.
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(at - now + 0.002));
        }
    }

    /// Drive the pending-count badge from the queue length. The reference
    /// renders `<Tag>` only while `queue.length > 0`, and `useSpring(value)`
    /// initializes at the source value — so the badge mounts showing the
    /// current count (no 0→N animation) and only springs between counts
    /// while mounted. Unmounting snaps the (invisible) spring back to zero.
    fn update_badge(&mut self, now: f64) {
        let len = self.queue.len() as i64;
        if len > 0 && !self.badge_mounted {
            self.badge.jump(len, now);
            self.badge_mounted = true;
        } else if len == 0 && self.badge_mounted {
            self.badge.jump(0, now);
            self.badge_mounted = false;
        } else {
            self.badge.set(len, now);
        }
    }

    fn update_window_title(&mut self, ctx: &Context) {
        let n = self.queue.len();
        let title = if n > 0 {
            format!("{APP_TITLE} — {n} pending")
        } else {
            APP_TITLE.to_string()
        };
        if self.window_title.as_deref() != Some(title.as_str()) {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.window_title = Some(title);
        }
    }

    // -----------------------------------------------------------------------
    // Desktop integration (macOS)
    // -----------------------------------------------------------------------

    #[cfg(target_os = "macos")]
    fn setup_desktop(&mut self, _ctx: &Context) {
        use tray_icon::menu::accelerator::{Accelerator, Code, Modifiers};
        use tray_icon::menu::{
            AboutMetadataBuilder, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
        };
        use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

        let ctx_slot = Arc::clone(&self.ctx_slot);
        let state = Arc::clone(&self.state);
        let quit_flag = Arc::clone(&self.quit_requested);
        let inject = Arc::clone(&self.pending_inject);

        // One global menu-event handler for the tray menu and the app menu.
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            match event.id.as_ref() {
                "sag-show" => reveal_slot(&ctx_slot),
                "sag-quit" | "app-quit" => {
                    quit_flag.store(true, Ordering::SeqCst);
                    state.quit();
                }
                "app-undo" => inject_key(&inject, Key::Z, egui::Modifiers::MAC_CMD),
                "app-redo" => inject_key(&inject, Key::Z, egui::Modifiers::MAC_CMD | egui::Modifiers::SHIFT),
                "app-cut" => inject.lock().unwrap().push(Event::Cut),
                "app-copy" => inject.lock().unwrap().push(Event::Copy),
                "app-paste" => {
                    let t = sys::clipboard_get().unwrap_or_default();
                    inject.lock().unwrap().push(Event::Paste(t));
                }
                "app-select-all" => inject_key(&inject, Key::A, egui::Modifiers::MAC_CMD),
                _ => {}
            }
        }));

        // Menu-bar tray icon: Show / Quit (the gateway keeps running while
        // the window is hidden; the tray is how the human reopens or quits).
        if let Some((rgba, w, h)) = load_icon_rgba() {
            let icon = match tray_icon::Icon::from_rgba(rgba, w, h) {
                Ok(icon) => icon,
                Err(e) => {
                    eprintln!("slack-approval-gateway: cannot load tray icon: {e}");
                    return;
                }
            };
            let menu = match Menu::with_items(&[
                &MenuItem::with_id("sag-show", "Show approval window", true, None),
                &MenuItem::with_id("sag-quit", "Quit SpeakEasy", true, None),
            ]) {
                Ok(menu) => menu,
                Err(e) => {
                    eprintln!("slack-approval-gateway: cannot build tray menu: {e}");
                    return;
                }
            };
            match TrayIconBuilder::new().with_id("slack-approval-gateway")
                .with_menu(Box::new(menu))
                // A left click opens the window (the menu is right-click
                // only); tray-icon's default shows the menu on left click.
                .with_menu_on_left_click(false)
                .with_icon(icon)
                .with_icon_as_template(true)
                .with_tooltip(APP_TITLE)
                .build()
            {
                Ok(tray) => self.tray = Some(tray),
                Err(e) => eprintln!("slack-approval-gateway: cannot set up tray icon: {e}"),
            }
        }

        let ctx_slot2 = Arc::clone(&self.ctx_slot);
        TrayIconEvent::set_event_handler(Some(move |event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                reveal_slot(&ctx_slot2);
            }
        }));

        // The application menu (the reference's default macOS menu). Quit and
        // the Edit items are normal items wired to the UI event loop: the
        // predefined quit would `terminate:` and bypass the gateway's
        // shutdown coordination, and the predefined Edit selectors have no
        // first responder in a native (non-webview) window.
        let about = AboutMetadataBuilder::new()
            .name(Some(APP_TITLE))
            .version(Some(env!("CARGO_PKG_VERSION")))
            .build();
        let app_menu = match Submenu::with_items(
            APP_TITLE,
            true,
            &[
                &PredefinedMenuItem::about(Some("About SpeakEasy"), Some(about)),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(Some("Hide SpeakEasy")),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::separator(),
                &MenuItem::with_id(
                    "app-quit",
                    "Quit SpeakEasy",
                    true,
                    Some(Accelerator::new(Modifiers::META, Code::KeyQ)),
                ),
            ],
        ) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("slack-approval-gateway: cannot build app menu: {e}");
                return;
            }
        };
        let edit_menu = match Submenu::with_items(
            "Edit",
            true,
            &[
                &MenuItem::with_id(
                    "app-undo",
                    "Undo",
                    true,
                    Some(Accelerator::new(Modifiers::META, Code::KeyZ)),
                ),
                &MenuItem::with_id(
                    "app-redo",
                    "Redo",
                    true,
                    Some(Accelerator::new(Modifiers::META | Modifiers::SHIFT, Code::KeyZ)),
                ),
                &PredefinedMenuItem::separator(),
                &MenuItem::with_id(
                    "app-cut",
                    "Cut",
                    true,
                    Some(Accelerator::new(Modifiers::META, Code::KeyX)),
                ),
                &MenuItem::with_id(
                    "app-copy",
                    "Copy",
                    true,
                    Some(Accelerator::new(Modifiers::META, Code::KeyC)),
                ),
                &MenuItem::with_id(
                    "app-paste",
                    "Paste",
                    true,
                    Some(Accelerator::new(Modifiers::META, Code::KeyV)),
                ),
                &MenuItem::with_id(
                    "app-select-all",
                    "Select All",
                    true,
                    Some(Accelerator::new(Modifiers::META, Code::KeyA)),
                ),
            ],
        ) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("slack-approval-gateway: cannot build edit menu: {e}");
                return;
            }
        };
        let view_menu = match Submenu::with_items("View", true, &[&PredefinedMenuItem::fullscreen(None)]) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("slack-approval-gateway: cannot build view menu: {e}");
                return;
            }
        };
        let window_menu = match Submenu::with_items(
            "Window",
            true,
            &[
                &PredefinedMenuItem::minimize(None),
                &PredefinedMenuItem::maximize(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::close_window(None),
            ],
        ) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("slack-approval-gateway: cannot build window menu: {e}");
                return;
            }
        };
        let help_menu = Submenu::new("Help", true);
        match Menu::with_items(&[&app_menu, &edit_menu, &view_menu, &window_menu, &help_menu]) {
            Ok(menu) => {
                menu.init_for_nsapp();
                self.app_menu = Some(menu);
            }
            Err(e) => eprintln!("slack-approval-gateway: cannot build menu bar: {e}"),
        }

        // Dock reopen: the reference's Tauri `Reopen` event. macOS cannot
        // distinguish a Dock click from other activations here, so this also
        // fires on Cmd+Tab etc.; the reveal is a no-op while visible.
        let ctx_slot3 = Arc::clone(&self.ctx_slot);
        self.dock_observer = sys::observe_dock_reopen(move || reveal_slot(&ctx_slot3));

        // The OS key-repeat flag of Enter/Escape/Space (egui-winit drops
        // it; see `shortcuts::HeldKeys`).
        self.key_down_monitor = sys::monitor_shortcut_key_downs();
    }

    #[cfg(not(target_os = "macos"))]
    fn setup_desktop(&mut self, _ctx: &Context) {}
}

#[cfg(target_os = "macos")]
fn inject_key(inject: &Arc<Mutex<Vec<Event>>>, key: Key, modifiers: egui::Modifiers) {
    inject.lock().unwrap().push(Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            mac_cmd: modifiers.mac_cmd,
            command: modifiers.mac_cmd || modifiers.command,
            shift: modifiers.shift,
            ..Default::default()
        },
    });
}

/// The card's focusable controls.
const CONTROLS: [FocusId; 5] = [
    FocusId::Textarea,
    FocusId::SubmitFeedback,
    FocusId::GiveFeedback,
    FocusId::Deny,
    FocusId::Approve,
];

fn button_id(id: FocusId) -> Id {
    match id {
        FocusId::Textarea => Id::unique("focus-textarea"),
        FocusId::SubmitFeedback => Id::unique("btn-submit-feedback"),
        FocusId::GiveFeedback => Id::unique("btn-give-feedback"),
        FocusId::Deny => Id::unique("btn-deny"),
        FocusId::Approve => Id::unique("btn-approve"),
    }
}

/// Center the window on the primary monitor (the reference's `"center":
/// true`). Called once, at creation; later reveals never re-center, so a
/// window the user moved stays where they put it.
fn center_on_primary_monitor(frame: &eframe::Frame) {
    let Some(window) = frame.winit_window() else {
        return;
    };
    let Some(monitor) = window.primary_monitor() else {
        return;
    };
    let size = window.outer_size();
    let m_pos = monitor.position();
    let m_size = monitor.size();
    let x = m_pos.x + (m_size.width as i32 - size.width as i32) / 2;
    let y = m_pos.y + (m_size.height as i32 - size.height as i32) / 2;
    let _ = window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
}

/// Reveal the (hidden) window from outside the UI pass (hooks, tray, menus).
fn reveal_slot(ctx_slot: &Arc<Mutex<Option<Context>>>) {
    if let Some(ctx) = ctx_slot.lock().unwrap().as_ref() {
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        ctx.request_repaint();
    }
}

fn load_icon_rgba() -> Option<(Vec<u8>, u32, u32)> {
    let bytes = include_bytes!("../../icons/icon.png");
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (w, h) = (img.width(), img.height());
    Some((img.into_raw(), w, h))
}

// ---------------------------------------------------------------------------
// eframe::App
// ---------------------------------------------------------------------------

impl eframe::App for ApprovalApp {
    fn logic(&mut self, ctx: &Context, frame: &mut eframe::Frame) {
        self.track_press(ctx);
        if let Some(rs) = frame.wgpu_render_state() {
            self.effects.set_render_state(rs.clone());
        }
        let now = self.anim_now(ctx);
        self.frame_now = now;

        // The responsive rem metrics for the current window width.
        let width = ctx.input(|i| i.viewport_rect().width());
        let m = Metrics::new(width);
        if !self.initialized {
            self.initialized = true;
            *self.ctx_slot.lock().unwrap() = Some(ctx.clone());
            // The reference window is centered (tauri.conf.json `"center":
            // true`). Center it on the primary monitor once, at creation: the
            // OS placement otherwise depends on which display happens to be
            // frontmost, which can put the window on another monitor.
            center_on_primary_monitor(frame);
            // Theme: `SLACK_GATEWAY_THEME=light|dark` overrides; otherwise the
            // system appearance (the reference's `prefers-color-scheme`).
            let theme = match std::env::var("SLACK_GATEWAY_THEME").as_deref() {
                Ok("light") => Theme::Light,
                Ok("dark") => Theme::Dark,
                _ => match ctx.system_theme() {
                    Some(egui::Theme::Dark) => Theme::Dark,
                    _ => Theme::Light,
                },
            };
            if theme != self.theme {
                self.theme = theme;
                self.pal = Palette::for_theme(theme);
            }
            self.apply_visuals(ctx);
            self.title_segs =
                components::title_segments(ctx, &m, APP_TITLE, self.pal.text_strong);
            self.title_spec = m.title;
            self.title_shared = vec![None; self.title_segs.len()];
            self.title_ppp = ctx.pixels_per_point();
            self.setup_desktop(ctx);
            self.refresh_cooldowns();
            // eframe-wgpu force-shows the window after its first painted
            // frame; the reference starts hidden (`visible: false`), so hide
            // it again in the same event-loop turn (no visible flash).
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            if self.is_demo {
                self.reveal(ctx);
            }
        } else if m.title != self.title_spec {
            // The window crossed a rem breakpoint: re-layout the title at the
            // new size (the reveal animation itself is time-based and keeps
            // running, exactly like the reference's re-render on resize).
            self.title_segs =
                components::title_segments(ctx, &m, APP_TITLE, self.pal.text_strong);
            self.title_spec = m.title;
            self.title_shared = vec![None; self.title_segs.len()];
            self.title_ppp = ctx.pixels_per_point();
        }



        // New and withdrawn requests.
        self.sync_queue(ctx);

        // The title reveal starts when the window is first shown (a hidden
        // webview throttles rAF in the reference, so the animation effectively
        // plays on first reveal).
        if !self.title_started && self.window_visible && !self.reduce_motion {
            self.title_started = true;
            self.title_fx.start(now);
        }

        // Per-request state reset (feedback, focus, submission state).
        self.track_current(ctx);

        // Keyboard (read in raw_input_hook, applied here in order).
        self.handle_keys(ctx, now);
        self.schedule_arming(ctx);

        // The close button only hides the window (never quits the gateway,
        // never approves anything).
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quit_requested.load(Ordering::SeqCst)
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.hide(ctx);
        }

        // Clipboard bridge: egui's CopyText output goes to the system
        // pasteboard (egui's own clipboard feature is disabled).
        let copy_text = ctx.output(|o| {
            o.commands.iter().find_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t.clone()),
                _ => None,
            })
        });
        if let Some(text) = copy_text {
            sys::clipboard_set(&text);
        }

        // Wheel scroll (raw events, only while the pointer is over main).
        let pointer = ctx.pointer_latest_pos();
        let over_main = pointer.map(|p| self.main_rect.contains(p)).unwrap_or(false);
        if over_main {
            let wheels = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::MouseWheel { delta, unit, .. } => Some((*delta, *unit)),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            });
            for (delta, unit) in wheels {
                let dy = match unit {
                    egui::MouseWheelUnit::Line => delta.y * LINE_STEP,
                    egui::MouseWheelUnit::Point => delta.y,
                    egui::MouseWheelUnit::Page => delta.y * self.main_rect.height(),
                };
                self.scroll_by(ctx, -dy, now);
            }
        }

        self.last_pointer = pointer;

        // The pending-count badge: mount semantics — snap to the count when
        // the queue first becomes non-empty (the reference mounts the Tag
        // with `useSpring(queue.length)`), spring between counts afterwards.
        self.update_badge(now);

        // Scrollbar idle fade.
        self.scroll.update(now);

        // Window title.
        self.update_window_title(ctx);

        // Keep painting while anything is animating; stop when settled.
        let mut repaint = false;
        if !self.reduce_motion && self.title_started && !self.title_fx.is_done(now) {
            repaint = true;
        }
        if !self.reduce_motion && !self.badge.is_settled(now) {
            repaint = true;
        }
        if self.scroll.fading(now) {
            repaint = true;
        }
        if now < self.hover_repaint_until {
            repaint = true;
        }
        if repaint {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.paint_frame(ui);
    }

    fn raw_input_hook(&mut self, ctx: &Context, raw_input: &mut egui::RawInput) {
        // Events injected by the app menu (Edit items).
        {
            let mut q = self.pending_inject.lock().unwrap();
            if !q.is_empty() {
                raw_input.events.append(&mut *q);
            }
        }
        // Paste bridge: egui-winit's in-app clipboard is empty (the
        // `clipboard` feature is off), so replace Paste events with the
        // system clipboard's text.
        if let Some(t) = sys::clipboard_get() {
            for e in raw_input.events.iter_mut() {
                if matches!(e, Event::Paste(_)) {
                    *e = Event::Paste(t.clone());
                }
            }
        }
        self.extract_keys(
            ctx,
            raw_input,
            sys::window_on_screen(),
            sys::shortcut_keys_down,
            sys::take_shortcut_key_downs(),
        );
    }

    fn clear_color(&self, _visuals: &Visuals) -> [f32; 4] {
        self.pal.page.to_normalized_gamma_f32()
    }
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------

impl ApprovalApp {
    /// Read the batch's keyboard input in event order, before egui sees it.
    ///
    /// Tab/Shift+Tab move the focus (egui's own focus navigation never sees
    /// them). Enter, Escape and Space go through `shortcuts::key_action`:
    /// they decide, activate the button focused with the keyboard or leave
    /// the textarea, and plain Escape never reaches egui. Arrows, Home/End,
    /// PageUp/PageDown and Space scroll the main area whenever the textarea
    /// is not focused (buttons do not consume them, and with nothing focused
    /// the document scrolls, exactly like the browser). Focus moves apply at
    /// once, so a later key in the same batch is judged with the focus it
    /// lands on; decisions, button activations and scrolling are queued for
    /// `handle_keys`. Text input and key presses other than
    /// Enter/Escape/Tab re-arm the keys. The keys act on the control egui's
    /// focus is on when an assistive technology moved it
    /// (`adopt_egui_focus`, and AccessKit focus requests in the batch).
    ///
    /// Some effects exist only after the logic and paint passes: a queued
    /// decision or button activation, and a click (it moves the focus in
    /// the paint pass). Once the batch has one, every later Enter, Escape
    /// and Space press in the batch is judged as not armed: it can still
    /// type in (or leave) the textarea it lands on, but it decides and
    /// activates nothing. Tab still moves the focus; when Give feedback is
    /// activated, the tab order is updated at once for the controls the
    /// panel adds or removes (`queue_activate`).
    ///
    /// `on_screen`: the window is actually shown. `keys_down`: the keys the
    /// OS reports down (`sys::shortcut_keys_down`), read when the window gets
    /// the focus. `key_downs`: the OS key-downs of Enter, Escape and Space
    /// since the last batch, in order, with the OS key-repeat flag
    /// (`sys::take_shortcut_key_downs`); the n-th press of a key in the batch
    /// is its n-th key-down there.
    fn extract_keys(
        &mut self,
        ctx: &Context,
        raw_input: &mut egui::RawInput,
        on_screen: bool,
        keys_down: impl FnOnce() -> Vec<Key>,
        key_downs: Vec<(Key, bool)>,
    ) {
        let now = raw_input.time.unwrap_or_else(|| ctx.input(|i| i.time));
        // A press in this batch went down after the previous batch was
        // taken; that is all that is known of its time.
        self.prev_batch_at = self.batch_at;
        self.batch_at = now;
        // Clicks arm from when the window came on screen (focused or not:
        // macOS delivers the first click to an inactive window).
        if !on_screen {
            self.on_screen_since = None;
        } else if self.on_screen_since.is_none() {
            self.on_screen_since = Some(now);
        }
        // The keys arm from when the window (on screen) got the focus.
        // Key-ups after a focus loss go to the other app, so held keys are
        // forgotten then (egui does the same with its own key state), and no
        // button activates on a Space key-up.
        let lost_focus = raw_input
            .events
            .iter()
            .any(|e| matches!(e, Event::WindowFocused(false)));
        let window_focused = raw_input.focused && on_screen;
        if lost_focus || !window_focused {
            self.focused_since = None;
            self.held_keys.clear();
            self.focus.space_down = None;
        }
        if window_focused && self.focused_since.is_none() {
            self.focused_since = Some(now);
            // A key already down when the window gets the focus (pressed in
            // the other app, or held through a focus steal) is held: none of
            // its repeats acts, even one that comes after arming.
            for key in keys_down() {
                self.held_keys.hold(key, now);
            }
        }
        // The front card as of the last pass; a new one is stamped
        // (`track_current`) and arms from the next pass.
        let front = self
            .queue
            .front()
            .filter(|r| self.current_id.as_ref() == Some(&r.id))
            .map(|r| r.id.clone());
        // The keys act on the control egui's focus is on (the paint pass
        // moves it to a clicked textarea; assistive technologies move it
        // anywhere).
        self.adopt_egui_focus(ctx.memory(|m| m.focused()));
        let mut batch = KeyBatch {
            front,
            unsettled: false,
            os_repeats: key_downs,
        };
        let mut i = 0;
        while i < raw_input.events.len() {
            let event = &raw_input.events[i];
            if self.read_key_event(ctx, event, now, &mut batch) {
                raw_input.events.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Read one raw input event (see `extract_keys`). Returns true when the
    /// app takes the event (egui never sees it).
    fn read_key_event(
        &mut self,
        ctx: &Context,
        event: &Event,
        now: f64,
        batch: &mut KeyBatch,
    ) -> bool {
        let &Event::Key {
            key,
            pressed,
            repeat,
            modifiers,
            ..
        } = event
        else {
            match event {
                Event::Text(_) | Event::Ime(_) | Event::Paste(_) => self.last_typed_at = now,
                // A click (its press, or the release that completes it)
                // moves the focus only in the paint pass. A press in the
                // window also re-establishes the focus after an orphaning
                // withdrawal: that click decides nothing, and the keys and
                // clicks arm again from it.
                Event::PointerButton {
                    button: egui::PointerButton::Primary,
                    pressed,
                    ..
                } => {
                    batch.unsettled = true;
                    if *pressed {
                        self.refocus_orphaned(now);
                    }
                }
                // An assistive technology moves the focus (egui applies it
                // in this pass): later keys act on that control, and on any
                // other widget Enter and Escape do nothing.
                Event::AccessKitActionRequest(request)
                    if request.action == egui::accesskit::Action::Focus =>
                {
                    self.follow_focus(self.control_of_node(request.target_node));
                }
                _ => {}
            }
            return false;
        };
        let front = batch.front.clone();
        let focused = self.focus.key_target();
        match key {
            Key::Tab => {
                if !(pressed && (!modifiers.any() || modifiers.shift_only())) {
                    return false;
                }
                self.refocus_orphaned(now);
                self.focus.move_focus(!modifiers.shift);
                self.sync_egui_focus(ctx);
                self.foreign_focus = false;
                ctx.request_repaint();
                true
            }
            Key::Enter | Key::Escape | Key::Space if !pressed => {
                self.held_keys.release(key);
                let on_button = focused.is_some_and(|f| f != FocusId::Textarea);
                if key != Key::Space || !on_button {
                    return false;
                }
                // A Space released on the button it went down on (still
                // focused) activates it.
                let down_on = self.focus.space_down.take();
                if let (Some(button), Some(request)) = (down_on, front) {
                    if focused == Some(button) && !batch.unsettled {
                        self.queue_activate(button, request);
                        batch.unsettled = true;
                    }
                }
                true
            }
            Key::Enter | Key::Escape | Key::Space => {
                // The n-th press of the key in this batch is its n-th OS
                // key-down: whether the OS reported a key-repeat.
                let os_repeat = batch
                    .os_repeats
                    .iter()
                    .position(|&(k, _)| k == key)
                    .is_some_and(|at| batch.os_repeats.remove(at).1);
                let (held, down_at) = self.held_keys.press(key, self.prev_batch_at, os_repeat);
                let armed_at = if batch.unsettled || front.is_none() {
                    None
                } else {
                    self.keys_armed_at()
                };
                let mut action = shortcuts::key_action(
                    key,
                    modifiers,
                    repeat || held,
                    down_at,
                    armed_at,
                    focused,
                );
                if self.foreign_focus && key != Key::Space {
                    // egui's focus is on a widget that is none of the
                    // card's controls (an assistive technology put it
                    // there): what the key would do there is not ours.
                    action = KeyAction::Swallow;
                }
                if self.orphaned_focus {
                    action = shortcuts::while_orphaned(action);
                    // A fresh plain Escape does what it would have done in
                    // the field that vanished: it leaves it, nothing more.
                    if key == Key::Escape && !modifiers.any() && !(repeat || held) {
                        self.refocus_orphaned(now);
                    }
                }
                if key == Key::Space {
                    // Space is typing too (once the press itself is judged).
                    self.last_typed_at = now;
                }
                match action {
                    KeyAction::PassThrough => return self.read_scroll_key(key, modifiers),
                    KeyAction::Swallow => {}
                    KeyAction::BlurTextarea => {
                        self.focus.blur();
                        self.sync_egui_focus(ctx);
                        ctx.request_repaint();
                    }
                    KeyAction::Approve | KeyAction::Deny => {
                        if let Some(request) = front {
                            let decision = if action == KeyAction::Approve {
                                Decision::Approve
                            } else {
                                Decision::Deny
                            };
                            self.pending_keys
                                .push(PendingKey::Decide { decision, request });
                            batch.unsettled = true;
                        }
                    }
                    KeyAction::Activate(button) => {
                        if let Some(request) = front {
                            self.queue_activate(button, request);
                            batch.unsettled = true;
                        }
                    }
                    KeyAction::PressButton(button) => {
                        self.focus.space_down = Some(button);
                    }
                }
                true
            }
            _ if pressed => {
                self.last_typed_at = now;
                self.read_scroll_key(key, modifiers)
            }
            _ => false,
        }
    }

    /// The user re-established an orphaned focus (Tab, a plain Escape, or
    /// a click in the window, at `now`): the keys act again, and they and
    /// the clicks arm again from now, so that key or click (and the ones
    /// right after it, still meant for the control that vanished) decides
    /// nothing.
    fn refocus_orphaned(&mut self, now: f64) {
        if self.orphaned_focus {
            self.orphaned_focus = false;
            self.refocused_at = now;
        }
    }

    /// Queue a keyboard activation of `button` on `request` (applied in
    /// `handle_keys`). Give feedback toggles the panel, which adds (or
    /// removes) the feedback textarea and an enabled Submit feedback before
    /// it in the tab order. The paint pass rebuilds the order only after
    /// this batch, so make the same change now: a later Tab or Shift+Tab in
    /// the batch then moves through the controls that will be there.
    fn queue_activate(&mut self, button: FocusId, request: String) {
        if button == FocusId::GiveFeedback && !self.deciding {
            let order = &mut self.focus.order;
            order.retain(|&f| f != FocusId::Textarea && f != FocusId::SubmitFeedback);
            if !self.feedback_open {
                let at = order
                    .iter()
                    .position(|&f| f == FocusId::GiveFeedback)
                    .unwrap_or(0);
                let mut panel = vec![FocusId::Textarea];
                if !self.feedback_text.trim().is_empty() {
                    panel.push(FocusId::SubmitFeedback);
                }
                order.splice(at..at, panel);
            }
        }
        self.pending_keys.push(PendingKey::Activate { button, request });
    }

    /// Queue a scroll key: arrows, Home/End, PageUp/PageDown, and Space
    /// with no control to act on (nothing focused, or a button focused by a
    /// click; Shift allowed: Shift+Space pages up). The textarea keeps them
    /// (cursor movement), and Cmd/Ctrl/Alt combinations are left alone.
    /// Returns true when the key was taken.
    fn read_scroll_key(&mut self, key: Key, modifiers: egui::Modifiers) -> bool {
        if self.focus.focused == Some(FocusId::Textarea)
            || modifiers.command
            || modifiers.mac_cmd
            || modifiers.alt
            || modifiers.ctrl
        {
            return false;
        }
        let page = (self.main_rect.height() - PAGE_OVERLAP).max(0.0);
        let pending = match key {
            Key::ArrowUp => PendingKey::Scroll(-LINE_STEP),
            Key::ArrowDown => PendingKey::Scroll(LINE_STEP),
            Key::ArrowLeft | Key::ArrowRight => PendingKey::Scroll(0.0),
            Key::Home => PendingKey::ScrollTo(0.0),
            Key::End => PendingKey::ScrollTo(f32::MAX),
            Key::PageUp => PendingKey::Scroll(-page),
            Key::PageDown => PendingKey::Scroll(page),
            Key::Space if self.focus.key_target().is_none() => {
                PendingKey::Page(if modifiers.shift { -1.0 } else { 1.0 })
            }
            _ => return false,
        };
        self.pending_keys.push(pending);
        true
    }

    /// Apply what `extract_keys` queued, in order: decisions and button
    /// activations (each only while the card its key was pressed on is
    /// still in front), and scrolling.
    fn handle_keys(&mut self, ctx: &Context, now: f64) {
        let pending_keys = std::mem::take(&mut self.pending_keys);
        for key in pending_keys {
            match key {
                // Never let a key decide a card that replaced the one it was
                // pressed on (withdrawn or decided since).
                PendingKey::Decide { decision, request } => {
                    if self.is_front(&request) {
                        self.decide(ctx, decision);
                    }
                }
                PendingKey::Activate { button, request } => {
                    if self.is_front(&request) {
                        self.activate(ctx, button);
                    }
                }
                PendingKey::Scroll(dy) => {
                    if dy != 0.0 {
                        self.scroll_by(ctx, dy, now);
                    }
                }
                PendingKey::Page(sign) => {
                    let page = (self.main_rect.height() - PAGE_OVERLAP).max(0.0);
                    self.scroll_by(ctx, sign * page, now);
                }
                PendingKey::ScrollTo(target) => {
                    let to = if target == f32::MAX {
                        self.max_scroll
                    } else {
                        target
                    };
                    self.scroll_by(ctx, to - self.scroll_offset, now);
                }
            }
        }
        // A key decided the front card: track the next one now, so this pass
        // paints it with its own (reset) state, and a click in this pass
        // counts as pressed before it appeared (see `decide`).
        self.track_current(ctx);
    }

    fn is_front(&self, request: &str) -> bool {
        self.queue.front().is_some_and(|r| r.id == request)
    }

    /// Enter, or a Space release, on the focused `button`: what a click on
    /// it does.
    fn activate(&mut self, ctx: &Context, button: FocusId) {
        // Every button is disabled while a decision is being submitted.
        if self.deciding {
            return;
        }
        match button {
            FocusId::Approve => self.decide(ctx, Decision::Approve),
            FocusId::Deny => self.decide(ctx, Decision::Deny),
            FocusId::GiveFeedback => {
                self.feedback_open = !self.feedback_open;
                ctx.request_repaint();
            }
            FocusId::SubmitFeedback => {
                // Shown only with the panel, enabled only with text.
                let text = self.feedback_text.trim();
                if self.feedback_open && !text.is_empty() {
                    let decision = Decision::Feedback(text.to_string());
                    self.decide(ctx, decision);
                }
            }
            FocusId::Textarea => {}
        }
    }
}
