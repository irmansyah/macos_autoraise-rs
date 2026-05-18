// event_tap.rs — CGEventTap setup for mouse-moved events
//
// This is the same low-level hook AutoRaise uses.
// CGEventTap intercepts HID mouse events BEFORE they reach any application,
// giving us the lowest possible latency — kernel-level, not polling.
//
// We install a PASSIVE tap (kCGEventTapOptionListenOnly) so we never
// block input — identical performance characteristic to AutoRaise.
//
// The tap fires our callback on every mouse-moved event and sends the
// coordinates to the raiser via a channel.

use std::sync::mpsc;
use core_foundation::runloop::{CFRunLoop, CFRunLoopSource, kCFRunLoopDefaultMode};
use core_foundation::base::TCFType;
use log::debug;

/// Message sent from the event tap to the raiser thread.
#[derive(Debug, Clone)]
pub struct MouseEvent {
    pub x: f64,
    pub y: f64,
    pub delta_x: f64,
    pub delta_y: f64,
}

// ── CGEvent types (subset we need) ───────────────────────────────────────────

type CGEventRef = *mut std::ffi::c_void;
type CGEventTapRef = *mut std::ffi::c_void;
type CFMachPortRef = *mut std::ffi::c_void;
type CGEventMask = u64;
type CGEventType = u32;
type CGEventField = u32;

const kCGEventMouseMoved: CGEventType = 5;
const kCGEventLeftMouseDragged: CGEventType = 6;
const kCGEventRightMouseDragged: CGEventType = 7;
const kCGEventOtherMouseDragged: CGEventType = 27;

const kCGMouseEventDeltaX: CGEventField = 1;
const kCGMouseEventDeltaY: CGEventField = 2;

const kCGHIDEventTap: u32 = 0;
const kCGHeadInsertEventTap: u32 = 0;
const kCGEventTapOptionListenOnly: u32 = 1; // passive — never blocks input

type CGEventTapCallBack = unsafe extern "C" fn(
    proxy: *mut std::ffi::c_void,
    etype: CGEventType,
    event: CGEventRef,
    user_info: *mut std::ffi::c_void,
) -> CGEventRef;

#[repr(C)]
struct CGPoint {
    x: f64,
    y: f64,
}

extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: CGEventMask,
        callback: CGEventTapCallBack,
        user_info: *mut std::ffi::c_void,
    ) -> CFMachPortRef;

    fn CFMachPortCreateRunLoopSource(
        allocator: *mut std::ffi::c_void,
        port: CFMachPortRef,
        order: std::ffi::c_long,
    ) -> *mut std::ffi::c_void; // CFRunLoopSourceRef

    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: CGEventField) -> i64;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {}

// ── Tap state passed through the C callback ───────────────────────────────────

struct TapState {
    sender: mpsc::SyncSender<MouseEvent>,
}

/// Install the CGEventTap and start sending MouseEvents to `sender`.
/// This MUST be called on the main thread (or any thread that runs a CFRunLoop).
/// It adds a source to the *current* thread's run loop.
pub fn install_event_tap(sender: mpsc::SyncSender<MouseEvent>) {
    // Mask: mouse moved + all drag variants
    let mask: CGEventMask =
        (1 << kCGEventMouseMoved)
        | (1 << kCGEventLeftMouseDragged)
        | (1 << kCGEventRightMouseDragged)
        | (1 << kCGEventOtherMouseDragged);

    let state = Box::new(TapState { sender });
    let state_ptr = Box::into_raw(state) as *mut std::ffi::c_void;

    unsafe {
        let tap = CGEventTapCreate(
            kCGHIDEventTap,
            kCGHeadInsertEventTap,
            kCGEventTapOptionListenOnly, // passive — no input blocking
            mask,
            tap_callback,
            state_ptr,
        );

        if tap.is_null() {
            panic!(
                "CGEventTapCreate failed — ensure Accessibility permission is granted.\n\
                 System Settings → Privacy & Security → Accessibility → add this binary."
            );
        }

        let source = CFMachPortCreateRunLoopSource(
            std::ptr::null_mut(),
            tap,
            0,
        );
        if source.is_null() {
            panic!("CFMachPortCreateRunLoopSource failed");
        }

        // Add source to current thread's run loop
        let rl = CFRunLoop::get_current();
        let mode = unsafe { kCFRunLoopDefaultMode };
        CFRunLoop::add_source(&rl, unsafe {
            &core_foundation::runloop::CFRunLoopSource::wrap_under_create_rule(
                source as *mut _
            )
        }, mode);

        CGEventTapEnable(tap, true);
        debug!("CGEventTap installed ✓");
    }
}

/// The C callback invoked by the kernel for each mouse event.
/// Runs on the run loop thread — must be fast (no allocation, no blocking).
unsafe extern "C" fn tap_callback(
    _proxy: *mut std::ffi::c_void,
    etype: CGEventType,
    event: CGEventRef,
    user_info: *mut std::ffi::c_void,
) -> CGEventRef {
    let state = &*(user_info as *const TapState);
    let pos = CGEventGetLocation(event);
    let dx = CGEventGetIntegerValueField(event, kCGMouseEventDeltaX) as f64;
    let dy = CGEventGetIntegerValueField(event, kCGMouseEventDeltaY) as f64;

    // Non-blocking send — drop the event if the raiser is behind
    let _ = state.sender.try_send(MouseEvent {
        x: pos.x,
        y: pos.y,
        delta_x: dx,
        delta_y: dy,
    });

    event // passive tap: always return the event unchanged
}
