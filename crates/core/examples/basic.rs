//! Basic analysis against the phones connected over USB, **without using ADB**:
//! USB descriptors, MTP/PTP (if the phone is in “File transfer” mode) and, if
//! the interface is busy, GVfs. Private data (serial number, device name) is
//! shown only by its shape.
//! Usage: cargo run -p adbscan-core --example basic
use std::time::Instant;

use adbscan_core::basic::{self, Inputs, MtpState, Status};
use adbscan_core::mtp::{self, MtpError};
use adbscan_core::usb;

fn main() {
    let devs = usb::scan(std::path::Path::new(usb::SYSFS_USB));
    if devs.is_empty() {
        println!("No Android phone over USB.");
        for f in basic::without_usb().facts {
            println!("  {}: {}", f.label, f.value);
        }
        return;
    }
    for d in &devs {
        println!("== {} ({}:{}) mode {:?}", d.display_name(), d.vendor_id, d.product_id, d.mode);
        for i in &d.interfaces {
            println!(
                "   interface {} {}/{}/{} “{}” endpoints {:?} driver {:?}",
                i.number,
                i.class,
                i.subclass,
                i.protocol,
                i.name,
                i.endpoints.iter().map(|e| format!("{:02x} {}", e.address, e.transfer)).collect::<Vec<_>>(),
                usb::interface_driver(d, i.number)
            );
        }
        let t = Instant::now();
        let r = mtp::probe(d);
        println!("   MTP in {:?}", t.elapsed());
        if let Ok(rep) = &r {
            println!("   extension “{}” · operations {} · properties {:04x?}", rep.info.vendor_extension, rep.info.operations.len(), rep.info.properties);
            println!("   type {:?}", rep.perceived_type);
        }
        let gvfs = if matches!(r, Err(MtpError::Busy)) { mtp::gvfs_storages(d) } else { None };
        let state = MtpState::Read(r);
        let inputs = Inputs { usb: d, node_access: usb::node_accessible(d), mtp: &state, gvfs: gvfs.as_deref(), known: None, nickname: None, adb_state: None };
        for g in basic::report(&inputs).into_iter().chain([basic::advanced_group()]) {
            println!("  [{}]", g.title);
            for f in g.facts {
                let mark = match f.status {
                    Status::Available => "✔",
                    Status::Unavailable => "✘",
                    Status::NeedsAdb => "🔑",
                    Status::Pending => "…",
                };
                let value = if f.private && f.status == Status::Available { format!("(private, {} characters)", f.value.chars().count()) } else { f.value };
                println!("    {mark} {}: {value}", f.label);
            }
        }
    }
}
