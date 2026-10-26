//! The native approval popup: the eframe application and its desktop
//! lifecycle (tray icon, menus, window show/hide/quit, clipboard, keyboard).
//!
//! The layout engine and the painting of every region live in
//! [`crate::ui::app::paint`]; this module owns the app state, the event-loop
//! integration (`eframe::App`), and the macOS desktop integration.

pub mod paint;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use eframe::egui;
use egui::{Context, Event, Id, Key, Pos2, Rect, ViewportCommand, Visuals};
use epaint::text::Galley;

use crate::core::{Decision, MessageRequest};
use crate::ui::anim::{AnimatedNumber, SegmentState, TextEffect};
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

/// A key event extracted in `raw_input_hook` (so egui's own focus navigation
/// never sees it) and processed in `App::logic`.
enum PendingKey {
    Tab { shift: bool },
    Escape,
    /// Scroll the main area by this many points (arrow keys).
    Scroll(f32),
    /// Scroll by ±one page (Space / Shift+Space, like the browser).
    Page(f32),
    /// Scroll to an absolute offset (Home/End); `f32::MAX` means bottom.
    ScrollTo(f32),
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

    /// Reset the per-request UI state when the current request changes:
    /// feedback text and visibility, focus and transient submission state
    /// all belong to exactly one request.
    fn track_current(&mut self, ctx: &Context) {
        let current_id = self.queue.front().map(|r| r.id.clone());
        if current_id == self.current_id {
            return;
        }
        if let Some(old) = self.current_id.take() {
            ctx.memory_mut(|m| m.surrender_focus(Id::unique("feedback").with(old)));
        }
        self.current_id = current_id;
        self.feedback_open = false;
        self.feedback_text.clear();
        self.submit_error = false;
        self.deciding = false;
        self.focus = FocusTracker::default();
        if let Some(req) = self.queue.front() {
            self.ta_id = Id::unique("feedback").with(&req.id);
        }
        // The card height may have changed: clamp (never reset) the scroll.
        self.scroll_offset = self.scroll_offset.clamp(0.0, self.max_scroll);
    }

    fn sync_egui_focus(&self, ctx: &Context) {
        let target = match self.focus.focused {
            Some(FocusId::Textarea) => Some(self.ta_id),
            Some(id) => Some(button_id(id)),
            None => None,
        };
        ctx.memory_mut(|m| match target {
            Some(id) => m.request_focus(id),
            None => {
                m.surrender_focus(self.ta_id);
                for id in [
                    FocusId::SubmitFeedback,
                    FocusId::GiveFeedback,
                    FocusId::Deny,
                    FocusId::Approve,
                ] {
                    m.surrender_focus(button_id(id));
                }
            }
        });
    }

    fn scroll_by(&mut self, ctx: &Context, dy: f32, now: f64) {
        let new = (self.scroll_offset + dy).clamp(0.0, self.max_scroll);
        if (new - self.scroll_offset).abs() > 1e-6 {
            self.scroll_offset = new;
            self.scroll.activity(now);
            ctx.request_repaint();
        }
    }

    /// The human decided. Mirrors the reference's `decide` with the required
    /// correction: the queue advances only after an accepted decision; a
    /// rejected submission retains the request and shows an error.
    fn decide(&mut self, ctx: &Context, decision: Decision) {
        let Some(current) = self.queue.front() else {
            return;
        };
        if self.deciding {
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
            .version(Some("0.1.0"))
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



        // Requests that arrived before the UI existed (or between passes).
        let incoming = self.state.drain_outbox();
        if !incoming.is_empty() {
            for req in incoming {
                self.queue.push_back(req);
            }
            self.reveal(ctx);
            ctx.request_repaint();
        }

        // The title reveal starts when the window is first shown (a hidden
        // webview throttles rAF in the reference, so the animation effectively
        // plays on first reveal).
        if !self.title_started && self.window_visible && !self.reduce_motion {
            self.title_started = true;
            self.title_fx.start(now);
        }

        // Per-request state reset (feedback, focus, submission state).
        self.track_current(ctx);

        // Keyboard (extracted in raw_input_hook so egui's own focus
        // navigation never sees Tab/Escape/arrows/page keys). Arrow, page and
        // space keys scroll the main area whenever the textarea is not
        // focused — like the browser, where buttons do not consume them.
        let pending_keys = std::mem::take(&mut self.pending_keys);
        for key in pending_keys {
            match key {
                PendingKey::Tab { shift } => {
                    self.focus.move_focus(!shift);
                    self.sync_egui_focus(ctx);
                    ctx.request_repaint();
                }
                PendingKey::Escape => {}
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
        // Extract the keys egui's own focus navigation would mishandle, and
        // the scrolling keys the browser would deliver to the scroll
        // container. Tab/Shift+Tab move focus; Escape is swallowed; arrows,
        // Home/End, PageUp/PageDown and Space scroll the main area whenever
        // the textarea is not focused (buttons do not consume them, and with
        // nothing focused the document scrolls — exactly like the browser).
        let ta_focused = ctx.memory(|m| m.focused()) == Some(self.ta_id);
        let nothing_focused = self.focus.focused.is_none();
        // Shift is allowed (Shift+Arrow / Shift+Space scroll like the plain
        // keys); Cmd/Ctrl/Alt combinations are left alone.
        let scroll_mods = |modifiers: &egui::Modifiers| {
            !modifiers.command && !modifiers.mac_cmd && !modifiers.alt && !modifiers.ctrl
        };
        let mut i = 0;
        while i < raw_input.events.len() {
            let extract = match &raw_input.events[i] {
                Event::Key {
                    key: Key::Tab,
                    pressed: true,
                    modifiers,
                    ..
                } => !modifiers.any() || modifiers.shift_only(),
                Event::Key {
                    key: Key::Escape,
                    pressed: true,
                    modifiers,
                    ..
                } => !modifiers.any(),
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if scroll_mods(modifiers) && !ta_focused => {
                    matches!(
                        key,
                        Key::ArrowUp
                            | Key::ArrowDown
                            | Key::ArrowLeft
                            | Key::ArrowRight
                            | Key::Home
                            | Key::End
                            | Key::PageUp
                            | Key::PageDown
                    )
                }
                Event::Key {
                    key: Key::Space,
                    pressed: true,
                    modifiers,
                    ..
                } if scroll_mods(modifiers) && !ta_focused && nothing_focused => true,
                _ => false,
            };
            if !extract {
                i += 1;
                continue;
            }
            let ev = raw_input.events.remove(i);
            if let Event::Key { key, modifiers, .. } = ev {
                let page = (self.main_rect.height() - PAGE_OVERLAP).max(0.0);
                match key {
                    Key::Tab => self.pending_keys.push(PendingKey::Tab { shift: modifiers.shift }),
                    Key::Escape => self.pending_keys.push(PendingKey::Escape),
                    Key::ArrowUp => self.pending_keys.push(PendingKey::Scroll(-LINE_STEP)),
                    Key::ArrowDown => self.pending_keys.push(PendingKey::Scroll(LINE_STEP)),
                    Key::ArrowLeft | Key::ArrowRight => {
                        self.pending_keys.push(PendingKey::Scroll(0.0))
                    }
                    Key::Home => self.pending_keys.push(PendingKey::ScrollTo(0.0)),
                    Key::End => self.pending_keys.push(PendingKey::ScrollTo(f32::MAX)),
                    Key::PageUp => self.pending_keys.push(PendingKey::Scroll(-page)),
                    Key::PageDown => self.pending_keys.push(PendingKey::Scroll(page)),
                    Key::Space => {
                        self.pending_keys.push(PendingKey::Page(if modifiers.shift {
                            -1.0
                        } else {
                            1.0
                        }))
                    }
                    _ => {}
                }
            }
        }
    }

    fn clear_color(&self, _visuals: &Visuals) -> [f32; 4] {
        self.pal.page.to_normalized_gamma_f32()
    }
}
