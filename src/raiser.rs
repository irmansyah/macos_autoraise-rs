// raiser.rs — Core auto-raise logic

#![allow(non_upper_case_globals, non_snake_case)]

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use log::{debug, info, warn};
use core_foundation::base::{CFTypeRef, TCFType};
use core_foundation::array::CFArrayRef;
use core_foundation::dictionary::CFDictionaryRef;
use core_foundation::string::{CFString, CFStringRef};

use crate::config::Config;
use crate::aerospace::AeroSpaceState;
use crate::event_tap::{self, MouseEvent};
use crate::accessibility;

// ── CGWindowListCopyWindowInfo ────────────────────────────────────────────────

type CGWindowID = u32;
const kCGWindowListOptionOnScreenOnly:     u32 = 1 << 0;
const kCGWindowListExcludeDesktopElements: u32 = 1 << 4;
const kCGNullWindowID: CGWindowID = 0;

extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relativeToWindow: CGWindowID) -> CFArrayRef;
}

// ── Modifier key ──────────────────────────────────────────────────────────────

extern "C" {
    fn CGEventSourceFlagsState(stateID: u32) -> u64;
}
const kCGEventSourceStateCombinedSessionState: u32 = 1;
const kCGEventFlagMaskAlternate: u64 = 0x0008_0000;
const kCGEventFlagMaskControl:   u64 = 0x0004_0000;

#[link(name = "CoreGraphics", kind = "framework")] extern "C" {}
#[link(name = "AppKit", kind = "framework")] extern "C" {}

// GCD — dispatch raise to main thread so AppKit calls work correctly
use dispatch::Queue;

use objc::runtime::{Class, Object};
use objc::{msg_send, sel, sel_impl};

// ── CGWindow dict keys ────────────────────────────────────────────────────────

const kCGWindowOwnerPID:  &str = "kCGWindowOwnerPID";
const kCGWindowNumber:    &str = "kCGWindowNumber";
const kCGWindowLayer:     &str = "kCGWindowLayer";
const kCGWindowBounds:    &str = "kCGWindowBounds";
const kCGWindowOwnerName: &str = "kCGWindowOwnerName";

// ── Public Raiser ─────────────────────────────────────────────────────────────

pub struct Raiser {
    config: Arc<Mutex<Config>>,
}

impl Raiser {
    pub fn new(config: Arc<Mutex<Config>>) -> Self {
        Self { config }
    }

    pub fn run(self) -> ! {
        let cfg = self.config.lock().unwrap().clone();
        let poll_millis      = cfg.poll_millis;
        let aerospace_cycles = cfg.aerospace_refresh_cycles;
        let aerospace_aware  = cfg.aerospace_aware;
        drop(cfg);

        // Bounded channel — drop oldest if raiser falls behind
        let (tx, rx) = std::sync::mpsc::sync_channel::<MouseEvent>(64);

        let config_clone = self.config.clone();
        thread::spawn(move || {
            let mut state = RaiserState::new(
                config_clone,
                aerospace_aware,
                aerospace_cycles,
                poll_millis,
            );
            loop {
                // Drain, keep only latest position
                let mut last: Option<MouseEvent> = None;
                while let Ok(ev) = rx.try_recv() {
                    last = Some(ev);
                }
                if let Some(ev) = last {
                    state.handle_mouse_event(ev);
                }
                thread::sleep(Duration::from_millis(poll_millis));
            }
        });

        event_tap::install_event_tap(tx);

        info!("Running. Move mouse over any window to trigger auto-raise.");
        event_tap::run_loop()
    }
}

// ── State machine ─────────────────────────────────────────────────────────────

struct RaiserState {
    config: Arc<Mutex<Config>>,
    aerospace: Option<AeroSpaceState>,
    poll_millis: u64,
    last_window_pid: Option<i32>,
    // how many consecutive ticks the mouse has been still
    still_ticks: u32,
    last_x: f64,
    last_y: f64,
    aerospace_cycle_counter: u32,
    aerospace_refresh_cycles: u32,
    // track whether we already raised the current window
    raised_pid: Option<i32>,
}

impl RaiserState {
    fn new(
        config: Arc<Mutex<Config>>,
        aerospace_aware: bool,
        aerospace_cycles: u32,
        poll_millis: u64,
    ) -> Self {
        let aerospace = if aerospace_aware {
            Some(AeroSpaceState::new(aerospace_cycles, poll_millis))
        } else {
            None
        };
        Self {
            config,
            aerospace,
            poll_millis,
            last_window_pid: None,
            still_ticks: 0,
            last_x: -9999.0,
            last_y: -9999.0,
            aerospace_cycle_counter: 0,
            aerospace_refresh_cycles: aerospace_cycles,
            raised_pid: None,
        }
    }

    fn handle_mouse_event(&mut self, ev: MouseEvent) {
        let (x, y) = (ev.x, ev.y);
        let moved = (x - self.last_x).abs() > 1.0 || (y - self.last_y).abs() > 1.0;

        // Periodic AeroSpace refresh
        self.aerospace_cycle_counter += 1;
        if self.aerospace_cycle_counter >= self.aerospace_refresh_cycles {
            self.aerospace_cycle_counter = 0;
            if let Some(ref mut s) = self.aerospace {
                s.refresh_if_due();
            }
        }

        let cfg = self.config.lock().unwrap().clone();

        // ── 1. Modifier key disable ───────────────────────────────────────────
        if self.modifier_key_held(&cfg.disable_key) {
            debug!("Raising disabled (modifier held)");
            self.last_x = x;
            self.last_y = y;
            return;
        }

        // ── 2. Find window under cursor ───────────────────────────────────────
        let win = match window_at_point(x, y) {
            Some(w) => w,
            None => {
                debug!("No window at ({x:.0},{y:.0})");
                self.last_window_pid = None;
                self.raised_pid = None;
                self.still_ticks = 0;
                self.last_x = x;
                self.last_y = y;
                return;
            }
        };

        debug!(
            "At ({x:.0},{y:.0}): app='{}' pid={} layer={}",
            win.app_name, win.pid, win.layer
        );

        self.last_x = x;
        self.last_y = y;

        // ── 3. Skip non-normal layers (menus, panels, HUD, dock) ─────────────
        if win.layer != 0 {
            debug!("  → skip layer {}", win.layer);
            return;
        }

        // ── 4. Ignore app list ────────────────────────────────────────────────
        let app_lower = win.app_name.to_lowercase();
        if cfg.ignore_apps.iter().any(|a| a.to_lowercase() == app_lower) {
            debug!("  → ignored app");
            return;
        }

        // ── 5. Ignore title list ──────────────────────────────────────────────
        if !cfg.ignore_titles.is_empty() {
            if let Some(title) = accessibility::get_window_title(win.pid) {
                let tl = title.to_lowercase();
                if cfg.ignore_titles.iter().any(|t| tl.contains(&t.to_lowercase())) {
                    debug!("  → ignored title '{title}'");
                    return;
                }
            }
        }

        // ── 6. AeroSpace: skip tiled windows ─────────────────────────────────
        if let Some(ref as_state) = self.aerospace {
            if as_state.available {
                if let Some(ax_id) = accessibility::get_ax_window_id(win.pid) {
                    if !as_state.should_raise(ax_id) {
                        debug!("  → tiled (AeroSpace manages focus)");
                        // Reset so moving back to a floating window works
                        if self.last_window_pid != Some(win.pid) {
                            self.raised_pid = None;
                        }
                        self.last_window_pid = Some(win.pid);
                        return;
                    }
                }
            }
        }

        // ── 7. Detect window change ───────────────────────────────────────────
        let new_window = self.last_window_pid != Some(win.pid);
        if new_window {
            debug!("  → new window, resetting state");
            self.still_ticks = 0;
            self.raised_pid = None;
        }
        self.last_window_pid = Some(win.pid);

        // Track stillness
        if moved { self.still_ticks = 0; } else { self.still_ticks += 1; }

        // ── 8. Already raised this window? ────────────────────────────────────
        if self.raised_pid == Some(win.pid) {
            return;
        }

        // ── 9. Delay gate ─────────────────────────────────────────────────────
        let delay = cfg.delay;
        if delay == 0 {
            return; // raising disabled
        }
        if delay > 1 && cfg.require_mouse_stop && self.still_ticks < delay {
            debug!("  → waiting ({}/{})", self.still_ticks, delay);
            return;
        }

        // ── 10. Raise! ────────────────────────────────────────────────────────
        let pid = win.pid;
        let bounds = win.bounds; // Capture bounds
        let app = win.app_name.clone();
        
        let border_width = cfg.border_width;
        let border_color = cfg.border_color.clone();

        debug!("  → RAISING '{}' pid={}", app, pid);
        self.raised_pid = Some(pid);

        // Update the function call:
        raise_on_main_thread(pid, bounds, border_width, border_color);
    }

    fn modifier_key_held(&self, key: &str) -> bool {
        if key == "disabled" { return false; }
        unsafe {
            let flags = CGEventSourceFlagsState(kCGEventSourceStateCombinedSessionState);
            match key {
                "control" => (flags & kCGEventFlagMaskControl) != 0,
                "option"  => (flags & kCGEventFlagMaskAlternate) != 0,
                _         => false,
            }
        }
    }
}

// ── Raise dispatched to main thread ──────────────────────────────────────────

fn raise_on_main_thread(pid: i32, bounds: (f64, f64, f64, f64), b_width: f64, b_color: String) {
    Queue::main().exec_async(move || {
        unsafe { 
            do_raise(pid); 
            
            // Draw the border immediately after raising
            crate::border::update_border(bounds.0, bounds.1, bounds.2, bounds.3, b_width, &b_color);
        };
    });
}

unsafe fn do_raise(pid: i32) {
    // AXRaise — brings window to front within the app
    accessibility::raise_app_window(pid, None);

    // Activate the app — switches focus
    let cls = match Class::get("NSRunningApplication") {
        Some(c) => c,
        None    => return,
    };
    let app: *mut Object = msg_send![cls,
        runningApplicationWithProcessIdentifier: pid as i32
    ];
    if !app.is_null() {
        // NSApplicationActivateIgnoringOtherApps = 2
        let _: bool = msg_send![app, activateWithOptions: 2u64];
    }
}

// ── CoreFoundation Raw FFI ────────────────────────────────────────────────────

extern "C" {
    fn CFArrayGetCount(theArray: CFArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(theArray: CFArrayRef, idx: isize) -> CFTypeRef;
    fn CFDictionaryGetValue(theDict: CFDictionaryRef, key: CFTypeRef) -> CFTypeRef;
    fn CFNumberGetValue(number: CFTypeRef, theType: i64, valuePtr: *mut std::ffi::c_void) -> bool;
    fn CFRelease(cf: CFTypeRef);
}

// ── Window hit-test ───────────────────────────────────────────────────────────

#[derive(Debug)]
struct WindowInfo {
    pid:       i32,
    app_name:  String,
    window_id: u32,
    layer:     i32,
    bounds:    (f64, f64, f64, f64),
}

fn window_at_point(x: f64, y: f64) -> Option<WindowInfo> {
    unsafe {
        let list = CGWindowListCopyWindowInfo(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            kCGNullWindowID,
        );
        if list.is_null() {
            warn!("[raiser] CGWindowListCopyWindowInfo returned NULL");
            return None;
        }

        let count = CFArrayGetCount(list);
        debug!("[raiser] window list count={}, cursor=({x:.1},{y:.1})", count);

        for i in 0..count {
            let dict_ptr = CFArrayGetValueAtIndex(list, i) as CFDictionaryRef;
            if dict_ptr.is_null() { continue; }

            let layer = get_dict_int(dict_ptr, kCGWindowLayer).unwrap_or(999) as i32;
            let bounds = match get_dict_bounds(dict_ptr, kCGWindowBounds) {
                Some(b) => b,
                None    => continue,
            };

            debug!("[raiser]   layer={layer:3} bounds=({:.0},{:.0},{:.0},{:.0})",
                bounds.0, bounds.1, bounds.2, bounds.3);

            // Point-in-bounds test
            if x < bounds.0 || y < bounds.1
                || x > bounds.0 + bounds.2
                || y > bounds.1 + bounds.3
            {
                continue;
            }

            let pid = get_dict_int(dict_ptr, kCGWindowOwnerPID).unwrap_or(0) as i32;
            if pid == 0 { continue; }

            let app_name = get_dict_string(dict_ptr, kCGWindowOwnerName)
                .unwrap_or_else(|| "Unknown".to_string());
            let window_id = get_dict_int(dict_ptr, kCGWindowNumber).unwrap_or(0) as u32;

            // Replace the old return Some(...) inside window_at_point with:
            CFRelease(list as CFTypeRef);
            return Some(WindowInfo { pid, app_name, window_id, layer, bounds });
        }
        
        // Release the list if no match found
        CFRelease(list as CFTypeRef);
        None
    }
}

// ── Safe CFDictionary helpers ─────────────────────────────────────────────────

unsafe fn get_dict_int(dict: CFDictionaryRef, key: &str) -> Option<i64> {
    let k = CFString::new(key);
    let val_ptr = CFDictionaryGetValue(dict, k.as_CFTypeRef());
    if val_ptr.is_null() { return None; }

    let mut val: i64 = 0;
    // kCFNumberSInt64Type = 4
    if CFNumberGetValue(val_ptr, 4, &mut val as *mut i64 as *mut _) {
        Some(val)
    } else {
        None
    }
}

unsafe fn get_dict_f64(dict: CFDictionaryRef, key: &str) -> Option<f64> {
    let k = CFString::new(key);
    let val_ptr = CFDictionaryGetValue(dict, k.as_CFTypeRef());
    if val_ptr.is_null() { return None; }

    let mut val: f64 = 0.0;
    // kCFNumberFloat64Type = 13
    if CFNumberGetValue(val_ptr, 13, &mut val as *mut f64 as *mut _) {
        Some(val)
    } else {
        None
    }
}

unsafe fn get_dict_string(dict: CFDictionaryRef, key: &str) -> Option<String> {
    let k = CFString::new(key);
    let val_ptr = CFDictionaryGetValue(dict, k.as_CFTypeRef());
    if val_ptr.is_null() { return None; }

    let cf_str = CFString::wrap_under_get_rule(val_ptr as CFStringRef);
    Some(cf_str.to_string())
}

unsafe fn get_dict_bounds(dict: CFDictionaryRef, key: &str) -> Option<(f64, f64, f64, f64)> {
    let k = CFString::new(key);
    let val_ptr = CFDictionaryGetValue(dict, k.as_CFTypeRef());
    if val_ptr.is_null() { return None; }

    let sub_dict = val_ptr as CFDictionaryRef;
    let x = get_dict_f64(sub_dict, "X").unwrap_or(0.0);
    let y = get_dict_f64(sub_dict, "Y").unwrap_or(0.0);
    let w = get_dict_f64(sub_dict, "Width").unwrap_or(0.0);
    let h = get_dict_f64(sub_dict, "Height").unwrap_or(0.0);
    
    if w == 0.0 || h == 0.0 { return None; }
    Some((x, y, w, h))
}
