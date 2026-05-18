// raiser.rs — Core auto-raise logic

#![allow(non_upper_case_globals, non_snake_case)]

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use log::{debug, info};
use core_foundation::runloop::CFRunLoop;
use core_foundation::base::{CFTypeRef, TCFType};
use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::number::{CFNumber, CFNumberRef};

use crate::config::Config;
use crate::aerospace::AeroSpaceState;
use crate::event_tap::{self, MouseEvent};
use crate::accessibility;

// ── CGWindowListCopyWindowInfo ────────────────────────────────────────────────

type CGWindowID = u32;
const kCGWindowListOptionOnScreenOnly: u32     = 1 << 0;
const kCGWindowListExcludeDesktopElements: u32 = 1 << 4;
const kCGNullWindowID: CGWindowID = 0;

extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relativeToWindow: CGWindowID) -> CFArrayRef;
}

// ── Modifier key state ────────────────────────────────────────────────────────

extern "C" {
    fn CGEventSourceFlagsState(stateID: u32) -> u64;
}
const kCGEventSourceStateCombinedSessionState: u32 = 1;
const kCGEventFlagMaskAlternate: u64 = 0x0008_0000;
const kCGEventFlagMaskControl:   u64 = 0x0004_0000;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {}

// ── NSRunningApplication ──────────────────────────────────────────────────────

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

    pub fn run(self) {
        let cfg = self.config.lock().unwrap().clone();
        let poll_millis      = cfg.poll_millis;
        let aerospace_cycles = cfg.aerospace_refresh_cycles;
        let aerospace_aware  = cfg.aerospace_aware;
        drop(cfg);

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
        unsafe { CFRunLoop::run_current() };
    }
}

// ── State machine ─────────────────────────────────────────────────────────────

struct RaiserState {
    config: Arc<Mutex<Config>>,
    aerospace: Option<AeroSpaceState>,
    poll_millis: u64,
    last_window_pid: Option<i32>,
    hover_ticks: u32,
    still_ticks: u32,
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

        self.aerospace_cycle_counter += 1;
        if self.aerospace_cycle_counter >= self.aerospace_refresh_cycles {
            self.aerospace_cycle_counter = 0;
            if let Some(ref mut s) = self.aerospace {
                s.refresh_if_due();
            }
        }

        let cfg = self.config.lock().unwrap().clone();

        // 1. Modifier key
        if self.modifier_key_held(&cfg.disable_key) {
            debug!("Disabled by modifier key");
            return;
        }

        // 2. Window under cursor
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

        debug!("Window: pid={} app='{}' layer={}", win.pid, win.app_name, win.layer);

        // 3. Skip panels/menus (layer != 0)
        if win.layer != 0 { return; }

        // 4. Ignore lists
        let app_lower = win.app_name.to_lowercase();
        if cfg.ignore_apps.iter().any(|a| a.to_lowercase() == app_lower) {
            debug!("Ignoring app: {}", win.app_name);
            return;
        }
        if !cfg.ignore_titles.is_empty() {
            if let Some(title) = accessibility::get_window_title(win.pid) {
                let tl = title.to_lowercase();
                if cfg.ignore_titles.iter().any(|t| tl.contains(&t.to_lowercase())) {
                    debug!("Ignoring title: {title}");
                    return;
                }
            }
        }

        // 5. AeroSpace: skip tiled windows
        if let Some(ref as_state) = self.aerospace {
            if as_state.available {
                if let Some(ax_id) = accessibility::get_ax_window_id(win.pid) {
                    if !as_state.should_raise(ax_id) {
                        debug!("Skipping tiled: app='{}' ax_id={ax_id}", win.app_name);
                        if self.last_window_pid != Some(win.pid) {
                            self.hover_ticks = 0;
                        }
                        self.last_window_pid = Some(win.pid);
                        self.last_x = x;
                        self.last_y = y;
                        return;
                    }
                }
            }
        }

        // 6. Delay logic
        let new_window = self.last_window_pid != Some(win.pid);
        if new_window { self.hover_ticks = 0; self.still_ticks = 0; }
        self.last_window_pid = Some(win.pid);
        self.last_x = x;
        self.last_y = y;

        if moved { self.still_ticks = 0; } else { self.still_ticks += 1; }

        let delay = cfg.delay;
        if delay == 0 { return; }
        if delay == 1 {
            if !new_window { return; }
        } else if cfg.require_mouse_stop && self.still_ticks < delay {
            return;
        }

        self.hover_ticks += 1;
        if self.hover_ticks > 1 { return; }

        // 7. Raise
        debug!("Raising: app='{}' pid={}", win.app_name, win.pid);
        self.do_raise(win.pid);
    }

    fn do_raise(&self, pid: i32) {
        accessibility::raise_app_window(pid, None);
        unsafe {
            let cls = match Class::get("NSRunningApplication") {
                Some(c) => c,
                None    => return,
            };
            let app: *mut Object = msg_send![cls,
                runningApplicationWithProcessIdentifier: pid as i32
            ];
            if !app.is_null() {
                let _: bool = msg_send![app, activateWithOptions: 2u64];
            }
        }
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

// ── Window hit-test ───────────────────────────────────────────────────────────

#[derive(Debug)]
struct WindowInfo {
    pid:       i32,
    app_name:  String,
    window_id: u32,
    layer:     i32,
}

fn window_at_point(x: f64, y: f64) -> Option<WindowInfo> {
    unsafe {
        let list = CGWindowListCopyWindowInfo(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            kCGNullWindowID,
        );
        if list.is_null() { return None; }

        let arr = CFArray::<CFDictionary>::wrap_under_create_rule(list as *mut _);

        for i in 0..arr.len() {
            let dict_ptr = &arr.get(i as isize) as *const _ as CFDictionaryRef;
            let dict = CFDictionary::<CFString, CFTypeRef>::wrap_under_get_rule(dict_ptr);

            let layer  = get_dict_int(&dict, kCGWindowLayer).unwrap_or(999) as i32;
            let bounds = match get_dict_bounds(&dict, kCGWindowBounds) {
                Some(b) => b,
                None    => continue,
            };

            if x < bounds.0 || y < bounds.1
                || x > bounds.0 + bounds.2
                || y > bounds.1 + bounds.3
            {
                continue;
            }

            let pid = get_dict_int(&dict, kCGWindowOwnerPID).unwrap_or(0) as i32;
            if pid == 0 { continue; }

            let app_name  = get_dict_string(&dict, kCGWindowOwnerName)
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
    CFNumber::wrap_under_get_rule(*val_ptr as CFNumberRef).to_i64()
}

unsafe fn get_dict_string(
    dict: &CFDictionary<CFString, CFTypeRef>,
    key: &str,
) -> Option<String> {
    let k = CFString::new(key);
    let val_ptr = dict.find(k.as_concrete_TypeRef())?;
    Some(CFString::wrap_under_get_rule(*val_ptr as CFStringRef).to_string())
}

unsafe fn get_dict_bounds(
    dict: &CFDictionary<CFString, CFTypeRef>,
    key: &str,
) -> Option<(f64, f64, f64, f64)> {
    let k = CFString::new(key);
    let val_ptr = dict.find(k.as_concrete_TypeRef())?;
    let sub = CFDictionary::<CFString, CFTypeRef>::wrap_under_get_rule(
        *val_ptr as CFDictionaryRef,
    );
    let x = get_dict_int(&sub, "X").unwrap_or(0) as f64;
    let y = get_dict_int(&sub, "Y").unwrap_or(0) as f64;
    let w = get_dict_int(&sub, "Width").unwrap_or(0) as f64;
    let h = get_dict_int(&sub, "Height").unwrap_or(0) as f64;
    Some((x, y, w, h))
}
