#!/bin/sh
# Runs ADB Scan from the source code.
set -e
cd "$(dirname "$0")/.."
. scripts/env-no-dev.sh
cargo run -p adb-scan --release -- "$@"
