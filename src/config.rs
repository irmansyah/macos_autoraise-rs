// config.rs — Configuration loading from ~/.config/autoraise-rs/config.toml

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
        }
    }
}

impl Config {
    pub fn load_or_default() -> Self {
        let path = match config_path() {
            Some(p) => p,
            None => {
                eprintln!("[config] Could not determine config directory");
                return Config::default();
            }
        };

        eprintln!("[config] Looking for config at: {}", path.display());

        if !path.exists() {
            eprintln!("[config] File not found — using defaults");
            return Config::default();
        }

        match fs::read_to_string(&path) {
            Ok(content) => {
                eprintln!("[config] File found, parsing...");
                match toml::from_str::<Config>(&content) {
                    Ok(cfg) => {
                        eprintln!("[config] Loaded OK — poll_millis={} delay={} aerospace_aware={}",
                            cfg.poll_millis, cfg.delay, cfg.aerospace_aware);
                        cfg
                    }
                    Err(e) => {
                        eprintln!("[config] Parse error: {e}");
                        Config::default()
                    }
                }
            }
            Err(e) => {
                eprintln!("[config] Read error: {e}");
                Config::default()
            }
        }
    }

}

fn config_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config").join("autoraise-rs").join("config.toml"))
}
