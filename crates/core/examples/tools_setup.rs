//! Downloads and installs the managed tools (platform-tools and scrcpy).
//!
//! `XDG_DATA_HOME=$(mktemp -d) cargo run -p adbscan-core --example tools_setup`
//! leaves them in that temporary folder instead of
//! `~/.local/share/adb-scan/tools`.

use adbscan_core::adb::Tool;
use adbscan_core::tools_setup::{self, Stage};
use std::sync::atomic::AtomicBool;

fn main() {
    let cancel = AtomicBool::new(false);
    for p in tools_setup::PACKAGES {
        println!("{} {} → {}", p.title, p.version, tools_setup::install_dir(&p).display());
        let last = std::cell::Cell::new(-1i64);
        let progress = |s: Stage| match s {
            Stage::Downloading(got, total) => {
                let pct = (got * 100 / total.max(1)) as i64;
                if pct / 25 != last.get() / 25 {
                    last.set(pct);
                    println!("  downloading {pct} %");
                }
            }
            other => println!("  {other:?}"),
        };
        match tools_setup::install(&p, &progress, &cancel) {
            Ok(dir) => println!("  installed in {} (version {:?})", dir.display(), tools_setup::installed_version(&p)),
            Err(e) => println!("  ERROR: {e}"),
        }
    }
    for t in [Tool::Adb, Tool::Fastboot, Tool::Scrcpy] {
        let st = tools_setup::tool_state(t, "");
        println!("{}: {:?} · {:?}", t.name(), st.found, st.version);
    }
}
