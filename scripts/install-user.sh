#!/bin/sh
# Installs ADB Scan for the current user only (no sudo):
#   ~/.local/bin/adb-scan, menu launcher and icons.
# The udev rules need the .deb package (or copying them with sudo).
set -e
cd "$(dirname "$0")/.."
. scripts/env-no-dev.sh
cargo build -p adb-scan --release
BIN="$HOME/.local/bin"
APPS="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICONS="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"
mkdir -p "$BIN" "$APPS" "$ICONS"
install -m 755 target/release/adb-scan "$BIN/adb-scan"
cp assets/icons/hicolor/scalable/apps/*.svg "$ICONS/"
sed "s|^Exec=adb-scan|Exec=$BIN/adb-scan|" packaging/adb-scan.desktop > "$APPS/adb-scan.desktop"
gtk-update-icon-cache -q -t "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$APPS" 2>/dev/null || true
echo "Installed to $BIN/adb-scan. It appears in the menu as \"ADB Scan\"."
