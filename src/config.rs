// src/config.rs

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_poll_millis")]
    pub poll_millis: u64,

    #[serde(default = "default_delay")]
    pub delay: u32,

    #[serde(default)]
    pub ignore_apps: Vec<String>,

    #[serde(default)]
    pub ignore_titles: Vec<String>,

    #[serde(default = "default_disable_key")]
    pub disable_key: String,

    #[serde(default = "default_true")]
    pub require_mouse_stop: bool,

    #[serde(default = "default_true")]
    pub aerospace_aware: bool,

    #[serde(default = "default_aerospace_refresh")]
    pub aerospace_refresh_cycles: u32,

    /// Show a border highlight around the raised window
    #[serde(default = "default_true")]
    pub show_border: bool,

    /// Border width in points (only used when show_border = true)
    #[serde(default = "default_border_width")]
    pub border_width: f64,

    /// Border color as hex string e.g. "#FF3366"
    #[serde(default = "default_border_color")]
    pub border_color: String,
    
    /// Shell command run when focus moves to a different app.
    /// Placeholders: {app_id} {app_name}. Env: AUTORAISE_APP_ID etc.
    #[serde(default)]
    pub on_focus_command: Option<String>,
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
            show_border: true,
            border_width: default_border_width(),
            border_color: default_border_color(),
            on_focus_command: None,
        }
    }
}

impl Config {
    pub fn load_or_default() -> Self {
        // Dev build (`cargo run`): prefer <project>/dev_config.toml
        if cfg!(debug_assertions) {
            let dev = dev_config_path();
            eprintln!("[config] dev build — looking for {}", dev.display());
            if dev.exists() {
                return Self::load_toml(&dev);
            }
            eprintln!("[config] dev_config.toml not found — falling back to ~/.config");
        }

        match config_path() {
            Some(path) => {
                eprintln!("[config] Looking for config at: {}", path.display());
                if path.exists() {
                    Self::load_toml(&path)
                } else {
                    eprintln!("[config] File not found — using defaults");
                    Config::default()
                }
            }
            None => {
                eprintln!("[config] Could not determine config directory");
                Config::default()
            }
        }
    }

    fn load_toml(path: &std::path::Path) -> Self {
        match fs::read_to_string(path) {
            Ok(content) => match toml::from_str::<Config>(&content) {
                Ok(cfg) => {
                    eprintln!(
                        "[config] Loaded {} OK — poll_millis={} delay={} aerospace_aware={}",
                        path.display(), cfg.poll_millis, cfg.delay, cfg.aerospace_aware
                    );
                    cfg
                }
                Err(e) => {
                    eprintln!("[config] Parse error in {}: {e}", path.display());
                    Config::default()
                }
            },
            Err(e) => {
                eprintln!("[config] Read error for {}: {e}", path.display());
                Config::default()
            }
        }
    }
}

/// <project root>/dev_config.toml. The path is baked in at compile time,
/// so it works no matter which directory you run `cargo run` from.
fn dev_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dev_config.toml")
}

fn config_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config").join("autoraise-rs").join("config.toml"))
}

