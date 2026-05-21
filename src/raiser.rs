// raiser.rs — Core auto-raise logic

#![allow(non_upper_case_globals, non_snake_case, unexpected_cfgs)]

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
#[link(name = "AppKit",       kind = "framework")] extern "C" {}

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

// ── Context-change notification ───────────────────────────────────────────────
// We listen on a Unix domain socket at ~/.cache/autoraise-rs/notify.sock.
// AeroSpace's exec-on-workspace-change fires:
//   echo -n x | nc -U ~/.cache/autoraise-rs/notify.sock
// CGDisplayRegisterReconfigurationCallback fires on monitor plug/unplug.
// Both instantly call context_changed() → hide border + reset raise state.

use std::sync::atomic::{AtomicBool, Ordering as AOrdering};

static CONTEXT_CHANGED: AtomicBool = AtomicBool::new(false);

/// Called from socket thread or CGDisplay callback — signals the raiser thread.
fn signal_context_changed() {
    CONTEXT_CHANGED.store(true, AOrdering::Relaxed);
}

/// Spawn a thread that listens on the Unix socket for workspace-change signals.
fn spawn_socket_listener() {
    use std::os::unix::net::UnixListener;
    use std::io::Read;

    let home = std::env::var("HOME").unwrap_or_default();
    let dir  = std::path::PathBuf::from(&home).join(".cache").join("autoraise-rs");
    let sock = dir.join("notify.sock");

    std::fs::create_dir_all(&dir).ok();
    std::fs::remove_file(&sock).ok(); // remove stale socket from last run

    thread::spawn(move || {
        let listener = match UnixListener::bind(&sock) {
            Ok(l)  => { info!("Listening on {:?}", sock); l }
            Err(e) => { warn!("Could not bind notify socket: {e}"); return; }
        };
        for stream in listener.incoming() {
            match stream {
                Ok(mut s) => {
                    let mut buf = [0u8; 1];
                    let _ = s.read(&mut buf);
                    debug!("workspace-change signal received");
                    signal_context_changed();
                }
                Err(_) => break,
            }
        }
    });
}

/// Register a CGDisplay reconfiguration callback for monitor plug/unplug.
fn register_display_callback() {
    extern "C" {
        fn CGDisplayRegisterReconfigurationCallback(
            callback: unsafe extern "C" fn(display: u32, flags: u32, user_info: *mut std::ffi::c_void),
            user_info: *mut std::ffi::c_void,
        ) -> i32;
    }
    unsafe extern "C" fn on_display_change(_: u32, _: u32, _: *mut std::ffi::c_void) {
        debug!("display reconfiguration detected");
        signal_context_changed();
    }
    unsafe { CGDisplayRegisterReconfigurationCallback(on_display_change, std::ptr::null_mut()); }
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

        // Start instant workspace/monitor change listeners
        spawn_socket_listener();
        register_display_callback();

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
                // Check for instant context-change signal first
                if CONTEXT_CHANGED.swap(false, AOrdering::Relaxed) {
                    debug!("context changed — hiding border, resetting raise state");
                    state.raised_pid      = None;
                    state.last_window_pid = None;
                    let cfg_c = state.config.lock().unwrap().clone();
                    if cfg_c.show_border {
                        Queue::main().exec_async(|| unsafe { crate::border::hide_border() });
                    }
                    // Also force AeroSpace floating list refresh
                    if let Some(ref mut s) = state.aerospace {
                        s.invalidate();
                    }
                }

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
    last_window_pid: Option<i32>,
    still_ticks: u32,
    last_x: f64,
    last_y: f64,
    raised_pid: Option<i32>,
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
            last_window_pid: None,
            still_ticks: 0,
            last_x: -9999.0,
            last_y: -9999.0,
            raised_pid: None,
            aerospace_cycle_counter: 0,
            aerospace_refresh_cycles: aerospace_cycles,
        }
    }

    fn handle_mouse_event(&mut self, ev: MouseEvent) {
        let (x, y) = (ev.x, ev.y);
        let moved = (x - self.last_x).abs() > 1.0 || (y - self.last_y).abs() > 1.0;
        self.last_x = x;
        self.last_y = y;

        // Periodic AeroSpace refresh
        self.aerospace_cycle_counter += 1;
        if self.aerospace_cycle_counter >= self.aerospace_refresh_cycles {
            self.aerospace_cycle_counter = 0;
            if let Some(ref mut s) = self.aerospace {
                s.refresh_if_due();
            }
        }

        let cfg = self.config.lock().unwrap().clone();

        // 1. Modifier key disable
        if self.modifier_key_held(&cfg.disable_key) {
            debug!("modifier held — skip");
            return;
        }

        // 2. Find window under cursor
        let win = match window_at_point(x, y) {
            Some(w) => w,
            None => {
                debug!("no window at ({x:.0},{y:.0})");
                self.last_window_pid = None;
                self.raised_pid = None;
                self.still_ticks = 0;
                // Hide border when mouse is over no window
                if cfg.show_border {
                    Queue::main().exec_async(|| unsafe { crate::border::hide_border() });
                }
                return;
            }
        };

        debug!("at ({x:.0},{y:.0}): app='{}' pid={} layer={}", win.app_name, win.pid, win.layer);

        // 3. Skip true system UI layers (menus=25, dock=20, screensaver=1000, etc.)
        // Normal windows = 0, AeroSpace floating windows may be 0 or 3 (NSFloatingWindowLevel)
        // We allow layers 0..=5 through; anything higher is system UI we don't touch.
        if win.layer > 5 {
            debug!("  → skip system layer {}", win.layer);
            return;
        }

        // 4. Ignore app list
        let app_lower = win.app_name.to_lowercase();
        if cfg.ignore_apps.iter().any(|a| a.to_lowercase() == app_lower) {
            debug!("  → ignored app '{}'", win.app_name);
            return;
        }

        // 5. Ignore title list
        if !cfg.ignore_titles.is_empty() {
            if let Some(title) = accessibility::get_window_title(win.pid) {
                let tl = title.to_lowercase();
                if cfg.ignore_titles.iter().any(|t| tl.contains(&t.to_lowercase())) {
                    debug!("  → ignored title '{title}'");
                    return;
                }
            }
        }

        // 6. AeroSpace awareness
        if let Some(ref as_state) = self.aerospace {
            if as_state.available {
                if let Some(ax_id) = accessibility::get_ax_window_id(win.pid) {
                    let is_floating = as_state.floating_window_ids.contains(&ax_id);

                    if is_floating {
                        debug!("  → floating: no raise, draw border only (bounds={:?})", win.bounds);
                        pin_floating_on_top(win.pid);

                        let new_float = self.last_window_pid != Some(win.pid);
                        self.last_window_pid = Some(win.pid);

                        if new_float && cfg.show_border {
                            let bounds = win.bounds;
                            let bwidth = cfg.border_width;
                            let bcolor = cfg.border_color.clone();
                            Queue::main().exec_async(move || unsafe {
                                crate::border::update_border(
                                    bounds.0, bounds.1, bounds.2, bounds.3,
                                    bwidth, &bcolor,
                                );
                            });
                        }
                        return;
                    }

                    // Tiled window → fall through to raise logic below
                    debug!("  → tiled: will raise on hover");
                }
            }
        }

        // 7. Detect window change
        let new_window = self.last_window_pid != Some(win.pid);
        if new_window {
            debug!("  → new window, resetting state");
            self.still_ticks = 0;
            self.raised_pid  = None;
        }
        self.last_window_pid = Some(win.pid);

        if moved { self.still_ticks = 0; } else { self.still_ticks += 1; }

        // 8. Already raised this window?
        if self.raised_pid == Some(win.pid) { return; }

        // 9. Delay gate
        let delay = cfg.delay;
        if delay == 0 { return; }
        if delay > 1 && cfg.require_mouse_stop && self.still_ticks < delay {
            debug!("  → waiting ({}/{})", self.still_ticks, delay);
            return;
        }

        // 10. Raise
        debug!("  → RAISING '{}' pid={} wid={}", win.app_name, win.pid, win.window_id);
        self.raised_pid = Some(win.pid);

        let bounds      = win.bounds;
        let window_id   = win.window_id;
        let show_border = cfg.show_border;
        let bwidth      = cfg.border_width;
        let bcolor      = cfg.border_color.clone();
        raise_on_main_thread(win.pid, window_id, bounds, show_border, bwidth, bcolor);
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

// ── Workspace / monitor helpers ───────────────────────────────────────────────

// ── Pin floating window above tiled layer ─────────────────────────────────────
// Sets NSWindowLevel to floating (3) so it always renders above normal (0) windows.
// Called every time mouse enters a floating window — idempotent, cheap.

fn pin_floating_on_top(pid: i32) {
    Queue::main().exec_async(move || {
        unsafe {
            let cls = match Class::get("NSRunningApplication") {
                Some(c) => c,
                None    => return,
            };
            let app: *mut Object = msg_send![cls,
                runningApplicationWithProcessIdentifier: pid as i32
            ];
            if app.is_null() { return; }

            // Get all windows for this app via AX and set their level
            let ax_app = crate::accessibility::ax_app_element(pid);
            if ax_app.is_null() { return; }

            crate::accessibility::set_windows_floating(ax_app);
        }
    });
}

// ── Raise tiled window on main thread ────────────────────────────────────────

fn raise_on_main_thread(
    pid: i32,
    window_id: u32,
    bounds: (f64, f64, f64, f64),
    show_border: bool,
    border_width: f64,
    border_color: String,
) {
    Queue::main().exec_async(move || {
        unsafe {
            do_raise(pid, window_id);
            if show_border {
                crate::border::update_border(
                    bounds.0, bounds.1, bounds.2, bounds.3,
                    border_width, &border_color,
                );
            }
        }
    });
}

unsafe fn do_raise(pid: i32, window_id: u32) {
    // Raise the EXACT window by CGWindowID — prevents pulling windows from other spaces
    accessibility::raise_app_window(pid, Some(window_id));

    // NSApplicationActivateAllWindows (1) = only activate windows on CURRENT space
    // Does NOT switch spaces or pull windows from other monitors
    let cls = match Class::get("NSRunningApplication") {
        Some(c) => c,
        None    => return,
    };
    let app: *mut Object = msg_send![cls,
        runningApplicationWithProcessIdentifier: pid as i32
    ];
    if !app.is_null() {
        let _: bool = msg_send![app, activateWithOptions: 1u64];
    }
}

// ── CoreFoundation Raw FFI ────────────────────────────────────────────────────
// CFArray uses CFTypeRef to match accessibility.rs and avoid clashing_extern_declarations

extern "C" {
    fn CFArrayGetCount(arr: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(arr: CFTypeRef, idx: isize) -> *mut std::ffi::c_void;
    fn CFDictionaryGetValue(theDict: CFDictionaryRef, key: CFTypeRef) -> CFTypeRef;
    fn CFNumberGetValue(number: CFTypeRef, theType: i64, valuePtr: *mut std::ffi::c_void) -> bool;
    fn CFRelease(cf: CFTypeRef);
}

// ── Window hit-test ───────────────────────────────────────────────────────────

#[derive(Debug)]
struct WindowInfo {
    pid:       i32,
    app_name:  String,
    layer:     i32,
    bounds:    (f64, f64, f64, f64),
    window_id: u32, // CGWindowID — used to target exact window on correct monitor
}

fn window_at_point(x: f64, y: f64) -> Option<WindowInfo> {
    unsafe {
        let list = CGWindowListCopyWindowInfo(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            kCGNullWindowID,
        );
        if list.is_null() {
            warn!("CGWindowListCopyWindowInfo returned NULL — grant Screen Recording permission");
            return None;
        }

        let count = CFArrayGetCount(list as CFTypeRef);

        for i in 0..count {
            let dict_ptr = CFArrayGetValueAtIndex(list as CFTypeRef, i) as CFDictionaryRef;
            if dict_ptr.is_null() { continue; }

            let layer  = get_dict_int(dict_ptr, kCGWindowLayer).unwrap_or(999) as i32;
            let bounds = match get_dict_bounds(dict_ptr, kCGWindowBounds) {
                Some(b) => b,
                None    => continue,
            };

            if x < bounds.0 || y < bounds.1
                || x > bounds.0 + bounds.2
                || y > bounds.1 + bounds.3
            {
                continue;
            }

            let pid = get_dict_int(dict_ptr, kCGWindowOwnerPID).unwrap_or(0) as i32;
            if pid == 0 { continue; }

            let app_name  = get_dict_string(dict_ptr, kCGWindowOwnerName)
                .unwrap_or_else(|| "Unknown".to_string());
            let window_id = get_dict_int(dict_ptr, kCGWindowNumber).unwrap_or(0) as u32;

            CFRelease(list as CFTypeRef);
            return Some(WindowInfo { pid, app_name, layer, bounds, window_id });
        }

        CFRelease(list as CFTypeRef);
        None
    }
}

// ── CFDictionary helpers ──────────────────────────────────────────────────────

unsafe fn get_dict_int(dict: CFDictionaryRef, key: &str) -> Option<i64> {
    let k = CFString::new(key);
    let val_ptr = CFDictionaryGetValue(dict, k.as_CFTypeRef());
    if val_ptr.is_null() { return None; }
    let mut val: i64 = 0;
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
    let sub = val_ptr as CFDictionaryRef;
    let x = get_dict_f64(sub, "X").unwrap_or(0.0);
    let y = get_dict_f64(sub, "Y").unwrap_or(0.0);
    let w = get_dict_f64(sub, "Width").unwrap_or(0.0);
    let h = get_dict_f64(sub, "Height").unwrap_or(0.0);
    if w == 0.0 || h == 0.0 { return None; }
    Some((x, y, w, h))
}
