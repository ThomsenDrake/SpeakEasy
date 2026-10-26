//! macOS system bridges for the native UI:
//!
//! - the system clipboard (NSPasteboard) — egui's `clipboard` feature is
//!   off (its arboard dependency cannot be resolved in the build
//!   environment), so copy/paste is bridged here instead;
//! - locale-aware medium time formatting (CFDateFormatter) — what the
//!   reference's `toLocaleTimeString()` renders for the "received" line;
//! - the "reduce motion" accessibility preference (CFPreferences), plus an
//!   env override for testing;
//! - the Dock-reopen observer (NSNotificationCenter).
//!
//! Non-macOS builds get stubs; the tray icon and the clipboard bridge are
//! macOS-only in this app.

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod imp {
    use core_foundation_sys::base::{kCFAllocatorDefault, CFRelease, CFTypeRef};
    use core_foundation_sys::date::kCFAbsoluteTimeIntervalSince1970;
    use core_foundation_sys::date_formatter::{
        kCFDateFormatterMediumStyle, kCFDateFormatterNoStyle, CFDateFormatterCreate,
        CFDateFormatterCreateStringWithAbsoluteTime,
    };
    use core_foundation_sys::locale::CFLocaleCopyCurrent;
    use core_foundation_sys::number::{kCFBooleanTrue, CFBooleanRef};
    use core_foundation_sys::preferences::CFPreferencesCopyAppValue;
    use core_foundation_sys::string::{
        kCFStringEncodingUTF8, CFStringCreateWithCString, CFStringGetCString, CFStringRef,
    };
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::{
        NSNotification, NSNotificationCenter, NSOperationQueue, NSString,
    };
    use std::ptr::NonNull;

    /// Write `text` to the system clipboard (replacing its contents).
    pub fn clipboard_set(text: &str) {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        let s = NSString::from_str(text);
        pasteboard.setString_forType(&s, unsafe { NSPasteboardTypeString });
    }

    /// Read the system clipboard's text, if any.
    pub fn clipboard_get() -> Option<String> {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard
            .stringForType(unsafe { NSPasteboardTypeString })
            .map(|s| s.to_string())
    }

    fn cf_string_to_string(s: CFStringRef) -> Option<String> {
        if s.is_null() {
            return None;
        }
        unsafe {
            for cap in [64usize, 512, 4096] {
                let mut buf = vec![0u8; cap];
                let ok = CFStringGetCString(
                    s,
                    buf.as_mut_ptr() as *mut std::ffi::c_char,
                    cap as isize,
                    kCFStringEncodingUTF8,
                );
                if ok != 0 {
                    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                    return Some(String::from_utf8_lossy(&buf[..len]).into_owned());
                }
            }
        }
        None
    }

    fn cf_string_literal(s: &'static std::ffi::CStr) -> CFStringRef {
        unsafe {
            CFStringCreateWithCString(
                kCFAllocatorDefault,
                s.as_ptr(),
                kCFStringEncodingUTF8,
            )
        }
    }

    /// `new Date(unix_secs * 1000).toLocaleTimeString()` — the system
    /// locale's medium time style (e.g. "12:00:00 PM" in en-US).
    pub fn format_time_medium(unix_secs: u64) -> String {
        unsafe {
            let locale = CFLocaleCopyCurrent();
            let formatter = CFDateFormatterCreate(
                kCFAllocatorDefault,
                locale,
                kCFDateFormatterNoStyle,
                kCFDateFormatterMediumStyle,
            );
            if !formatter.is_null() {
                let at = unix_secs as f64 - kCFAbsoluteTimeIntervalSince1970;
                let s = CFDateFormatterCreateStringWithAbsoluteTime(
                    kCFAllocatorDefault,
                    formatter,
                    at,
                );
                let out = cf_string_to_string(s);
                if !s.is_null() {
                    CFRelease(s as CFTypeRef);
                }
                CFRelease(formatter as CFTypeRef);
                if let Some(out) = out {
                    if !locale.is_null() {
                        CFRelease(locale as CFTypeRef);
                    }
                    return out;
                }
            }
            if !locale.is_null() {
                CFRelease(locale as CFTypeRef);
            }
        }
        fallback_time_medium(unix_secs)
    }

    /// The macOS "reduce motion" accessibility preference, or the
    /// `SLACK_GATEWAY_REDUCE_MOTION` env override (any non-empty value
    /// other than "0" enables it).
    pub fn reduce_motion() -> bool {
        if let Ok(v) = std::env::var("SLACK_GATEWAY_REDUCE_MOTION") {
            return !v.is_empty() && v != "0";
        }
        unsafe {
            let key = cf_string_literal(c"reduceMotion");
            let app = cf_string_literal(c"com.apple.universalaccess");
            let value = CFPreferencesCopyAppValue(key, app);
            let enabled = !value.is_null() && (value as CFBooleanRef) == kCFBooleanTrue;
            if !value.is_null() {
                CFRelease(value);
            }
            if !key.is_null() {
                CFRelease(key as CFTypeRef);
            }
            if !app.is_null() {
                CFRelease(app as CFTypeRef);
            }
            enabled
        }
    }

    /// Whether the app's window is actually on screen: mapped and visible,
    /// and at least partially un-occluded (`NSWindow.isVisible` plus the
    /// `NSWindowOcclusionStateVisible` occlusion bit). This is the signal a
    /// hidden webview's rAF throttle keys on, so the animation clock freezes
    /// while the window is hidden or fully covered — exactly like the
    /// reference. The app is single-window, so `windows().firstObject()` is
    /// our window (empty before winit creates it).
    pub fn window_on_screen() -> bool {
        use objc2_app_kit::{NSApplication, NSWindowOcclusionState};
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return false;
        };
        let app = NSApplication::sharedApplication(mtm);
        match app.windows().firstObject() {
            Some(w) => {
                w.isVisible() && w.occlusionState().contains(NSWindowOcclusionState::Visible)
            }
            None => false,
        }
    }

    /// Keeps the Dock-reopen observer alive. Dropping the observer token
    /// does not unregister it (the process exit does), but keeping it is
    /// the documented pattern.
    pub struct DockReopenObserver {
        _token: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>,
        _block: block2::RcBlock<dyn Fn(NonNull<NSNotification>)>,
    }

    /// Observe `NSApplicationDidBecomeActive` on the main queue and call
    /// `on_reopen` (the Dock icon click / `open -a` path — the app's
    /// "reveal" hook reopens the hidden window). Must be called on the main
    /// thread once the event loop is running.
    ///
    /// Note: macOS gives no API to distinguish a Dock-icon activation from
    /// any other activation (e.g. Cmd+Tab) without replacing winit's
    /// application delegate, so this also fires on those; the reveal hook is
    /// a no-op while the window is already visible.
    pub fn observe_dock_reopen(
        on_reopen: impl Fn() + Send + Sync + 'static,
    ) -> Option<DockReopenObserver> {
        use objc2_app_kit::NSApplicationDidBecomeActiveNotification;
        let on_reopen = std::sync::Arc::new(on_reopen);
        let block: block2::RcBlock<dyn Fn(NonNull<NSNotification>)> =
            block2::RcBlock::new(move |_note: NonNull<NSNotification>| {
                // The block only captures an Arc and runs on the main queue.
                on_reopen();
            });
        let center = NSNotificationCenter::defaultCenter();
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSApplicationDidBecomeActiveNotification),
                None,
                Some(&NSOperationQueue::mainQueue()),
                &block,
            )
        };
        Some(DockReopenObserver {
            _token: token,
            _block: block,
        })
    }

    /// Fallback (only if CFDateFormatter fails, which should not happen).
    fn fallback_time_medium(unix_secs: u64) -> String {
        let secs = unix_secs % 86_400;
        let (h24, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
        let (h12, ampm) = match h24 % 12 {
            0 => (12, if h24 < 12 { "AM" } else { "PM" }),
            h => (h, if h24 < 12 { "AM" } else { "PM" }),
        };
        format!("{h12}:{m:02}:{s:02} {ampm}")
    }
}

// ---------------------------------------------------------------------------
// Non-macOS stubs
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "macos"))]
mod imp {
    /// The system clipboard is not bridged on non-macOS builds (the tray and
    /// clipboard bridge are macOS-only in this app).
    pub fn clipboard_set(_text: &str) {}

    pub fn clipboard_get() -> Option<String> {
        None
    }

    pub fn format_time_medium(unix_secs: u64) -> String {
        let secs = unix_secs % 86_400;
        let (h24, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
        let (h12, ampm) = match h24 % 12 {
            0 => (12, if h24 < 12 { "AM" } else { "PM" }),
            h => (h, if h24 < 12 { "AM" } else { "PM" }),
        };
        format!("{h12}:{m:02}:{s:02} {ampm}")
    }

    pub fn reduce_motion() -> bool {
        if let Ok(v) = std::env::var("SLACK_GATEWAY_REDUCE_MOTION") {
            return !v.is_empty() && v != "0";
        }
        false
    }

    /// Non-macOS: no on-screen query is bridged; the caller falls back to
    /// the app's own visibility flag.
    pub fn window_on_screen() -> bool {
        true
    }

    pub struct DockReopenObserver;

    pub fn observe_dock_reopen(
        _on_reopen: impl Fn() + Send + Sync + 'static,
    ) -> Option<DockReopenObserver> {
        None
    }
}

pub use imp::*;
