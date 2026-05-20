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
    /// Uses two queries to find floating windows — compatible with all AeroSpace versions:
    ///   1. Get ALL window IDs
    ///   2. Get only TILED window IDs (those on real workspaces)
    ///   3. Floating = All - Tiled
    pub fn refresh(&mut self) {
        self.last_refresh = Instant::now();

        // Query 1: all window IDs
        let all = self.query_window_ids(&[
            "list-windows", "--all",
            "--format", "%{window-id}",
        ]);

        // Query 2: tiled window IDs (only windows in real workspaces, not floating)
        let tiled = self.query_window_ids(&[
            "list-windows", "--all",
            "--format", "%{window-id}",
            "--filter-tiling-windows",
        ]);

        // If --filter-tiling-windows isn't supported either, fall back to
        // checking each window's workspace — floating windows show workspace "_"
        let (all_ids, tiled_ids) = match (all, tiled) {
            (Some(a), Some(t)) => (a, t),
            (Some(a), None)    => {
                // Fallback: use workspace query to find tiled windows
                let tiled_fb = self.query_tiled_via_workspace();
                (a, tiled_fb)
            }
            _ => {
                warn!("AeroSpace queries failed — treating all windows as tiled");
                self.floating_window_ids.clear();
                return;
            }
        };

        // Floating = windows in all_ids but NOT in tiled_ids
        let floating: HashSet<u32> = all_ids.difference(&tiled_ids).copied().collect();
        debug!("AeroSpace: {} total, {} tiled, {} floating",
            all_ids.len(), tiled_ids.len(), floating.len());
        self.floating_window_ids = floating;
    }

    fn query_window_ids(&self, args: &[&str]) -> Option<HashSet<u32>> {
        let out = Command::new("aerospace").args(args).output().ok()?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // Unknown flag → return None so caller can fall back
            if stderr.contains("Unknown") || stderr.contains("unrecognized") {
                return None;
            }
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let ids = text.lines()
            .filter_map(|l| l.trim().parse::<u32>().ok())
            .collect();
        Some(ids)
    }

    /// Fallback: get tiled window IDs by querying each real workspace.
    /// Floating windows in AeroSpace don't belong to any named workspace.
    fn query_tiled_via_workspace(&self) -> HashSet<u32> {
        // Get list of workspace names
        let ws_out = Command::new("aerospace")
            .args(["list-workspaces", "--all"])
            .output();
        let ws_names = match ws_out {
            Ok(o) if o.status.success() => {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
            }
            _ => return HashSet::new(),
        };

        let mut tiled = HashSet::new();
        for ws in &ws_names {
            let out = Command::new("aerospace")
                .args(["list-windows", "--workspace", ws, "--format", "%{window-id}"])
                .output();
            if let Ok(o) = out {
                let text = String::from_utf8_lossy(&o.stdout);
                for line in text.lines() {
                    if let Ok(id) = line.trim().parse::<u32>() {
                        tiled.insert(id);
                    }
                }
            }
        }
        debug!("AeroSpace fallback: {} tiled window IDs across {} workspaces",
            tiled.len(), ws_names.len());
        tiled
    }

    /// Refresh if the refresh interval has elapsed.
    pub fn refresh_if_due(&mut self) {
        if self.last_refresh.elapsed() >= self.refresh_interval {
            self.refresh();
        }
    }

    /// Force refresh on next refresh_if_due call (e.g. after workspace change).
    pub fn invalidate(&mut self) {
        self.last_refresh = Instant::now() - self.refresh_interval - Duration::from_secs(1);
    }
}
