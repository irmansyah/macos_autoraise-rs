// aerospace.rs — AeroSpace tiling WM integration
//
// AeroSpace exposes a CLI: `aerospace list-windows --all`
// with --format '%{window-id} %{parent-container-layout} %{app-name} %{window-title}'
//
// A window whose parent-container-layout == "floating" is a FLOATING window.
// These are the only ones we should auto-raise; tiled windows are managed
// by AeroSpace's own focus system (hjkl navigation).
//
// We also track which workspace each window belongs to, plus the currently
// focused workspace, so that hovering a tiled window on a non-focused
// workspace (e.g. visible on a different monitor) does not steal focus.
//
// Strategy:
//   - Every N poll cycles, refresh the set of floating window IDs and the
//     window-id -> workspace map, plus the currently focused workspace.
//   - On each raise decision, check if the candidate window ID is floating,
//     and whether it belongs to the focused workspace.
//   - If aerospace is not installed/running, fall back to raising everything.

use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::time::{Duration, Instant};
use log::{debug, warn};

pub struct AeroSpaceState {
    /// Window IDs (AX window IDs) that are currently floating
    pub floating_window_ids: HashSet<u32>,
    /// Window ID -> workspace name, for every window AeroSpace knows about
    pub window_workspace: HashMap<u32, String>,
    /// The name of the currently focused AeroSpace workspace, if known
    pub current_workspace: Option<String>,
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
            window_workspace: HashMap::new(),
            current_workspace: None,
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

    /// Refresh floating window set, window->workspace map, and focused
    /// workspace from AeroSpace.
    ///
    /// Uses two queries when possible — compatible with all AeroSpace versions:
    ///   1. Get ALL window IDs tagged with their workspace
    ///   2. Get only TILED window IDs (those on real workspaces)
    ///   3. Floating = All - Tiled
    ///
    /// Falls back to enumerating windows per-workspace if `--format` doesn't
    /// support the workspace token or `--filter-tiling-windows` is unknown.
    pub fn refresh(&mut self) {
        self.last_refresh = Instant::now();
        self.current_workspace = self.query_focused_workspace();

        // Query 1: all window IDs, tagged with their workspace
        let all_map = self.query_window_workspace_map();

        // Query 2: tiled window IDs (only windows in real workspaces, not floating)
        let tiled = self.query_window_ids(&[
            "list-windows", "--all",
            "--format", "%{window-id}",
            "--filter-tiling-windows",
        ]);

        let (all_ids, tiled_ids, window_workspace): (HashSet<u32>, HashSet<u32>, HashMap<u32, String>) =
            match (all_map, tiled) {
                (Some(map), Some(t)) => {
                    let ids = map.keys().copied().collect();
                    (ids, t, map)
                }
                (Some(map), None) => {
                    // --filter-tiling-windows unsupported — derive tiled set
                    // from the per-workspace enumeration instead.
                    let tiled_fb = self.query_windows_via_workspace_enum();
                    let tiled_ids_fb = tiled_fb.keys().copied().collect();
                    let ids = map.keys().copied().collect();
                    (ids, tiled_ids_fb, map)
                }
                (None, maybe_tiled) => {
                    // Workspace-tagged format unsupported — fall back to
                    // enumerating windows per real workspace for the map too.
                    let map_fb = self.query_windows_via_workspace_enum();
                    let all = self.query_window_ids(&["list-windows", "--all", "--format", "%{window-id}"]);
                    let all_ids = match all {
                        Some(a) => a,
                        None => {
                            warn!("AeroSpace queries failed — treating all windows as tiled");
                            self.floating_window_ids.clear();
                            self.window_workspace.clear();
                            return;
                        }
                    };
                    let tiled_ids = match maybe_tiled {
                        Some(t) => t,
                        None => map_fb.keys().copied().collect(),
                    };
                    (all_ids, tiled_ids, map_fb)
                }
            };

        let floating: HashSet<u32> = all_ids.difference(&tiled_ids).copied().collect();
        debug!(
            "AeroSpace: {} total, {} tiled, {} floating, focused-workspace={:?}",
            all_ids.len(), tiled_ids.len(), floating.len(), self.current_workspace
        );
        self.floating_window_ids = floating;
        self.window_workspace = window_workspace;
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

    /// Query every window's ID tagged with its workspace name in one call.
    /// Returns None if the format token isn't supported by this AeroSpace
    /// version, so the caller can fall back to per-workspace enumeration.
    fn query_window_workspace_map(&self) -> Option<HashMap<u32, String>> {
        let out = Command::new("aerospace")
            .args(["list-windows", "--all", "--format", "%{window-id}\t%{workspace}"])
            .output().ok()?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("Unknown") || stderr.contains("unrecognized") {
                return None;
            }
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut map = HashMap::new();
        for line in text.lines() {
            let mut parts = line.splitn(2, '\t');
            let id_part = match parts.next() { Some(p) => p, None => continue };
            let ws_part = parts.next().unwrap_or("").trim();
            if ws_part.is_empty() { continue; }
            if let Ok(id) = id_part.trim().parse::<u32>() {
                map.insert(id, ws_part.to_string());
            }
        }
        Some(map)
    }

    /// Query which workspace currently has focus.
    fn query_focused_workspace(&self) -> Option<String> {
        let out = Command::new("aerospace")
            .args(["list-workspaces", "--focused"])
            .output().ok()?;
        if !out.status.success() { return None; }
        let text = String::from_utf8_lossy(&out.stdout);
        let name = text.lines().next()?.trim().to_string();
        if name.is_empty() { None } else { Some(name) }
    }

    /// Fallback: enumerate windows per real workspace, building a
    /// window-id -> workspace map. Floating windows in some AeroSpace
    /// versions report a pseudo-workspace ("_") and so won't appear here —
    /// that's fine, since floating windows aren't gated on workspace match.
    fn query_windows_via_workspace_enum(&self) -> HashMap<u32, String> {
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
            _ => return HashMap::new(),
        };

        let mut map = HashMap::new();
        for ws in &ws_names {
            let out = Command::new("aerospace")
                .args(["list-windows", "--workspace", ws, "--format", "%{window-id}"])
                .output();
            if let Ok(o) = out {
                let text = String::from_utf8_lossy(&o.stdout);
                for line in text.lines() {
                    if let Ok(id) = line.trim().parse::<u32>() {
                        map.insert(id, ws.clone());
                    }
                }
            }
        }
        debug!("AeroSpace fallback: {} windows across {} workspaces", map.len(), ws_names.len());
        map
    }

    /// True if `window_id` is on the currently focused workspace, or if we
    /// don't have enough information to say otherwise (fail-open, matching
    /// the existing "raise everything if AeroSpace unavailable" behavior).
    pub fn window_matches_current_workspace(&self, window_id: u32) -> bool {
        if !self.available { return true; }
        match (&self.current_workspace, self.window_workspace.get(&window_id)) {
            (Some(cur), Some(ws)) => cur == ws,
            _ => true,
        }
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
