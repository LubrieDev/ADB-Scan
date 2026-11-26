//! The read scripts of the newer pages (Preinstalled apps, Permissions,
//! Storage, Developer) only read: no verb that changes the phone.

use adbscan_core::{debloat, devtools, permissions, storage};

#[test]
fn read_scripts_only_read() {
    for script in [debloat::STATE_SCRIPT, permissions::SCRIPT, devtools::STATE_SCRIPT, storage::SCAN_SCRIPT] {
        for forbidden in [
            "setprop",
            "settings put",
            "settings delete",
            " set ",
            "appops set",
            "pm uninstall",
            "pm install",
            "install-existing",
            "pm grant",
            "pm revoke",
            "pm clear",
            "pm disable",
            "pm enable",
            "disallow_listener",
            "am broadcast",
            "wm size ",
            "wm density ",
            "reverse",
            "forward",
            "rm ",
            "kill",
            "> /",
            ">/data",
            ">/sdcard",
        ] {
            assert!(!script.contains(forbidden), "“{forbidden}” in the script:\n{script}");
        }
    }
}
