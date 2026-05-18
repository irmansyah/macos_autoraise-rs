// aerospace.rs — AeroSpace tiling WM integration
//
// AeroSpace exposes a CLI: `aerospace list-windows --all`
// with --format '%{window-id} %{parent-container-layout} %{app-name} %{window-title}'
//
// A window whose parent-container-layout == "floating" is a FLOATING window.
// These are the only ones we should auto-raise; tiled windows are managed
// by AeroSpace's own focus system (hjkl navigation).
//
// Strategy:
//   - Every N poll cycles, refresh the set of floating window IDs.
//   - On each raise decision, check if the candidate window ID is in the set.
//   - If aerospace is not installed/running, fall back to raising everything.

use std::collections::HashSet;
use std::process::Command;
use std::time::{Duration, Instant};
use log::{debug, warn};

pub struct AeroSpaceState {
    /// Window IDs (AX window IDs) that are currently floating
    pub floating_window_ids: HashSet<u32>,
    /// Whether AeroSpace is available at all
    pub available: bool,
    last_refresh: Instant,
    refresh_interval: Duration,
}

impl AeroSpaceState {
    pub fn new(refresh_cycles: u32, poll_millis: u64) -> Self {
        let refresh_ms = refresh_cycles as u64 * poll_millis;
        let mut state = Self {
            floating_window_ids: HashSet::new(),
            available: false,
            last_refresh: Instant::now() - Duration::from_secs(60), // force first refresh
            refresh_interval: Duration::from_millis(refresh_ms),
        };
        state.probe_availability();
        state
    }

    /// Check if `aerospace` binary exists and is runnable.
    fn probe_availability(&mut self) {
        let result = Command::new("aerospace")
            .arg("version")
            .output();
        match result {
            Ok(out) if out.status.success() => {
                let version = String::from_utf8_lossy(&out.stdout);
                debug!("AeroSpace detected: {}", version.trim());
                self.available = true;
                self.refresh();
            }
            _ => {
                debug!("AeroSpace not detected — will raise all windows");
                self.available = false;
            }
        }
    }

    /// Refresh floating window set from `aerospace list-windows`.
    pub fn refresh(&mut self) {
        self.last_refresh = Instant::now();

        let output = Command::new("aerospace")
            .args([
                "list-windows",
                "--all",
                "--format",
                // window-id is AeroSpace's internal ID; we also fetch the
                // raw AX window ID via %{window-id} (same as what AXUI exposes).
                "%{window-id}\t%{parent-container-layout}\t%{app-name}",
            ])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                let mut new_set = HashSet::new();

                for line in text.lines() {
                    let parts: Vec<&str> = line.splitn(3, '\t').collect();
                    if parts.len() < 2 { continue; }

                    let id_str = parts[0].trim();
                    let layout = parts[1].trim();
                    let app = if parts.len() > 2 { parts[2].trim() } else { "" };

                    if layout == "floating" {
                        if let Ok(id) = id_str.parse::<u32>() {
                            debug!("AeroSpace float: id={id} app={app}");
                            new_set.insert(id);
                        }
                    }
                }

                debug!("AeroSpace: {} floating windows", new_set.len());
                self.floating_window_ids = new_set;
            }
            Ok(out) => {
                // AeroSpace returns non-zero when no windows exist — not a failure
                let stderr = String::from_utf8_lossy(&out.stderr);
                if !stderr.contains("no windows") {
                    warn!("aerospace list-windows failed: {}", stderr.trim());
                }
                self.floating_window_ids.clear();
            }
            Err(e) => {
                warn!("Failed to run aerospace: {e}");
                self.available = false;
            }
        }
    }

    /// Refresh if the refresh interval has elapsed.
    pub fn refresh_if_due(&mut self) {
        if self.last_refresh.elapsed() >= self.refresh_interval {
            self.refresh();
        }
    }

    /// Returns true if this window should be raised:
    /// - AeroSpace not available → always raise
    /// - Window is floating → raise
    /// - Window is tiled → skip (AeroSpace handles tiled focus)
    pub fn should_raise(&self, ax_window_id: u32) -> bool {
        if !self.available {
            return true; // AeroSpace not running, raise everything
        }
        self.floating_window_ids.contains(&ax_window_id)
    }

    /// Force an immediate refresh (e.g. after a layout change event).
    pub fn invalidate(&mut self) {
        self.last_refresh = Instant::now() - self.refresh_interval - Duration::from_secs(1);
    }
}
