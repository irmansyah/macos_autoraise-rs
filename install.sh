#!/usr/bin/env bash
# install.sh — Build autoraise-rs and install it as a login service
# Usage: bash install.sh

set -euo pipefail

BINARY_NAME="autoraise-rs"
INSTALL_DIR="/usr/local/bin"
PLIST_NAME="com.user.autoraise-rs.plist"
PLIST_DEST="$HOME/Library/LaunchAgents/$PLIST_NAME"
LOG_FILE="/tmp/autoraise-rs.log"

echo "==> Building autoraise-rs (release)..."
cargo build --release

BINARY_PATH="$(pwd)/target/release/$BINARY_NAME"

echo "==> Installing binary to $INSTALL_DIR/$BINARY_NAME..."
sudo cp "$BINARY_PATH" "$INSTALL_DIR/$BINARY_NAME"
sudo chmod +x "$INSTALL_DIR/$BINARY_NAME"

echo "==> Writing sample config..."
mkdir -p "$HOME/.config/autoraise-rs"
if [ ! -f "$HOME/.config/autoraise-rs/config.toml" ]; then
    "$INSTALL_DIR/$BINARY_NAME" --help > /dev/null  # ensure it runs
    # Write sample config manually since --write-config isn't wired yet
    cat > "$HOME/.config/autoraise-rs/config.toml" << 'EOF'
# autoraise-rs configuration
poll_millis = 50
delay = 1
require_mouse_stop = true
aerospace_aware = true
aerospace_refresh_cycles = 10
disable_key = "control"
ignore_apps = []
ignore_titles = []
EOF
    echo "    Config written to ~/.config/autoraise-rs/config.toml"
else
    echo "    Config already exists, skipping."
fi

echo "==> Installing launchd plist..."
# Update plist path to actual install location
sed "s|/usr/local/bin/autoraise-rs|$INSTALL_DIR/$BINARY_NAME|g" \
    "$PLIST_NAME" > "$PLIST_DEST"

echo "==> Loading launchd agent..."
# Unload first if already loaded
launchctl unload "$PLIST_DEST" 2>/dev/null || true
launchctl load -w "$PLIST_DEST"

echo ""
echo "✅  autoraise-rs installed and running!"
echo ""
echo "   Binary:  $INSTALL_DIR/$BINARY_NAME"
echo "   Config:  ~/.config/autoraise-rs/config.toml"
echo "   Log:     $LOG_FILE"
echo "   LaunchAgent: $PLIST_DEST"
echo ""
echo "⚠️  IMPORTANT: Grant Accessibility permission if prompted:"
echo "   System Settings → Privacy & Security → Accessibility"
echo "   Add '$INSTALL_DIR/$BINARY_NAME' and enable it."
echo ""
echo "   To stop:    launchctl unload $PLIST_DEST"
echo "   To start:   launchctl load -w $PLIST_DEST"
echo "   To uninstall: launchctl unload $PLIST_DEST && sudo rm $INSTALL_DIR/$BINARY_NAME"
