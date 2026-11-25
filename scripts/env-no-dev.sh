#!/bin/sh
# Build without the GTK4/libadwaita -dev packages installed.
#
# Downloads the -dev packages with "apt-get download" (no root), extracts them
# into ~/.cache/adb-scan-build/sysroot and sets up an environment that points
# pkg-config there. It is only for building: at runtime the system libraries
# are used (libgtk-4-1, libadwaita-1-0, libvte-2.91-gtk4-0).
#
# Usage:  . scripts/env-no-dev.sh   (then cargo build)
#
# Not needed if you already ran "sudo apt install libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev".

ROOT="$HOME/.cache/adb-scan-build"
SYS="$ROOT/sysroot"
export PATH="$HOME/.cargo/bin:$PATH"

if pkg-config --exists gtk4 libadwaita-1 vte-2.91-gtk4 2>/dev/null; then
    return 0 2>/dev/null || exit 0
fi

if [ ! -f "$SYS/usr/lib/x86_64-linux-gnu/pkgconfig/gtk4.pc" ]; then
    echo "Preparing the build sysroot in $SYS…"
    mkdir -p "$ROOT/debs" "$SYS"
    (
        cd "$ROOT/debs" || exit 1
        PKGS=$(apt-get install -s libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev 2>/dev/null | awk '/^Inst/{print $2}' | grep -- '-dev$')
        # Linking also needs the VTE library if it is not installed.
        dpkg -s libvte-2.91-gtk4-0 >/dev/null 2>&1 || PKGS="$PKGS libvte-2.91-gtk4-0"
        # shellcheck disable=SC2086
        apt-get download $PKGS >/dev/null 2>&1
    )
    for d in "$ROOT"/debs/*.deb; do dpkg-deb -x "$d" "$SYS"; done
    # The development .so links point to the system libraries.
    (
        cd "$SYS/usr/lib/x86_64-linux-gnu" || exit 1
        for l in $(find . -maxdepth 1 -type l -name '*.so'); do
            t=$(readlink "$l")
            case "$t" in
                /*) ;;
                *) [ -e "/usr/lib/x86_64-linux-gnu/$t" ] && ln -sf "/usr/lib/x86_64-linux-gnu/$t" "$l" ;;
            esac
        done
    )
fi

export PKG_CONFIG_PATH="$SYS/usr/lib/x86_64-linux-gnu/pkgconfig:$SYS/usr/share/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="$SYS"
