//! Output-window fullscreen, decided once per platform.
//!
//! macOS uses "simple" fullscreen: a borderless window covering its screen,
//! with no Space. Native fullscreen gives every projector its own Space and an
//! animated transition, and output windows are dropped and rebuilt whenever
//! the monitor set or projection layout changes. On macOS 13 that teardown
//! crashed AppKit in `-[_NSFullScreenSpace dealloc]`. Other platforms keep
//! winit's borderless fullscreen.

use winit::monitor::MonitorHandle;
use winit::window::{Window, WindowAttributes};

pub(crate) use imp::{enter_after_create, hide_cursor, is_fullscreen, on_monitor, set_fullscreen};

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use winit::platform::macos::{MonitorHandleExtMacOS, WindowExtMacOS};
    use winit::window::WindowLevel;

    /// Open the window near `monitor`; [`enter_after_create`] puts it on the
    /// monitor's screen exactly and makes it fullscreen.
    pub(crate) fn on_monitor(attrs: WindowAttributes, monitor: &MonitorHandle) -> WindowAttributes {
        // No size: AppKit adjusts a titled window larger than its screen as it
        // is shown. Simple fullscreen sets the size.
        attrs.with_position(monitor.position().to_logical::<f64>(monitor.scale_factor()))
    }

    /// Simple fullscreen covers whichever screen the window is on, so first
    /// move it onto the monitor's own screen, as winit does before native
    /// fullscreen. That uses AppKit's screen object rather than coordinates,
    /// which put every output on the main projector at autostart.
    pub(crate) fn enter_after_create(window: &Window, monitor: &MonitorHandle) {
        move_to_screen(window, monitor);
        set_fullscreen(window, true);
    }

    pub(crate) fn is_fullscreen(window: &Window) -> bool {
        window.simple_fullscreen()
    }

    // ponytail: simple fullscreen auto-hides the menu bar and Dock app-wide and
    // each window restores the options it saw on entry; windows dropped while
    // fullscreen leave them auto-hidden until quit. Fine for a show machine.
    pub(crate) fn set_fullscreen(window: &Window, fullscreen: bool) {
        window.set_simple_fullscreen(fullscreen);
        // Without a Space of its own a fullscreen output is an ordinary window,
        // so the control window could come up in front of it. Float it above
        // normal windows while fullscreen; Esc / F11 hands the screen back.
        window.set_window_level(if fullscreen {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        });
    }

    /// Hide the pointer app-wide while any output is fullscreen. winit hides
    /// it with cursor rects, which AppKit honours only in the key window — at
    /// autostart that is the control window behind the outputs, so the arrow
    /// stayed on the projectors. Called every tick; acts only on a change, so
    /// CoreGraphics' hide count stays balanced.
    pub(crate) fn hide_cursor(hidden: bool) {
        use std::sync::atomic::{AtomicBool, Ordering};
        static HIDDEN: AtomicBool = AtomicBool::new(false);
        if HIDDEN.swap(hidden, Ordering::Relaxed) == hidden {
            return;
        }
        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            fn CGMainDisplayID() -> u32;
            fn CGDisplayHideCursor(display: u32) -> i32;
            fn CGDisplayShowCursor(display: u32) -> i32;
        }
        // SAFETY: plain CoreGraphics calls on the main display id; balanced
        // by the swap above.
        unsafe {
            if hidden {
                CGDisplayHideCursor(CGMainDisplayID());
            } else {
                CGDisplayShowCursor(CGMainDisplayID());
            }
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSRect {
        origin: NSPoint,
        size: (f64, f64),
    }

    /// `[window setFrameOrigin:screen.frame.origin]` — winit's own move before
    /// native fullscreen (`window_delegate.rs`, `set_fullscreen`).
    fn move_to_screen(window: &Window, monitor: &MonitorHandle) {
        use objc::runtime::Object;
        use objc::{msg_send, sel, sel_impl};
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

        let Some(screen) = monitor.ns_screen() else {
            log::warn!("Output monitor has no NSScreen; leaving the window where it opened");
            return;
        };
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return;
        };
        unsafe {
            let view = appkit.ns_view.as_ptr() as *mut Object;
            let ns_window: *mut Object = msg_send![view, window];
            if ns_window.is_null() {
                return;
            }
            let frame: NSRect = msg_send![screen as *mut Object, frame];
            let _: () = msg_send![ns_window, setFrameOrigin: frame.origin];
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;
    use winit::window::Fullscreen;

    pub(crate) fn on_monitor(attrs: WindowAttributes, monitor: &MonitorHandle) -> WindowAttributes {
        attrs.with_fullscreen(Some(Fullscreen::Borderless(Some(monitor.clone()))))
    }

    pub(crate) fn enter_after_create(_window: &Window, _monitor: &MonitorHandle) {}

    /// Per-window cursor hiding works here; nothing app-wide to do.
    pub(crate) fn hide_cursor(_hidden: bool) {}

    pub(crate) fn is_fullscreen(window: &Window) -> bool {
        window.fullscreen().is_some()
    }

    pub(crate) fn set_fullscreen(window: &Window, fullscreen: bool) {
        window.set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
    }
}
