#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
BUILD_DIR="$SCRIPT_DIR/target/release"

# Per-user install locations
APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/clippo"
BIN_DIR="$APP_DIR/bin"
LINK_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
SYSTEMD_USER_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

# Binaries in build output
DAEMON_SRC="$BUILD_DIR/daemon"
UI_SRC="$BUILD_DIR/ui"

# Names we install as
DAEMON_NAME="clippo_daemon"
UI_NAME="clippo_ui"

# Service file (expected next to this script)
SERVICE_SRC="$SCRIPT_DIR/clippo_daemon.service"
SERVICE_NAME="clippo_daemon.service"
SERVICE_DST="$SYSTEMD_USER_DIR/$SERVICE_NAME"

echo "Installing Clippo for user: $USER"

# ---- checks ----
if [ "${EUID:-$(id -u)}" -eq 0 ]; then
  echo "Error: do not run this script as root. It installs a user service."
  exit 1
fi

if ! command -v systemctl >/dev/null 2>&1; then
  echo "Error: systemctl is required for installation."
  exit 1
fi

if ! systemctl --user --version >/dev/null 2>&1; then
  echo "Error: systemd user services are not available in this session."
  exit 1
fi

if [ ! -f "$DAEMON_SRC" ]; then
  echo "Error: 'daemon' binary not found at $DAEMON_SRC."
  echo "Build first: cargo build --release"
  exit 1
fi

if [ ! -f "$UI_SRC" ]; then
  echo "Error: 'ui' binary not found at $UI_SRC."
  echo "Build first: cargo build --release"
  exit 1
fi

if [ ! -f "$SERVICE_SRC" ]; then
  echo "Error: service file not found at $SERVICE_SRC."
  exit 1
fi

# ---- install dirs ----
echo "Creating install directories..."
mkdir -p "$BIN_DIR" "$LINK_DIR" "$SYSTEMD_USER_DIR"

# ---- install binaries ----
echo "Installing daemon to $BIN_DIR/$DAEMON_NAME..."
install -m 755 "$DAEMON_SRC" "$BIN_DIR/$DAEMON_NAME"

echo "Installing ui to $BIN_DIR/$UI_NAME..."
install -m 755 "$UI_SRC" "$BIN_DIR/$UI_NAME"

# ---- convenience symlinks ----
echo "Creating/Updating symlinks in $LINK_DIR..."
ln -sf "$BIN_DIR/$DAEMON_NAME" "$LINK_DIR/$DAEMON_NAME"
ln -sf "$BIN_DIR/$UI_NAME" "$LINK_DIR/$UI_NAME"

# ---- install user service ----
echo "Installing user systemd service to $SERVICE_DST..."
install -m 644 "$SERVICE_SRC" "$SERVICE_DST"

echo "Reloading user systemd daemon..."
systemctl --user daemon-reload

if systemctl --user is-active --quiet "$SERVICE_NAME"; then
  echo "Restarting $SERVICE_NAME to apply updates..."
  systemctl --user restart "$SERVICE_NAME"
else
  echo "Enabling and starting $SERVICE_NAME..."
  systemctl --user enable --now "$SERVICE_NAME"
fi

echo
echo "Installation complete."
echo "Daemon status: systemctl --user status $SERVICE_NAME"
echo "Run the UI with: $LINK_DIR/$UI_NAME"
if [[ ":$PATH:" != *":$LINK_DIR:"* ]]; then
  echo "Note: $LINK_DIR is not in PATH. Add it to run clippo_ui directly."
fi
