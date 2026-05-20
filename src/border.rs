// border.rs — Transparent overlay window for drawing borders

#![allow(non_snake_case, non_upper_case_globals)]

use objc::{class, msg_send, sel, sel_impl};
use objc::runtime::{Object, YES, NO};
use std::sync::atomic::{AtomicPtr, Ordering};
const nil: *mut Object = std::ptr::null_mut();

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct NSPoint { x: f64, y: f64 }

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct NSSize { w: f64, h: f64 }

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct NSRect { origin: NSPoint, size: NSSize }

// Global pointer to our transparent border window
static BORDER_WINDOW: AtomicPtr<Object> = AtomicPtr::new(std::ptr::null_mut());

fn parse_hex_color(hex: &str) -> (f64, f64, f64) {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
        (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0)
    } else {
        (1.0, 0.0, 0.0)
    }
}

pub unsafe fn update_border(x: f64, y: f64, w: f64, h: f64, width: f64, color_hex: &str) {
    if width <= 0.0 {
        hide_border();
        return;
    }

    let ns_window_cls = class!(NSWindow);
    let ns_screen_cls = class!(NSScreen);
    let ns_color_cls = class!(NSColor);
    let ns_view_cls = class!(NSView);

    // Coordinate conversion: CGWindow uses top-left origin, NSWindow uses bottom-left
    let screens: *mut Object = msg_send![ns_screen_cls, screens];
    let primary_screen: *mut Object = msg_send![screens, objectAtIndex:0u64];
    let screen_frame: NSRect = msg_send![primary_screen, frame];
    let screen_h = screen_frame.size.h;

    let flipped_y = screen_h - y - h;
    let frame = NSRect {
        origin: NSPoint { x, y: flipped_y },
        size: NSSize { w, h }
    };

    let mut win = BORDER_WINDOW.load(Ordering::Relaxed);

    if win.is_null() {
        win = msg_send![ns_window_cls, alloc];
        win = msg_send![win, initWithContentRect:frame
                           styleMask:0u64
                           backing:2u64
                           defer:NO];

        let clear_color: *mut Object = msg_send![ns_color_cls, clearColor];
        let _: () = msg_send![win, setBackgroundColor:clear_color];
        let _: () = msg_send![win, setOpaque:NO];
        let _: () = msg_send![win, setHasShadow:NO];
        let _: () = msg_send![win, setIgnoresMouseEvents:YES];
        let _: () = msg_send![win, setLevel:3i32];

        let view: *mut Object = msg_send![ns_view_cls, alloc];
        let view: *mut Object = msg_send![view, initWithFrame:frame];
        let _: () = msg_send![view, setWantsLayer:YES];
        let _: () = msg_send![win, setContentView:view];

        BORDER_WINDOW.store(win, Ordering::Relaxed);
    } else {
        let _: () = msg_send![win, setFrame:frame display:YES];
    }

    let (r, g, b) = parse_hex_color(color_hex);
    let ns_col: *mut Object = msg_send![ns_color_cls,
        colorWithRed:r green:g blue:b alpha:1.0f64];
    let cg_col: *mut std::ffi::c_void = msg_send![ns_col, CGColor];

    let view: *mut Object = msg_send![win, contentView];
    let layer: *mut Object = msg_send![view, layer];

    let _: () = msg_send![layer, setBorderWidth: width];
    let _: () = msg_send![layer, setBorderColor: cg_col];
    let _: () = msg_send![layer, setCornerRadius: 10.0f64];

    let _: () = msg_send![win, orderFront: nil];
    BORDER_WINDOW.store(win, Ordering::Relaxed);
}

pub unsafe fn hide_border() {
    let win = BORDER_WINDOW.load(Ordering::Relaxed);
    if !win.is_null() {
        let _: () = msg_send![win, orderOut: nil];
    }
}
