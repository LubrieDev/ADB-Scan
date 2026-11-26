//! Event-based ADB device monitor (`adb track-devices -l`).
//!
//! The protocol sends `LLLL<data>` messages (length as 4 hexadecimal digits),
//! and an empty list is `0000` without a newline, so it is read in blocks, not
//! lines. If the stream fails, `adb devices` is polled every 2 s and the stream
//! is retried with an increasing backoff.

use crate::{i18n::tr, t};
use std::io::Read;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::adb::Adb;
use crate::exec;
use crate::models::AdbDevice;
use crate::parsers;

#[derive(Debug, Clone)]
pub enum MonitorEvent {
    Devices(Vec<AdbDevice>),
    /// Recoverable problem (ADB server down, retries…).
    Problem(String),
}

/// Extracts complete messages from an ADB protocol buffer.
pub fn drain_messages(buf: &mut Vec<u8>) -> Vec<String> {
    let mut out = Vec::new();
    loop {
        if buf.len() < 4 {
            break;
        }
        let Ok(hex) = std::str::from_utf8(&buf[..4]) else {
            buf.clear();
            break;
        };
        let Ok(len) = usize::from_str_radix(hex, 16) else {
            // Out of sync: everything is discarded and the next message is
            // awaited.
            buf.clear();
            break;
        };
        if buf.len() < 4 + len {
            break;
        }
        let payload = String::from_utf8_lossy(&buf[4..4 + len]).into_owned();
        buf.drain(..4 + len);
        out.push(payload);
    }
    out
}

pub struct DeviceMonitor {
    stop: Arc<AtomicBool>,
    refresh: Arc<AtomicU32>,
    child_pid: Arc<Mutex<Option<u32>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl DeviceMonitor {
    pub fn start<F>(adb: Adb, on_event: F) -> Self
    where
        F: Fn(MonitorEvent) + Send + Sync + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let refresh = Arc::new(AtomicU32::new(0));
        let child_pid = Arc::new(Mutex::new(None));
        let (s, r, c) = (stop.clone(), refresh.clone(), child_pid.clone());
        let handle = thread::Builder::new().name("adb-monitor".into()).spawn(move || run(adb, s, r, c, Arc::new(on_event))).expect("monitor thread");
        Self { stop, refresh, child_pid, handle: Some(handle) }
    }

    /// Forces an immediate `adb devices` query.
    pub fn refresh(&self) {
        self.refresh.fetch_add(1, Ordering::SeqCst);
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(pid) = self.child_pid.lock().unwrap().take() {
            // A child of ours with its own group.
            exec::terminate(pid);
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for DeviceMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

fn poll_once(adb: &Adb, on_event: &(dyn Fn(MonitorEvent) + Send + Sync), last: &mut Option<Vec<AdbDevice>>) {
    match adb.devices() {
        Ok(d) => {
            if last.as_ref() != Some(&d) {
                *last = Some(d.clone());
                on_event(MonitorEvent::Devices(d));
            }
        }
        Err(e) => on_event(MonitorEvent::Problem(e.to_string())),
    }
}

fn run(adb: Adb, stop: Arc<AtomicBool>, refresh: Arc<AtomicU32>, child_pid: Arc<Mutex<Option<u32>>>, on_event: Arc<dyn Fn(MonitorEvent) + Send + Sync>) {
    let mut last: Option<Vec<AdbDevice>> = None;
    let _ = adb.start_server();
    poll_once(&adb, &*on_event, &mut last);
    let mut backoff = Duration::from_secs(2);
    let mut seen_refresh = refresh.load(Ordering::SeqCst);

    while !stop.load(Ordering::SeqCst) {
        let mut cmd = exec::base_command(adb.path());
        cmd.args(["track-devices", "-l"]).stdout(Stdio::piped()).stderr(Stdio::null());
        let started = Instant::now();
        match exec::spawn(cmd) {
            Ok(mut child) => {
                *child_pid.lock().unwrap() = Some(child.id());
                let mut stdout = child.stdout.take().expect("stdout");
                // Reader in another thread; this thread handles refreshes and
                // stopping.
                let (tx, rx) = std::sync::mpsc::channel::<Option<Vec<u8>>>();
                let reader = thread::spawn(move || {
                    let mut chunk = [0u8; 4096];
                    loop {
                        match stdout.read(&mut chunk) {
                            Ok(0) | Err(_) => {
                                let _ = tx.send(None);
                                break;
                            }
                            Ok(n) => {
                                if tx.send(Some(chunk[..n].to_vec())).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
                let mut buf = Vec::new();
                loop {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let r = refresh.load(Ordering::SeqCst);
                    if r != seen_refresh {
                        seen_refresh = r;
                        last = None;
                        poll_once(&adb, &*on_event, &mut last);
                    }
                    match rx.recv_timeout(Duration::from_millis(200)) {
                        Ok(Some(bytes)) => {
                            buf.extend_from_slice(&bytes);
                            for msg in drain_messages(&mut buf) {
                                let devices = parsers::parse_devices(&msg);
                                if last.as_ref() != Some(&devices) {
                                    last = Some(devices.clone());
                                    on_event(MonitorEvent::Devices(devices));
                                }
                            }
                        }
                        Ok(None) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
                // Our child (own group).
                exec::terminate(child.id());
                let _ = child.wait();
                let _ = reader.join();
                *child_pid.lock().unwrap() = None;
            }
            Err(e) => on_event(MonitorEvent::Problem(t!("Could not start adb track-devices: {}", e))),
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if started.elapsed() > Duration::from_secs(30) {
            backoff = Duration::from_secs(2);
        }
        on_event(MonitorEvent::Problem(tr("The connection with the ADB server was lost. Retrying…").into()));
        // Meanwhile, fallback polling.
        let until = Instant::now() + backoff;
        while Instant::now() < until && !stop.load(Ordering::SeqCst) {
            let _ = adb.start_server();
            poll_once(&adb, &*on_event, &mut last);
            thread::sleep(Duration::from_secs(2).min(backoff));
        }
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_framing() {
        let mut buf = b"0000".to_vec();
        buf.extend_from_slice(b"001eSERIAL\tdevice usb:2-3 model:X\n");
        buf.extend_from_slice(b"00");
        let msgs = drain_messages(&mut buf);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0], "");
        assert!(msgs[1].starts_with("SERIAL\tdevice"));
        assert_eq!(buf, b"00", "an incomplete message is kept");
        buf.extend_from_slice(b"00");
        assert_eq!(drain_messages(&mut buf), vec![String::new()]);
    }

    #[test]
    fn garbage_resyncs() {
        let mut buf = b"zzzz123".to_vec();
        assert!(drain_messages(&mut buf).is_empty());
        assert!(buf.is_empty());
    }
}
