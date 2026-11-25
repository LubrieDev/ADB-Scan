#!/bin/sh
# Builds dist/adb-scan_<version>_amd64.deb (without debhelper).
set -e
cd "$(dirname "$0")/.."
. scripts/env-no-dev.sh
cargo build -p adb-scan --release
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
PKG=dist/deb
rm -rf "$PKG"
mkdir -p "$PKG/DEBIAN" "$PKG/usr/bin" "$PKG/usr/share/applications" "$PKG/usr/share/metainfo" \
         "$PKG/usr/share/icons/hicolor/scalable/apps" "$PKG/lib/udev/rules.d" "$PKG/usr/share/doc/adb-scan"
install -m 755 target/release/adb-scan "$PKG/usr/bin/adb-scan"
install -m 644 packaging/adb-scan.desktop "$PKG/usr/share/applications/adb-scan.desktop"
install -m 644 packaging/com.adbscan.AdbScan.metainfo.xml "$PKG/usr/share/metainfo/"
install -m 644 assets/icons/hicolor/scalable/apps/*.svg "$PKG/usr/share/icons/hicolor/scalable/apps/"
install -m 644 packaging/51-adb-scan.rules "$PKG/lib/udev/rules.d/51-adb-scan.rules"
install -m 644 README.md "$PKG/usr/share/doc/adb-scan/README.md"
install -m 644 docs/USAGE.md "$PKG/usr/share/doc/adb-scan/USAGE.md"
install -m 644 docs/LIMITATIONS.md "$PKG/usr/share/doc/adb-scan/LIMITATIONS.md"
install -m 644 assets/icons/LICENSE-lucide "$PKG/usr/share/doc/adb-scan/LICENSE-lucide"
install -m 755 packaging/debian/postinst packaging/debian/postrm "$PKG/DEBIAN/"
SIZE=$(du -sk "$PKG/usr" "$PKG/lib" | awk '{s+=$1} END {print s}')
cat > "$PKG/DEBIAN/control" <<CONTROL
Package: adb-scan
Version: $VERSION
Section: utils
Priority: optional
Architecture: amd64
Maintainer: ADB Scan <adb-scan@localhost>
Installed-Size: $SIZE
Depends: libgtk-4-1 (>= 4.14), libadwaita-1-0 (>= 1.5), libvte-2.91-gtk4-0 (>= 0.76), libc6, adb
Recommends: fastboot, scrcpy, curl, avahi-utils, libglib2.0-bin
Suggests: tshark
Description: monitoring, diagnostics and tools for Android phones
 ADB Scan is a native app (Rust, GTK 4 and libadwaita) that detects and
 monitors Android devices. Without debugging it offers a basic analysis over
 USB and MTP; with USB or wireless debugging (QR code or manual pairing):
 live telemetry, battery, performance, apps, files, logcat, a terminal,
 screen mirroring with scrcpy, flashing with safeguards, wireless ADB and a
 security audit without root. Interface in English and Spanish.
 .
 Everything is processed and stored locally, with no telemetry. It only
 connects to the Internet when asked: the public IP (queried by the phone)
 and downloading platform-tools or scrcpy from the tools assistant.
CONTROL
(cd "$PKG" && find usr lib -type f -exec md5sum {} + > DEBIAN/md5sums)
mkdir -p dist
OUT="dist/adb-scan_${VERSION}_amd64.deb"
dpkg-deb --root-owner-group --build "$PKG" "$OUT" >/dev/null
echo "Package: $OUT ($(du -h "$OUT" | cut -f1))"
echo "Install with: sudo apt install ./$OUT"
