// Minimal stub for @mistral/next-hotkey's useHotkey.
// In the real package, this registers a global keyboard shortcut. In the demo
// prototype, the sidebar's Ctrl/Cmd+B toggle isn't critical — we no-op so the
// import compiles. Spec-driven runs don't use keyboard shortcuts.

export const useHotkey = (_keys: string | string[], _handler: () => void) => {
  // intentionally a no-op
};
