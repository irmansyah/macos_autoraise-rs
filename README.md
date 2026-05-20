# macos_autoraise-rs 🚀

[![Language](https://img.shields.io/badge/language-Rust-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-macOS-lightgrey.svg)](https://www.apple.com/macos/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A blazing-fast, lightweight background daemon written in pure Rust that brings **True Focus-Follows-Mouse (FFM)** functionality to macOS. Engineered specifically for keyboard-driven power users, tiling window manager enthusiasts, and minimalist dotfile architectures.

Unlike legacy utilities, `macos_autoraise-rs` is optimized out-of-the-box to work seamlessly with the modern **AeroSpace** tiling layout paradigm, featuring intelligent state filtering and hardware-accelerated focused window borders.

---

## ⚡ Key Features

* **Zero-Allocation Hot Path:** Legacy C++/Objective-C focus tools rely on heavy object pools or Cocoa reference-counting structures. This daemon intercepts hardware pointer events directly from the macOS Window Server via low-level C-FFI (`CGEventTap`). It processes coordinate maps inside an isolated real-time thread with **virtually zero transient heap allocations**, keeping your CPU footprint at a steady ~0%.
* **AeroSpace Native Intelligence:** Traditional FFM engines constantly fight tiling window trees—accidentally triggering focus loops, re-raising parent containers, or conflicting with keyboard layout shifts (`hjkl`). This tool runs a tight loop with the AeroSpace IPC interface, caching floating layers inside a high-speed hash map. Tiled structures are automatically bypassed, leaving focus to AeroSpace, while loose floating frames raise seamlessly.
* **Hardware-Accelerated Borders:** Draws a clean, configurable focused border highlight around the active window layer using modern AppKit `CALayer` structures. It instantiates directly within the event thread for instantaneous rendering without requiring external drawing packages.
* **Type-Safe Configuration:** Managed through a straightforward, easily scriptable `config.toml` that supports explicit application filters, custom padding, and temporary modifier-key overrides.

---

## ⚙️ Configuration (`config.toml`)

The daemon automatically looks for its configuration file at `~/.config/autoraise-rs/config.toml`. 

```toml
# Time interval in milliseconds to poll mouse position coordinates
poll_millis              = 50

# Deliberate delay multiplier before executing window raise actions
delay                    = 1

# Require the mouse pointer to physically come to a complete halt before raising
require_mouse_stop       = true

# Enable real-time integration with the AeroSpace window tree
aerospace_aware          = true
aerospace_refresh_cycles = 10

# Global modifier key override (e.g. "Option", "Control", "Command") to temporarily disable FFM
disable_key              = ""

# Native Focus Border Highlights
border_width             = 4.0
border_color             = "#FF3366" # Hexadecimal color format

# Application filters to completely exclude from focus shifts
ignore_apps              = [
  "1Password",
  "Notification Center",
  "Control Center",
  "Spotlight",
  "System Settings",
  "Raycast"
]
ignore_titles            = []

---

## 📦 Installation & Setup

### Prerequisites

You need the standard Rust toolchain installed on your Mac. If you don't have it, set it up via `rustup`:

```bash
curl --proto '=https' --tlsv1.2 -sSf [https://sh.rustup.rs](https://sh.rustup.rs) | sh

```

### 1. Clone & Compile

Clone the repository and build the production-optimized binary:

```bash
git clone [https://github.com/irmansyah/macos_autoraise-rs.git](https://github.com/irmansyah/macos_autoraise-rs.git)
cd macos_autoraise-rs
cargo build --release

```

### 2. Copy Binary and Configuration

Create your standard XDG configurations path and move the compiled executable to your local path:

```bash
# Install the binary
sudo cp target/release/autoraise-rs /usr/local/bin/

# Set up configuration directories
mkdir -p ~/.config/autoraise-rs/
cp config.toml ~/.config/autoraise-rs/config.toml

```

### 3. Configure Autostart Daemon via launchd

To have `macos_autoraise-rs` launch automatically in the background when your user profile logs in, register it as a native macOS User Agent.

Create a plist file at `~/Library/LaunchAgents/com.irmansyah.autoraise-rs.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "[http://www.apple.com/DTDs/PropertyList-1.0.dtd](http://www.apple.com/DTDs/PropertyList-1.0.dtd)">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.irmansyah.autoraise-rs</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/autoraise-rs</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ProcessType</key>
    <string>Background</string>
    <key>Nice</key>
    <integer>10</integer>
    <key>StandardOutPath</key>
    <string>/tmp/autoraise-rs.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/autoraise-rs.log</string>
</dict>
</plist>

```

Bootstrap and activate the background service manually for the first time:

```bash
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.irmansyah.autoraise-rs.plist
launchctl kickstart -k gui/$(id -u)/com.irmansyah.autoraise-rs

```

---

## 🔒 Security & Accessibility Permissions

Because this background process monitors hardware cursor inputs (`CGEventTap`) and interacts with core UI frameworks (`AXUIElementPerformAction`) to bring application windows to the absolute foreground, macOS handles it under strict security sandboxing.

1. Upon the first runtime initiation, macOS will present an **Accessibility Permissions Required** warning prompt.
2. Navigate to **System Settings ➔ Privacy & Security ➔ Accessibility**.
3. Locate `autoraise-rs` in the list (or your terminal app if running interactively) and toggle the permission slider to **ON**.

> ⚠️ **Development Note:** When you manually recompile the project using `cargo build --release` after code changes, macOS might quietly block the updated binary path without throwing a fresh alert popup. If the service drops execution streams unexpectedly, clear the OS permission cache: **remove** the binary from the Accessibility table using the `-` button and add/trigger it freshly.

### Debugging & Logs

To trace active event polling thresholds or debug parsing adjustments, watch your system standard error dump stream:

```bash
tail -f /tmp/autoraise-rs.log

```

---

## 🤝 Contributing

Contributions, enhancements, and structural cleanups are welcome! Please ensure any submitted pull requests strictly pass `cargo clippy` and respect the zero-allocation constraint within the primary event matching runtime loop.

Licensed under the [MIT License](https://www.google.com/search?q=LICENSE).

```

```
