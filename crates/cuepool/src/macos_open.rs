//! Open projects handed over by Finder: double-click, Open With, or a file
//! dropped on the Dock icon.
//!
//! AppKit delivers those as `application:openURLs:` on the app delegate.
//! winit's `WinitApplicationDelegate` does not answer it, so AppKit fell back
//! to its document machinery and told the operator "CuePool cannot open files
//! of this type". This grafts the method onto winit's delegate the same way
//! `macos_quit` grafts `applicationShouldTerminate:`.
//!
//! As there, the hook touches no shared state — AppKit can call it from a
//! nested run loop while the main thread holds the state lock (`rfd`'s file
//! picker). It only queues the paths; `about_to_wait` hands them to the GUI as
//! `AppCommand::OpenProject`, the File > Open path, unsaved-changes check
//! included.

// objc FFI: see macos_quit.rs.
#![allow(clippy::missing_transmute_annotations, clippy::manual_c_str_literals)]

use cuepool_gui::logging::PERSIST_TARGET;
use std::path::PathBuf;
use std::sync::Mutex;

/// Paths Finder asked us to open, drained by the event loop. A leaf lock:
/// held only to push or take, never across anything else.
static PENDING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Paths queued since the last call, oldest first.
pub fn take_pending() -> Vec<PathBuf> {
    std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Install the hook. Must run after the event loop is built, like
/// `macos_quit::install`, and before it runs: a cold launch from Finder sends
/// the open request right after `applicationDidFinishLaunching:`.
pub fn install() {
    use objc::runtime::{Class, NO, Object, Sel, class_addMethod};
    use objc::{sel, sel_impl};
    use std::mem;
    use std::os::raw::c_char;

    extern "C" fn open_urls(_self: &Object, _sel: Sel, _app: *mut Object, urls: *mut Object) {
        let paths = unsafe { file_paths(urls) };
        log::info!(target: PERSIST_TARGET, "Finder asked to open {paths:?}");
        PENDING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(paths);
    }

    unsafe {
        let Some(delegate_class) = Class::get("WinitApplicationDelegate") else {
            log::warn!(
                target: PERSIST_TARGET,
                "WinitApplicationDelegate class not found — opening projects from Finder \
                 will not work."
            );
            return;
        };
        let cls = delegate_class as *const _ as *mut Class;
        // void return; (id self, SEL _cmd, NSApplication *app, NSArray<NSURL *> *urls).
        let enc = "v@:@@\0".as_ptr() as *const c_char;
        let added = class_addMethod(
            cls,
            sel!(application:openURLs:),
            mem::transmute::<extern "C" fn(&Object, Sel, *mut Object, *mut Object), _>(open_urls),
            enc,
        );
        if added == NO {
            log::warn!(
                target: PERSIST_TARGET,
                "WinitApplicationDelegate already answers application:openURLs: — the \
                 Finder open hook was not installed."
            );
        }
    }
}

/// Filesystem paths of the file URLs in an `NSArray<NSURL *>`.
unsafe fn file_paths(urls: *mut objc::runtime::Object) -> Vec<PathBuf> {
    use objc::runtime::{NO, Object};
    use objc::{msg_send, sel, sel_impl};
    use std::ffi::{CStr, OsStr};
    use std::os::raw::c_char;
    use std::os::unix::ffi::OsStrExt;

    if urls.is_null() {
        return Vec::new();
    }
    unsafe {
        let count: usize = msg_send![urls, count];
        (0..count)
            .filter_map(|index| {
                let url: *mut Object = msg_send![urls, objectAtIndex: index];
                let is_file: objc::runtime::BOOL = msg_send![url, isFileURL];
                if is_file == NO {
                    return None;
                }
                let path: *mut Object = msg_send![url, path];
                let utf8: *const c_char = msg_send![path, UTF8String];
                (!utf8.is_null())
                    .then(|| PathBuf::from(OsStr::from_bytes(CStr::from_ptr(utf8).to_bytes())))
            })
            .collect()
    }
}
