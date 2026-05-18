// accessibility.rs — macOS Accessibility API wrappers
//
// We use the raw C AX API through Rust's FFI.  This is identical to what
// AutoRaise (ObjC++) calls; zero overhead difference.
//
// Key calls:
//   AXIsProcessTrusted()             — check permissions
//   AXUIElementCreateApplication()   — app element from PID
//   AXUIElementCopyAttributeValue()  — read attributes
//   AXUIElementPerformAction()       — raise / press
//   _AXUIElementGetWindow()          — get raw window ID (same private API AutoRaise uses)

use core_foundation::base::{CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use log::debug;

// ── Raw C bindings ────────────────────────────────────────────────────────────

#[allow(improper_ctypes)]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateApplication(pid: libc::pid_t) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> i32;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> i32;
    fn _AXUIElementGetWindow(element: AXUIElementRef, out: *mut u32) -> i32;
    fn CFRelease(cf: CFTypeRef);
}

pub type AXUIElementRef = *mut std::ffi::c_void;

// AX attribute / action name constants
const kAXFocusedWindowAttribute: &str = "AXFocusedWindow";
const kAXRaiseAction: &str            = "AXRaise";
const kAXMainAttribute: &str          = "AXMain";
const kAXTitleAttribute: &str         = "AXTitle";

const kAXErrorSuccess: i32 = 0;

// ── Public API ────────────────────────────────────────────────────────────────

/// Returns true if this process has Accessibility permission.
pub fn is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Raise the frontmost window of the given PID.
/// Returns the raw CGWindowID on success.
pub fn raise_app_window(pid: i32, window_ax_id_hint: Option<u32>) -> Option<u32> {
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

        // Get raw CGWindowID
        let mut wid: u32 = 0;
        let id_err = _AXUIElementGetWindow(win, &mut wid);

        // If caller gave a hint and it doesn't match, skip this window
        if let Some(hint) = window_ax_id_hint {
            if id_err == kAXErrorSuccess && wid != 0 && wid != hint {
                CFRelease(win as CFTypeRef);
                CFRelease(app_elem as CFTypeRef);
                return None;
            }
        }

        // AXRaise
        let raise_action = CFString::new(kAXRaiseAction);
        let raise_err = AXUIElementPerformAction(win, raise_action.as_concrete_TypeRef());

        if raise_err == kAXErrorSuccess {
            debug!("AXRaise succeeded for pid={pid} wid={wid}");
        } else {
            // Fallback: set AXMain = true
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

/// Get the window title of the frontmost window for a PID (used for ignoreTitles).
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
            // wrap_under_create_rule takes ownership — will CFRelease on drop
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

// ── Linker directives ─────────────────────────────────────────────────────────

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {}
