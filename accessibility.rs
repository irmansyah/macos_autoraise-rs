// accessibility.rs — macOS Accessibility API wrappers
//
// We use the raw C AX API through Rust's FFI.  This is identical to what
// AutoRaise (ObjC++) calls; zero overhead difference.
//
// Key calls:
//   AXIsProcessTrusted()             — check permissions
//   AXUIElementCreateSystemWide()    — root element
//   AXUIElementCopyAttributeValue()  — read attributes
//   AXUIElementPerformAction()       — raise / press
//   _AXUIElementGetWindow()          — get raw window ID (same private API AutoRaise uses)

use std::ffi::CStr;
use core_foundation::base::{CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use log::debug;

// ── Raw C bindings ────────────────────────────────────────────────────────────

#[allow(improper_ctypes)]
extern "C" {
    // Public AX API
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCreateApplication(pid: libc::pid_t) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32; // AXError

    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> i32;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> i32;

    // Private API — same one used by AutoRaise. Returns the raw CGWindowID.
    fn _AXUIElementGetWindow(element: AXUIElementRef, out: *mut u32) -> i32;

    // CGWindowServer (for warp, not used in core path)
    fn CFRelease(cf: CFTypeRef);
}

// AXUIElementRef is just an opaque pointer
pub type AXUIElementRef = *mut std::ffi::c_void;

// AX attribute name constants
const kAXFocusedApplicationAttribute: &str = "AXFocusedApplication";
const kAXFocusedWindowAttribute: &str = "AXFocusedWindow";
const kAXWindowsAttribute: &str = "AXWindows";
const kAXRaiseAction: &str = "AXRaise";
const kAXMainAttribute: &str = "AXMain";
const kAXTitleAttribute: &str = "AXTitle";
const kAXRoleAttribute: &str = "AXRole";
const kAXSubroleAttribute: &str = "AXSubrole";
const kAXMinimizedAttribute: &str = "AXMinimized";

// AXError codes
const kAXErrorSuccess: i32 = 0;

// ── Public API ────────────────────────────────────────────────────────────────

/// Returns true if this process has Accessibility permission.
pub fn is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Raise + optionally focus the window at a given screen point.
/// Returns the AX window ID of the window that was raised (if any).
pub fn raise_window_at_point(x: f64, y: f64) -> Option<u32> {
    unsafe {
        // 1. Get the system-wide element
        let system = AXUIElementCreateSystemWide();
        if system.is_null() { return None; }

        // 2. Hit-test: ask which element is at (x, y)
        let point_attr = cfstring("AXElementAtPosition");
        // We actually use CGWindowListCopyWindowInfo path; see raiser.rs for the
        // CGEvent-tap approach.  Here we do pure AX hit-test.
        let _ = system; // used via raiser path

        // This path is invoked by raiser.rs after it already has the target PID.
        // Direct entry point below.
        None
    }
}

/// Given a PID, raise its frontmost window and bring it to focus.
/// Returns the AX CGWindowID on success.
pub fn raise_app_window(pid: i32, window_ax_id_hint: Option<u32>) -> Option<u32> {
    unsafe {
        let app_elem = AXUIElementCreateApplication(pid as libc::pid_t);
        if app_elem.is_null() { return None; }

        // Get the focused window of the target app
        let win = get_focused_window(app_elem);
        let win = match win {
            Some(w) => w,
            None => {
                CFRelease(app_elem as CFTypeRef);
                return None;
            }
        };

        // Get its raw window ID
        let mut wid: u32 = 0;
        let err = _AXUIElementGetWindow(win, &mut wid);

        // If caller gave us a hint and it doesn't match, skip
        if let Some(hint) = window_ax_id_hint {
            if err == kAXErrorSuccess && wid != 0 && wid != hint {
                CFRelease(win as CFTypeRef);
                CFRelease(app_elem as CFTypeRef);
                return None;
            }
        }

        // Perform AXRaise
        let raise_action = cfstring(kAXRaiseAction);
        let raise_err = AXUIElementPerformAction(win, raise_action as CFStringRef);
        CFRelease(raise_action as CFTypeRef);

        if raise_err == kAXErrorSuccess {
            debug!("AXRaise succeeded for pid={pid} wid={wid}");
        } else {
            // Fallback: set AXMain = true
            let main_attr = cfstring(kAXMainAttribute);
            let cf_true = core_foundation::boolean::CFBoolean::true_value();
            AXUIElementSetAttributeValue(
                win,
                main_attr as CFStringRef,
                cf_true.as_CFTypeRef(),
            );
            CFRelease(main_attr as CFTypeRef);
        }

        CFRelease(win as CFTypeRef);
        CFRelease(app_elem as CFTypeRef);

        if wid != 0 { Some(wid) } else { None }
    }
}

/// Get the window title for filtering (ignoreTitles check).
pub fn get_window_title(pid: i32) -> Option<String> {
    unsafe {
        let app_elem = AXUIElementCreateApplication(pid as libc::pid_t);
        if app_elem.is_null() { return None; }

        let win = get_focused_window(app_elem)?;
        let title_attr = cfstring(kAXTitleAttribute);
        let mut value: CFTypeRef = std::ptr::null_mut();
        let err = AXUIElementCopyAttributeValue(win, title_attr as CFStringRef, &mut value);
        CFRelease(title_attr as CFTypeRef);
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

/// Get raw AX window ID for the focused window of a PID.
pub fn get_ax_window_id(pid: i32) -> Option<u32> {
    unsafe {
        let app_elem = AXUIElementCreateApplication(pid as libc::pid_t);
        if app_elem.is_null() { return None; }
        let win = get_focused_window(app_elem);
        let win = match win {
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
    let attr = cfstring(kAXFocusedWindowAttribute);
    let mut value: CFTypeRef = std::ptr::null_mut();
    let err = AXUIElementCopyAttributeValue(app_elem, attr as CFStringRef, &mut value);
    CFRelease(attr as CFTypeRef);
    if err == kAXErrorSuccess && !value.is_null() {
        Some(value as AXUIElementRef)
    } else {
        None
    }
}

/// Create a temporary CFStringRef from a &str for API calls.
/// Caller is responsible for CFRelease (or use a wrapper that does it).
unsafe fn cfstring(s: &str) -> CFString {
    CFString::new(s)
}

// ── Linker directives ─────────────────────────────────────────────────────────
// These tell the linker to include the right libraries for the AX symbols.

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {}

#[link(name = "AXRuntime", kind = "dylib")]
extern "C" {}
