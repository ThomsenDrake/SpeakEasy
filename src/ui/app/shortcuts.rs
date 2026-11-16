//! The Enter/Escape decision shortcuts: plain Enter approves and plain
//! Escape denies the front request, through the same `decide` path as the
//! buttons.
//!
//! A decision has real consequences (Approve sends a Slack message, Deny
//! blocks the purpose for a cooldown), and the window steals the focus when
//! a request arrives, so a key meant for another app can land here. The
//! shortcuts therefore arm only [`ARM_DELAY`] after the request became the
//! front card *and* the window got the focus, and only a fresh press (the
//! key went down after arming, not a key-repeat) acts. The gate itself is
//! the pure [`key_action`]; `ApprovalApp::extract_keys` feeds it.

use std::collections::HashMap;

use egui::{Key, Modifiers};

use crate::ui::components::FocusId;

/// Seconds the current request must have been the front card, with the
/// window on screen and focused, before Enter/Escape can decide it. Longer
/// than the usual delay before a held key starts repeating, so a key held
/// down while the window appeared repeats (and is ignored as held) before
/// the shortcuts arm.
pub(crate) const ARM_DELAY: f64 = 1.0;

/// What an Enter/Escape press does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyAction {
    /// Leave the key to egui: a newline in the textarea, or the focused
    /// button's own Enter activation.
    PassThrough,
    /// Drop the key: it must not reach egui (the focused button) either.
    Swallow,
    Approve,
    Deny,
    /// Leave the feedback textarea; its text and the panel stay.
    BlurTextarea,
}

/// When the shortcuts arm for the current card: [`ARM_DELAY`] after it
/// became the front card (`front_since`) and the window got the focus
/// (`focused_since`), whichever came last. `None` (never) with no card,
/// while the window is hidden or unfocused, or while a decision is being
/// submitted.
pub(crate) fn armed_at(
    front_since: Option<f64>,
    focused_since: Option<f64>,
    deciding: bool,
) -> Option<f64> {
    if deciding {
        return None;
    }
    Some(front_since?.max(focused_since?) + ARM_DELAY)
}

/// Gate one Enter/Escape press. `repeat`: the key was already down (a
/// key-repeat); `down_at`: when the key went down; `focused`: the control
/// with keyboard focus.
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
        // needs a fresh plain press, so a focused Deny is safe too.
        Key::Enter if !(plain && fresh) => KeyAction::Swallow,
        Key::Enter => match focused {
            // Approve (the focused Approve button would activate itself on
            // the same press: the event is consumed so it decides once).
            None | Some(FocusId::Approve) => KeyAction::Approve,
            Some(_) => KeyAction::PassThrough,
        },
        // Modified Escape is not ours (egui ignores it too).
        Key::Escape if !plain => KeyAction::PassThrough,
        Key::Escape if in_textarea => KeyAction::BlurTextarea,
        Key::Escape if fresh => KeyAction::Deny,
        Key::Escape => KeyAction::Swallow,
        _ => KeyAction::PassThrough,
    }
}

/// When each shortcut key went down (absent: up). egui-winit drops the OS
/// key-repeat flag (egui recomputes it later, from its own key state, which
/// never sees the keys the app extracts), so a press of a key already down
/// is a repeat.
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
    use super::super::{ApprovalApp, DemoFlags};
    use super::*;
    use crate::core::Decision;
    use crate::ui::demo;
    use crate::ui::theme::Theme;
    use crate::ui::UiState;
    use egui::{Context, Event, RawInput};
    use std::sync::mpsc::{self, Receiver};
    use std::sync::{Arc, Mutex};

    const PLAIN: Modifiers = Modifiers::NONE;
    /// Armed at t=10 for every `key_action` case below.
    const ARMED: Option<f64> = Some(10.0);

    fn act(key: Key, modifiers: Modifiers, focused: Option<FocusId>) -> KeyAction {
        key_action(key, modifiers, false, 11.0, ARMED, focused)
    }

    #[test]
    fn armed_after_the_delay_from_front_card_and_window_focus() {
        // Whichever came last starts the delay.
        assert_eq!(armed_at(Some(5.0), Some(2.0), false), Some(5.0 + ARM_DELAY));
        assert_eq!(armed_at(Some(2.0), Some(5.0), false), Some(5.0 + ARM_DELAY));
        // Rule 5: no card, a hidden/unfocused window, or a submission in
        // flight never arms.
        assert_eq!(armed_at(None, Some(2.0), false), None);
        assert_eq!(armed_at(Some(2.0), None, false), None);
        assert_eq!(armed_at(Some(2.0), Some(2.0), true), None);
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
    fn a_focused_button_keeps_enter_and_escape_still_denies() {
        // Rule 4: Enter goes to the focused button (egui activates it)...
        for b in [
            FocusId::Deny,
            FocusId::GiveFeedback,
            FocusId::SubmitFeedback,
        ] {
            assert_eq!(act(Key::Enter, PLAIN, Some(b)), KeyAction::PassThrough);
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
        // A focused Approve approves once (the event is consumed).
        let approve = Some(FocusId::Approve);
        assert_eq!(act(Key::Enter, PLAIN, approve), KeyAction::Approve);
        assert_eq!(act(Key::Escape, PLAIN, approve), KeyAction::Deny);
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
    }

    // -----------------------------------------------------------------------
    // End to end: the raw-input hook's extraction and the logic pass.
    // -----------------------------------------------------------------------

    type Reply = Receiver<Result<Decision, crate::core::ProviderError>>;

    /// An app with one request pending in the bridge (decisions arrive on
    /// the returned receiver).
    fn app_with_request() -> (ApprovalApp, Reply) {
        let state = Arc::new(UiState::new());
        let mut app = ApprovalApp::new(
            Arc::clone(&state),
            Arc::new(Mutex::new(None)),
            Theme::Light,
            false,
            false,
            DemoFlags::default(),
        );
        let (tx, rx) = mpsc::channel();
        let req = demo::DemoSeed::Single.requests().remove(0);
        state.enqueue(req.id.clone(), tx, req).unwrap();
        app.queue.push_back(state.drain_outbox().remove(0));
        (app, rx)
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

    fn tap(k: Key) -> Vec<Event> {
        vec![key(k, true, PLAIN), key(k, false, PLAIN)]
    }

    /// One frame: the raw-input hook's key extraction, then the parts of
    /// `logic` that track the front card and apply the extracted keys.
    /// Returns the events left for egui.
    fn pass(
        ctx: &Context,
        app: &mut ApprovalApp,
        time: f64,
        focused: bool,
        events: Vec<Event>,
    ) -> Vec<Event> {
        let mut input = RawInput {
            time: Some(time),
            focused,
            events,
            ..Default::default()
        };
        app.extract_keys(ctx, &mut input, true);
        let left = input.events.clone();
        let mut out = ctx.run_ui(input, |ui| {
            app.track_current(ui.ctx());
            app.handle_keys(ui.ctx(), time);
        });
        out.textures_delta.clear();
        left
    }

    #[test]
    fn enter_approves_the_front_card_only_once_armed() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        // The card arrives in a focused window at t=1: armed at t=2.
        pass(&ctx, &mut app, 1.0, true, vec![]);
        let left = pass(&ctx, &mut app, 1.5, true, tap(Key::Enter));
        assert!(
            rx.try_recv().is_err(),
            "Enter before arming must not approve"
        );
        assert!(
            !left.iter().any(|e| matches!(
                e,
                Event::Key {
                    key: Key::Enter,
                    pressed: true,
                    ..
                }
            )),
            "a swallowed Enter must not reach egui"
        );
        pass(&ctx, &mut app, 2.1, true, tap(Key::Enter));
        assert!(matches!(rx.try_recv(), Ok(Ok(Decision::Approve))));
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
            pass(&ctx, &mut app, t, true, vec![key(Key::Escape, true, PLAIN)]);
        }
        assert!(rx.try_recv().is_err(), "a held Escape must not deny");
        // Released, then pressed again: a deliberate Escape denies.
        pass(
            &ctx,
            &mut app,
            6.6,
            true,
            vec![key(Key::Escape, false, PLAIN)],
        );
        pass(&ctx, &mut app, 6.8, true, tap(Key::Escape));
        assert!(matches!(rx.try_recv(), Ok(Ok(Decision::Deny))));
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
        assert!(rx.try_recv().is_err());
        // Not on screen: nothing, however long it has been.
        let mut input = RawInput {
            time: Some(9.0),
            focused: true,
            events: tap(Key::Enter),
            ..Default::default()
        };
        app.extract_keys(&ctx, &mut input, false);
        assert!(app.pending_keys.is_empty());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn nothing_happens_while_deciding_or_with_an_empty_queue() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        app.deciding = true;
        pass(&ctx, &mut app, 3.0, true, tap(Key::Enter));
        pass(&ctx, &mut app, 3.2, true, tap(Key::Escape));
        assert!(rx.try_recv().is_err());
        app.deciding = false;
        app.queue.clear();
        pass(&ctx, &mut app, 4.0, true, tap(Key::Enter));
        pass(&ctx, &mut app, 6.0, true, tap(Key::Enter));
        assert!(app.pending_keys.is_empty());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn escape_leaves_the_textarea_then_denies() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        app.feedback_open = true;
        app.feedback_text = "Shorter, please".to_string();
        app.focus.focus(FocusId::Textarea, true);
        // Enter in the field is left to the text edit (a newline).
        let left = pass(&ctx, &mut app, 2.5, true, tap(Key::Enter));
        assert_eq!(left.len(), 2, "Enter must reach the textarea");
        assert!(rx.try_recv().is_err());
        // Escape leaves the field; the text and the panel stay.
        pass(&ctx, &mut app, 3.0, true, tap(Key::Escape));
        assert!(
            rx.try_recv().is_err(),
            "Escape in the textarea must not deny"
        );
        assert_eq!(app.focus.focused, None);
        assert!(app.feedback_open);
        assert_eq!(app.feedback_text, "Shorter, please");
        // A second Escape denies.
        pass(&ctx, &mut app, 3.5, true, tap(Key::Escape));
        assert!(matches!(rx.try_recv(), Ok(Ok(Decision::Deny))));
    }

    #[test]
    fn a_press_never_decides_the_card_that_replaced_its_own() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        // Enter on the armed card A...
        let mut input = RawInput {
            time: Some(3.0),
            focused: true,
            events: tap(Key::Enter),
            ..Default::default()
        };
        app.extract_keys(&ctx, &mut input, true);
        // ...which is withdrawn before the logic pass applies the key; B
        // (pending in the bridge too) takes the front.
        let mut b = app.queue.pop_front().unwrap();
        b.id = "request-b".to_string();
        let (tx_b, rx_b) = mpsc::channel();
        app.state.enqueue(b.id.clone(), tx_b, b).unwrap();
        app.queue.push_back(app.state.drain_outbox().remove(0));
        let mut out = ctx.run_ui(input, |ui| {
            app.track_current(ui.ctx());
            app.handle_keys(ui.ctx(), 3.0);
        });
        out.textures_delta.clear();
        assert!(rx_b.try_recv().is_err(), "B must not be approved");
        assert!(rx.try_recv().is_err());
        assert_eq!(app.queue.len(), 1);
    }

    #[test]
    fn holding_escape_in_the_textarea_only_leaves_it() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        app.feedback_open = true;
        app.focus.focus(FocusId::Textarea, true);
        // Pressed once armed and held: the press leaves the field, and its
        // repeats (the field no longer focused) are not a second Escape.
        for t in [3.0, 3.5, 3.6, 3.7] {
            pass(&ctx, &mut app, t, true, vec![key(Key::Escape, true, PLAIN)]);
        }
        assert_eq!(app.focus.focused, None);
        assert!(rx.try_recv().is_err(), "a held Escape must not deny");
    }

    #[test]
    fn enter_goes_to_a_focused_button_and_a_focused_approve_decides_once() {
        let (mut app, rx) = app_with_request();
        let ctx = Context::default();
        pass(&ctx, &mut app, 1.0, true, vec![]);
        // A focused Deny keeps Enter: the event is left for the button.
        app.focus.focus(FocusId::Deny, true);
        let left = pass(&ctx, &mut app, 2.5, true, tap(Key::Enter));
        assert_eq!(left.len(), 2, "Enter must reach the focused button");
        assert!(app.pending_keys.is_empty());
        // A focused Approve: decided by the shortcut, the event consumed
        // (so the button cannot activate on it as well).
        app.focus.focus(FocusId::Approve, true);
        let left = pass(&ctx, &mut app, 3.0, true, tap(Key::Enter));
        assert!(!left
            .iter()
            .any(|e| matches!(e, Event::Key { pressed: true, .. })));
        assert!(matches!(rx.try_recv(), Ok(Ok(Decision::Approve))));
    }
}
