//! The keyboard decision gate: plain Enter approves and plain Escape
//! denies the front request, through the same `decide` path as the
//! buttons, and Enter/Space activate a button focused with the keyboard
//! (Tab, or an assistive technology that moves egui's focus).
//!
//! A decision has real consequences (Approve sends a Slack message, Deny
//! blocks the purpose for a cooldown), and the window steals the focus when
//! a request arrives, so a key meant for another app, or typed for the
//! feedback field of a card that was just replaced, can land here. A key
//! can therefore decide (or activate a button) only [`ARM_DELAY`] after the
//! latest of: the request became the front card, the window got the focus,
//! the user last typed, a new request brought the window forward, and the
//! user re-established an orphaned focus (see below). Only a fresh press
//! acts: the key went down after arming, so not a key-repeat (the OS flag,
//! or a key seen down already) and not a key that was already down when
//! the window got the focus. A press is known only to have gone down after
//! the previous input batch was taken, so it counts as armed only when
//! that batch was taken after arming; the app asks for a frame at the
//! arming time so that such a batch exists. The gate itself is the pure
//! [`key_action`]; `ApprovalApp::extract_keys` feeds it every raw key event
//! in order, with the focus as moved by the Tab presses before it.
//!
//! Three more rules keep a key on the card and control it was meant for:
//! - Enter and Space act on a button only when it was focused with the
//!   keyboard (its focus ring is visible). A button focused by a click
//!   counts as no focus: Enter approves and Space scrolls.
//! - When the front card is withdrawn while one of its controls has the
//!   focus, the keys the user is pressing were meant for that control.
//!   Until the user moves the focus again (Tab, a click, or a plain Escape,
//!   which only does that), nothing decides or activates
//!   ([`while_orphaned`]); the key or click that moves it decides nothing
//!   itself and restarts both arming delays.
//! - In one input batch, a key after a queued decision or button
//!   activation, or after a click, is judged as not armed: the focus and
//!   the controls it would act on change only in the logic and paint
//!   passes.
//!
//! Clicks on the decision buttons have their own, shorter delay
//! ([`click_armed`]): a click that was aimed at the card before this one,
//! or at another app the window just appeared over or came in front of,
//! decides nothing.

use std::collections::HashMap;

use egui::{Key, Modifiers};

use crate::ui::components::FocusId;

/// Seconds that must pass, with the window on screen and focused, after the
/// current request became the front card and after the user last typed,
/// before a key can decide it or activate a button.
///
/// A key that is already down when the window gets the focus is marked as
/// held from the OS key state (`sys::shortcut_keys_down`), so its repeats
/// never act, however long the macOS "Delay until repeat" is. If the OS
/// does not report such a key as down, only this delay protects against it:
/// it is longer than the usual delay before a held key starts repeating, so
/// the key's first repeat comes before arming and the key counts as held
/// from then on. With the longest "Delay until repeat" settings, the first
/// repeat can come after arming.
pub(crate) const ARM_DELAY: f64 = 1.0;

/// What an Enter, Escape or Space press does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyAction {
    /// Leave the key to egui: a newline or a space in the textarea, a
    /// modified Escape, or (for the caller's scroll keys) Space with nothing
    /// focused.
    PassThrough,
    /// Drop the key: it must not reach egui either.
    Swallow,
    Approve,
    Deny,
    /// Leave the feedback textarea; its text and the panel stay.
    BlurTextarea,
    /// Enter on this focused button: do what a click on it does.
    Activate(FocusId),
    /// Space went down on this focused button. The button activates when
    /// Space is released while it still has the focus (like a native button).
    PressButton(FocusId),
}

/// When the keys arm for the current card: [`ARM_DELAY`] after the latest of
/// `front_since` (it became the front card), `focused_since` (the window got
/// the focus), `typed_at` (the last text input, or key press other than
/// Enter, Escape and Tab: an Enter or Escape meant for the feedback field
/// of a card that was replaced must not decide the new one) and
/// `restarted_at` (a new request brought the window forward, or the user
/// re-established an orphaned focus). `None` (never) with no card, while
/// the window is hidden or unfocused, or while a decision is being
/// submitted.
pub(crate) fn armed_at(
    front_since: Option<f64>,
    focused_since: Option<f64>,
    typed_at: f64,
    restarted_at: f64,
    deciding: bool,
) -> Option<f64> {
    if deciding {
        return None;
    }
    Some(
        front_since?
            .max(focused_since?)
            .max(typed_at)
            .max(restarted_at)
            + ARM_DELAY,
    )
}

/// Gate one Enter, Escape or Space press (other keys pass through).
/// `repeat`: the key was already down (a key-repeat, or a key held since
/// before the window got the focus); `down_at`: the earliest time the key
/// can have gone down (the time of the input batch before the one that
/// brought the press); `focused`: the control the key acts on
/// (`FocusTracker::key_target`: the textarea, or a button focused with the
/// keyboard).
pub(crate) fn key_action(
    key: Key,
    modifiers: Modifiers,
    repeat: bool,
    down_at: f64,
    armed_at: Option<f64>,
    focused: Option<FocusId>,
) -> KeyAction {
    let plain = !modifiers.any();
    // The press must start once armed: no repeat, no key held from before.
    let fresh = !repeat && armed_at.is_some_and(|t| down_at >= t);
    let in_textarea = focused == Some(FocusId::Textarea);
    match key {
        // Enter edits the feedback text; it never decides from there.
        Key::Enter if in_textarea => KeyAction::PassThrough,
        // Everything else Enter can do (activate a focused button, approve)
        // needs a fresh plain press.
        Key::Enter if !(plain && fresh) => KeyAction::Swallow,
        Key::Enter => match focused {
            None | Some(FocusId::Approve) => KeyAction::Approve,
            Some(button) => KeyAction::Activate(button),
        },
        // Modified Escape is not ours (egui ignores it too).
        Key::Escape if !plain => KeyAction::PassThrough,
        Key::Escape if in_textarea => KeyAction::BlurTextarea,
        Key::Escape if fresh => KeyAction::Deny,
        Key::Escape => KeyAction::Swallow,
        // Space types in the textarea and scrolls with nothing focused. On a
        // focused button it is gated exactly like Enter.
        Key::Space => match focused {
            None | Some(FocusId::Textarea) => KeyAction::PassThrough,
            Some(button) if plain && fresh => KeyAction::PressButton(button),
            Some(_) => KeyAction::Swallow,
        },
        _ => KeyAction::PassThrough,
    }
}

/// What a key does while the focus is orphaned: the front card was
/// withdrawn while one of its controls had the focus, and the user has not
/// moved the focus since. Nothing decides and no button activates; Enter in
/// a textarea, Escape leaving it, and Space scrolling are left as they are.
pub(crate) fn while_orphaned(action: KeyAction) -> KeyAction {
    match action {
        KeyAction::Approve
        | KeyAction::Deny
        | KeyAction::Activate(_)
        | KeyAction::PressButton(_) => KeyAction::Swallow,
        other => other,
    }
}

/// Seconds after the card became the front card, after the window came on
/// screen, and after a new request brought the window forward or the user
/// re-established an orphaned focus, before a click on Approve, Deny or
/// Submit feedback decides it. A click pressed sooner was aimed at what was
/// there before: the card that was just withdrawn or decided, the app the
/// window appeared over, or the control that vanished with the card. No one
/// reads a request and clicks in this time.
pub(crate) const CLICK_ARM: f64 = 0.5;

/// When clicks arm for the front card: [`CLICK_ARM`] after the latest of
/// `front_since` (it became the front card), `on_screen_since` (the window
/// came on screen; `None`: not known to be on screen, which adds no delay
/// of its own) and `restarted_at` (as for [`armed_at`]).
pub(crate) fn clicks_armed_at(
    front_since: f64,
    on_screen_since: Option<f64>,
    restarted_at: f64,
) -> f64 {
    let since = on_screen_since.map_or(front_since, |t| t.max(front_since));
    since.max(restarted_at) + CLICK_ARM
}

/// Whether a click whose press went down no earlier than `pressed_after`
/// (the time of the input batch before the one that brought the press) may
/// decide the front card (see [`clicks_armed_at`]).
pub(crate) fn click_armed(
    pressed_after: f64,
    front_since: f64,
    on_screen_since: Option<f64>,
    restarted_at: f64,
) -> bool {
    pressed_after >= clicks_armed_at(front_since, on_screen_since, restarted_at)
}

/// When each gated key (Enter, Escape, Space) went down (absent: up).
/// egui-winit drops the OS key-repeat flag (egui recomputes it later, from
/// its own key state, which never sees the keys the app extracts), so a
/// press of a key already down is a repeat, and so is a press the OS
/// reported as a repeat (`sys::take_shortcut_key_downs`): the first repeat
/// of a key whose key-down went to a menu, the app switcher or a screenshot
/// selection while the window kept the focus.
#[derive(Debug, Default)]
pub(crate) struct HeldKeys(HashMap<Key, f64>);

impl HeldKeys {
    /// A press of `key` that went down no earlier than `at`; `os_repeat`:
    /// the OS reported it as a key-repeat. Returns whether it repeats a held
    /// key, and when the key went down (at the earliest).
    pub fn press(&mut self, key: Key, at: f64, os_repeat: bool) -> (bool, f64) {
        if let Some(&down_at) = self.0.get(&key) {
            return (true, down_at);
        }
        self.0.insert(key, at);
        (os_repeat, at)
    }

    /// `key` was already down at `now` (the window just got the focus): its
    /// presses are repeats until it is released.
    pub fn hold(&mut self, key: Key, now: f64) {
        self.0.entry(key).or_insert(now);
    }

    pub fn release(&mut self, key: Key) {
        self.0.remove(&key);
    }

    /// The window lost the focus: key-ups now go to another app.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::super::{button_id, ApprovalApp, DemoFlags};
    use super::*;
    use crate::core::{Decision, MessageRequest};
    use crate::ui::demo;
    use crate::ui::theme::Theme;
    use crate::ui::UiState;
    use egui::{vec2, Context, Event, ImeEvent, PointerButton, Pos2, RawInput, Rect};
    use std::sync::mpsc::{self, Receiver};
    use std::sync::{Arc, Mutex};

    const PLAIN: Modifiers = Modifiers::NONE;
    /// Armed at t=10 for every `key_action` case below.
    const ARMED: Option<f64> = Some(10.0);
    const NEVER_TYPED: f64 = f64::NEG_INFINITY;
    /// No restart of the arming delays (`restarted_at`).
    const NEVER: f64 = f64::NEG_INFINITY;
    const BUTTONS: [FocusId; 4] = [
        FocusId::SubmitFeedback,
        FocusId::GiveFeedback,
        FocusId::Deny,
        FocusId::Approve,
    ];

    fn act(key: Key, modifiers: Modifiers, focused: Option<FocusId>) -> KeyAction {
        key_action(key, modifiers, false, 11.0, ARMED, focused)
    }

    #[test]
    fn armed_after_the_delay_from_front_card_window_focus_and_typing() {
        // Whichever came last starts the delay.
        assert_eq!(
            armed_at(Some(5.0), Some(2.0), NEVER_TYPED, NEVER, false),
            Some(5.0 + ARM_DELAY)
        );
        assert_eq!(
            armed_at(Some(2.0), Some(5.0), NEVER_TYPED, NEVER, false),
            Some(5.0 + ARM_DELAY)
        );
        assert_eq!(
            armed_at(Some(2.0), Some(5.0), 7.0, NEVER, false),
            Some(7.0 + ARM_DELAY)
        );
        assert_eq!(
            armed_at(Some(5.0), Some(2.0), 3.0, NEVER, false),
            Some(5.0 + ARM_DELAY)
        );
        // A new request brought the window forward, or the user
        // re-established an orphaned focus, last.
        assert_eq!(
            armed_at(Some(2.0), Some(3.0), 4.0, 8.0, false),
            Some(8.0 + ARM_DELAY)
        );
        // Rule 5: no card, a hidden/unfocused window, or a submission in
        // flight never arms.
        assert_eq!(armed_at(None, Some(2.0), NEVER_TYPED, NEVER, false), None);
        assert_eq!(armed_at(Some(2.0), None, NEVER_TYPED, NEVER, false), None);
        assert_eq!(armed_at(Some(2.0), Some(2.0), NEVER_TYPED, NEVER, true), None);
        assert!((0.75..=1.0).contains(&ARM_DELAY));
    }

    #[test]
    fn plain_enter_approves_and_plain_escape_denies_once_armed() {
        assert_eq!(act(Key::Enter, PLAIN, None), KeyAction::Approve);
        assert_eq!(act(Key::Escape, PLAIN, None), KeyAction::Deny);
        // Exactly at the arming instant counts as armed.
        assert_eq!(
            key_action(Key::Enter, PLAIN, false, 10.0, ARMED, None),
            KeyAction::Approve
        );
    }

    #[test]
    fn presses_before_arming_or_never_armed_do_nothing() {
        // Rule 1: the window just took the focus / the card just arrived.
        for key in [Key::Enter, Key::Escape] {
            assert_eq!(
                key_action(key, PLAIN, false, 9.9, ARMED, None),
                KeyAction::Swallow
            );
            // Rule 5: empty queue, hidden window, deciding.
            assert_eq!(
                key_action(key, PLAIN, false, 11.0, None, None),
                KeyAction::Swallow
            );
        }
    }

    #[test]
    fn key_repeats_and_keys_held_from_before_arming_do_nothing() {
        for key in [Key::Enter, Key::Escape] {
            // A repeat of a press that started after arming.
            assert_eq!(
                key_action(key, PLAIN, true, 11.0, ARMED, None),
                KeyAction::Swallow
            );
            // The key went down before arming and is still held: its
            // repeats arrive after arming, still with the old key-down.
            assert_eq!(
                key_action(key, PLAIN, true, 9.0, ARMED, None),
                KeyAction::Swallow
            );
            assert_eq!(
                key_action(key, PLAIN, false, 9.0, ARMED, None),
                KeyAction::Swallow
            );
        }
    }

    #[test]
    fn only_plain_keys_are_shortcuts() {
        // Rule 2.
        for m in [
            Modifiers::SHIFT,
            Modifiers::COMMAND,
            Modifiers::MAC_CMD,
            Modifiers::CTRL,
            Modifiers::ALT,
        ] {
            // Modified Enter does nothing (and never reaches a focused
            // button); modified Escape is left to egui, which ignores it.
            assert_eq!(act(Key::Enter, m, None), KeyAction::Swallow);
            assert_eq!(act(Key::Enter, m, Some(FocusId::Deny)), KeyAction::Swallow);
            assert_eq!(act(Key::Escape, m, None), KeyAction::PassThrough);
            // Modified Space never activates a focused button either.
            assert_eq!(act(Key::Space, m, Some(FocusId::Deny)), KeyAction::Swallow);
        }
    }

    #[test]
    fn the_feedback_textarea_keeps_enter_and_escape_only_leaves_it() {
        // Rule 3: Enter types a newline (armed or not, any modifiers)...
        let ta = Some(FocusId::Textarea);
        assert_eq!(act(Key::Enter, PLAIN, ta), KeyAction::PassThrough);
        assert_eq!(
            act(Key::Enter, Modifiers::SHIFT, ta),
            KeyAction::PassThrough
        );
        assert_eq!(
            key_action(Key::Enter, PLAIN, true, 9.0, None, ta),
            KeyAction::PassThrough
        );
        // ...Space types a space...
        assert_eq!(
            key_action(Key::Space, PLAIN, true, 9.0, None, ta),
            KeyAction::PassThrough
        );
        // ...and Escape blurs the field instead of denying.
        assert_eq!(act(Key::Escape, PLAIN, ta), KeyAction::BlurTextarea);
        assert_eq!(
            key_action(Key::Escape, PLAIN, false, 9.0, None, ta),
            KeyAction::BlurTextarea
        );
        // The second Escape (the field no longer focused) denies.
        assert_eq!(act(Key::Escape, PLAIN, None), KeyAction::Deny);
    }

    #[test]
    fn a_focused_button_gets_enter_and_escape_still_denies() {
        // Rule 4: Enter activates the focused button...
        for b in [
            FocusId::Deny,
            FocusId::GiveFeedback,
            FocusId::SubmitFeedback,
        ] {
            assert_eq!(act(Key::Enter, PLAIN, Some(b)), KeyAction::Activate(b));
            // ...but not before arming (a focus steal keeps a focused
            // button), nor as a repeat.
            assert_eq!(
                key_action(Key::Enter, PLAIN, false, 9.0, ARMED, Some(b)),
                KeyAction::Swallow
            );
            assert_eq!(
                key_action(Key::Enter, PLAIN, true, 11.0, ARMED, Some(b)),
                KeyAction::Swallow
            );
            assert_eq!(act(Key::Escape, PLAIN, Some(b)), KeyAction::Deny);
        }
        // A focused Approve approves.
        let approve = Some(FocusId::Approve);
        assert_eq!(act(Key::Enter, PLAIN, approve), KeyAction::Approve);
        assert_eq!(act(Key::Escape, PLAIN, approve), KeyAction::Deny);
    }

    #[test]
    fn space_on_a_focused_button_is_gated_like_enter() {
        for b in BUTTONS {
            assert_eq!(act(Key::Space, PLAIN, Some(b)), KeyAction::PressButton(b));
            // Not before arming (a space typed right after a focus steal),
            // never armed, nor a repeat or a key held from before arming.
            assert_eq!(
                key_action(Key::Space, PLAIN, false, 9.9, ARMED, Some(b)),
                KeyAction::Swallow
            );
            assert_eq!(
                key_action(Key::Space, PLAIN, false, 11.0, None, Some(b)),
                KeyAction::Swallow
            );
            assert_eq!(
                key_action(Key::Space, PLAIN, true, 11.0, ARMED, Some(b)),
                KeyAction::Swallow
            );
            assert_eq!(
                key_action(Key::Space, PLAIN, true, 9.0, ARMED, Some(b)),
                KeyAction::Swallow
            );
        }
        // With nothing focused Space scrolls (the caller's scroll keys).
        assert_eq!(
            key_action(Key::Space, PLAIN, true, 9.0, None, None),
            KeyAction::PassThrough
        );
    }

    #[test]
    fn held_keys_report_repeats_until_released() {
        let mut held = HeldKeys::default();
        assert_eq!(held.press(Key::Enter, 1.0, false), (false, 1.0));
        assert_eq!(held.press(Key::Enter, 1.5, false), (true, 1.0));
        assert_eq!(held.press(Key::Escape, 1.6, false), (false, 1.6));
        held.release(Key::Enter);
        assert_eq!(held.press(Key::Enter, 2.0, false), (false, 2.0));
        held.clear();
        assert_eq!(held.press(Key::Escape, 3.0, false), (false, 3.0));
        // A key already down when the window got the focus is held until
        // released (its first repeat may come much later).
        held.hold(Key::Space, 4.0);
        assert_eq!(held.press(Key::Space, 9.0, false), (true, 4.0));
        held.release(Key::Space);
        assert_eq!(held.press(Key::Space, 9.5, false), (false, 9.5));
        // Holding never moves an earlier key-down.
        held.hold(Key::Space, 12.0);
        assert_eq!(held.press(Key::Space, 12.5, false), (true, 9.5));
    }

    #[test]
    fn a_press_the_os_reports_as_a_repeat_is_held_until_released() {
        // The first repeat of a key whose key-down went elsewhere (a menu,
        // the app switcher) while the window kept the focus.
        let mut held = HeldKeys::default();
        assert_eq!(held.press(Key::Enter, 5.0, true), (true, 5.0));
        // Its next repeats are held too, even without the OS flag.
        assert_eq!(held.press(Key::Enter, 5.1, false), (true, 5.0));
        held.release(Key::Enter);
        assert_eq!(held.press(Key::Enter, 6.0, false), (false, 6.0));
    }

    // -----------------------------------------------------------------------
    // End to end: the raw-input hook's key reading, the logic pass and the
    // paint pass.
    // -----------------------------------------------------------------------

    type Reply = Receiver<Result<Decision, crate::core::ProviderError>>;

    /// The demo request, as request `id`.
    fn request(id: &str) -> MessageRequest {
        let mut req = demo::DemoSeed::Single.requests().remove(0);
        req.id = id.to_string();
        req
    }

    /// Queue request `id` in the bridge, as a gateway worker does (the app
    /// takes it on its next frame); its decision arrives on the receiver.
    fn enqueue(state: &UiState, id: &str) -> Reply {
        let (tx, rx) = mpsc::channel();
        state.enqueue(id.to_string(), tx, request(id)).unwrap();
        rx
    }

    /// An app with these requests pending, in order.
    fn app_with_requests(ids: &[&str]) -> (ApprovalApp, Vec<Reply>) {
        let app = ApprovalApp::new(
            Arc::new(UiState::new()),
            Arc::new(Mutex::new(None)),
            Theme::Light,
            false,
            false,
            DemoFlags::default(),
        );
        let replies = ids.iter().map(|id| enqueue(&app.state, id)).collect();
        (app, replies)
    }

    /// An app with request "a" pending.
    fn app_with_request() -> (ApprovalApp, Reply) {
        let (app, mut replies) = app_with_requests(&["a"]);
        (app, replies.remove(0))
    }

    fn approved(rx: &Reply) -> bool {
        matches!(rx.try_recv(), Ok(Ok(Decision::Approve)))
    }

    fn denied(rx: &Reply) -> bool {
        matches!(rx.try_recv(), Ok(Ok(Decision::Deny)))
    }

    fn undecided(rx: &Reply) -> bool {
        rx.try_recv().is_err()
    }

    fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        }
    }

    fn press(k: Key) -> Event {
        key(k, true, PLAIN)
    }

    fn release(k: Key) -> Event {
        key(k, false, PLAIN)
    }

    fn tap(k: Key) -> Vec<Event> {
        vec![press(k), release(k)]
    }

    fn tab() -> Event {
        press(Key::Tab)
    }

    fn shift_tab() -> Event {
        key(Key::Tab, true, Modifiers::SHIFT)
    }

    fn typed(text: &str) -> Event {
        Event::Text(text.to_string())
    }

    /// Space as egui-winit delivers it: the key, its text, the key-up.
    fn space() -> Vec<Event> {
        vec![press(Key::Space), typed(" "), release(Key::Space)]
    }

    fn pointer(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: PLAIN,
        }
    }

    /// The window (tall enough that the whole card, buttons included, is
    /// on screen without scrolling).
    fn screen() -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(720.0, 1400.0))
    }

    /// The raw-input hook's part of a frame at `time`: the key reading,
    /// after the frames the app asked for before `time` (`idle_until`).
    /// `keys_down`: the keys the OS reports down if the window gets the
    /// focus in this frame.
    fn hook(
        ctx: &Context,
        app: &mut ApprovalApp,
        time: f64,
        focused: bool,
        events: Vec<Event>,
        keys_down: &[Key],
    ) -> RawInput {
        idle_until(ctx, app, time);
        frame_input(ctx, app, time, focused, events, keys_down, &[])
    }

    /// The raw-input hook's part of a frame at `time`, with no frame
    /// before it (the frame before took long: the app's own wake-up frames
    /// did not run). `key_downs`: the OS key-downs of the gated keys in this
    /// batch, with the OS key-repeat flag.
    fn frame_input(
        ctx: &Context,
        app: &mut ApprovalApp,
        time: f64,
        focused: bool,
        events: Vec<Event>,
        keys_down: &[Key],
        key_downs: &[(Key, bool)],
    ) -> RawInput {
        let mut input = RawInput {
            time: Some(time),
            focused,
            screen_rect: Some(screen()),
            events,
            ..Default::default()
        };
        app.extract_keys(ctx, &mut input, true, || keys_down.to_vec(), key_downs.to_vec());
        input
    }

    /// Time passes until `time` with no input: the frames the app asked
    /// for (at the times the keys or clicks arm) run, as eframe runs them on
    /// its own. Call it before changing the bridge (a withdrawal, a new
    /// request) "at" `time`, so that the frames before `time` do not see it.
    fn idle_until(ctx: &Context, app: &mut ApprovalApp, time: f64) {
        while let Some(at) = app.wake_at.filter(|&at| at < time) {
            let mut input = RawInput {
                time: Some(at),
                focused: app.focused_since.is_some(),
                screen_rect: Some(screen()),
                ..Default::default()
            };
            let on_screen = app.on_screen_since.is_some();
            app.extract_keys(ctx, &mut input, on_screen, Vec::new, Vec::new());
            run(ctx, app, input);
        }
    }

    /// The rest of the frame: the steps of `logic` that take the bridge's
    /// requests, track the front card, apply the keys and schedule the
    /// arming frames, then the paint pass (where clicks decide). Returns the
    /// repaint delay egui reports.
    fn run(ctx: &Context, app: &mut ApprovalApp, input: RawInput) -> std::time::Duration {
        let time = input.time.unwrap();
        let mut out = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            app.track_press(&ctx);
            app.frame_now = time;
            app.sync_queue(&ctx);
            app.track_current(&ctx);
            app.handle_keys(&ctx, time);
            app.schedule_arming(&ctx);
            app.paint_frame(ui);
        });
        out.textures_delta.clear();
        out.viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(std::time::Duration::MAX, |v| v.repaint_delay)
    }

    /// One frame. Returns the events left for egui.
    fn pass(
        ctx: &Context,
        app: &mut ApprovalApp,
        time: f64,
        focused: bool,
        events: Vec<Event>,
    ) -> Vec<Event> {
        let input = hook(ctx, app, time, focused, events, &[]);
        let left = input.events.clone();
        run(ctx, app, input);
        left
    }

    /// Open the feedback panel on the current card, with `text` in it, and
    /// paint it at `time` (so its controls are in the tab order).
    fn open_feedback(ctx: &Context, app: &mut ApprovalApp, time: f64, text: &str) {
        app.feedback_open = true;
        app.feedback_text = text.to_string();
        pass(ctx, app, time, true, vec![]);
    }

    /// Give the feedback textarea the keyboard focus (app and egui).
    fn focus_textarea(ctx: &Context, app: &mut ApprovalApp) {
        app.focus.focus(FocusId::Textarea, true);
        app.sync_egui_focus(ctx);
    }

    fn has_key_press(events: &[Event], k: Key) -> bool {
        events
            .iter()
            .any(|e| matches!(e, Event::Key { key, pressed: true, .. } if *key == k))
    }

    #[test]
    fn enter_approves_the_front_card_only_once_armed() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        // The card arrives in a focused window at t=1: armed at t=2.
        pass(&ctx, &mut app, 1.0, true, vec![]);
        let left = pass(&ctx, &mut app, 1.5, true, tap(Key::Enter));
        assert!(undecided(&rx), "Enter before arming must not approve");
        assert!(
            !has_key_press(&left, Key::Enter),
            "a swallowed Enter must not reach egui"
        );
        pass(&ctx, &mut app, 2.1, true, tap(Key::Enter));
        assert!(approved(&rx));
        assert!(app.queue.is_empty());
    }

    #[test]
    fn a_key_held_while_the_window_took_the_focus_does_not_decide() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        // The window is up but another app has the focus.
        pass(&ctx, &mut app, 1.0, false, vec![]);
        // It takes the focus at t=5 while Escape is held down there: the
        // key's repeats keep arriving, before and after arming (t=6).
        pass(&ctx, &mut app, 5.0, true, vec![Event::WindowFocused(true)]);
        for t in [5.3, 5.9, 6.1, 6.5] {
            pass(&ctx, &mut app, t, true, vec![press(Key::Escape)]);
        }
        assert!(undecided(&rx), "a held Escape must not deny");
        // Released, then pressed again: a deliberate Escape denies.
        pass(&ctx, &mut app, 6.6, true, vec![release(Key::Escape)]);
        pass(&ctx, &mut app, 6.8, true, tap(Key::Escape));
        assert!(denied(&rx));
    }

    #[test]
    fn a_key_held_through_a_focus_steal_never_decides_however_long_the_repeat_delay() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        // The window is up; another app has the focus and Escape is held
        // down there.
        pass(&ctx, &mut app, 1.0, false, vec![]);
        // The window takes the focus at t=5 (armed at t=6); the OS reports
        // Escape down.
        let input = hook(
            &ctx,
            &mut app,
            5.0,
            true,
            vec![Event::WindowFocused(true)],
            &[Key::Escape],
        );
        run(&ctx, &mut app, input);
        // With the longest "Delay until repeat" (1.8 s) the first repeat
        // comes only after arming (egui-winit drops the repeat flag).
        for t in [6.8, 6.9, 7.0] {
            pass(&ctx, &mut app, t, true, vec![press(Key::Escape)]);
        }
        assert!(undecided(&rx), "a held Escape must not deny");
        // Released, then pressed again: a deliberate Escape denies.
        pass(&ctx, &mut app, 7.1, true, vec![release(Key::Escape)]);
        pass(&ctx, &mut app, 7.3, true, tap(Key::Escape));
        assert!(denied(&rx));
    }

    #[test]
    fn space_held_through_a_focus_steal_never_activates_a_focused_button() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        // Approve focused with Shift+Tab.
        pass(&ctx, &mut app, 1.5, true, vec![shift_tab()]);
        assert_eq!(app.focus.focused, Some(FocusId::Approve));
        // Away, and back at t=5 with Space held down in the other app.
        pass(
            &ctx,
            &mut app,
            2.0,
            false,
            vec![Event::WindowFocused(false)],
        );
        let input = hook(
            &ctx,
            &mut app,
            5.0,
            true,
            vec![Event::WindowFocused(true)],
            &[Key::Space],
        );
        run(&ctx, &mut app, input);
        // Its first repeats come after arming (t=6), then it is released.
        for t in [6.8, 6.9] {
            pass(&ctx, &mut app, t, true, vec![press(Key::Space), typed(" ")]);
        }
        pass(&ctx, &mut app, 7.0, true, vec![release(Key::Space)]);
        assert!(undecided(&rx), "a held Space must not approve");
        // A deliberate Space approves (armed again a second after the
        // repeats' typing).
        pass(&ctx, &mut app, 8.0, true, space());
        assert!(approved(&rx));
    }

    #[test]
    fn losing_the_focus_rearms_and_a_hidden_window_never_decides() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        // Away and back at t=3: armed again only at t=4.
        pass(
            &ctx,
            &mut app,
            2.5,
            false,
            vec![Event::WindowFocused(false)],
        );
        pass(&ctx, &mut app, 3.0, true, vec![Event::WindowFocused(true)]);
        pass(&ctx, &mut app, 3.5, true, tap(Key::Enter));
        assert!(undecided(&rx));
        // Not on screen: nothing, however long it has been.
        let mut input = RawInput {
            time: Some(9.0),
            focused: true,
            events: tap(Key::Enter),
            ..Default::default()
        };
        app.extract_keys(&ctx, &mut input, false, Vec::new, Vec::new());
        assert!(app.pending_keys.is_empty());
        assert!(undecided(&rx));
    }

    #[test]
    fn nothing_happens_while_deciding_or_with_an_empty_queue() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        app.deciding = true;
        pass(&ctx, &mut app, 3.0, true, tap(Key::Enter));
        pass(&ctx, &mut app, 3.2, true, tap(Key::Escape));
        assert!(undecided(&rx));
        app.deciding = false;
        app.queue.clear();
        pass(&ctx, &mut app, 4.0, true, tap(Key::Enter));
        pass(&ctx, &mut app, 6.0, true, tap(Key::Enter));
        assert!(app.pending_keys.is_empty());
        assert!(undecided(&rx));
    }

    #[test]
    fn escape_leaves_the_textarea_then_denies() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        open_feedback(&ctx, &mut app, 1.1, "Shorter, please");
        focus_textarea(&ctx, &mut app);
        // Enter in the field is left to the text edit: a newline.
        let left = pass(&ctx, &mut app, 2.5, true, tap(Key::Enter));
        assert!(
            has_key_press(&left, Key::Enter),
            "Enter must reach the textarea"
        );
        assert!(app.feedback_text.contains('\n'));
        assert!(undecided(&rx));
        // Escape leaves the field; the text and the panel stay.
        let text = app.feedback_text.clone();
        pass(&ctx, &mut app, 3.0, true, tap(Key::Escape));
        assert!(undecided(&rx), "Escape in the textarea must not deny");
        assert_eq!(app.focus.focused, None);
        assert!(app.feedback_open);
        assert_eq!(app.feedback_text, text);
        // A second Escape denies.
        pass(&ctx, &mut app, 3.5, true, tap(Key::Escape));
        assert!(denied(&rx));
    }

    #[test]
    fn a_press_never_decides_the_card_that_replaced_its_own() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        // Enter on the armed card A...
        let input = hook(&ctx, &mut app, 3.0, true, tap(Key::Enter), &[]);
        // ...which is withdrawn before the logic pass applies the key; B
        // takes the front.
        assert!(app.state.withdraw("a"));
        let rx_b = enqueue(&app.state, "b");
        run(&ctx, &mut app, input);
        assert!(undecided(&rx_b), "B must not be approved");
        assert!(undecided(&rx));
        assert_eq!(app.queue.len(), 1);
    }

    #[test]
    fn holding_escape_in_the_textarea_only_leaves_it() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        open_feedback(&ctx, &mut app, 1.1, "");
        focus_textarea(&ctx, &mut app);
        // Pressed once armed and held: the press leaves the field, and its
        // repeats (the field no longer focused) are not a second Escape.
        for t in [3.0, 3.5, 3.6, 3.7] {
            pass(&ctx, &mut app, t, true, vec![press(Key::Escape)]);
        }
        assert_eq!(app.focus.focused, None);
        assert!(undecided(&rx), "a held Escape must not deny");
    }

    #[test]
    fn enter_activates_a_focused_button_and_a_focused_approve_decides_once() {
        let (mut app, rx) = app_with_requests(&["a", "b", "c"]);
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        // Enter on a focused Give feedback opens the panel; the event never
        // reaches egui.
        app.focus.focus(FocusId::GiveFeedback, true);
        let left = pass(&ctx, &mut app, 2.5, true, tap(Key::Enter));
        assert!(!has_key_press(&left, Key::Enter));
        assert!(app.feedback_open);
        assert!(undecided(&rx[0]));
        // On a focused Deny it denies A.
        app.focus.focus(FocusId::Deny, true);
        pass(&ctx, &mut app, 2.7, true, tap(Key::Enter));
        assert!(denied(&rx[0]));
        // On a focused Approve it approves B, once: C stays.
        assert_eq!(app.current_id.as_deref(), Some("b"));
        app.focus.focus(FocusId::Approve, true);
        pass(&ctx, &mut app, 3.8, true, tap(Key::Enter));
        assert!(approved(&rx[1]));
        assert!(undecided(&rx[2]));
        assert_eq!(app.queue.len(), 1);
    }

    #[test]
    fn typing_re_arms() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        // Armed at t=2...
        pass(&ctx, &mut app, 1.0, true, vec![]);
        // ...but text typed at t=3 re-arms the keys at t=4.
        pass(&ctx, &mut app, 3.0, true, vec![typed("x")]);
        pass(&ctx, &mut app, 3.5, true, tap(Key::Enter));
        pass(&ctx, &mut app, 3.6, true, tap(Key::Escape));
        assert!(undecided(&rx));
        // So do IME input and any other key press (an arrow key here):
        // armed again at t=4.9.
        pass(
            &ctx,
            &mut app,
            3.8,
            true,
            vec![Event::Ime(ImeEvent::Commit("é".to_string()))],
        );
        pass(&ctx, &mut app, 3.9, true, tap(Key::ArrowDown));
        pass(&ctx, &mut app, 4.5, true, tap(Key::Enter));
        assert!(undecided(&rx));
        // Enter and Escape are not typing: a second after the last
        // keystroke, Enter approves.
        pass(&ctx, &mut app, 4.95, true, tap(Key::Enter));
        assert!(approved(&rx));
    }

    #[test]
    fn front_card_replaced_while_the_textarea_had_focus_then_enter() {
        let (mut app, rx_a) = app_with_request();
        let ctx = Context::default();
        // The user opens A's feedback field and types into it.
        pass(&ctx, &mut app, 1.0, true, vec![]);
        open_feedback(&ctx, &mut app, 1.1, "");
        focus_textarea(&ctx, &mut app);
        for (t, text) in [(2.0, "Too"), (2.5, " long"), (3.0, ",")] {
            pass(&ctx, &mut app, t, true, vec![typed(text)]);
        }
        assert_eq!(app.feedback_text, "Too long,");
        // A's hook stops waiting and B arrives: B is the front card from
        // t=3.2, with no field.
        assert!(app.state.withdraw("a"));
        let rx_b = enqueue(&app.state, "b");
        pass(&ctx, &mut app, 3.2, true, vec![typed(" ")]);
        assert_eq!(app.current_id.as_deref(), Some("b"));
        assert_eq!(app.focus.focused, None);
        assert!(app.orphaned_focus);
        // The user keeps typing, and more than a second after B appeared
        // presses Enter for a new line.
        for (t, text) in [(3.6, "please"), (4.0, ".")] {
            pass(&ctx, &mut app, t, true, vec![typed(text)]);
        }
        pass(&ctx, &mut app, 4.3, true, tap(Key::Enter));
        // A pause, then another new line, a second after the last
        // keystroke: still nothing (the field it is meant for is gone).
        pass(&ctx, &mut app, 5.1, true, tap(Key::Enter));
        assert!(
            undecided(&rx_b),
            "keys meant for A's feedback must not decide B"
        );
        assert!(undecided(&rx_a));
        // Escape (to leave that field) only gives the keys back; they arm
        // again a second later.
        pass(&ctx, &mut app, 5.2, true, tap(Key::Escape));
        assert!(undecided(&rx_b), "Escape must only leave the lost field");
        assert!(!app.orphaned_focus);
        pass(&ctx, &mut app, 5.4, true, tap(Key::Enter));
        assert!(undecided(&rx_b), "the keys arm again only a second later");
        // Then a deliberate Enter approves B.
        pass(&ctx, &mut app, 6.3, true, tap(Key::Enter));
        assert!(approved(&rx_b));
    }

    #[test]
    fn two_keys_in_one_batch_decide_only_the_first_card() {
        let (mut app, rx) = app_with_requests(&["a", "b", "c"]);
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        // Deny focused with Tab (Give feedback, then Deny).
        pass(&ctx, &mut app, 1.5, true, vec![tab(), tab()]);
        assert_eq!(app.focus.focused, Some(FocusId::Deny));
        // One batch: Escape denies A, then Enter on the focused Deny.
        pass(
            &ctx,
            &mut app,
            2.6,
            true,
            [tap(Key::Escape), tap(Key::Enter)].concat(),
        );
        assert!(denied(&rx[0]));
        assert!(undecided(&rx[1]), "the second key must not deny B");
        // B was tracked in that pass: painted with its own state, armed a
        // second later.
        assert_eq!(app.current_id.as_deref(), Some("b"));
        assert_eq!(app.front_since, 2.6);
        assert_eq!(app.focus.focused, None);
        // Two Enters in one batch: B is approved, C is not.
        pass(
            &ctx,
            &mut app,
            3.7,
            true,
            [tap(Key::Enter), tap(Key::Enter)].concat(),
        );
        assert!(approved(&rx[1]));
        assert!(undecided(&rx[2]), "the second Enter must not approve C");
        assert_eq!(app.queue.len(), 1);
    }

    #[test]
    fn decide_waits_until_the_new_front_card_is_tracked() {
        let (mut app, rx) = app_with_requests(&["a", "b"]);
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        let input = |time| RawInput {
            time: Some(time),
            screen_rect: Some(screen()),
            ..Default::default()
        };
        // A left the queue in this pass; B is in front, still untracked
        // (A's per-request state, A's `front_since`).
        app.queue.pop_front();
        let mut out = ctx.run_ui(input(2.0), |ui| app.decide(ui.ctx(), Decision::Approve));
        out.textures_delta.clear();
        assert!(
            undecided(&rx[1]),
            "B must not be decided before it is tracked"
        );
        // Once tracked, B can be decided.
        let mut out = ctx.run_ui(input(2.1), |ui| {
            app.track_current(ui.ctx());
            app.decide(ui.ctx(), Decision::Approve);
        });
        out.textures_delta.clear();
        assert!(approved(&rx[1]));
    }

    #[test]
    fn a_key_and_a_click_in_one_batch_decide_only_the_first_card() {
        let (mut app, rx) = app_with_requests(&["a", "b", "c"]);
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        // Every card has the same content: its Approve button is here.
        let approve = ctx
            .read_response(button_id(FocusId::Approve))
            .expect("the Approve button is painted")
            .rect
            .center();
        assert!(app.main_rect.contains(approve));
        let moved = Event::PointerMoved(approve);
        // One batch: Enter approves A, and a click (aimed at A) lands on
        // the Approve button.
        pass(
            &ctx,
            &mut app,
            2.5,
            true,
            [
                tap(Key::Enter),
                vec![
                    moved.clone(),
                    pointer(approve, true),
                    pointer(approve, false),
                ],
            ]
            .concat(),
        );
        assert!(approved(&rx[0]));
        assert!(undecided(&rx[1]), "the click must not approve B");
        // A click pressed in the batch with the key and released in the
        // next one is not B's either.
        pass(
            &ctx,
            &mut app,
            3.6,
            true,
            [tap(Key::Enter), vec![moved.clone(), pointer(approve, true)]].concat(),
        );
        assert!(approved(&rx[1]));
        pass(&ctx, &mut app, 3.7, true, vec![pointer(approve, false)]);
        assert!(undecided(&rx[2]), "the click must not approve C");
        // A deliberate click on C approves it.
        pass(
            &ctx,
            &mut app,
            5.0,
            true,
            vec![moved, pointer(approve, true)],
        );
        pass(&ctx, &mut app, 5.1, true, vec![pointer(approve, false)]);
        assert!(approved(&rx[2]));
    }

    #[test]
    fn a_space_on_a_focused_button_is_a_deliberate_press_too() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        // Deny focused with Tab.
        pass(&ctx, &mut app, 1.5, true, vec![tab(), tab()]);
        assert_eq!(app.focus.focused, Some(FocusId::Deny));
        // Another app takes the focus; the window takes it back at t=5
        // (armed at t=6). A space typed right after it does nothing.
        pass(
            &ctx,
            &mut app,
            2.0,
            false,
            vec![Event::WindowFocused(false)],
        );
        pass(&ctx, &mut app, 5.0, true, vec![Event::WindowFocused(true)]);
        pass(&ctx, &mut app, 5.2, true, space());
        assert!(
            undecided(&rx),
            "a space right after a focus steal must not deny"
        );
        // Space down on Deny, Tab away, Space up: nothing activates (like
        // a native button).
        pass(
            &ctx,
            &mut app,
            6.5,
            true,
            vec![press(Key::Space), typed(" "), tab(), release(Key::Space)],
        );
        assert_eq!(app.focus.focused, Some(FocusId::Approve));
        assert!(undecided(&rx));
        // Back on Deny, a deliberate Space (armed a second after the last
        // one) denies.
        pass(&ctx, &mut app, 7.0, true, vec![shift_tab()]);
        pass(&ctx, &mut app, 7.6, true, space());
        assert!(denied(&rx));
    }

    #[test]
    fn tab_tab_enter_in_one_batch_activates_the_button_tabbed_to() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        // Give feedback, Deny, then Enter: Deny.
        pass(
            &ctx,
            &mut app,
            2.5,
            true,
            [vec![tab(), tab()], tap(Key::Enter)].concat(),
        );
        assert!(denied(&rx));
    }

    #[test]
    fn tab_enter_in_one_batch_opens_the_feedback_panel() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        let left = pass(
            &ctx,
            &mut app,
            2.5,
            true,
            [vec![tab()], tap(Key::Enter)].concat(),
        );
        assert!(!has_key_press(&left, Key::Enter));
        assert_eq!(app.focus.focused, Some(FocusId::GiveFeedback));
        assert!(app.feedback_open);
        assert!(undecided(&rx));
        // Enter, then Tab on to Approve in the same batch: the Enter closes
        // the panel and nothing else (it does not follow the focus).
        pass(
            &ctx,
            &mut app,
            3.0,
            true,
            [tap(Key::Enter), vec![tab(), tab()]].concat(),
        );
        assert!(!app.feedback_open);
        assert_eq!(app.focus.focused, Some(FocusId::Approve));
        assert!(undecided(&rx));
    }

    #[test]
    fn tab_into_the_textarea_then_enter_in_one_batch_types_a_newline() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        // The panel is open: the textarea is the first control.
        pass(&ctx, &mut app, 1.0, true, vec![]);
        open_feedback(&ctx, &mut app, 1.1, "");
        pass(
            &ctx,
            &mut app,
            2.5,
            true,
            [vec![tab()], tap(Key::Enter)].concat(),
        );
        assert_eq!(app.focus.focused, Some(FocusId::Textarea));
        assert_eq!(app.feedback_text, "\n");
        assert!(undecided(&rx));
    }

    #[test]
    fn shift_tab_then_space_in_one_batch_activates_approve() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        pass(
            &ctx,
            &mut app,
            2.5,
            true,
            [vec![shift_tab()], space()].concat(),
        );
        assert!(approved(&rx));
    }

    #[test]
    fn tab_out_of_the_textarea_then_enter_in_one_batch_submits_the_feedback() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        open_feedback(&ctx, &mut app, 1.1, "Shorter, please");
        focus_textarea(&ctx, &mut app);
        // Tab to Submit feedback, then Enter: the feedback is sent, and no
        // newline is typed.
        pass(
            &ctx,
            &mut app,
            2.5,
            true,
            [vec![tab()], tap(Key::Enter)].concat(),
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(Ok(Decision::Feedback(text))) if text == "Shorter, please"
        ));
    }

    // -----------------------------------------------------------------------
    // The front card withdrawn under a focused control (orphaned focus).
    // -----------------------------------------------------------------------

    #[test]
    fn while_orphaned_nothing_decides_or_activates() {
        for action in [
            KeyAction::Approve,
            KeyAction::Deny,
            KeyAction::Activate(FocusId::Deny),
            KeyAction::PressButton(FocusId::Approve),
        ] {
            assert_eq!(while_orphaned(action), KeyAction::Swallow);
        }
        // A newline or a space in a textarea, leaving it, and scrolling
        // stay as they are.
        for action in [
            KeyAction::PassThrough,
            KeyAction::Swallow,
            KeyAction::BlurTextarea,
        ] {
            assert_eq!(while_orphaned(action), action);
        }
    }

    /// A is withdrawn while the user writes feedback for it; B takes the
    /// front at t=2.2.
    fn withdrawn_while_writing_feedback() -> (Context, ApprovalApp, Reply, Reply) {
        let (mut app, rx_a) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        open_feedback(&ctx, &mut app, 1.1, "");
        focus_textarea(&ctx, &mut app);
        pass(&ctx, &mut app, 2.0, true, vec![typed("Too long")]);
        assert_eq!(app.feedback_text, "Too long");
        assert!(app.state.withdraw("a"));
        let rx_b = enqueue(&app.state, "b");
        pass(&ctx, &mut app, 2.2, true, vec![]);
        assert_eq!(app.current_id.as_deref(), Some("b"));
        assert_eq!(app.focus.focused, None);
        assert!(app.orphaned_focus);
        (ctx, app, rx_a, rx_b)
    }

    #[test]
    fn enter_for_a_new_line_in_a_withdrawn_cards_field_never_approves_the_next() {
        let (ctx, mut app, rx_a, rx_b) = withdrawn_while_writing_feedback();
        // The user pauses to think, then presses Enter for a new line, well
        // after B armed (t=3.2); and again much later.
        pass(&ctx, &mut app, 3.4, true, tap(Key::Enter));
        pass(&ctx, &mut app, 8.0, true, tap(Key::Enter));
        assert!(undecided(&rx_b), "Enter meant for A's field must not approve B");
        assert!(undecided(&rx_a));
        assert_eq!(app.queue.len(), 1);
        // Escape (to leave the field) only gives the keys back, a second
        // later...
        pass(&ctx, &mut app, 8.2, true, tap(Key::Escape));
        assert!(undecided(&rx_b), "Escape must only leave the lost field");
        // ...then a deliberate Enter approves B.
        pass(&ctx, &mut app, 9.3, true, tap(Key::Enter));
        assert!(approved(&rx_b));
    }

    #[test]
    fn escape_to_leave_a_withdrawn_cards_field_never_denies_the_next() {
        let (ctx, mut app, _rx_a, rx_b) = withdrawn_while_writing_feedback();
        pass(&ctx, &mut app, 3.4, true, tap(Key::Escape));
        assert!(undecided(&rx_b), "Escape meant to leave A's field must not deny B");
        assert!(!app.orphaned_focus);
        // A key-repeat of an Escape held down since before does not count
        // as that Escape.
        let (ctx, mut app, _rx_a, rx_b) = withdrawn_while_writing_feedback();
        app.held_keys.hold(Key::Escape, 2.2);
        pass(&ctx, &mut app, 3.4, true, vec![press(Key::Escape)]);
        assert!(app.orphaned_focus);
        assert!(undecided(&rx_b));
        pass(&ctx, &mut app, 3.5, true, vec![release(Key::Escape)]);
        // A fresh Escape leaves the lost field; the next, deliberate one (a
        // second later) denies B.
        pass(&ctx, &mut app, 3.6, true, tap(Key::Escape));
        assert!(!app.orphaned_focus);
        assert!(undecided(&rx_b));
        pass(&ctx, &mut app, 4.7, true, tap(Key::Escape));
        assert!(denied(&rx_b));
    }

    /// Deny is focused with Tab on A; A is withdrawn and B takes the front
    /// at t=2.0.
    fn withdrawn_with_deny_focused() -> (Context, ApprovalApp, Vec<Reply>) {
        let (mut app, rx) = app_with_requests(&["a", "b"]);
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        pass(&ctx, &mut app, 1.5, true, vec![tab(), tab()]);
        assert_eq!(app.focus.focused, Some(FocusId::Deny));
        assert!(app.state.withdraw("a"));
        pass(&ctx, &mut app, 2.0, true, vec![]);
        assert_eq!(app.current_id.as_deref(), Some("b"));
        assert_eq!(app.focus.focused, None);
        assert!(app.orphaned_focus);
        (ctx, app, rx)
    }

    #[test]
    fn enter_meant_for_a_withdrawn_cards_focused_deny_never_approves_the_next() {
        let (ctx, mut app, rx) = withdrawn_with_deny_focused();
        // Enter (to deny A, through the Deny button that vanished).
        pass(&ctx, &mut app, 3.1, true, tap(Key::Enter));
        assert!(undecided(&rx[1]), "Enter meant for A's Deny must not approve B");
        // A Space would have pressed that Deny: it only scrolls now.
        pass(&ctx, &mut app, 3.2, true, space());
        assert!(undecided(&rx[1]));
        assert!(undecided(&rx[0]));
        assert_eq!(app.queue.len(), 1);
    }

    #[test]
    fn tab_or_a_click_gives_the_keys_back_after_an_orphaning_withdrawal() {
        // Tab: Give feedback, then Deny; Enter on the Deny the user tabbed
        // to denies B, once the keys arm again (a second after the Tab).
        let (ctx, mut app, rx) = withdrawn_with_deny_focused();
        pass(&ctx, &mut app, 3.1, true, vec![tab()]);
        assert!(!app.orphaned_focus);
        pass(&ctx, &mut app, 3.2, true, vec![tab()]);
        pass(&ctx, &mut app, 3.3, true, tap(Key::Enter));
        assert!(undecided(&rx[1]), "the keys arm again only a second later");
        pass(&ctx, &mut app, 4.2, true, tap(Key::Enter));
        assert!(denied(&rx[1]));

        // A click in the window (on the card's text): then Enter approves,
        // a second later.
        let (ctx, mut app, rx) = withdrawn_with_deny_focused();
        let body = app.main_rect.left_top() + vec2(30.0, 30.0);
        pass(&ctx, &mut app, 3.1, true, click_at(body));
        assert!(!app.orphaned_focus);
        assert!(undecided(&rx[1]));
        pass(&ctx, &mut app, 3.3, true, tap(Key::Enter));
        assert!(undecided(&rx[1]), "the keys arm again only a second later");
        pass(&ctx, &mut app, 4.2, true, tap(Key::Enter));
        assert!(approved(&rx[1]));
    }

    #[test]
    fn the_keys_stay_held_back_through_a_trip_to_another_app() {
        let (ctx, mut app, rx_a, rx_b) = withdrawn_while_writing_feedback();
        // The user switches away and back, then (armed again at t=6)
        // presses Enter for a new line in A's field.
        pass(&ctx, &mut app, 3.0, false, vec![Event::WindowFocused(false)]);
        pass(&ctx, &mut app, 5.0, true, vec![Event::WindowFocused(true)]);
        pass(&ctx, &mut app, 6.5, true, tap(Key::Enter));
        assert!(undecided(&rx_b));
        assert!(undecided(&rx_a));
        assert!(app.orphaned_focus);
    }

    #[test]
    fn a_withdrawal_while_away_holds_the_keys_back_too() {
        let (mut app, rx) = app_with_requests(&["a", "b"]);
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        pass(&ctx, &mut app, 1.5, true, vec![tab(), tab()]);
        // Away with Deny focused; A is withdrawn meanwhile.
        pass(&ctx, &mut app, 2.0, false, vec![Event::WindowFocused(false)]);
        assert!(app.state.withdraw("a"));
        pass(&ctx, &mut app, 3.0, false, vec![]);
        assert!(app.orphaned_focus);
        // Back at t=5; Enter (for the Deny that was focused) at t=6.5.
        pass(&ctx, &mut app, 5.0, true, vec![Event::WindowFocused(true)]);
        pass(&ctx, &mut app, 6.5, true, tap(Key::Enter));
        assert!(undecided(&rx[1]));
    }

    #[test]
    fn the_users_own_decision_or_nothing_focused_does_not_hold_the_keys_back() {
        // Enter on a Deny focused with Tab denies A: B is the user's next
        // card, and Enter approves it once armed.
        let (mut app, rx) = app_with_requests(&["a", "b"]);
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        pass(&ctx, &mut app, 1.5, true, vec![tab(), tab()]);
        pass(&ctx, &mut app, 2.5, true, tap(Key::Enter));
        assert!(denied(&rx[0]));
        assert!(!app.orphaned_focus);
        pass(&ctx, &mut app, 3.6, true, tap(Key::Enter));
        assert!(approved(&rx[1]));

        // A click on Approve with the textarea focused: the same.
        let (mut app, rx) = app_with_requests(&["a", "b"]);
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        open_feedback(&ctx, &mut app, 1.1, "");
        focus_textarea(&ctx, &mut app);
        pass(&ctx, &mut app, 1.2, true, vec![]);
        let approve = button_center(&ctx, FocusId::Approve);
        pass(&ctx, &mut app, 2.0, true, click_at(approve));
        assert!(approved(&rx[0]));
        pass(&ctx, &mut app, 2.1, true, vec![]);
        assert!(!app.orphaned_focus);

        // A withdrawn with nothing focused: only the arming delay applies.
        let (mut app, rx) = app_with_requests(&["a", "b"]);
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        idle_until(&ctx, &mut app, 2.0);
        assert!(app.state.withdraw("a"));
        pass(&ctx, &mut app, 2.0, true, vec![]);
        assert!(!app.orphaned_focus);
        pass(&ctx, &mut app, 3.1, true, tap(Key::Enter));
        assert!(approved(&rx[1]));

        // A card behind the front one withdrawn: the focus stays.
        let (mut app, _rx) = app_with_requests(&["a", "b"]);
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        pass(&ctx, &mut app, 1.5, true, vec![tab(), tab()]);
        assert!(app.state.withdraw("b"));
        pass(&ctx, &mut app, 2.0, true, vec![]);
        assert!(!app.orphaned_focus);
        assert_eq!(app.focus.focused, Some(FocusId::Deny));
    }

    // -----------------------------------------------------------------------
    // One input batch: keys after an effect that only the logic or paint
    // pass applies.
    // -----------------------------------------------------------------------

    /// One frame, checking that the tab order the key reading predicted is
    /// the one the paint pass built.
    fn pass_predicting_order(
        ctx: &Context,
        app: &mut ApprovalApp,
        time: f64,
        events: Vec<Event>,
    ) {
        let input = hook(ctx, app, time, true, events, &[]);
        let predicted = app.focus.order.clone();
        run(ctx, app, input);
        assert_eq!(predicted, app.focus.order, "the tab order the keys used");
    }

    #[test]
    fn enter_on_give_feedback_then_shift_tab_then_enter_in_one_batch_types_a_newline() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        pass(&ctx, &mut app, 1.5, true, vec![tab()]);
        assert_eq!(app.focus.focused, Some(FocusId::GiveFeedback));
        // Enter opens the panel, Shift+Tab goes back into the textarea now
        // before Give feedback, Enter types a new line there.
        pass_predicting_order(
            &ctx,
            &mut app,
            2.6,
            [tap(Key::Enter), vec![shift_tab()], tap(Key::Enter)].concat(),
        );
        assert!(app.feedback_open);
        assert_eq!(app.focus.focused, Some(FocusId::Textarea));
        assert_eq!(app.feedback_text, "\n");
        assert!(undecided(&rx));
        // Tab to Give feedback, Enter, Shift+Tab, Enter: the same.
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        pass_predicting_order(
            &ctx,
            &mut app,
            2.5,
            [vec![tab()], tap(Key::Enter), vec![shift_tab()], tap(Key::Enter)].concat(),
        );
        assert!(app.feedback_open);
        assert_eq!(app.focus.focused, Some(FocusId::Textarea));
        assert!(undecided(&rx));
    }

    #[test]
    fn a_key_after_an_activation_in_one_batch_never_decides_or_activates() {
        // The panel reopens with its text: Shift+Tab lands on Submit
        // feedback, and the Enter after it waits for its own frame.
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        app.feedback_text = "Shorter, please".to_string();
        pass(&ctx, &mut app, 1.5, true, vec![tab()]);
        pass_predicting_order(
            &ctx,
            &mut app,
            2.6,
            [tap(Key::Enter), vec![shift_tab()], tap(Key::Enter)].concat(),
        );
        assert!(app.feedback_open);
        assert_eq!(app.focus.focused, Some(FocusId::SubmitFeedback));
        assert!(undecided(&rx), "the Enter must not submit in the same batch");
        pass(&ctx, &mut app, 2.7, true, tap(Key::Enter));
        assert!(matches!(
            rx.try_recv(),
            Ok(Ok(Decision::Feedback(text))) if text == "Shorter, please"
        ));

        // Closing the panel: Shift+Tab from Give feedback (now the first
        // control) goes to the page, and the Enter after it does nothing.
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        open_feedback(&ctx, &mut app, 1.1, "");
        app.focus.focus(FocusId::GiveFeedback, true);
        pass_predicting_order(
            &ctx,
            &mut app,
            2.6,
            [tap(Key::Enter), vec![shift_tab()], tap(Key::Enter)].concat(),
        );
        assert!(!app.feedback_open);
        assert_eq!(app.focus.focused, None);
        assert!(undecided(&rx), "the Enter must not approve in the same batch");

        // Space released on Give feedback, then Escape: no deny.
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        pass(&ctx, &mut app, 1.5, true, vec![tab()]);
        pass(&ctx, &mut app, 2.6, true, vec![press(Key::Space)]);
        pass(
            &ctx,
            &mut app,
            3.7,
            true,
            [vec![release(Key::Space)], tap(Key::Escape)].concat(),
        );
        assert!(app.feedback_open);
        assert!(undecided(&rx), "the Escape must not deny in the same batch");
    }

    /// The window center of a painted button.
    fn button_center(ctx: &Context, id: FocusId) -> Pos2 {
        ctx.read_response(button_id(id))
            .expect("the button is painted")
            .rect
            .center()
    }

    /// A click at `pos` (pointer moved there, pressed, released).
    fn click_at(pos: Pos2) -> Vec<Event> {
        vec![Event::PointerMoved(pos), pointer(pos, true), pointer(pos, false)]
    }

    /// The feedback panel open with nothing focused, armed; returns where
    /// the textarea is.
    fn panel_open_nothing_focused(ctx: &Context, app: &mut ApprovalApp) -> Pos2 {
        pass(ctx, app, 1.0, true, vec![]);
        open_feedback(ctx, app, 1.1, "");
        assert_eq!(app.focus.focused, None);
        ctx.read_response(app.ta_id.with("click"))
            .expect("the textarea is painted")
            .rect
            .center()
    }

    #[test]
    fn a_click_in_the_textarea_then_enter_or_escape_in_one_batch_decides_nothing() {
        for k in [Key::Enter, Key::Escape] {
            let (mut app, rx) = app_with_request();
            let ctx = Context::default();
            let field = panel_open_nothing_focused(&ctx, &mut app);
            pass(&ctx, &mut app, 2.6, true, [click_at(field), tap(k)].concat());
            assert!(undecided(&rx), "{k:?} after a click in the field must not decide");
            assert_eq!(app.queue.len(), 1);
            assert_eq!(app.focus.focused, Some(FocusId::Textarea));
            // In the next frame, Enter types a new line in the field.
            pass(&ctx, &mut app, 2.7, true, tap(Key::Enter));
            assert_eq!(app.feedback_text, "\n");
            assert!(undecided(&rx));
        }
    }

    #[test]
    fn a_click_then_a_key_in_one_batch_decides_nothing() {
        // A click on Give feedback, then Enter: the panel opens, nothing
        // is approved.
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        let give = button_center(&ctx, FocusId::GiveFeedback);
        pass(&ctx, &mut app, 2.6, true, [click_at(give), tap(Key::Enter)].concat());
        assert!(app.feedback_open);
        assert!(undecided(&rx), "Enter after a click must not approve");
        // The release of a press from the frame before, then Enter.
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        let field = panel_open_nothing_focused(&ctx, &mut app);
        pass(
            &ctx,
            &mut app,
            2.5,
            true,
            vec![Event::PointerMoved(field), pointer(field, true)],
        );
        pass(
            &ctx,
            &mut app,
            2.6,
            true,
            [vec![pointer(field, false)], tap(Key::Enter)].concat(),
        );
        assert!(undecided(&rx), "Enter after a click's release must not approve");
        assert_eq!(app.focus.focused, Some(FocusId::Textarea));
    }

    // -----------------------------------------------------------------------
    // Clicks on the decision buttons.
    // -----------------------------------------------------------------------

    #[test]
    fn clicks_arm_after_the_front_card_and_the_window_appear() {
        assert!(!click_armed(5.4, 5.0, Some(1.0), NEVER));
        assert!(click_armed(5.0 + CLICK_ARM, 5.0, Some(1.0), NEVER));
        assert!(!click_armed(5.4, 1.0, Some(5.0), NEVER));
        assert!(click_armed(5.6, 1.0, Some(5.0), NEVER));
        assert!(click_armed(5.6, 5.0, None, NEVER));
        // A new request brought the window forward, or the user
        // re-established an orphaned focus, at t=5.
        assert!(!click_armed(5.4, 1.0, Some(1.0), 5.0));
        assert!(click_armed(5.5, 1.0, Some(1.0), 5.0));
        assert_eq!(clicks_armed_at(1.0, Some(2.0), 5.0), 5.0 + CLICK_ARM);
        assert!((0.3..=0.5).contains(&CLICK_ARM));
    }

    #[test]
    fn a_click_aimed_at_a_withdrawn_card_never_decides_the_next() {
        let (mut app, rx) = app_with_requests(&["a", "b"]);
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        let approve = button_center(&ctx, FocusId::Approve);
        // A is withdrawn; B is painted in its place at t=2.0. The click
        // aimed at A's Approve is pressed at 2.05.
        idle_until(&ctx, &mut app, 2.0);
        assert!(app.state.withdraw("a"));
        pass(&ctx, &mut app, 2.0, true, vec![]);
        pass(
            &ctx,
            &mut app,
            2.05,
            true,
            vec![Event::PointerMoved(approve), pointer(approve, true)],
        );
        pass(&ctx, &mut app, 2.1, true, vec![pointer(approve, false)]);
        assert!(undecided(&rx[1]), "a click aimed at A must not approve B");
        // A deliberate click on B approves it.
        pass(&ctx, &mut app, 2.6, true, click_at(approve));
        assert!(approved(&rx[1]));
    }

    #[test]
    fn a_click_right_after_the_window_appears_never_decides() {
        // Where Approve is (every card has the same layout).
        let (mut probe, _rx) = app_with_request();
        let probe_ctx = Context::default();
        pass(&probe_ctx, &mut probe, 1.0, true, vec![]);
        let approve = button_center(&probe_ctx, FocusId::Approve);

        // The card arrives while another app has the focus, and the window
        // pops up over it: a click 100 ms later (macOS delivers it to the
        // inactive window) does nothing.
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 5.0, false, vec![]);
        pass(&ctx, &mut app, 5.1, false, click_at(approve));
        assert!(undecided(&rx), "a click aimed at another app must not approve");
        // Half a second on, a click on the (still inactive) window does.
        pass(&ctx, &mut app, 5.6, false, click_at(approve));
        assert!(approved(&rx));

        // The window comes on screen long after the card became the front
        // card (it was hidden): clicks arm from then.
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        let mut input = RawInput {
            time: Some(4.0),
            screen_rect: Some(screen()),
            ..Default::default()
        };
        app.extract_keys(&ctx, &mut input, false, Vec::new, Vec::new());
        run(&ctx, &mut app, input);
        assert_eq!(app.on_screen_since, None);
        pass(&ctx, &mut app, 5.0, true, vec![]);
        pass(&ctx, &mut app, 5.1, true, click_at(approve));
        assert!(undecided(&rx));
        pass(&ctx, &mut app, 5.6, true, click_at(approve));
        assert!(approved(&rx));
    }

    // -----------------------------------------------------------------------
    // A button focused by a click is no focus for the keys.
    // -----------------------------------------------------------------------

    #[test]
    fn enter_and_space_ignore_a_button_focused_by_a_click() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        // A click on Give feedback opens the panel and focuses the button,
        // with no ring.
        let give = button_center(&ctx, FocusId::GiveFeedback);
        pass(&ctx, &mut app, 2.6, true, click_at(give));
        assert!(app.feedback_open);
        assert_eq!(app.focus.focused, Some(FocusId::GiveFeedback));
        assert!(!app.focus.keyboard);
        // Space scrolls: it does not press Give feedback (the panel stays).
        let left = pass(&ctx, &mut app, 4.0, true, space());
        assert!(!has_key_press(&left, Key::Space), "the app scrolls with Space");
        assert!(app.feedback_open, "Space must not press a clicked button");
        assert!(undecided(&rx));
        // Enter approves, as with nothing focused.
        pass(&ctx, &mut app, 5.1, true, tap(Key::Enter));
        assert!(approved(&rx));
    }

    #[test]
    fn an_ignored_click_leaves_no_focus_for_space_or_enter() {
        let (mut app, rx) = app_with_requests(&["a", "b"]);
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        let deny = button_center(&ctx, FocusId::Deny);
        // A is withdrawn; B takes its place at t=2.0, and the click aimed at
        // A's Deny lands on B's at 2.05: it is ignored...
        idle_until(&ctx, &mut app, 2.0);
        assert!(app.state.withdraw("a"));
        pass(&ctx, &mut app, 2.0, true, vec![]);
        pass(&ctx, &mut app, 2.05, true, click_at(deny));
        assert!(undecided(&rx[1]), "a click aimed at A must not deny B");
        // ...and leaves no focus on B's Deny.
        assert_eq!(app.focus.focused, None);
        // A Space (to scroll) does not press it later.
        pass(&ctx, &mut app, 3.5, true, space());
        assert!(undecided(&rx[1]), "Space must not press B's Deny");
        assert_eq!(app.queue.len(), 1);
    }

    // -----------------------------------------------------------------------
    // The key or click that ends an orphaned focus decides nothing.
    // -----------------------------------------------------------------------

    #[test]
    fn shift_tab_after_an_orphaning_withdrawal_never_lets_the_next_enter_approve() {
        // Shift+Tab (meant to go back from A's Deny to its Give feedback)
        // lands on B's Approve; the Enter right after it does nothing.
        let (ctx, mut app, rx) = withdrawn_with_deny_focused();
        pass(&ctx, &mut app, 3.5, true, vec![shift_tab()]);
        assert_eq!(app.focus.focused, Some(FocusId::Approve));
        assert!(!app.orphaned_focus);
        pass(&ctx, &mut app, 3.6, true, tap(Key::Enter));
        assert!(undecided(&rx[1]), "the Enter meant for A must not approve B");
        // A second after the Shift+Tab, Enter on the Approve shown focused
        // approves B.
        pass(&ctx, &mut app, 4.6, true, tap(Key::Enter));
        assert!(approved(&rx[1]));

        // The same in one batch.
        let (ctx, mut app, rx) = withdrawn_with_deny_focused();
        pass(
            &ctx,
            &mut app,
            3.5,
            true,
            [vec![shift_tab()], tap(Key::Enter)].concat(),
        );
        assert!(undecided(&rx[1]), "the Enter meant for A must not approve B");
    }

    #[test]
    fn a_click_that_ends_an_orphaned_focus_decides_nothing() {
        let (ctx, mut app, rx) = withdrawn_with_deny_focused();
        let deny = button_center(&ctx, FocusId::Deny);
        // The click meant to put the focus back on A's Deny lands on B's,
        // long after B's clicks armed (t=2.5): it only ends the orphaned
        // focus.
        pass(&ctx, &mut app, 3.5, true, click_at(deny));
        assert!(!app.orphaned_focus);
        assert!(undecided(&rx[1]), "the click meant for A must not deny B");
        // Clicks arm again half a second later.
        pass(&ctx, &mut app, 3.7, true, click_at(deny));
        assert!(undecided(&rx[1]));
        pass(&ctx, &mut app, 4.1, true, click_at(deny));
        assert!(denied(&rx[1]));
    }

    // -----------------------------------------------------------------------
    // A new request brings the window forward.
    // -----------------------------------------------------------------------

    #[test]
    fn a_new_request_that_brings_the_window_forward_re_arms_clicks_and_keys() {
        // A is on screen, partly behind another app's window that has the
        // focus.
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, false, vec![]);
        let approve = button_center(&ctx, FocusId::Approve);
        // B arrives at t=5 and the window comes forward: a click 100 ms
        // later, aimed at the other app, does nothing.
        idle_until(&ctx, &mut app, 5.0);
        let _rx_b = enqueue(&app.state, "b");
        pass(&ctx, &mut app, 5.0, false, vec![]);
        pass(&ctx, &mut app, 5.1, false, click_at(approve));
        assert!(undecided(&rx), "a click aimed at another app must not approve");
        // Half a second on, a click approves A.
        pass(&ctx, &mut app, 5.6, false, click_at(approve));
        assert!(approved(&rx));

        // The keys too, in a window that already had the focus.
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        idle_until(&ctx, &mut app, 5.0);
        let _rx_b = enqueue(&app.state, "b");
        pass(&ctx, &mut app, 5.0, true, vec![]);
        pass(&ctx, &mut app, 5.5, true, tap(Key::Enter));
        assert!(undecided(&rx), "Enter right after a new request must not approve");
        pass(&ctx, &mut app, 6.1, true, tap(Key::Enter));
        assert!(approved(&rx));
    }

    // -----------------------------------------------------------------------
    // The OS key-repeat flag.
    // -----------------------------------------------------------------------

    #[test]
    fn the_first_os_repeat_of_a_key_pressed_elsewhere_never_decides() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        // Enter went down in a menu (or the app switcher) while the window
        // kept the focus; its first auto-repeat reaches the window long
        // after arming, flagged as a repeat by the OS.
        idle_until(&ctx, &mut app, 3.0);
        let input = frame_input(
            &ctx,
            &mut app,
            3.0,
            true,
            vec![press(Key::Enter)],
            &[],
            &[(Key::Enter, true)],
        );
        run(&ctx, &mut app, input);
        // Its next repeats are held too.
        pass(&ctx, &mut app, 3.1, true, vec![press(Key::Enter)]);
        assert!(undecided(&rx), "a key-repeat must not approve");
        // Released, then pressed again: a deliberate Enter approves.
        pass(&ctx, &mut app, 3.2, true, vec![release(Key::Enter)]);
        pass(&ctx, &mut app, 3.4, true, tap(Key::Enter));
        assert!(approved(&rx));
    }

    #[test]
    fn os_key_downs_pair_with_the_presses_in_order() {
        // One batch: Escape's last OS repeat, its release, then a fresh
        // press of Escape. The repeat does nothing; the fresh press denies.
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        idle_until(&ctx, &mut app, 3.0);
        let input = frame_input(
            &ctx,
            &mut app,
            3.0,
            true,
            vec![press(Key::Escape), release(Key::Escape), press(Key::Escape)],
            &[],
            &[(Key::Escape, true), (Key::Escape, false)],
        );
        run(&ctx, &mut app, input);
        assert!(denied(&rx));
        // The repeat alone does not deny.
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        idle_until(&ctx, &mut app, 3.0);
        let input = frame_input(
            &ctx,
            &mut app,
            3.0,
            true,
            vec![press(Key::Escape), release(Key::Escape)],
            &[],
            &[(Key::Escape, true)],
        );
        run(&ctx, &mut app, input);
        assert!(undecided(&rx));
    }

    // -----------------------------------------------------------------------
    // egui's focus (assistive technologies).
    // -----------------------------------------------------------------------

    #[test]
    fn enter_acts_on_the_button_an_assistive_technology_focused() {
        // VoiceOver puts egui's focus on Deny (the app's tracker had none):
        // Enter presses Deny, it does not approve.
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        ctx.memory_mut(|m| m.request_focus(button_id(FocusId::Deny)));
        pass(&ctx, &mut app, 2.5, true, tap(Key::Enter));
        assert!(denied(&rx), "Enter on the Deny VoiceOver focused must not approve");

        // The same with the AccessKit focus request in the batch, before
        // the Enter.
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        let focus_deny = Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Focus,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: button_id(FocusId::Deny).accesskit_id(),
            data: None,
        });
        pass(
            &ctx,
            &mut app,
            2.5,
            true,
            [vec![focus_deny], tap(Key::Enter)].concat(),
        );
        assert!(denied(&rx), "Enter on the Deny VoiceOver focused must not approve");
    }

    #[test]
    fn enter_and_escape_do_nothing_with_egui_focus_on_another_widget() {
        // egui's focus is on a widget that is none of the controls (here
        // the feedback well's click area).
        for k in [Key::Enter, Key::Escape] {
            let (mut app, rx) = app_with_request();
            let ctx = Context::default();
            pass(&ctx, &mut app, 1.0, true, vec![]);
            open_feedback(&ctx, &mut app, 1.1, "");
            ctx.memory_mut(|m| m.request_focus(app.ta_id.with("click")));
            pass(&ctx, &mut app, 2.5, true, tap(k));
            assert!(undecided(&rx), "{k:?} must do nothing there");
            assert_eq!(app.focus.focused, None);
            // Tab takes the focus back to the card's controls.
            pass(&ctx, &mut app, 2.6, true, vec![tab()]);
            assert_eq!(app.focus.focused, Some(FocusId::Textarea));
            assert_eq!(ctx.memory(|m| m.focused()), Some(app.ta_id));
        }
    }

    #[test]
    fn egui_focus_never_carries_over_to_the_next_card() {
        // Deny focused with Tab on A, then Enter denies A: egui's focus
        // (Deny's id is the same on every card) leaves with A.
        let (mut app, rx) = app_with_requests(&["a", "b"]);
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        pass(&ctx, &mut app, 1.5, true, vec![tab(), tab()]);
        assert_eq!(ctx.memory(|m| m.focused()), Some(button_id(FocusId::Deny)));
        pass(&ctx, &mut app, 2.5, true, tap(Key::Enter));
        assert!(denied(&rx[0]));
        assert_eq!(ctx.memory(|m| m.focused()), None);
        pass(&ctx, &mut app, 2.6, true, vec![]);
        assert_eq!(app.focus.focused, None);
    }

    // -----------------------------------------------------------------------
    // A press is known only to have come after the batch before it.
    // -----------------------------------------------------------------------

    #[test]
    fn a_key_pressed_before_arming_but_read_after_a_long_frame_never_decides() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        // The window is up; another app has the focus.
        pass(&ctx, &mut app, 1.0, false, vec![]);
        // The window takes the focus at t=5 (armed at t=6). Enter, still
        // meant for the other app, goes down right after, but the next
        // batch is taken only at t=6.5 (the frame before took long, and the
        // app's own frame at t=6 did not run).
        idle_until(&ctx, &mut app, 5.0);
        pass(&ctx, &mut app, 5.0, true, vec![Event::WindowFocused(true)]);
        let input = frame_input(&ctx, &mut app, 6.5, true, tap(Key::Enter), &[], &[]);
        run(&ctx, &mut app, input);
        assert!(undecided(&rx), "a key pressed before arming must not approve");
        // In the next batch, Enter (pressed after t=6.5) approves.
        pass(&ctx, &mut app, 6.6, true, tap(Key::Enter));
        assert!(approved(&rx));
    }

    #[test]
    fn a_click_pressed_before_arming_but_read_after_a_long_frame_never_decides() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        // The card appears at t=1 (clicks arm at t=1.5); a click pressed
        // right then is read only at t=1.6.
        pass(&ctx, &mut app, 1.0, true, vec![]);
        let approve = button_center(&ctx, FocusId::Approve);
        let input = frame_input(&ctx, &mut app, 1.6, true, click_at(approve), &[], &[]);
        run(&ctx, &mut app, input);
        assert!(undecided(&rx), "a click pressed before arming must not approve");
        pass(&ctx, &mut app, 1.7, true, click_at(approve));
        assert!(approved(&rx));
    }

    #[test]
    fn the_app_asks_for_a_frame_when_the_keys_and_clicks_arm() {
        let (mut app, _rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        // Clicks arm at t=1.5, the keys at t=2.
        assert_eq!(app.wake_at, Some(1.0 + CLICK_ARM));
        // (The first quiet frame still carries the repaint the new request
        // asked for.)
        for t in [1.2, 1.3] {
            let input = frame_input(&ctx, &mut app, t, true, vec![], &[], &[]);
            let delay = run(&ctx, &mut app, input).as_secs_f64();
            if t == 1.3 {
                // Just after t=1.5, never before it.
                assert!((0.2..0.21).contains(&delay), "repaint after {delay}");
            }
        }
        let input = frame_input(&ctx, &mut app, 1.5, true, vec![], &[], &[]);
        run(&ctx, &mut app, input);
        assert_eq!(app.wake_at, Some(1.0 + ARM_DELAY));
        let input = frame_input(&ctx, &mut app, 2.0, true, vec![], &[], &[]);
        run(&ctx, &mut app, input);
        assert_eq!(app.wake_at, None);
    }
}
