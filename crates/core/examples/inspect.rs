//! Platform sheet and app inspection against a real phone, without showing
//! identifiers: `cargo run -p adbscan-core --example inspect [package…]`
//!
//! Without packages, it inspects every user app and measures the time.

use adbscan_core::adb::Adb;
use adbscan_core::apk::ComponentKind;
use adbscan_core::app_inspect;
use adbscan_core::probe::Probe;
use std::time::Instant;

fn show<T: std::fmt::Debug>(p: &Probe<T>) -> String {
    match p {
        Probe::Ok(v) => format!("{v:?}"),
        other => other.status_text().unwrap_or_default(),
    }
}

fn main() {
    let adb = Adb::locate(None).expect("adb");
    let Some(dev) = adb.devices().expect("adb devices").into_iter().find(|d| d.connected()) else {
        println!("No authorised device");
        return;
    };
    let serial = dev.serial;

    let t = Instant::now();
    let p = adb.platform_report(&serial).expect("platform");
    println!("Platform in {:?}", t.elapsed());
    println!("  SELinux: {}", show(&p.selinux));
    println!("  Kernel: {}", show(&p.kernel.clone().map(|k| format!("{} · {} · GKI {:?}", k.release, k.compiler, k.gki))));
    println!("  Page: {}", show(&p.page_size));
    println!("  GLES: {}", show(&p.gles));
    println!("  Vulkan: {}", show(&p.vulkan));
    println!("  Clusters: {}", show(&p.clusters.clone().map(|c| c.iter().map(|c| c.cpus_label()).collect::<Vec<_>>())));
    println!("  Thermal limits: {}", show(&p.thermal_limits.clone().map(|l| l.len())));
    println!("  Features: {}", show(&p.features.clone().map(|f| f.len())));
    println!("  Mainline: {}", show(&p.mainline));

    let mut packages: Vec<String> = std::env::args().skip(1).collect();
    if packages.is_empty() {
        packages = adb.shell_text(&serial, &["pm", "list", "packages", "-3"]).lines().filter_map(|l| l.trim().strip_prefix("package:").map(String::from)).collect();
    }
    let (mut ok_manifest, mut total_ms, mut slowest) = (0, 0u128, (0u128, String::new()));
    for pkg in &packages {
        let t = Instant::now();
        let a = match adb.inspect_package(&serial, pkg) {
            Ok(a) => a,
            Err(e) => {
                println!("{pkg}: {e}");
                continue;
            }
        };
        let ms = t.elapsed().as_millis();
        total_ms += ms;
        if ms > slowest.0 {
            slowest = (ms, pkg.clone());
        }
        let comps = match &a.manifest {
            Probe::Ok(m) => {
                ok_manifest += 1;
                let open = m.components.iter().filter(|c| c.is_open(m.target_sdk)).count();
                format!(
                    "{} act · {} svc · {} rcv · {} prov · {open} open",
                    m.of_kind(ComponentKind::Activity).count(),
                    m.of_kind(ComponentKind::Service).count(),
                    m.of_kind(ComponentKind::Receiver).count(),
                    m.of_kind(ComponentKind::Provider).count()
                )
            }
            other => other.status_text().unwrap_or_default(),
        };
        let exits = a.exits.ok().map(|e| format!("{} exits ({} failures)", e.len(), e.iter().filter(|x| x.is_failure()).count())).unwrap_or_else(|| show(&a.exits));
        if packages.len() <= 5 {
            println!("\n{pkg} in {ms} ms");
            println!("  extras: {}", show(&a.extras.clone().map(|e| (e.primary_abi, e.splits.len(), e.signing_scheme, e.dexopt))));
            println!("  links: {}", show(&a.links.clone().map(|l| (l.signatures.len(), l.domains.len()))));
            println!("  bucket: {}", show(&a.standby_bucket.clone().map(|b| app_inspect::standby_bucket_label(b).0)));
            println!("  app ops: {}", show(&a.appops.clone().map(|o| o.iter().map(|o| o.op.clone()).collect::<Vec<_>>())));
            println!("  battery: {}", show(&a.battery_exemption));
            println!("  exits: {exits}");
            println!("  processes: {}", show(&a.processes.clone().map(|p| { p.iter().map(|p| app_inspect::proc_state_label(&p.state).to_string()).collect::<Vec<_>>() })));
            println!("  components: {comps}");
        }
    }
    if !packages.is_empty() {
        println!("\n{} apps · manifest read in {ok_manifest} · average {} ms · slowest {} ({} ms)", packages.len(), total_ms / packages.len() as u128, slowest.1, slowest.0);
    }
}
