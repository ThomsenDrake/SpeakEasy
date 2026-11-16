//! The keyboard decision gate: plain Enter approves and plain Escape
//! denies the front request, through the same `decide` path as the
//! buttons, and Enter/Space activate a button focused with Tab.
//!
//! A decision has real consequences (Approve sends a Slack message, Deny
//! blocks the purpose for a cooldown), and the window steals the focus when
//! a request arrives, so a key meant for another app, or typed for the
//! feedback field of a card that was just replaced, can land here. A key
//! can therefore decide (or activate a button) only [`ARM_DELAY`] after the
//! latest of: the request became the front card, the window got the focus,
//! and the user last typed. Only a fresh press acts: the key went down
//! after arming, so not a key-repeat and not a key that was already down
//! when the window got the focus. The gate itself is the pure
//! [`key_action`]; `ApprovalApp::extract_keys` feeds it every raw key event
//! in order, with the focus as moved by the Tab presses before it.

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
/// the focus) and `typed_at` (the last text input, or key press other than
/// Enter, Escape and Tab: an Enter or Escape meant for the feedback field
/// of a card that was replaced must not decide the new one). `None` (never)
/// with no card, while the window is hidden or unfocused, or while a
/// decision is being submitted.
pub(crate) fn armed_at(
    front_since: Option<f64>,
    focused_since: Option<f64>,
    typed_at: f64,
    deciding: bool,
) -> Option<f64> {
    if deciding {
        return None;
    }
    Some(front_since?.max(focused_since?).max(typed_at) + ARM_DELAY)
}

/// Gate one Enter, Escape or Space press (other keys pass through).
/// `repeat`: the key was already down (a key-repeat, or a key held since
/// before the window got the focus); `down_at`: when the key went down;
/// `focused`: the control with keyboard focus when the key arrives.
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

/// When each gated key (Enter, Escape, Space) went down (absent: up).
/// egui-winit drops the OS key-repeat flag (egui recomputes it later, from
/// its own key state, which never sees the keys the app extracts), so a
/// press of a key already down is a repeat.
#[derive(Debug, Default)]
pub(crate) struct HeldKeys(HashMap<Key, f64>);

impl HeldKeys {
    /// A press of `key` at `now`: whether it repeats a held key, and when
    /// the key went down.
    pub fn press(&mut self, key: Key, now: f64) -> (bool, f64) {
        if let Some(&down_at) = self.0.get(&key) {
            return (true, down_at);
        }
        self.0.insert(key, now);
        (false, now)
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
            armed_at(Some(5.0), Some(2.0), NEVER_TYPED, false),
            Some(5.0 + ARM_DELAY)
        );
        assert_eq!(
            armed_at(Some(2.0), Some(5.0), NEVER_TYPED, false),
            Some(5.0 + ARM_DELAY)
        );
        assert_eq!(
            armed_at(Some(2.0), Some(5.0), 7.0, false),
            Some(7.0 + ARM_DELAY)
        );
        assert_eq!(
            armed_at(Some(5.0), Some(2.0), 3.0, false),
            Some(5.0 + ARM_DELAY)
        );
        // Rule 5: no card, a hidden/unfocused window, or a submission in
        // flight never arms.
        assert_eq!(armed_at(None, Some(2.0), NEVER_TYPED, false), None);
        assert_eq!(armed_at(Some(2.0), None, NEVER_TYPED, false), None);
        assert_eq!(armed_at(Some(2.0), Some(2.0), NEVER_TYPED, true), None);
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
        assert_eq!(held.press(Key::Enter, 1.0), (false, 1.0));
        assert_eq!(held.press(Key::Enter, 1.5), (true, 1.0));
        assert_eq!(held.press(Key::Escape, 1.6), (false, 1.6));
        held.release(Key::Enter);
        assert_eq!(held.press(Key::Enter, 2.0), (false, 2.0));
        held.clear();
        assert_eq!(held.press(Key::Escape, 3.0), (false, 3.0));
        // A key already down when the window got the focus is held until
        // released (its first repeat may come much later).
        held.hold(Key::Space, 4.0);
        assert_eq!(held.press(Key::Space, 9.0), (true, 4.0));
        held.release(Key::Space);
        assert_eq!(held.press(Key::Space, 9.5), (false, 9.5));
        // Holding never moves an earlier key-down.
        held.hold(Key::Space, 12.0);
        assert_eq!(held.press(Key::Space, 12.5), (true, 9.5));
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

    /// The raw-input hook's part of a frame at `time`: the key reading.
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
        let mut input = RawInput {
            time: Some(time),
            focused,
            screen_rect: Some(screen()),
            events,
            ..Default::default()
        };
        app.extract_keys(ctx, &mut input, true, || keys_down.to_vec());
        input
    }

    /// The rest of the frame: the steps of `logic` that take the bridge's
    /// requests, track the front card and apply the keys, then the paint
    /// pass (where clicks decide).
    fn run(ctx: &Context, app: &mut ApprovalApp, input: RawInput) {
        let time = input.time.unwrap();
        let mut out = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            app.track_press(&ctx);
            app.frame_now = time;
            app.sync_queue(&ctx);
            app.track_current(&ctx);
            app.handle_keys(&ctx, time);
            app.paint_frame(ui);
        });
        out.textures_delta.clear();
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
        app.extract_keys(&ctx, &mut input, false, Vec::new);
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
        // The user keeps typing, and more than a second after B appeared
        // presses Enter for a new line, then Escape.
        for (t, text) in [(3.6, "please"), (4.0, ".")] {
            pass(&ctx, &mut app, t, true, vec![typed(text)]);
        }
        pass(&ctx, &mut app, 4.3, true, tap(Key::Enter));
        pass(&ctx, &mut app, 4.4, true, tap(Key::Escape));
        assert!(
            undecided(&rx_b),
            "keys meant for A's feedback must not decide B"
        );
        assert!(undecided(&rx_a));
        // A second after the last keystroke, Enter approves B.
        pass(&ctx, &mut app, 5.1, true, tap(Key::Enter));
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
}
