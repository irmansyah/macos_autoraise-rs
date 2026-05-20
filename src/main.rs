#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]

mod accessibility;
mod aerospace;
mod border;
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
    /// Poll interval in milliseconds (min 20) — overrides config file
    #[arg(long)]
    poll_millis: Option<u64>,

    /// Raise delay in ticks (0=off, 1=instant, 2+=stop required) — overrides config file
    #[arg(long)]
    delay: Option<u32>,

    /// Comma-separated app names to ignore — appended to config file list
    #[arg(long)]
    ignore_apps: Option<String>,

    /// Comma-separated window title substrings to ignore — appended to config file list
    #[arg(long)]
    ignore_titles: Option<String>,

    /// Key that temporarily disables raising: control | option | disabled
    #[arg(long)]
    disable_key: Option<String>,

    /// AeroSpace integration on/off — overrides config file
    #[arg(long)]
    aerospace_aware: Option<bool>,

    /// Poll cycles between AeroSpace refresh — overrides config file
    #[arg(long)]
    aerospace_refresh_cycles: Option<u32>,

    /// Require mouse to stop before raising — overrides config file
    #[arg(long)]
    require_mouse_stop: Option<bool>,

    /// Verbose logging
    #[arg(long, default_value_t = false)]
    verbose: bool,
}

fn main() {
    let cli = Cli::parse();

    let log_level = if cli.verbose { "debug" } else { "info" };
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(log_level)
    ).init();

    // Load config file first, then selectively override with CLI args
    let mut cfg = Config::load_or_default();

    // Only override fields that were explicitly passed on the CLI
    if let Some(v) = cli.poll_millis            { cfg.poll_millis = v.max(20); }
    if let Some(v) = cli.delay                  { cfg.delay = v; }
    if let Some(v) = cli.disable_key            { cfg.disable_key = v; }
    if let Some(v) = cli.require_mouse_stop     { cfg.require_mouse_stop = v; }
    if let Some(v) = cli.aerospace_aware        { cfg.aerospace_aware = v; }
    if let Some(v) = cli.aerospace_refresh_cycles { cfg.aerospace_refresh_cycles = v; }

    // Ignore lists: CLI additions are appended to file config
    if let Some(apps) = cli.ignore_apps {
        let extra: Vec<String> = apps
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        cfg.ignore_apps.extend(extra);
    }
    if let Some(titles) = cli.ignore_titles {
        let extra: Vec<String> = titles
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        cfg.ignore_titles.extend(extra);
    }

    info!("autoraise-rs starting");
    info!("  poll_millis:         {}ms",  cfg.poll_millis);
    info!("  delay:               {} ticks", cfg.delay);
    info!("  require_mouse_stop:  {}",    cfg.require_mouse_stop);
    info!("  aerospace_aware:     {}",    cfg.aerospace_aware);
    info!("  disable_key:         {}",    cfg.disable_key);
    info!("  show_border:         {}",    cfg.show_border);
    if !cfg.ignore_apps.is_empty() {
        info!("  ignore_apps:         {:?}", cfg.ignore_apps);
    }

    if !accessibility::is_trusted() {
        warn!("Accessibility permissions NOT granted!");
        warn!("Open: System Settings → Privacy & Security → Accessibility");
        warn!("Add this binary and enable it, then re-run.");
        std::process::exit(1);
    }
    info!("Accessibility: granted ✓");

    let config = Arc::new(Mutex::new(cfg));
    let raiser = Raiser::new(config.clone());
    raiser.run()
}
