// autoraise-rs — Focus-follows-mouse & auto-raise for macOS
// Written in Rust using raw macOS Accessibility + CGEvent APIs.
// AeroSpace-aware: tiled windows are EXCLUDED from auto-raise;
// only floating windows (or non-AeroSpace windows) are raised.
//
// Build:  cargo build --release
// Run:    ./target/release/autoraise-rs --help

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]

mod accessibility;
mod aerospace;
mod config;
mod event_tap;
mod raiser;

use std::sync::{Arc, Mutex};
use clap::Parser;
use log::{info, warn};

use config::Config;
use raiser::Raiser;

#[derive(Parser, Debug)]
#[command(name = "autoraise-rs", about = "Focus-follows-mouse for macOS (AeroSpace-aware)", version)]
struct Cli {
    /// Poll interval in milliseconds (min 20)
    #[arg(long, default_value_t = 50)]
    poll_millis: u64,

    /// Raise delay in poll ticks (0 = instant, 1 = no stop required, 2+ = must stop)
    #[arg(long, default_value_t = 1)]
    delay: u32,

    /// Comma-separated app names to ignore (e.g. "Finder,Activity Monitor")
    #[arg(long, default_value = "")]
    ignore_apps: String,

    /// Comma-separated window title substrings to ignore
    #[arg(long, default_value = "")]
    ignore_titles: String,

    /// Key that temporarily disables raising: control | option | disabled
    #[arg(long, default_value = "control")]
    disable_key: String,

    /// AeroSpace integration: skip tiled windows, only raise floating ones
    #[arg(long, default_value_t = true)]
    aerospace_aware: bool,

    /// How often (in poll cycles) to refresh AeroSpace floating-window list
    #[arg(long, default_value_t = 10)]
    aerospace_refresh_cycles: u32,

    /// Require mouse to stop moving before raising
    #[arg(long, default_value_t = true)]
    require_mouse_stop: bool,

    /// Verbose logging
    #[arg(long, default_value_t = false)]
    verbose: bool,
}

fn main() {
    let cli = Cli::parse();

    // Set up logging
    let log_level = if cli.verbose { "debug" } else { "info" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level)).init();

    // Merge CLI + config file (~/.config/autoraise-rs/config.toml)
    let mut cfg = Config::load_or_default();
    cfg.apply_cli(&cli);

    info!("autoraise-rs starting");
    info!("  poll_millis:         {}ms", cfg.poll_millis);
    info!("  delay:               {} ticks", cfg.delay);
    info!("  require_mouse_stop:  {}", cfg.require_mouse_stop);
    info!("  aerospace_aware:     {}", cfg.aerospace_aware);
    info!("  disable_key:         {}", cfg.disable_key);
    if !cfg.ignore_apps.is_empty() {
        info!("  ignore_apps:         {:?}", cfg.ignore_apps);
    }

    // Check Accessibility permissions
    if !accessibility::is_trusted() {
        warn!("Accessibility permissions NOT granted!");
        warn!("Open: System Settings → Privacy & Security → Accessibility");
        warn!("Add this binary and enable it, then re-run.");
        std::process::exit(1);
    }
    info!("Accessibility: granted ✓");

    let config = Arc::new(Mutex::new(cfg));
    let raiser = Raiser::new(config.clone());
    raiser.run(); // blocks forever on the run loop
}
