// border.rs — Border overlay using CGS private API
//
// Instead of a persistent NSWindow (which bleeds across spaces/fullscreen),
// we use a per-draw CGS window that is created, drawn, and destroyed each time.
// This is exactly what tools like yabai and SketchyBar use for overlays.
//
// CGS (CoreGraphicsServer) is the private API that sits below NSWindow.
// It lets us create a window that is truly space-local and fullscreen-safe.

#![allow(non_snake_case, non_upper_case_globals, dead_code)]

use log::debug;
use std::sync::Mutex;

// ── CGS types ─────────────────────────────────────────────────────────────────

type CGSConnectionID = u32;
type CGSWindowID     = u32;
type CGSRegionRef    = *mut std::ffi::c_void;
type CGContextRef    = *mut std::ffi::c_void;
type CGColorSpaceRef = *mut std::ffi::c_void;
type CGColorRef      = *mut std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CGSRect { x: f64, y: f64, w: f64, h: f64 }

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint { x: f64, y: f64 }

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize  { width: f64, height: f64 }

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect  { origin: CGPoint, size: CGSize }

// CGS window tags
const kCGSOnAllWorkspaces:  u32 = 0; // do NOT use — bleeds everywhere
const kCGBackingStoreBuffered: u32 = 2;

// CGWindowLevel values
// kCGFloatingWindowLevelKey maps to level 3 which bleeds.
// Use kCGNormalWindowLevelKey + 1 = 1 to stay space-local.
const BORDER_WINDOW_LEVEL: i32 = 1;

extern "C" {
    fn CGSMainConnectionID() -> CGSConnectionID;

    fn CGSNewWindow(
        cid:         CGSConnectionID,
        backing:     u32,
        x:           f64,
        y:           f64,
        region:      CGSRegionRef,
        window_id:   *mut CGSWindowID,
    ) -> i32;

    fn CGSReleaseWindow(cid: CGSConnectionID, wid: CGSWindowID) -> i32;

    fn CGSSetWindowLevel(
        cid:   CGSConnectionID,
        wid:   CGSWindowID,
        level: i32,
    ) -> i32;

    fn CGSSetWindowOpacity(
        cid:     CGSConnectionID,
        wid:     CGSWindowID,
        opacity: bool,
    ) -> i32;

    fn CGSSetWindowAlpha(
        cid:   CGSConnectionID,
        wid:   CGSWindowID,
        alpha: f32,
    ) -> i32;

    fn CGSOrderWindow(
        cid:   CGSConnectionID,
        wid:   CGSWindowID,
        place: i32,    // 1 = above
        rel:   CGSWindowID, // 0 = frontmost
    ) -> i32;

    fn CGSNewRegionWithRect(rect: *const CGRect, region: *mut CGSRegionRef) -> i32;
    fn CGSReleaseRegion(region: CGSRegionRef);

    fn CGSAddActivationRegion(cid: CGSConnectionID, wid: CGSWindowID, region: CGSRegionRef) -> i32;

    fn CGWindowContextCreate(
        cid:     CGSConnectionID,
        wid:     CGSWindowID,
        options: *const std::ffi::c_void,
    ) -> CGContextRef;

    // CGContext drawing
    fn CGContextClearRect(ctx: CGContextRef, rect: CGRect);
    fn CGContextSetRGBStrokeColor(ctx: CGContextRef, r: f64, g: f64, b: f64, a: f64);
    fn CGContextSetLineWidth(ctx: CGContextRef, w: f64);
    fn CGContextStrokeRect(ctx: CGContextRef, rect: CGRect);
    fn CGContextFlush(ctx: CGContextRef);
    fn CGContextRelease(ctx: CGContextRef);

    // Screen height for coordinate flip
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGMainDisplayID() -> u32;

    // NSApp needed to init AppKit for NSWindow fallback
    fn CGSSetWindowTags(
        cid:  CGSConnectionID,
        wid:  CGSWindowID,
        tags: *const u32,
        tag_size: i32,
    ) -> i32;
}

#[link(name = "CoreGraphics", kind = "framework")] extern "C" {}
#[link(name = "ApplicationServices", kind = "framework")] extern "C" {}

// ── State ─────────────────────────────────────────────────────────────────────

struct BorderState {
    cid: CGSConnectionID,
    wid: Option<CGSWindowID>,
}

impl BorderState {
    fn new() -> Self {
        Self {
            cid: unsafe { CGSMainConnectionID() },
            wid: None,
        }
    }

    fn destroy_window(&mut self) {
        if let Some(wid) = self.wid.take() {
            unsafe { CGSReleaseWindow(self.cid, wid); }
        }
    }
}

static BORDER: Mutex<Option<BorderState>> = Mutex::new(None);

fn with_border<F: FnOnce(&mut BorderState)>(f: F) {
    let mut guard = BORDER.lock().unwrap();
    if guard.is_none() {
        *guard = Some(BorderState::new());
    }
    f(guard.as_mut().unwrap());
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Draw a border around the given CGWindow-coordinate rect.
/// x,y are top-left origin (CGWindow space); we flip to CG space internally.
pub unsafe fn update_border(x: f64, y: f64, w: f64, h: f64, border_width: f64, color_hex: &str) {
    if border_width <= 0.0 {
        hide_border();
        return;
    }

    let (r, g, b) = parse_hex_color(color_hex);

    // Flip Y: CGWindow uses top-left origin, CoreGraphics uses bottom-left
    let screen  = CGDisplayBounds(CGMainDisplayID());
    let screen_h = screen.size.height;
    let cg_y    = screen_h - y - h;

    with_border(|state| {
        // Always destroy and recreate — guarantees we're on the current space
        state.destroy_window();

        let cid = state.cid;

        // Create region for the window shape (full bounds)
        let region_rect = CGRect {
            origin: CGPoint { x, y: cg_y },
            size:   CGSize  { width: w, height: h },
        };
        let mut region: CGSRegionRef = std::ptr::null_mut();
        CGSNewRegionWithRect(&region_rect, &mut region);
        if region.is_null() { return; }

        // Create the CGS window
        let mut wid: CGSWindowID = 0;
        let err = CGSNewWindow(cid, kCGBackingStoreBuffered, x, cg_y, region, &mut wid);
        CGSReleaseRegion(region);

        if err != 0 || wid == 0 {
            debug!("CGSNewWindow failed: {err}");
            return;
        }

        // Level 1 = just above normal windows, does NOT cross into fullscreen spaces
        CGSSetWindowLevel(cid, wid, BORDER_WINDOW_LEVEL);

        // Transparent background
        CGSSetWindowOpacity(cid, wid, false);
        CGSSetWindowAlpha(cid, wid, 1.0);

        // Mouse clicks pass through — tag 0x02000000 = kCGSIgnoreMouseEvents
        let tags: [u32; 1] = [0x02000000];
        CGSSetWindowTags(cid, wid, tags.as_ptr(), 32);

        // Get drawing context
        let ctx = CGWindowContextCreate(cid, wid, std::ptr::null());
        if ctx.is_null() {
            CGSReleaseWindow(cid, wid);
            return;
        }

        // Draw the border
        let draw_rect = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size:   CGSize  { width: w, height: h },
        };

        CGContextClearRect(ctx, draw_rect);
        CGContextSetRGBStrokeColor(ctx, r, g, b, 1.0);
        CGContextSetLineWidth(ctx, border_width);

        // Inset by half border_width so stroke is fully inside the window bounds
        let inset = border_width / 2.0;
        let stroke_rect = CGRect {
            origin: CGPoint { x: inset,     y: inset },
            size:   CGSize  { width: w - border_width, height: h - border_width },
        };
        CGContextStrokeRect(ctx, stroke_rect);
        CGContextFlush(ctx);
        CGContextRelease(ctx);

        // Show above frontmost window
        CGSOrderWindow(cid, wid, 1, 0);

        state.wid = Some(wid);
        debug!("border drawn wid={wid} at ({x:.0},{cg_y:.0},{w:.0},{h:.0})");
    });
}

/// Hide and destroy the border window.
pub unsafe fn hide_border() {
    with_border(|state| {
        state.destroy_window();
    });
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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
