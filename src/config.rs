// config.rs — Configuration loading from ~/.config/autoraise-rs/config.toml
// merged with CLI arguments.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use log::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Poll interval in milliseconds
    #[serde(default = "default_poll_millis")]
    pub poll_millis: u64,

    /// Raise delay in ticks (0 = off, 1 = instant, 2+ = stop required)
    #[serde(default = "default_delay")]
    pub delay: u32,

    /// App names to never auto-raise
    #[serde(default)]
    pub ignore_apps: Vec<String>,

    /// Window title substrings to never auto-raise
    #[serde(default)]
    pub ignore_titles: Vec<String>,

    /// Temporarily disable key: "control" | "option" | "disabled"
    #[serde(default = "default_disable_key")]
    pub disable_key: String,

    /// Require mouse to stop before raising
    #[serde(default = "default_true")]
    pub require_mouse_stop: bool,

    /// Skip tiled AeroSpace windows (only raise floating ones)
    #[serde(default = "default_true")]
    pub aerospace_aware: bool,

    /// Poll cycles between AeroSpace floating window list refresh
    #[serde(default = "default_aerospace_refresh")]
    pub aerospace_refresh_cycles: u32,

    /// Border width (0.0 means disabled)
    #[serde(default = "default_border_width")]
    pub border_width: f64,

    /// Border hex color
    #[serde(default = "default_border_color")]
    pub border_color: String,
}

fn default_poll_millis() -> u64 { 50 }
fn default_delay() -> u32 { 1 }
fn default_disable_key() -> String { "control".to_string() }
fn default_true() -> bool { true }
fn default_aerospace_refresh() -> u32 { 10 }
fn default_border_width() -> f64 { 4.0 }
fn default_border_color() -> String { "#FF3366".to_string() }

impl Default for Config {
    fn default() -> Self {
        Self {
            poll_millis: default_poll_millis(),
            delay: default_delay(),
            ignore_apps: vec![],
            ignore_titles: vec![],
            disable_key: default_disable_key(),
            require_mouse_stop: true,
            aerospace_aware: true,
            aerospace_refresh_cycles: default_aerospace_refresh(),
            border_width: default_border_width(),
            border_color: default_border_color(),
        }
    }
}

impl Config {
    /// Load from ~/.config/autoraise-rs/config.toml, or return defaults.
    pub fn load_or_default() -> Self {
        if let Some(path) = config_path() {
            if path.exists() {
                match fs::read_to_string(&path) {
                    Ok(content) => {
                        match toml::from_str::<Config>(&content) {
                            Ok(cfg) => {
                                info!("Loaded config from {:?}", path);
                                return cfg;
                            }
                            Err(e) => {
                                eprintln!("Warning: failed to parse config file: {e}. Using defaults.");
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Warning: could not read config file: {e}");
                    }
                }
            }
        }
        Config::default()
    }

    /// Write a sample config to the default location.
    pub fn write_sample() -> std::io::Result<()> {
        let sample = r#"# autoraise-rs configuration
# ~/.config/autoraise-rs/config.toml

# How often to poll mouse position (milliseconds, min 20)
poll_millis = 50

# Raise delay in poll ticks:
#   0 = disable raising entirely
#   1 = raise instantly on hover (no stop required)
#   2+ = mouse must stop for (delay * poll_millis) ms before raising
delay = 1

# Require mouse to stop before raising (delay > 1 enforces this automatically)
require_mouse_stop = true

# AeroSpace tiling WM integration:
#   true  = ONLY raise floating windows; skip tiled ones
#           (AeroSpace manages tiled focus itself via hjkl bindings)
#   false = raise all windows regardless of AeroSpace layout
aerospace_aware = true

# How many poll cycles between AeroSpace floating-window list refresh
# Lower = more responsive to layout changes, slightly more CPU
aerospace_refresh_cycles = 10

# Temporarily disable auto-raise while holding this key:
# "control" | "option" | "disabled"
disable_key = "control"

# App names to never auto-raise (exact match, case-insensitive)
ignore_apps = []
# Example:
# ignore_apps = ["Finder", "Activity Monitor", "System Preferences"]

# Window title substrings to ignore (case-insensitive contains)
ignore_titles = []
# Example:
# ignore_titles = ["Picture in Picture", "Quick Look"]
"#;
        let path = config_path().expect("Could not determine config path");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, sample)?;
        println!("Sample config written to {:?}", path);
        Ok(())
    }

    /// Override config fields with non-default CLI values.
    pub fn apply_cli(&mut self, cli: &crate::Cli) {
        if cli.poll_millis != 50 { self.poll_millis = cli.poll_millis; }
        if cli.delay != 1 { self.delay = cli.delay; }
        if cli.disable_key != "control" { self.disable_key = cli.disable_key.clone(); }
        if !cli.require_mouse_stop { self.require_mouse_stop = false; }
        if !cli.aerospace_aware { self.aerospace_aware = false; }
        if cli.aerospace_refresh_cycles != 10 {
            self.aerospace_refresh_cycles = cli.aerospace_refresh_cycles;
        }
        // Merge ignore lists (CLI additions, comma-separated)
        if !cli.ignore_apps.is_empty() {
            let extra: Vec<String> = cli.ignore_apps
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            self.ignore_apps.extend(extra);
        }
        if !cli.ignore_titles.is_empty() {
            let extra: Vec<String> = cli.ignore_titles
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            self.ignore_titles.extend(extra);
        }
        // Clamp poll_millis
        if self.poll_millis < 20 { self.poll_millis = 20; }
    }
}

fn config_path() -> Option<PathBuf> {
    dirs_next::config_dir().map(|d| d.join("autoraise-rs").join("config.toml"))
}
