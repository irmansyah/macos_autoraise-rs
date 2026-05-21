// accessibility.rs — macOS Accessibility API wrappers

use core_foundation::base::{CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use log::debug;

// ── Raw C bindings ────────────────────────────────────────────────────────────

#[allow(improper_ctypes)]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateApplication(pid: libc::pid_t) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element:   AXUIElementRef,
        attribute: CFStringRef,
        value:     *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> i32;
    fn AXUIElementSetAttributeValue(
        element:   AXUIElementRef,
        attribute: CFStringRef,
        value:     CFTypeRef,
    ) -> i32;
    fn _AXUIElementGetWindow(element: AXUIElementRef, out: *mut u32) -> i32;
    fn CFRelease(cf: CFTypeRef);
    fn CFArrayGetCount(arr: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(arr: CFTypeRef, idx: isize) -> *mut std::ffi::c_void;
}

pub type AXUIElementRef = *mut std::ffi::c_void;

const kAXFocusedWindowAttribute: &str = "AXFocusedWindow";
const kAXWindowsAttribute: &str       = "AXWindows";
const kAXRaiseAction: &str            = "AXRaise";
const kAXMainAttribute: &str          = "AXMain";
const kAXTitleAttribute: &str         = "AXTitle";
const kAXErrorSuccess: i32 = 0;

// ── Public API ────────────────────────────────────────────────────────────────

pub fn is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Return the AXUIElement for an app by PID.
/// Caller must CFRelease the returned element when done.
pub fn ax_app_element(pid: i32) -> AXUIElementRef {
    unsafe { AXUIElementCreateApplication(pid as libc::pid_t) }
}

/// AXRaise all windows of an app element so they stay above tiled windows.
/// Used for floating windows — keeps them on top without changing focus.
/// Takes ownership of app_elem and releases it.
pub fn set_windows_floating(app_elem: AXUIElementRef) {
    if app_elem.is_null() { return; }
    unsafe {
        let wins_attr = CFString::new(kAXWindowsAttribute);
        let mut value: CFTypeRef = std::ptr::null_mut();
        let err = AXUIElementCopyAttributeValue(
            app_elem,
            wins_attr.as_concrete_TypeRef(),
            &mut value,
        );
        if err == kAXErrorSuccess && !value.is_null() {
            let count = CFArrayGetCount(value);
            let raise_action = CFString::new(kAXRaiseAction);
            for i in 0..count {
                let win = CFArrayGetValueAtIndex(value, i);
                if !win.is_null() {
                    AXUIElementPerformAction(win, raise_action.as_concrete_TypeRef());
                }
            }
            CFRelease(value);
        }
        CFRelease(app_elem as CFTypeRef);
    }
}

/// Raise a specific window of the given PID by matching its CGWindowID.
/// If window_ax_id_hint is None, raises the first window on the current space.
/// This avoids pulling windows from other spaces/monitors.
pub fn raise_app_window(pid: i32, window_ax_id_hint: Option<u32>) -> Option<u32> {
    unsafe {
        let app_elem = AXUIElementCreateApplication(pid as libc::pid_t);
        if app_elem.is_null() { return None; }

        // Get ALL windows for this app, not just the focused one.
        // The focused window might be on a different space/monitor.
        let wins_attr = CFString::new(kAXWindowsAttribute);
        let mut wins_val: CFTypeRef = std::ptr::null_mut();
        let err = AXUIElementCopyAttributeValue(
            app_elem,
            wins_attr.as_concrete_TypeRef(),
            &mut wins_val,
        );

        if err != kAXErrorSuccess || wins_val.is_null() {
            // Fallback: raise focused window
            CFRelease(app_elem as CFTypeRef);
            return raise_focused_window(pid, window_ax_id_hint);
        }

        let count = CFArrayGetCount(wins_val);
        let raise_action = CFString::new(kAXRaiseAction);
        let mut raised_wid: Option<u32> = None;

        for i in 0..count {
            let win = CFArrayGetValueAtIndex(wins_val, i) as AXUIElementRef;
            if win.is_null() { continue; }

            let mut wid: u32 = 0;
            _AXUIElementGetWindow(win, &mut wid);

            // If caller gave a hint, only raise the matching window
            if let Some(hint) = window_ax_id_hint {
                if wid != hint { continue; }
            }

            let raise_err = AXUIElementPerformAction(win, raise_action.as_concrete_TypeRef());
            if raise_err == kAXErrorSuccess {
                debug!("AXRaise succeeded for pid={pid} wid={wid}");
                raised_wid = Some(wid);
                break; // Found and raised the right window
            }
        }

        CFRelease(wins_val);
        CFRelease(app_elem as CFTypeRef);
        raised_wid
    }
}

/// Fallback: raise the app's currently focused window.
fn raise_focused_window(pid: i32, window_ax_id_hint: Option<u32>) -> Option<u32> {
    unsafe {
        let app_elem = AXUIElementCreateApplication(pid as libc::pid_t);
        if app_elem.is_null() { return None; }

        let win = match get_focused_window(app_elem) {
            Some(w) => w,
            None => { CFRelease(app_elem as CFTypeRef); return None; }
        };

        let mut wid: u32 = 0;
        let id_err = _AXUIElementGetWindow(win, &mut wid);

        if let Some(hint) = window_ax_id_hint {
            if id_err == kAXErrorSuccess && wid != 0 && wid != hint {
                CFRelease(win as CFTypeRef);
                CFRelease(app_elem as CFTypeRef);
                return None;
            }
        }

        let raise_action = CFString::new(kAXRaiseAction);
        let raise_err = AXUIElementPerformAction(win, raise_action.as_concrete_TypeRef());

        if raise_err != kAXErrorSuccess {
            let main_attr = CFString::new(kAXMainAttribute);
            let cf_true   = core_foundation::boolean::CFBoolean::true_value();
            AXUIElementSetAttributeValue(
                win,
                main_attr.as_concrete_TypeRef(),
                cf_true.as_CFTypeRef(),
            );
        }

        CFRelease(win as CFTypeRef);
        CFRelease(app_elem as CFTypeRef);
        if wid != 0 { Some(wid) } else { None }
    }
}

/// Get the window title of the frontmost window for a PID.
pub fn get_window_title(pid: i32) -> Option<String> {
    unsafe {
        let app_elem = AXUIElementCreateApplication(pid as libc::pid_t);
        if app_elem.is_null() { return None; }

        let win = match get_focused_window(app_elem) {
            Some(w) => w,
            None => {
                CFRelease(app_elem as CFTypeRef);
                return None;
            }
        };

        let title_attr = CFString::new(kAXTitleAttribute);
        let mut value: CFTypeRef = std::ptr::null_mut();
        let err = AXUIElementCopyAttributeValue(
            win,
            title_attr.as_concrete_TypeRef(),
            &mut value,
        );

        CFRelease(win as CFTypeRef);
        CFRelease(app_elem as CFTypeRef);

        if err == kAXErrorSuccess && !value.is_null() {
            let cf_str = CFString::wrap_under_create_rule(value as CFStringRef);
            Some(cf_str.to_string())
        } else {
            None
        }
    }
}

/// Get the raw CGWindowID for the frontmost window of a PID.
pub fn get_ax_window_id(pid: i32) -> Option<u32> {
    unsafe {
        let app_elem = AXUIElementCreateApplication(pid as libc::pid_t);
        if app_elem.is_null() { return None; }

        let win = match get_focused_window(app_elem) {
            Some(w) => w,
            None => {
                CFRelease(app_elem as CFTypeRef);
                return None;
            }
        };

        let mut wid: u32 = 0;
        _AXUIElementGetWindow(win, &mut wid);

        CFRelease(win as CFTypeRef);
        CFRelease(app_elem as CFTypeRef);

        if wid != 0 { Some(wid) } else { None }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

unsafe fn get_focused_window(app_elem: AXUIElementRef) -> Option<AXUIElementRef> {
    let attr = CFString::new(kAXFocusedWindowAttribute);
    let mut value: CFTypeRef = std::ptr::null_mut();
    let err = AXUIElementCopyAttributeValue(
        app_elem,
        attr.as_concrete_TypeRef(),
        &mut value,
    );
    if err == kAXErrorSuccess && !value.is_null() {
        Some(value as AXUIElementRef)
    } else {
        None
    }
}

// ── Linker ────────────────────────────────────────────────────────────────────

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {}
