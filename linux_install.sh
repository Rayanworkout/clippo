#!/bin/bash
set -e

BUILD_DIR="./target/release"

# App install location
APP_DIR="/opt/clippo"
BIN_DIR="$APP_DIR/bin"

# Convenience symlink location
LINK_DIR="/usr/local/bin"

# Binaries in build output
DAEMON_SRC="$BUILD_DIR/daemon"
UI_SRC="$BUILD_DIR/ui"

# Names we install as
DAEMON_NAME="clippo_daemon"
UI_NAME="clippo_ui"

# Service file (expected next to this script)
SERVICE_SRC="./clippo_daemon.service"
SERVICE_DST="/etc/systemd/system/clippo_daemon.service"

# ---- checks ----
if [ ! -f "$DAEMON_SRC" ]; then
  echo "Error: 'daemon' binary not found at $DAEMON_SRC. Exiting."
  exit 1
fi

if [ ! -f "$UI_SRC" ]; then
  echo "Error: 'ui' binary not found at $UI_SRC. Exiting."
  exit 1
fi

if [ ! -f "$SERVICE_SRC" ]; then
  echo "Error: service file not found at $SERVICE_SRC. It should be next to this script."
  exit 1
fi

# ---- install dirs ----
echo "Creating install directories under $APP_DIR..."
sudo mkdir -p "$BIN_DIR"

# ---- install binaries (copy instead of mv, so rebuilds don't break) ----
echo "Installing daemon to $BIN_DIR/$DAEMON_NAME..."
sudo cp "$DAEMON_SRC" "$BIN_DIR/$DAEMON_NAME"

echo "Installing ui to $BIN_DIR/$UI_NAME..."
sudo cp "$UI_SRC" "$BIN_DIR/$UI_NAME"

echo "Setting execute permissions..."
sudo chmod +x "$BIN_DIR/$DAEMON_NAME" "$BIN_DIR/$UI_NAME"

# ---- optional convenience symlinks ----
echo "Ensuring $LINK_DIR exists..."
sudo mkdir -p "$LINK_DIR"

echo "Creating/Updating symlinks in $LINK_DIR..."
sudo ln -sf "$BIN_DIR/$DAEMON_NAME" "$LINK_DIR/$DAEMON_NAME"
sudo ln -sf "$BIN_DIR/$UI_NAME" "$LINK_DIR/$UI_NAME"

# ---- install + patch systemd service to point to /opt/clippo ----
echo "Installing systemd service to $SERVICE_DST..."
sudo cp "$SERVICE_SRC" "$SERVICE_DST"

# Update ExecStart lines to use the /opt/clippo binaries.
# (Assumes the service file contains an ExecStart=... line.)
sudo sed -i \
  -e "s|^ExecStart=.*daemon.*|ExecStart=$BIN_DIR/$DAEMON_NAME|g" \
  -e "s|^ExecStart=.*clippo_daemon.*|ExecStart=$BIN_DIR/$DAEMON_NAME|g" \
  "$SERVICE_DST"

echo "Installation complete, launching the daemon and the ui ..."

sudo systemctl daemon-reload
sudo systemctl --now enable clippo_daemon.service

nohup "$BIN_DIR/$UI_NAME" &>/dev/null &