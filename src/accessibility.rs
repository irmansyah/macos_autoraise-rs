// accessibility.rs — macOS Accessibility API wrappers

use core_foundation::array::CFArrayRef;
use core_foundation::base::{CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::window::CGWindowID;

// ── Raw C bindings ────────────────────────────────────────────────────────────

// unsafe fn do_raise(pid: i32, window_id: u32) {
//     // Modify your accessibility function call to accept the specific window_id
//     // This tells AXUIElement to perform AXPerformAction(window_ref, kAXRaiseAction)
//     accessibility::raise_app_window(pid, Some(window_id));
//
//     // Activate the application WITHOUT bringing all windows forward
//     let cls = match Class::get("NSRunningApplication") {
//         Some(c) => c,
//         None    => return,
//     };
//     let app: *mut Object = msg_send![cls,
//         runningApplicationWithProcessIdentifier: pid as i32
//     ];
//     if !app.is_null() {
//         // Option 1u64 is NSApplicationActivateIgnoringOtherApps
//         // This brings the process focus up without shifting internal window orders natively
//         let _: bool = msg_send![app, activateWithOptions: 1u64];
//     }
// }

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

/// Raise the frontmost window of the given PID.
pub unsafe fn raise_app_window(pid: i32, target_window_id: Option<u32>) {
    // 1. Get AXUIElement for the application
    let app_ref = AXUIElementCreateApplication(pid);
    if app_ref.is_null() { return; }

    // 2. Get copy of window list attribute
    let mut values: CFArrayRef = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(app_ref, CFString::new("AXWindows").as_concrete_TypeRef(), &mut values as *mut _ as *mut _);
    
    if err == 0 && !values.is_null() {
        let count = CFArrayGetCount(values as CFTypeRef);
        for i in 0..count {
            let win_element = CFArrayGetValueAtIndex(values as CFTypeRef, i) as AXUIElementRef;
            if win_element.is_null() { continue; }

            if let Some(target_id) = target_window_id {
                // Fetch the unique AX ID of this window element
                let mut cgid: CGWindowID = 0;
                let _ = _AXUIElementGetWindow(win_element, &mut cgid);
                
                if cgid != target_id {
                    continue; // Skip windows that aren't the one under the cursor
                }
            }

            // Perform the raise action on the exact matching window
            AXUIElementPerformAction(win_element, CFString::new("AXRaise").as_concrete_TypeRef());
            break;
        }
        CFRelease(values as CFTypeRef);
    }
    CFRelease(app_ref as CFTypeRef);
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
