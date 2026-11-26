//! Wireless debugging (Android 11+): pairing with a QR code or manually, and
//! discovery of the services the phone announces.
//!
//! - **QR**: ADB Scan generates a service name and a password and shows them as
//!   `WIFI:T:ADB;S:<name>;P:<password>;;` (Android's format). The phone scans
//!   it in “Wireless debugging → Pair device with QR code” and announces over
//!   mDNS an `_adb-tls-pairing._tcp` service with that same name; then `adb
//!   pair IP:port password` is run.
//! - **Manual**: IP address, pairing port and 6-digit code shown by the phone
//!   (“Pair device with pairing code”).
//!
//! In both cases, after pairing it connects to the **connection port**, which
//! is a different one: that of the `_adb-tls-connect._tcp` service at the same
//! IP (or the one the user types). Pairing and connecting are done by
//! [`crate::adb::Adb::pair`] and [`crate::adb::Adb::connect`]; that logic is
//! not duplicated here.
//!
//! Discovery: `adb mdns services` if the installed `adb` has mDNS
//! (Debian/Ubuntu's 34.x does not) and, in addition, `avahi-browse` if
//! available. Only announcements are listened to; the network is not scanned.

use std::time::Duration;

use crate::adb::Adb;

pub const PAIRING_TYPE: &str = "_adb-tls-pairing._tcp";
pub const CONNECT_TYPE: &str = "_adb-tls-connect._tcp";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    /// Waiting for pairing (QR or code).
    Pairing,
    /// Ready to connect (device paired).
    Connect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdnsService {
    pub name: String,
    pub kind: ServiceKind,
    /// `IP:port`.
    pub address: String,
}

impl MdnsService {
    pub fn ip(&self) -> &str {
        self.address.rsplit_once(':').map(|(h, _)| h).unwrap_or(&self.address)
    }
}

fn kind_of(text: &str) -> Option<ServiceKind> {
    if text.contains("adb-tls-pairing") {
        Some(ServiceKind::Pairing)
    } else if text.contains("adb-tls-connect") {
        Some(ServiceKind::Connect)
    } else {
        None
    }
}

/// `adb mdns services`: `name\ttype\tIP:port`.
pub fn parse_adb_mdns(text: &str) -> Vec<MdnsService> {
    text.lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() < 3 || !p[2].contains(':') {
                return None;
            }
            Some(MdnsService { name: p[0].to_string(), kind: kind_of(p[1])?, address: p[2].to_string() })
        })
        .collect()
}

/// Undoes avahi's escapes (`\032` = space, `\.` = dot).
fn avahi_unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(u8::is_ascii_digit) {
            let n: u32 = s[i + 1..i + 4].parse().unwrap_or(32);
            out.push(n.min(255) as u8);
            i += 4;
        } else if b[i] == b'\\' && i + 1 < b.len() {
            out.push(b[i + 1]);
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `avahi-browse -tpr <type>`: resolved lines
/// `=;interface;IPv4;name;type;domain;host;IP;port;txt`. IPv4 only (adb does
/// not accept IPv6 link-local addresses without an interface).
pub fn parse_avahi(text: &str) -> Vec<MdnsService> {
    let mut out: Vec<MdnsService> = Vec::new();
    for l in text.lines() {
        let f: Vec<&str> = l.split(';').collect();
        if f.len() < 9 || f[0] != "=" || f[2] != "IPv4" {
            continue;
        }
        let Some(kind) = kind_of(f[4]) else { continue };
        let (ip, port) = (f[7], f[8]);
        if ip.is_empty() || port.parse::<u16>().is_err() {
            continue;
        }
        let s = MdnsService { name: avahi_unescape(f[3]), kind, address: format!("{ip}:{port}") };
        // It repeats once per interface.
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Wireless debugging services announced right now on the local network. It
/// takes up to a few seconds (avahi resolution).
pub fn discover(adb: Option<&Adb>) -> Vec<MdnsService> {
    let mut out: Vec<MdnsService> = Vec::new();
    if let Some(adb) = adb {
        if let Ok(r) = crate::exec::run(adb.path(), &["mdns".into(), "services".into()], Duration::from_secs(5), None) {
            out.extend(parse_adb_mdns(&r.stdout));
        }
    }
    // avahi only exists on Linux: on Windows adb's own mDNS is used.
    #[cfg(unix)]
    if let Some(avahi) = crate::exec::which("avahi-browse") {
        for t in [PAIRING_TYPE, CONNECT_TYPE] {
            if let Ok(r) = crate::exec::run(&avahi, &["-tpr".into(), t.into()], Duration::from_secs(6), None) {
                for s in parse_avahi(&r.stdout) {
                    if !out.iter().any(|o| o.address == s.address && o.kind == s.kind) {
                        out.push(s);
                    }
                }
            }
        }
    }
    out
}

/// Connection port of an already paired phone, by its IP.
pub fn connect_address_for(services: &[MdnsService], ip: &str) -> Option<String> {
    services.iter().find(|s| s.kind == ServiceKind::Connect && s.ip() == ip).map(|s| s.address.clone())
}

/// Credentials of a QR pairing (valid only for it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrPairing {
    /// Name of the service the phone will announce.
    pub name: String,
    pub password: String,
}

const ALNUM: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Fills `buf` from the system's random generator: `/dev/urandom`.
#[cfg(unix)]
fn fill_random(buf: &mut [u8]) -> bool {
    use std::io::Read;
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(buf)).is_ok()
}

/// Fills `buf` from the system's random generator: `BCryptGenRandom`.
#[cfg(windows)]
fn fill_random(buf: &mut [u8]) -> bool {
    use windows_sys::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    // SAFETY: valid buffer of the length given; no algorithm handle with the
    // system's preferred generator.
    unsafe { BCryptGenRandom(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG) == 0 }
}

fn random_alnum(n: usize) -> String {
    let mut buf = vec![0u8; n * 2];
    // The system's generator is always there; if it failed, the clock is
    // mixed in.
    if !fill_random(&mut buf) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (t >> ((i % 16) * 8)) as u8 ^ (i as u8).wrapping_mul(97);
        }
    }
    // Rejection sampling to avoid bias: only bytes below a multiple of the
    // alphabet.
    let limit = 256 - (256 % ALNUM.len());
    let mut out: String = buf.iter().filter(|b| (**b as usize) < limit).map(|b| ALNUM[*b as usize % ALNUM.len()] as char).take(n).collect();
    while out.len() < n {
        out.push('x');
    }
    out
}

impl QrPairing {
    pub fn new() -> Self {
        Self { name: format!("adbscan-{}", random_alnum(8)), password: random_alnum(12) }
    }

    /// QR content in the format Android reads. Name and password are
    /// alphanumeric: they need no escaping.
    pub fn payload(&self) -> String {
        format!("WIFI:T:ADB;S:{};P:{};;", self.name, self.password)
    }

    /// The pairing service the phone announced after scanning.
    pub fn find<'a>(&self, services: &'a [MdnsService]) -> Option<&'a MdnsService> {
        services.iter().find(|s| s.kind == ServiceKind::Pairing && s.name == self.name)
    }
}

impl Default for QrPairing {
    fn default() -> Self {
        Self::new()
    }
}

/// QR modules (`true` = black), row by row, and their width.
pub fn qr_matrix(payload: &str) -> Option<(usize, Vec<bool>)> {
    let code = qrcode::QrCode::with_error_correction_level(payload.as_bytes(), qrcode::EcLevel::M).ok()?;
    let width = code.width();
    let cells = code.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect();
    Some((width, cells))
}

/// IP and port typed separately, or pasted together in the IP field.
pub fn join_host_port(ip: &str, port: &str) -> String {
    let ip = ip.trim();
    let port = port.trim();
    if port.is_empty() && ip.contains(':') && !ip.contains("::") {
        return ip.to_string();
    }
    format!("{ip}:{port}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adb_mdns_output() {
        let t = "List of discovered mdns services\nadb-A1B2C3D4E5F6-AbCdEf\t_adb-tls-connect._tcp.\t192.168.1.50:41235\nadbscan-xyz\t_adb-tls-pairing._tcp.\t192.168.1.50:37001\n";
        let s = parse_adb_mdns(t);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].kind, ServiceKind::Connect);
        assert_eq!(s[1].kind, ServiceKind::Pairing);
        assert_eq!(s[1].ip(), "192.168.1.50");
        assert_eq!(connect_address_for(&s, "192.168.1.50").as_deref(), Some("192.168.1.50:41235"));
        assert_eq!(connect_address_for(&s, "192.168.1.9"), None);
    }

    #[test]
    fn avahi_output() {
        let t = "+;wlp2s0;IPv4;adbscan-Ab12;_adb-tls-pairing._tcp;local\n\
                 =;wlp2s0;IPv6;adbscan-Ab12;_adb-tls-pairing._tcp;local;Android.local;fe80::1;37001;\n\
                 =;wlp2s0;IPv4;adbscan-Ab12;_adb-tls-pairing._tcp;local;Android.local;192.168.1.50;37001;\n\
                 =;eth0;IPv4;adbscan-Ab12;_adb-tls-pairing._tcp;local;Android.local;192.168.1.50;37001;\n\
                 =;wlp2s0;IPv4;adb-A1B2\\032x;_adb-tls-connect._tcp;local;Android.local;192.168.1.50;41235;\"x=y\"\n";
        let s = parse_avahi(t);
        assert_eq!(s.len(), 2, "no IPv6 and no per-interface duplicates");
        assert_eq!(s[0], MdnsService { name: "adbscan-Ab12".into(), kind: ServiceKind::Pairing, address: "192.168.1.50:37001".into() });
        assert_eq!(s[1].name, "adb-A1B2 x");
        assert_eq!(s[1].kind, ServiceKind::Connect);
    }

    #[test]
    fn qr_credentials() {
        let a = QrPairing::new();
        let b = QrPairing::new();
        assert_ne!(a, b, "each pairing uses new credentials");
        assert!(a.name.starts_with("adbscan-") && a.name.len() == 16);
        assert_eq!(a.password.len(), 12);
        assert!(a.password.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(a.payload(), format!("WIFI:T:ADB;S:{};P:{};;", a.name, a.password));
        let found = vec![
            MdnsService { name: "other".into(), kind: ServiceKind::Pairing, address: "10.0.0.2:1".into() },
            MdnsService { name: a.name.clone(), kind: ServiceKind::Connect, address: "10.0.0.3:2".into() },
            MdnsService { name: a.name.clone(), kind: ServiceKind::Pairing, address: "10.0.0.4:3".into() },
        ];
        assert_eq!(a.find(&found).unwrap().address, "10.0.0.4:3");
    }

    #[test]
    fn qr_matrix_is_square() {
        let (w, cells) = qr_matrix(&QrPairing::new().payload()).unwrap();
        assert!(w >= 21 && (w - 17) % 4 == 0, "width of a valid QR version: {w}");
        assert_eq!(cells.len(), w * w);
        // Position pattern: top-left corner black.
        assert!(cells[0] && cells[6] && cells[6 * w]);
    }

    #[test]
    fn host_and_port() {
        assert_eq!(join_host_port(" 192.168.1.5 ", "37001"), "192.168.1.5:37001");
        assert_eq!(join_host_port("192.168.1.5:37001", ""), "192.168.1.5:37001");
    }
}
