#!/bin/bash
set -e

BUILD_DIR="./target/release"

# Destination directory (common location for system-wide binaries on Linux)
DEST_DIR="/usr/local/bin"

# Check if the daemon binary exists in the script's directory
if [ ! -f "$BUILD_DIR/daemon" ]; then
    echo "Error: 'daemon' binary not found. Exiting."
    exit 1
fi

# Check if the ui binary exists in the script's directory
if [ ! -f "$BUILD_DIR/ui" ]; then
    echo "Error: 'ui' binary not found. Exiting."
    exit 1
fi

# Ensure the destination directory exists, create it if it doesn't
if [ ! -d "$DEST_DIR" ]; then
    echo "$DEST_DIR does not exist. Creating it..."
    sudo mkdir -p "$DEST_DIR"
fi

# Move the files to the destination directory
echo "Installing daemon binary to $DEST_DIR..."
sudo mv "$BUILD_DIR/daemon" "$DEST_DIR/clippo_daemon"

echo "Installing ui binary to $DEST_DIR..."
sudo mv "$BUILD_DIR/ui" "$DEST_DIR/clippo_ui"

# Make sure the binaries are executable
echo "Setting execute permissions on the binaries..."
sudo chmod +x "$DEST_DIR/clippo_daemon" "$DEST_DIR/clippo_ui"

# Copying service file
if [ ! -f "./clippo_daemon.service" ]; then
    echo "The service file was not found. It should be downloaded from the repo and put nex to this script."
    exit 1
fi

sudo cp "./clippo_daemon.service" "/etc/systemd/system"

echo "Installation complete, launching the daemon and the ui ..."

sudo systemctl daemon-reload && sudo systemctl --now enable clippo_daemon.service

nohup clippo_ui &>/dev/null &


