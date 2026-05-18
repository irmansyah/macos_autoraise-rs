// raiser.rs — Core auto-raise logic
//
// Architecture:
//   Main thread  → CFRunLoop with CGEventTap  (kernel events, zero latency)
//   Raiser thread ← receives MouseEvent via channel → decides raise/skip
//
// AeroSpace integration:
//   When aerospace_aware=true, we call `aerospace list-windows` periodically
//   to get the set of floating window IDs.  Only floating windows are raised;
//   tiled windows are left alone (AeroSpace manages their focus).
//
// Raise decision per mouse event:
//   1. Find window under cursor (CGWindowListCopyWindowInfo → PID lookup)
//   2. Check modifier key (disable_key held → skip)
//   3. Check ignore_apps / ignore_titles
//   4. Check AeroSpace: is window floating? (if aerospace_aware=true)
//   5. Apply delay (ticks the mouse must be still)
//   6. AXRaise + NSRunningApplication activate

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::collections::VecDeque;
use log::{debug, info, warn};
use core_foundation::runloop::CFRunLoop;

use crate::config::Config;
use crate::aerospace::AeroSpaceState;
use crate::event_tap::{self, MouseEvent};
use crate::accessibility;

// ── CGWindowListCopyWindowInfo bindings ───────────────────────────────────────

use core_foundation::base::{CFTypeRef, TCFType, kCFAllocatorDefault};
use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::number::{CFNumber, CFNumberRef};

type CGWindowID = u32;

const kCGWindowListOptionOnScreenOnly: u32 = 1 << 0;
const kCGWindowListExcludeDesktopElements: u32 = 1 << 4;
const kCGNullWindowID: CGWindowID = 0;

extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relativeToWindow: CGWindowID) -> CFArrayRef;
    // CGS private for getting window under point
    fn CGWindowListCopyWindowInfo_at_point(x: f64, y: f64) -> CFArrayRef;
}

// Modifier key flags (from CGEventFlags)
const kCGEventFlagMaskControl: u64 = 0x0000_0040_0000_0000; // wait, wrong — use NSEvent
// Actually use CGEventSourceFlagsState
extern "C" {
    fn CGEventSourceFlagsState(stateID: u32) -> u64;
}
const kCGEventSourceStateCombinedSessionState: u32 = 1;
const kCGEventFlagMaskAlternate: u64  = 0x00000080; // option
const kCGEventFlagMaskControl2: u64   = 0x00040000; // control (correct mask)

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {}

// NSRunningApplication activation
use objc::runtime::{Class, Object};
use objc::{msg_send, sel, sel_impl};

// ── Window info key strings ───────────────────────────────────────────────────
const kCGWindowOwnerPID: &str   = "kCGWindowOwnerPID";
const kCGWindowNumber: &str     = "kCGWindowNumber";
const kCGWindowLayer: &str      = "kCGWindowLayer";
const kCGWindowBounds: &str     = "kCGWindowBounds";
const kCGWindowOwnerName: &str  = "kCGWindowOwnerName";
const kCGWindowName: &str       = "kCGWindowName";
const kCGWindowIsOnscreen: &str = "kCGWindowIsOnscreen";

// ── Raiser ────────────────────────────────────────────────────────────────────

pub struct Raiser {
    config: Arc<Mutex<Config>>,
}

impl Raiser {
    pub fn new(config: Arc<Mutex<Config>>) -> Self {
        Self { config }
    }

    /// Block forever: installs CGEventTap on main thread, raiser on background.
    pub fn run(self) {
        let cfg = self.config.lock().unwrap().clone();
        let poll_millis = cfg.poll_millis;
        let aerospace_cycles = cfg.aerospace_refresh_cycles;
        let aerospace_aware = cfg.aerospace_aware;
        drop(cfg);

        // Bounded channel — at most 64 queued events.
        // If raiser falls behind, new events simply overwrite the backlog
        // (try_send with drop).  This keeps latency bounded.
        let (tx, rx) = std::sync::mpsc::sync_channel::<MouseEvent>(64);

        // Raiser runs on a background thread
        let config_clone = self.config.clone();
        thread::spawn(move || {
            let mut state = RaiserState::new(
                config_clone,
                aerospace_aware,
                aerospace_cycles,
                poll_millis,
            );
            // Drain the channel with our poll interval
            loop {
                // Collect all pending events, keep only the latest
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

        // Install CGEventTap on main thread
        event_tap::install_event_tap(tx);

        // Run the main run loop — this is where the CGEventTap fires
        info!("Running. Move mouse over any window to trigger auto-raise.");
        unsafe { CFRunLoop::run_current() };
    }
}

// ── Per-event state machine ───────────────────────────────────────────────────

struct RaiserState {
    config: Arc<Mutex<Config>>,
    aerospace: Option<AeroSpaceState>,
    poll_millis: u64,

    // Raise delay tracking
    last_window_pid: Option<i32>,
    hover_ticks: u32,       // how many ticks mouse has been over same window
    still_ticks: u32,       // how many ticks since last significant movement
    last_x: f64,
    last_y: f64,
    aerospace_cycle_counter: u32,
    aerospace_refresh_cycles: u32,
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
            hover_ticks: 0,
            still_ticks: 0,
            last_x: -1.0,
            last_y: -1.0,
            aerospace_cycle_counter: 0,
            aerospace_refresh_cycles: aerospace_cycles,
        }
    }

    fn handle_mouse_event(&mut self, ev: MouseEvent) {
        let (x, y) = (ev.x, ev.y);
        let moved = (x - self.last_x).abs() > 0.5 || (y - self.last_y).abs() > 0.5;

        // Periodic AeroSpace refresh
        self.aerospace_cycle_counter += 1;
        if self.aerospace_cycle_counter >= self.aerospace_refresh_cycles {
            self.aerospace_cycle_counter = 0;
            if let Some(ref mut as_state) = self.aerospace {
                as_state.refresh_if_due();
            }
        }

        let cfg = self.config.lock().unwrap().clone();

        // ── 1. Modifier key disable check ─────────────────────────────────────
        if self.modifier_key_held(&cfg.disable_key) {
            debug!("Disabled by modifier key");
            return;
        }

        // ── 2. Find window under cursor ───────────────────────────────────────
        let win = match window_at_point(x, y) {
            Some(w) => w,
            None => {
                self.last_window_pid = None;
                self.hover_ticks = 0;
                self.last_x = x;
                self.last_y = y;
                return;
            }
        };

        debug!(
            "Window under cursor: pid={} app='{}' layer={}",
            win.pid, win.app_name, win.layer
        );

        // ── 3. Skip non-normal windows (panels, menus, overlays) ─────────────
        if win.layer != 0 {
            // Layer 0 = normal windows. Positive = panels/menus/HUD.
            // We never raise anything that isn't a normal window.
            return;
        }

        // ── 4. Ignore list checks ─────────────────────────────────────────────
        let app_lower = win.app_name.to_lowercase();
        if cfg.ignore_apps.iter().any(|a| a.to_lowercase() == app_lower) {
            debug!("Ignoring app: {}", win.app_name);
            return;
        }
        if !cfg.ignore_titles.is_empty() {
            if let Some(title) = accessibility::get_window_title(win.pid) {
                let title_lower = title.to_lowercase();
                if cfg.ignore_titles.iter().any(|t| title_lower.contains(&t.to_lowercase())) {
                    debug!("Ignoring title: {title}");
                    return;
                }
            }
        }

        // ── 5. AeroSpace floating check ───────────────────────────────────────
        if let Some(ref as_state) = self.aerospace {
            if as_state.available {
                // Get AX window ID for this PID
                if let Some(ax_id) = accessibility::get_ax_window_id(win.pid) {
                    if !as_state.should_raise(ax_id) {
                        debug!(
                            "Skipping tiled window: app='{}' ax_id={ax_id} — AeroSpace manages focus",
                            win.app_name
                        );
                        // Reset hover so we don't immediately raise if it becomes floating
                        if self.last_window_pid != Some(win.pid) {
                            self.hover_ticks = 0;
                        }
                        self.last_window_pid = Some(win.pid);
                        self.last_x = x;
                        self.last_y = y;
                        return;
                    }
                } else {
                    // Can't get AX ID — could be a non-AeroSpace window, raise it
                    debug!("Could not get AX window ID for pid={}; raising anyway", win.pid);
                }
            }
        }

        // ── 6. Delay / hover tick logic ───────────────────────────────────────
        let new_window = self.last_window_pid != Some(win.pid);
        if new_window {
            self.hover_ticks = 0;
            self.still_ticks = 0;
        }
        self.last_window_pid = Some(win.pid);
        self.last_x = x;
        self.last_y = y;

        if moved {
            self.still_ticks = 0;
        } else {
            self.still_ticks += 1;
        }

        // require_mouse_stop: with delay > 1, mouse must stop (still_ticks >= delay)
        let delay = cfg.delay;
        if delay == 0 {
            return; // raising disabled
        }
        if delay == 1 {
            // Instant raise: raise on first tick over a new window
            if !new_window { return; }
        } else {
            // Delayed raise: mouse must be still for `delay` ticks
            if cfg.require_mouse_stop && self.still_ticks < delay {
                return;
            }
        }

        self.hover_ticks += 1;
        if self.hover_ticks > 1 {
            return; // already raised this window
        }

        // ── 7. Raise! ─────────────────────────────────────────────────────────
        debug!("Raising: app='{}' pid={}", win.app_name, win.pid);
        self.do_raise(win.pid);
    }

    fn do_raise(&self, pid: i32) {
        // AXRaise the window
        accessibility::raise_app_window(pid, None);

        // Activate the application (NSRunningApplication)
        unsafe {
            let cls = Class::get("NSRunningApplication").expect("NSRunningApplication missing");
            let apps: *mut Object = msg_send![cls,
                runningApplicationsWithBundleIdentifier: pid
            ];
            // Use runningApplicationWithProcessIdentifier:
            let app: *mut Object = msg_send![cls,
                runningApplicationWithProcessIdentifier: pid as i32
            ];
            if !app.is_null() {
                // NSApplicationActivationPolicyRegular = 0
                // activateWithOptions: NSApplicationActivateIgnoringOtherApps = 2
                let _: bool = msg_send![app, activateWithOptions: 2u64];
            }
        }
    }

    fn modifier_key_held(&self, key: &str) -> bool {
        if key == "disabled" { return false; }
        unsafe {
            let flags = CGEventSourceFlagsState(kCGEventSourceStateCombinedSessionState);
            match key {
                "control" => (flags & kCGEventFlagMaskControl2) != 0,
                "option"  => (flags & kCGEventFlagMaskAlternate) != 0,
                _         => false,
            }
        }
    }
}

// ── Window info from CGWindowListCopyWindowInfo ───────────────────────────────

#[derive(Debug)]
struct WindowInfo {
    pid: i32,
    app_name: String,
    window_id: u32,
    layer: i32,
}

/// Find the topmost on-screen normal window at screen point (x, y).
/// We iterate the window list (which is in Z order, frontmost first)
/// and pick the first window whose bounds contain (x, y).
fn window_at_point(x: f64, y: f64) -> Option<WindowInfo> {
    unsafe {
        let list = CGWindowListCopyWindowInfo(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            kCGNullWindowID,
        );
        if list.is_null() { return None; }

        let arr = CFArray::<CFDictionary>::wrap_under_create_rule(list as *mut _);
        let count = arr.len();

        for i in 0..count {
            let dict_ref = arr.get(i as isize);
            // dict_ref is a *const CFDictionary, cast to CFDictionaryRef
            let dict_ptr = dict_ref as *const _ as CFDictionaryRef;
            let dict = CFDictionary::<CFString, CFTypeRef>::wrap_under_get_rule(dict_ptr);

            // Layer
            let layer: i32 = get_dict_int(&dict, kCGWindowLayer).unwrap_or(999) as i32;

            // Bounds dict
            let bounds = match get_dict_bounds(&dict, kCGWindowBounds) {
                Some(b) => b,
                None => continue,
            };

            // Point-in-bounds test
            if x < bounds.0 || y < bounds.1
                || x > bounds.0 + bounds.2
                || y > bounds.1 + bounds.3
            {
                continue;
            }

            let pid = get_dict_int(&dict, kCGWindowOwnerPID).unwrap_or(0) as i32;
            if pid == 0 { continue; }

            let app_name = get_dict_string(&dict, kCGWindowOwnerName)
                .unwrap_or_else(|| "Unknown".to_string());
            let window_id = get_dict_int(&dict, kCGWindowNumber).unwrap_or(0) as u32;

            return Some(WindowInfo { pid, app_name, window_id, layer });
        }

        None
    }
}

// ── CFDictionary helpers ──────────────────────────────────────────────────────

unsafe fn get_dict_int(
    dict: &CFDictionary<CFString, CFTypeRef>,
    key: &str,
) -> Option<i64> {
    let k = CFString::new(key);
    let val_ptr = dict.find(k.as_concrete_TypeRef())?;
    let num = CFNumber::wrap_under_get_rule(*val_ptr as CFNumberRef);
    num.to_i64()
}

unsafe fn get_dict_string(
    dict: &CFDictionary<CFString, CFTypeRef>,
    key: &str,
) -> Option<String> {
    let k = CFString::new(key);
    let val_ptr = dict.find(k.as_concrete_TypeRef())?;
    let s = CFString::wrap_under_get_rule(*val_ptr as CFStringRef);
    Some(s.to_string())
}

/// Returns (x, y, width, height) from the CGWindowBounds sub-dictionary.
unsafe fn get_dict_bounds(
    dict: &CFDictionary<CFString, CFTypeRef>,
    key: &str,
) -> Option<(f64, f64, f64, f64)> {
    let k = CFString::new(key);
    let val_ptr = dict.find(k.as_concrete_TypeRef())?;
    let sub = CFDictionary::<CFString, CFTypeRef>::wrap_under_get_rule(
        *val_ptr as CFDictionaryRef
    );
    let x = get_dict_int(&sub, "X").unwrap_or(0) as f64;
    let y = get_dict_int(&sub, "Y").unwrap_or(0) as f64;
    let w = get_dict_int(&sub, "Width").unwrap_or(0) as f64;
    let h = get_dict_int(&sub, "Height").unwrap_or(0) as f64;
    Some((x, y, w, h))
}
