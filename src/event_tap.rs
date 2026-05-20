// event_tap.rs — CGEventTap + run loop

use std::sync::mpsc;
use log::debug;

#[derive(Debug, Clone)]
pub struct MouseEvent {
    pub x: f64,
    pub y: f64,
    pub delta_x: f64,
    pub delta_y: f64,
}

type CGEventRef         = *mut std::ffi::c_void;
type CFMachPortRef      = *mut std::ffi::c_void;
type CFRunLoopRef       = *mut std::ffi::c_void;
type CFRunLoopSourceRef = *mut std::ffi::c_void;
type CFStringRef        = *const std::ffi::c_void;
type CGEventMask        = u64;
type CGEventType        = u32;
type CGEventField       = u32;
type CFTimeInterval     = f64;

const kCGEventMouseMoved:          CGEventType = 5;
const kCGEventLeftMouseDragged:    CGEventType = 6;
const kCGEventRightMouseDragged:   CGEventType = 7;
const kCGEventOtherMouseDragged:   CGEventType = 27;
const kCGMouseEventDeltaX:         CGEventField = 1;
const kCGMouseEventDeltaY:         CGEventField = 2;
const kCGHIDEventTap:              u32 = 0;
const kCGHeadInsertEventTap:       u32 = 0;
const kCGEventTapOptionListenOnly: u32 = 1;

type CGEventTapCallBack = unsafe extern "C" fn(
    proxy:     *mut std::ffi::c_void,
    etype:     CGEventType,
    event:     CGEventRef,
    user_info: *mut std::ffi::c_void,
) -> CGEventRef;

#[repr(C)]
struct CGPoint { x: f64, y: f64 }

extern "C" {
    fn CGEventTapCreate(
        tap:                u32,
        place:              u32,
        options:            u32,
        events_of_interest: CGEventMask,
        callback:           CGEventTapCallBack,
        user_info:          *mut std::ffi::c_void,
    ) -> CFMachPortRef;

    fn CFMachPortCreateRunLoopSource(
        allocator: *mut std::ffi::c_void,
        port:      CFMachPortRef,
        order:     std::ffi::c_long,
    ) -> CFRunLoopSourceRef;

    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRunInMode(mode: CFStringRef, seconds: CFTimeInterval, return_after_source_handled: u8) -> i32;

    static kCFRunLoopDefaultMode: CFStringRef;

    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: CGEventField) -> i64;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

#[link(name = "CoreGraphics",        kind = "framework")] extern "C" {}
#[link(name = "CoreFoundation",      kind = "framework")] extern "C" {}
#[link(name = "ApplicationServices", kind = "framework")] extern "C" {}

// ── Tap state — uses a raw fn pointer + eprintln so it works even if the
//   logger isn't flushed (lets us confirm the callback fires at all)      ──────

struct TapState {
    sender: mpsc::SyncSender<MouseEvent>,
    fired:  std::sync::atomic::AtomicBool,
}

pub fn install_event_tap(sender: mpsc::SyncSender<MouseEvent>) {
    let mask: CGEventMask =
        (1 << kCGEventMouseMoved)
        | (1 << kCGEventLeftMouseDragged)
        | (1 << kCGEventRightMouseDragged)
        | (1 << kCGEventOtherMouseDragged);

    let state = Box::new(TapState {
        sender,
        fired: std::sync::atomic::AtomicBool::new(false),
    });
    let state_ptr = Box::into_raw(state) as *mut std::ffi::c_void;

    unsafe {
        let tap = CGEventTapCreate(
            kCGHIDEventTap,
            kCGHeadInsertEventTap,
            kCGEventTapOptionListenOnly,
            mask,
            tap_callback,
            state_ptr,
        );
        if tap.is_null() {
            panic!(
                "CGEventTapCreate returned null.\n\
                 Grant Accessibility: System Settings → Privacy & Security → Accessibility\n\
                 Add the binary and toggle it ON, then re-run."
            );
        }

        eprintln!("[tap] CGEventTapCreate OK: {:?}", tap);

        let source = CFMachPortCreateRunLoopSource(std::ptr::null_mut(), tap, 0);
        if source.is_null() {
            panic!("CFMachPortCreateRunLoopSource failed");
        }
        eprintln!("[tap] RunLoopSource OK: {:?}", source);

        let rl = CFRunLoopGetCurrent();
        eprintln!("[tap] RunLoop: {:?}", rl);

        CFRunLoopAddSource(rl, source, kCFRunLoopDefaultMode);
        eprintln!("[tap] Source added to run loop");

        CGEventTapEnable(tap, true);
        eprintln!("[tap] Tap enabled — move mouse now");

        debug!("CGEventTap installed ✓");
    }
}

pub fn run_loop() -> ! {
    eprintln!("[tap] Entering run loop...");
    let mut ticks: u64 = 0;
    unsafe {
        loop {
            let result = CFRunLoopRunInMode(kCFRunLoopDefaultMode, 1.0, 0);
            ticks += 1;
            // Print a heartbeat every 5 seconds so we know the loop is alive
            if ticks % 5 == 0 {
                eprintln!("[tap] run loop alive (tick={ticks}, result={result}) — move mouse to test");
            }
        }
    }
}

unsafe extern "C" fn tap_callback(
    _proxy:    *mut std::ffi::c_void,
    _etype:    CGEventType,
    event:     CGEventRef,
    user_info: *mut std::ffi::c_void,
) -> CGEventRef {
    let state = &*(user_info as *const TapState);

    // First-fire diagnostic — print directly so it can't be lost
    if !state.fired.swap(true, std::sync::atomic::Ordering::Relaxed) {
        eprintln!("[tap] *** FIRST CALLBACK FIRED *** tap is working!");
    }

    let pos = CGEventGetLocation(event);
    let dx  = CGEventGetIntegerValueField(event, kCGMouseEventDeltaX) as f64;
    let dy  = CGEventGetIntegerValueField(event, kCGMouseEventDeltaY) as f64;

    // eprintln!("[tap] mouse ({:.0},{:.0})", pos.x, pos.y);  // uncomment if needed

    let _ = state.sender.try_send(MouseEvent {
        x: pos.x, y: pos.y,
        delta_x: dx, delta_y: dy,
    });

    event
}
