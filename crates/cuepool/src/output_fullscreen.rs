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

pub(crate) use imp::{enter_after_create, is_fullscreen, on_monitor, set_fullscreen};

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use winit::platform::macos::WindowExtMacOS;

    /// Open the window on `monitor`; [`enter_after_create`] makes it fullscreen.
    pub(crate) fn on_monitor(attrs: WindowAttributes, monitor: &MonitorHandle) -> WindowAttributes {
        // Logical (point) coordinates in the monitor's own scale, so mixed-DPI
        // setups still land the window on the intended screen.
        let scale = monitor.scale_factor();
        attrs
            .with_position(monitor.position().to_logical::<f64>(scale))
            .with_inner_size(monitor.size().to_logical::<f64>(scale))
    }

    /// Simple fullscreen covers whichever screen the window is on, so this
    /// runs after [`on_monitor`] has placed it.
    pub(crate) fn enter_after_create(window: &Window) {
        window.set_simple_fullscreen(true);
    }

    pub(crate) fn is_fullscreen(window: &Window) -> bool {
        window.simple_fullscreen()
    }

    // ponytail: simple fullscreen auto-hides the menu bar and Dock app-wide and
    // each window restores the options it saw on entry; windows dropped while
    // fullscreen leave them auto-hidden until quit. Fine for a show machine.
    pub(crate) fn set_fullscreen(window: &Window, fullscreen: bool) {
        window.set_simple_fullscreen(fullscreen);
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;
    use winit::window::Fullscreen;

    pub(crate) fn on_monitor(attrs: WindowAttributes, monitor: &MonitorHandle) -> WindowAttributes {
        attrs.with_fullscreen(Some(Fullscreen::Borderless(Some(monitor.clone()))))
    }

    pub(crate) fn enter_after_create(_window: &Window) {}

    pub(crate) fn is_fullscreen(window: &Window) -> bool {
        window.fullscreen().is_some()
    }

    pub(crate) fn set_fullscreen(window: &Window, fullscreen: bool) {
        window.set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
    }
}
