# autoraise-rs

**Focus-follows-mouse & auto-raise for macOS — written in Rust.**

A faster, safer reimplementation of [AutoRaise](https://github.com/sbmpost/AutoRaise)
with first-class **AeroSpace tiling WM** integration.

---

## Why Rust over Objective-C++?

| | AutoRaise (ObjC++) | autoraise-rs (Rust) |
|---|---|---|
| **Kernel API** | CGEventTap | CGEventTap (identical) |
| **Window API** | AX + CGWindowList | AX + CGWindowList (identical) |
| **Latency** | Zero — passive tap | Zero — passive tap |
| **Memory safety** | Manual retain/release | Guaranteed by compiler |
| **Crash safety** | ObjC exceptions | Rust panics (recoverable) |
| **AeroSpace support** | None | ✅ Built-in |
| **Config format** | Key=value | TOML (structured) |

Same kernel-level APIs → **identical or better performance**. Rust adds
safety with zero runtime cost.

---

## AeroSpace Integration (Key Feature)

When `aerospace_aware = true` (default), autoraise-rs integrates with
[AeroSpace](https://github.com/nikitabobko/AeroSpace) tiling WM:

- **Tiled windows** → **skipped**. AeroSpace manages focus for tiled windows
  via its own `hjkl` bindings. Raising them would fight AeroSpace.
- **Floating windows** → **raised normally**. Dialogs, picture-in-picture,
  1Password, your terminal overlay — anything you've set to `layout floating`
  in AeroSpace gets auto-raised like a normal floating WM.

The floating window list is refreshed every `N` poll cycles (configurable)
by running `aerospace list-windows --all` in the background.

**If AeroSpace is not running**, autoraise-rs falls back to raising all windows.

---

## Build & Install

```bash
# Prerequisites: Rust toolchain (https://rustup.rs)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone and build
git clone <this-repo>
cd autoraise-rs
bash install.sh
```

`install.sh` will:
1. `cargo build --release`
2. Copy binary to `/usr/local/bin/autoraise-rs`
3. Write a sample config to `~/.config/autoraise-rs/config.toml`
4. Install and load a `launchd` agent (auto-start at login)

Then grant **Accessibility** permission when prompted:
> System Settings → Privacy & Security → Accessibility → add the binary

---

## Configuration

`~/.config/autoraise-rs/config.toml`:

```toml
# Poll interval in milliseconds (min 20, default 50)
poll_millis = 50

# Raise delay in ticks:
#   0 = disable raising
#   1 = raise instantly on hover
#   2+ = mouse must be still for (delay × poll_millis) ms
delay = 1

# Require mouse to stop before raising
require_mouse_stop = true

# AeroSpace integration — ONLY raise floating windows
aerospace_aware = true

# How often to refresh AeroSpace floating-window list (in poll cycles)
aerospace_refresh_cycles = 10

# Key that temporarily disables raising while held
# "control" | "option" | "disabled"
disable_key = "control"

# Apps to never auto-raise (case-insensitive)
ignore_apps = []
# ignore_apps = ["Finder", "Activity Monitor"]

# Window title substrings to ignore
ignore_titles = []
# ignore_titles = ["Picture in Picture", "Quick Look"]
```

---

## AeroSpace Config Tips

In your `~/.aerospace.toml`, mark windows you want auto-raised as floating:

```toml
# These will be auto-raised by autoraise-rs
[[on-window-detected]]
if.app-id = "com.1password.1password"
run = "layout floating"

[[on-window-detected]]
if.app-id = "com.apple.finder"
run = "layout floating"

[[on-window-detected]]
if.app-id = "com.iconfactory.Tot"
run = "layout floating"

# Tiled windows (default) are SKIPPED by autoraise-rs —
# use your hjkl bindings to focus them.
```

---

## Usage

```
autoraise-rs [OPTIONS]

Options:
  --poll-millis <N>               Poll interval ms [default: 50]
  --delay <N>                     Raise delay ticks [default: 1]
  --ignore-apps <a,b>             Comma-separated app names to skip
  --ignore-titles <x,y>           Title substrings to skip
  --disable-key <control|option>  Temporarily disable key [default: control]
  --aerospace-aware <bool>        AeroSpace integration [default: true]
  --aerospace-refresh-cycles <N>  Refresh interval [default: 10]
  --require-mouse-stop <bool>     Stop required before raise [default: true]
  --verbose                       Debug logging
  --help                          Show help
```

---

## Service Management

```bash
# Stop
launchctl unload ~/Library/LaunchAgents/com.user.autoraise-rs.plist

# Start
launchctl load -w ~/Library/LaunchAgents/com.user.autoraise-rs.plist

# Tail logs
tail -f /tmp/autoraise-rs.log

# Uninstall completely
launchctl unload ~/Library/LaunchAgents/com.user.autoraise-rs.plist
sudo rm /usr/local/bin/autoraise-rs
rm ~/Library/LaunchAgents/com.user.autoraise-rs.plist
```

---

## How It Works

```
Kernel (HID layer)
    │  CGEventTap (passive, zero latency, same as AutoRaise)
    ▼
Main Thread (CFRunLoop)
    │  MouseEvent { x, y, dx, dy }  via sync_channel
    ▼
Raiser Thread (poll every 50ms)
    │
    ├─ 1. CGWindowListCopyWindowInfo → find window under cursor
    ├─ 2. Layer check (skip menus/panels/overlays — layer != 0)
    ├─ 3. Modifier key check (control held? → skip)
    ├─ 4. Ignore app/title check
    ├─ 5. AeroSpace: is this window floating? (skip if tiled)
    ├─ 6. Delay tick (mouse must stop if delay > 1)
    └─ 7. AXRaise + NSRunningApplication activate
```

The CGEventTap fires on every mouse-moved HID event (kernel level).
The raiser thread drains the channel and processes only the *latest* position
per poll cycle — so fast mouse movement burns zero extra CPU.
