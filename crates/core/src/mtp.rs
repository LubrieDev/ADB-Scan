//! Read-only MTP/PTP access: what the phone shares when the user chooses “File
//! transfer” (or “photo transfer”), without developer options or USB debugging.
//!
//! Only descriptive data is requested: device information, storages (name,
//! capacity and free space) and three properties (battery, device name and
//! type). Files are **never** listed, read or written.
//!
//! The phone is reached through usbfs (`/dev/bus/usb/BBB/DDD`), with the same
//! access ADB Scan's udev rules already grant. If another program holds the
//! interface (the file manager claims it when mounting the phone), it is not
//! taken away: this is reported and, if mounted, the storages are read through
//! GVfs ([`gvfs_storages`]).
//!
//! Format (PTP, ISO 15740, and its MTP extension): containers with a 12-byte
//! header (length, type, code, transaction) in little endian; strings as a
//! length byte (including the NUL) and UTF-16LE.

use crate::{i18n::tr, t};
use std::path::Path;
use std::time::Duration;

use crate::probe::Probe;
use crate::usb::{UsbDevice, UsbFunction};

// Operations
const OP_GET_DEVICE_INFO: u16 = 0x1001;
const OP_OPEN_SESSION: u16 = 0x1002;
const OP_CLOSE_SESSION: u16 = 0x1003;
const OP_GET_STORAGE_IDS: u16 = 0x1004;
const OP_GET_STORAGE_INFO: u16 = 0x1005;
const OP_GET_DEVICE_PROP_VALUE: u16 = 0x1015;
// Responses
const RC_OK: u16 = 0x2001;
const RC_SESSION_ALREADY_OPEN: u16 = 0x201E;
const RC_DEVICE_BUSY: u16 = 0x2019;
// Device properties
pub const PROP_BATTERY_LEVEL: u16 = 0x5001;
pub const PROP_FRIENDLY_NAME: u16 = 0xD402;
pub const PROP_PERCEIVED_TYPE: u16 = 0xD407;
// Container types
const CT_COMMAND: u16 = 1;
const CT_DATA: u16 = 2;
const CT_RESPONSE: u16 = 3;

/// `DeviceInfo` (PTP 5.5.1).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceInfo {
    pub standard_version: u16,
    pub vendor_extension_id: u32,
    pub vendor_extension_version: u16,
    /// Android: “microsoft.com: 1.0; android.com: 1.0;”.
    pub vendor_extension: String,
    pub functional_mode: u16,
    pub operations: Vec<u16>,
    pub events: Vec<u16>,
    pub properties: Vec<u16>,
    pub capture_formats: Vec<u16>,
    pub playback_formats: Vec<u16>,
    pub manufacturer: String,
    pub model: String,
    pub device_version: String,
    pub serial: String,
}

impl DeviceInfo {
    /// Android's MTP extension (`android.com`) is only announced by Android.
    pub fn is_android(&self) -> bool {
        self.vendor_extension.to_lowercase().contains("android.com")
    }

    pub fn supports_property(&self, code: u16) -> bool {
        self.properties.contains(&code)
    }
}

/// `StorageInfo` (PTP 5.5.3).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StorageInfo {
    pub id: u32,
    pub storage_type: u16,
    pub filesystem_type: u16,
    pub access: u16,
    pub capacity: u64,
    pub free: u64,
    pub description: String,
    pub volume_label: String,
}

impl StorageInfo {
    pub fn kind_label(&self) -> &'static str {
        match self.storage_type {
            1 => tr("fixed (ROM)"),
            2 => tr("removable (ROM)"),
            3 => tr("internal"),
            4 => tr("removable"),
            _ => "",
        }
    }

    pub fn read_only(&self) -> bool {
        self.access == 1 || self.access == 2
    }

    pub fn name(&self) -> String {
        [&self.description, &self.volume_label].iter().find(|s| !s.trim().is_empty()).map(|s| s.trim().to_string()).unwrap_or_else(|| t!("Storage {}", format!("{:08x}", self.id)))
    }
}

/// Storage seen through GVfs (when the file manager has the phone mounted and
/// the MTP interface busy).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GvfsStorage {
    pub name: String,
    pub capacity: Option<u64>,
    pub free: Option<u64>,
}

/// What was read over MTP/PTP.
#[derive(Debug, Clone, PartialEq)]
pub struct MtpReport {
    pub function: UsbFunction,
    pub info: DeviceInfo,
    /// Battery level (0–100) if the phone publishes the property.
    pub battery: Probe<u8>,
    /// Device name the user set in Settings.
    pub friendly_name: Probe<String>,
    pub perceived_type: Probe<u32>,
    /// Empty with the phone locked: Android does not share the storage until it
    /// is unlocked.
    pub storages: Probe<Vec<StorageInfo>>,
}

pub fn perceived_type_label(code: u32) -> Option<&'static str> {
    Some(match code {
        1 => tr("Camera"),
        2 => tr("Media player"),
        3 => tr("Mobile phone"),
        4 => tr("Video player"),
        5 => tr("Personal organiser (PDA)"),
        6 => tr("Audio recorder"),
        _ => return None,
    })
}

/// Why it could not be read over MTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MtpError {
    /// The phone offers neither MTP nor PTP (“Charging only” mode, debugging…).
    NotOffered,
    /// No access to the USB node (the udev rules are missing).
    NoPermission,
    /// Another program holds the interface (file manager, gphoto…).
    Busy,
    /// The phone did not respond in time (locked, busy…).
    Timeout,
    /// It disconnected during the read.
    Gone,
    Protocol(String),
}

impl MtpError {
    pub fn message(&self) -> String {
        match self {
            Self::NotOffered => tr("The phone does not share files over USB: choose “File transfer” in its USB notification").into(),
            Self::NoPermission => tr("No access to the USB device: install ADB Scan's udev rules").into(),
            Self::Busy => tr("The file manager has it open (Linux Mint mounts the phone when it is connected)").into(),
            Self::Timeout => tr("The phone did not respond: unlock it and press Refresh").into(),
            Self::Gone => tr("It disconnected during the read").into(),
            Self::Protocol(e) => t!("Invalid MTP response ({})", e),
        }
    }

    pub fn to_probe<T>(&self) -> Probe<T> {
        match self {
            Self::NotOffered => Probe::Unsupported(self.message()),
            Self::NoPermission | Self::Busy => Probe::Restricted(self.message()),
            _ => Probe::Failed(self.message()),
        }
    }
}

// ---------------------------------------------------------------------------
// Parsers (pure)
// ---------------------------------------------------------------------------

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }

    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
    }

    /// PTP string: number of UTF-16 characters (including the final NUL) and
    /// the characters. 0 = empty string.
    fn string(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        if n == 0 {
            return Some(String::new());
        }
        let raw = self.take(n * 2)?;
        let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
        Some(String::from_utf16_lossy(&units).trim().to_string())
    }

    fn array16(&mut self) -> Option<Vec<u16>> {
        let n = self.u32()? as usize;
        // An absurd count is corrupt data, not a huge allocation.
        if n > self.buf.len() {
            return None;
        }
        (0..n).map(|_| self.u16()).collect()
    }

    fn array32(&mut self) -> Option<Vec<u32>> {
        let n = self.u32()? as usize;
        if n > self.buf.len() {
            return None;
        }
        (0..n).map(|_| self.u32()).collect()
    }
}

pub fn parse_device_info(data: &[u8]) -> Option<DeviceInfo> {
    let mut r = Reader::new(data);
    Some(DeviceInfo {
        standard_version: r.u16()?,
        vendor_extension_id: r.u32()?,
        vendor_extension_version: r.u16()?,
        vendor_extension: r.string()?,
        functional_mode: r.u16()?,
        operations: r.array16()?,
        events: r.array16()?,
        properties: r.array16()?,
        capture_formats: r.array16()?,
        playback_formats: r.array16()?,
        manufacturer: r.string()?,
        model: r.string()?,
        device_version: r.string()?,
        serial: r.string().unwrap_or_default(),
    })
}

pub fn parse_storage_ids(data: &[u8]) -> Option<Vec<u32>> {
    Reader::new(data).array32()
}

pub fn parse_storage_info(id: u32, data: &[u8]) -> Option<StorageInfo> {
    let mut r = Reader::new(data);
    Some(StorageInfo {
        id,
        storage_type: r.u16()?,
        filesystem_type: r.u16()?,
        access: r.u16()?,
        capacity: r.u64()?,
        free: r.u64()?,
        description: {
            let _free_objects = r.u32()?;
            r.string()?
        },
        volume_label: r.string().unwrap_or_default(),
    })
}

/// Unsigned integer property value (UINT8/16/32 according to its length).
pub fn parse_uint(data: &[u8]) -> Option<u32> {
    let mut r = Reader::new(data);
    match data.len() {
        1 => r.u8().map(u32::from),
        2 => r.u16().map(u32::from),
        4 | 8 => r.u32(),
        _ => None,
    }
}

pub fn parse_string(data: &[u8]) -> Option<String> {
    Reader::new(data).string()
}

/// Command container: header + parameters.
pub fn command(code: u16, transaction: u32, params: &[u32]) -> Vec<u8> {
    let len = 12 + 4 * params.len() as u32;
    let mut out = Vec::with_capacity(len as usize);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&CT_COMMAND.to_le_bytes());
    out.extend_from_slice(&code.to_le_bytes());
    out.extend_from_slice(&transaction.to_le_bytes());
    for p in params {
        out.extend_from_slice(&p.to_le_bytes());
    }
    out
}

/// Header of a container: (total length, type, code).
pub fn header(buf: &[u8]) -> Option<(u32, u16, u16)> {
    let mut r = Reader::new(buf);
    let len = r.u32()?;
    let kind = r.u16()?;
    let code = r.u16()?;
    Some((len, kind, code))
}

// ---------------------------------------------------------------------------
// GVfs (phone already mounted by the file manager)
// ---------------------------------------------------------------------------

/// GVfs URI of the phone in `gio mount -l`, by its USB serial number
/// (`mtp://Manufacturer_Model_SERIAL/`) or its position on the bus
/// (`mtp://[usb:002,012]/`, older versions).
pub fn find_gvfs_uri(mount_list: &str, dev: &UsbDevice) -> Option<String> {
    let bus_dev = format!("[usb:{:0>3},{:0>3}]", dev.busnum, dev.devnum);
    for line in mount_list.lines() {
        let Some((_, uri)) = line.split_once(" -> ") else {
            continue;
        };
        let uri = uri.trim();
        if !(uri.starts_with("mtp://") || uri.starts_with("gphoto2://")) {
            continue;
        }
        let host = uri.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("");
        let serial_match = dev.serial.len() >= 4 && host.ends_with(&dev.serial);
        if serial_match || host == bus_dev {
            return Some(if uri.ends_with('/') { uri.to_string() } else { format!("{uri}/") });
        }
    }
    None
}

/// Names of the root folders (`gio list -d`): one per storage.
pub fn parse_gio_list(text: &str) -> Vec<String> {
    text.lines().map(|l| l.split('\t').next().unwrap_or("").trim().to_string()).filter(|n| !n.is_empty()).collect()
}

/// `filesystem::size` and `filesystem::free` from `gio info -f`.
pub fn parse_gio_fs(text: &str) -> (Option<u64>, Option<u64>) {
    let get = |key: &str| text.lines().find_map(|l| l.trim().strip_prefix(key)).and_then(|v| v.trim().parse::<u64>().ok());
    (get("filesystem::size:"), get("filesystem::free:"))
}

/// Storages of the phone mounted by GVfs, without listing their content. `None`
/// if it is not mounted or there is no `gio`.
pub fn gvfs_storages(dev: &UsbDevice) -> Option<Vec<GvfsStorage>> {
    let gio = crate::exec::which("gio")?;
    let run = |args: &[&str]| -> Option<String> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let r = crate::exec::run(&gio, &args, Duration::from_secs(4), None).ok()?;
        r.ok().then_some(r.stdout)
    };
    let uri = find_gvfs_uri(&run(&["mount", "-l"])?, dev)?;
    let mut out = Vec::new();
    for name in parse_gio_list(&run(&["list", &uri])?) {
        // `gio list` gives display names; the child's path is the same.
        let child = format!("{uri}{name}");
        let (capacity, free) = run(&["info", "-f", &child]).map(|t| parse_gio_fs(&t)).unwrap_or((None, None));
        out.push(GvfsStorage { name, capacity, free });
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Transport: usbfs
// ---------------------------------------------------------------------------

#[repr(C)]
struct BulkTransfer {
    ep: libc::c_uint,
    len: libc::c_uint,
    timeout: libc::c_uint,
    data: *mut libc::c_void,
}

const fn ioc(dir: u32, nr: u32, size: usize) -> u32 {
    (dir << 30) | ((size as u32) << 16) | ((b'U' as u32) << 8) | nr
}
// <linux/usbdevice_fs.h>
const USBDEVFS_BULK: u32 = ioc(3, 2, std::mem::size_of::<BulkTransfer>());
const USBDEVFS_CLAIMINTERFACE: u32 = ioc(2, 15, std::mem::size_of::<libc::c_uint>());
const USBDEVFS_RELEASEINTERFACE: u32 = ioc(2, 16, std::mem::size_of::<libc::c_uint>());

const TIMEOUT_MS: u32 = 3000;
const CHUNK: usize = 64 * 1024;
/// Cap on a response: DeviceInfo and StorageInfo take a few hundred bytes.
const MAX_DATA: usize = 1024 * 1024;

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn io_error(code: i32) -> MtpError {
    match code {
        libc::ETIMEDOUT => MtpError::Timeout,
        libc::ENODEV | libc::ESHUTDOWN | libc::ENOENT => MtpError::Gone,
        libc::EBUSY => MtpError::Busy,
        libc::EACCES | libc::EPERM => MtpError::NoPermission,
        other => MtpError::Protocol(std::io::Error::from_raw_os_error(other).to_string()),
    }
}

/// Claimed MTP/PTP interface; released and closed when dropped.
struct Session {
    fd: i32,
    iface: libc::c_uint,
    ep_in: u8,
    ep_out: u8,
    transaction: u32,
    in_session: bool,
}

impl Session {
    fn open(node: &Path, iface: u8, ep_in: u8, ep_out: u8) -> Result<Self, MtpError> {
        let c = std::ffi::CString::new(node.as_os_str().as_encoded_bytes()).map_err(|_| MtpError::Protocol("path".into()))?;
        // SAFETY: valid NUL-terminated path.
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(io_error(errno()));
        }
        let n: libc::c_uint = iface.into();
        // SAFETY: open fd; the argument is a pointer to a valid integer.
        if unsafe { libc::ioctl(fd, USBDEVFS_CLAIMINTERFACE as _, &n as *const libc::c_uint) } < 0 {
            let e = errno();
            // SAFETY: fd opened by us.
            unsafe { libc::close(fd) };
            return Err(io_error(e));
        }
        Ok(Self { fd, iface: n, ep_in, ep_out, transaction: 0, in_session: false })
    }

    fn bulk(&self, ep: u8, buf: &mut [u8]) -> Result<usize, MtpError> {
        let mut t = BulkTransfer { ep: ep.into(), len: buf.len() as libc::c_uint, timeout: TIMEOUT_MS, data: buf.as_mut_ptr() as *mut libc::c_void };
        // SAFETY: `t.data` points to `buf`, which lives during the call and is
        // `t.len` bytes long.
        let n = unsafe { libc::ioctl(self.fd, USBDEVFS_BULK as _, &mut t as *mut BulkTransfer) };
        if n < 0 {
            return Err(io_error(errno()));
        }
        Ok(n as usize)
    }

    /// Sends a command and returns (response code, data).
    fn transact(&mut self, code: u16, params: &[u32]) -> Result<(u16, Vec<u8>), MtpError> {
        // Outside a session (GetDeviceInfo and OpenSession itself) the
        // transaction is 0; inside, 1, 2, 3…
        let tid = if self.in_session {
            self.transaction = self.transaction.wrapping_add(1).max(1);
            self.transaction
        } else {
            0
        };
        let mut cmd = command(code, tid, params);
        self.bulk(self.ep_out, &mut cmd)?;
        let mut data = Vec::new();
        let mut buf = vec![0u8; CHUNK];
        // Up to three empty reads (zero-length packets after data that is a
        // multiple of the packet size).
        let mut empty = 0;
        loop {
            let n = self.bulk(self.ep_in, &mut buf)?;
            if n == 0 {
                empty += 1;
                if empty > 3 {
                    return Err(MtpError::Protocol(tr("no response").into()));
                }
                continue;
            }
            let (len, kind, rcode) = header(&buf[..n]).ok_or_else(|| MtpError::Protocol("short header".into()))?;
            match kind {
                CT_DATA => {
                    let len = len as usize;
                    if !(12..=MAX_DATA).contains(&len) {
                        return Err(MtpError::Protocol(format!("length {len}")));
                    }
                    let mut whole = buf[..n].to_vec();
                    while whole.len() < len {
                        let m = self.bulk(self.ep_in, &mut buf)?;
                        if m == 0 {
                            break;
                        }
                        whole.extend_from_slice(&buf[..m]);
                    }
                    whole.truncate(len);
                    data = whole.split_off(12);
                }
                CT_RESPONSE => return Ok((rcode, data)),
                // Events or others: ignored.
                _ => {}
            }
        }
    }

    fn data(&mut self, code: u16, params: &[u32]) -> Result<Vec<u8>, MtpError> {
        match self.transact(code, params)? {
            (RC_OK, d) => Ok(d),
            (RC_DEVICE_BUSY, _) => Err(MtpError::Timeout),
            (rc, _) => Err(MtpError::Protocol(format!("response {rc:#06x}"))),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: fd opened and claimed by us.
        unsafe {
            libc::ioctl(self.fd, USBDEVFS_RELEASEINTERFACE as _, &self.iface as *const libc::c_uint);
            libc::close(self.fd);
        }
    }
}

/// Reads over MTP/PTP what the phone describes about itself. Read-only: it
/// opens a session, asks for information, storages and three properties, and
/// closes it. It takes a few tenths of a second.
pub fn probe(dev: &UsbDevice) -> Result<MtpReport, MtpError> {
    let iface = dev.transfer_interface().ok_or(MtpError::NotOffered)?;
    let function = iface.function(&dev.vendor_id, &dev.product_id).unwrap_or(UsbFunction::Mtp);
    let (ep_in, ep_out) = iface.bulk_pair().ok_or(MtpError::NotOffered)?;
    // If another program holds it, it is not even attempted: it is not taken
    // away.
    if crate::usb::interface_driver(dev, iface.number).is_some() {
        return Err(MtpError::Busy);
    }
    let node = crate::usb::node_path(dev).ok_or(MtpError::Gone)?;
    let mut s = Session::open(Path::new(&node), iface.number, ep_in, ep_out)?;

    let info = parse_device_info(&s.data(OP_GET_DEVICE_INFO, &[])?).ok_or_else(|| MtpError::Protocol("DeviceInfo".into()))?;
    match s.transact(OP_OPEN_SESSION, &[1])? {
        (RC_OK, _) | (RC_SESSION_ALREADY_OPEN, _) => s.in_session = true,
        (rc, _) => return Err(MtpError::Protocol(format!("OpenSession {rc:#06x}"))),
    }
    let storages = match s.data(OP_GET_STORAGE_IDS, &[]) {
        Ok(d) => match parse_storage_ids(&d) {
            Some(ids) => {
                let mut list = Vec::new();
                for id in ids {
                    if let Ok(d) = s.data(OP_GET_STORAGE_INFO, &[id]) {
                        if let Some(st) = parse_storage_info(id, &d) {
                            list.push(st);
                        }
                    }
                }
                Probe::Ok(list)
            }
            None => Probe::Failed(tr("invalid storage list").into()),
        },
        Err(e) => e.to_probe(),
    };
    let mut prop = |code: u16| -> Probe<Vec<u8>> {
        if !info.supports_property(code) {
            return Probe::Missing;
        }
        match s.data(OP_GET_DEVICE_PROP_VALUE, &[code as u32]) {
            Ok(d) => Probe::Ok(d),
            Err(e) => e.to_probe(),
        }
    };
    let battery = match prop(PROP_BATTERY_LEVEL) {
        Probe::Ok(d) => parse_uint(&d).filter(|v| *v <= 100).map(|v| Probe::Ok(v as u8)).unwrap_or(Probe::Missing),
        other => other.map(|_| 0),
    };
    let friendly_name = match prop(PROP_FRIENDLY_NAME) {
        Probe::Ok(d) => parse_string(&d).filter(|s| !s.is_empty()).map(Probe::Ok).unwrap_or(Probe::Missing),
        other => other.map(|_| String::new()),
    };
    let perceived_type = match prop(PROP_PERCEIVED_TYPE) {
        Probe::Ok(d) => parse_uint(&d).map(Probe::Ok).unwrap_or(Probe::Missing),
        other => other.map(|_| 0),
    };
    let _ = s.transact(OP_CLOSE_SESSION, &[]);
    Ok(MtpReport { function, info, battery, friendly_name, perceived_type, storages })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ptp_string(s: &str) -> Vec<u8> {
        if s.is_empty() {
            return vec![0];
        }
        let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        let mut out = vec![units.len() as u8];
        for u in units {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out
    }

    fn arr16(v: &[u16]) -> Vec<u8> {
        let mut out = (v.len() as u32).to_le_bytes().to_vec();
        for x in v {
            out.extend_from_slice(&x.to_le_bytes());
        }
        out
    }

    pub(crate) fn android_device_info() -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&100u16.to_le_bytes());
        d.extend_from_slice(&6u32.to_le_bytes());
        d.extend_from_slice(&100u16.to_le_bytes());
        d.extend(ptp_string("microsoft.com: 1.0; android.com: 1.0;"));
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend(arr16(&[0x1001, 0x1002, 0x1003, 0x1004, 0x1005, 0x1015]));
        d.extend(arr16(&[0x4002, 0x4003]));
        d.extend(arr16(&[PROP_FRIENDLY_NAME, PROP_BATTERY_LEVEL, PROP_PERCEIVED_TYPE]));
        d.extend(arr16(&[]));
        d.extend(arr16(&[0x3000, 0x3801]));
        d.extend(ptp_string("nubia"));
        d.extend(ptp_string("NX809J"));
        d.extend(ptp_string("1.0"));
        d.extend(ptp_string("A1B2C3D4E5F6"));
        d
    }

    #[test]
    fn device_info_android() {
        let info = parse_device_info(&android_device_info()).unwrap();
        assert!(info.is_android());
        assert_eq!(info.manufacturer, "nubia");
        assert_eq!(info.model, "NX809J");
        assert_eq!(info.serial, "A1B2C3D4E5F6");
        assert!(info.supports_property(PROP_BATTERY_LEVEL));
        assert_eq!(info.playback_formats, vec![0x3000, 0x3801]);
    }

    #[test]
    fn truncated_data_is_rejected() {
        let d = android_device_info();
        assert!(parse_device_info(&d[..20]).is_none());
        // Impossible element count: it neither allocates nor reads out of
        // bounds.
        let mut bad = d[..11].to_vec();
        bad.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_device_info(&bad).is_none());
    }

    #[test]
    fn storage_info() {
        let mut d = Vec::new();
        d.extend_from_slice(&3u16.to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend_from_slice(&512_000_000_000u64.to_le_bytes());
        d.extend_from_slice(&123_000_000_000u64.to_le_bytes());
        d.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        d.extend(ptp_string("Internal shared storage"));
        d.extend(ptp_string(""));
        let s = parse_storage_info(0x10001, &d).unwrap();
        assert_eq!(s.capacity, 512_000_000_000);
        assert_eq!(s.free, 123_000_000_000);
        assert_eq!(s.name(), "Internal shared storage");
        assert_eq!(s.kind_label(), "internal");
        assert!(!s.read_only());
        let mut ids = 2u32.to_le_bytes().to_vec();
        ids.extend_from_slice(&0x10001u32.to_le_bytes());
        ids.extend_from_slice(&0x20001u32.to_le_bytes());
        assert_eq!(parse_storage_ids(&ids), Some(vec![0x10001, 0x20001]));
        // Phone locked: empty list.
        assert_eq!(parse_storage_ids(&0u32.to_le_bytes()), Some(vec![]));
    }

    #[test]
    fn property_values() {
        assert_eq!(parse_uint(&[87]), Some(87));
        assert_eq!(parse_uint(&3u32.to_le_bytes()), Some(3));
        assert_eq!(parse_uint(&[]), None);
        assert_eq!(parse_string(&ptp_string("Ana's phone")).as_deref(), Some("Ana's phone"));
    }

    #[test]
    fn command_container() {
        let c = command(OP_GET_STORAGE_INFO, 7, &[0x10001]);
        assert_eq!(c.len(), 16);
        assert_eq!(header(&c), Some((16, CT_COMMAND, OP_GET_STORAGE_INFO)));
        assert_eq!(&c[8..12], &7u32.to_le_bytes());
    }

    #[test]
    fn ioctl_numbers_match_linux() {
        // Values of <linux/usbdevice_fs.h> on 64-bit.
        if std::mem::size_of::<usize>() == 8 {
            assert_eq!(USBDEVFS_BULK, 0xC018_5502);
        }
        assert_eq!(USBDEVFS_CLAIMINTERFACE, 0x8004_550F);
        assert_eq!(USBDEVFS_RELEASEINTERFACE, 0x8004_5510);
    }

    #[test]
    fn gvfs_mount_matching() {
        let dev = UsbDevice { serial: "A1B2C3D4E5F6".into(), busnum: "2".into(), devnum: "12".into(), ..Default::default() };
        let list = "Drive(0): WDC\n  Volume(0): x\nMount(0): REDMAGIC 11S Pro -> mtp://nubia_REDMAGIC_11S_Pro_A1B2C3D4E5F6/\n  Type: GProxyShadowMount\n";
        assert_eq!(find_gvfs_uri(list, &dev).as_deref(), Some("mtp://nubia_REDMAGIC_11S_Pro_A1B2C3D4E5F6/"));
        let old = "Mount(0): Phone -> mtp://[usb:002,012]/\n";
        assert_eq!(find_gvfs_uri(old, &dev).as_deref(), Some("mtp://[usb:002,012]/"));
        assert_eq!(find_gvfs_uri("Mount(0): Otro -> mtp://Google_Pixel_ABCD1234/\n", &dev), None);
        assert_eq!(find_gvfs_uri("Mount(0): Disco -> file:///media/x\n", &dev), None);
    }

    #[test]
    fn gio_outputs() {
        assert_eq!(parse_gio_list("Internal shared storage\nSD card\n"), vec!["Internal shared storage", "SD card"]);
        let info = "attributes:\n  filesystem::size: 512000000000\n  filesystem::free: 1000\n  filesystem::type: mtpfs\n";
        assert_eq!(parse_gio_fs(info), (Some(512_000_000_000), Some(1000)));
        assert_eq!(parse_gio_fs("attributes:\n"), (None, None));
    }
}
