//! Windows integration: start with the session (registry), restart, the log
//! file and opening a console window. The Linux equivalents live where they
//! are used (`.desktop` autostart, `sh` restart, the VTE terminal).

use std::ffi::c_void;
use std::io;
use std::path::Path;
use std::process::Command;

use adbscan_core::paths;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};
use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, CREATE_NEW_CONSOLE, PROCESS_SYNCHRONIZE};

/// Programs that start with the user's session.
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "ADB Scan";

/// Variable with the pid of the instance that asked for a restart.
pub const WAIT_PID_VAR: &str = "ADB_SCAN_WAIT_PID";

/// The log file is started again when it grows past this size.
const MAX_LOG_BYTES: u64 = 1024 * 1024;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn check(rc: WIN32_ERROR) -> io::Result<()> {
    if rc == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(rc as i32))
    }
}

/// Whether ADB Scan is in the session's startup programs.
pub fn autostart_enabled() -> bool {
    let (key, name) = (wide(RUN_KEY), wide(RUN_VALUE));
    let mut size = 0u32;
    // SAFETY: NUL-terminated UTF-16 strings; only the size is asked for.
    unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) == ERROR_SUCCESS }
}

/// Adds or removes ADB Scan from the session's startup programs. Without a
/// tray icon on Windows, it starts with its window (no `--hidden`).
pub fn set_autostart(enable: bool) -> io::Result<()> {
    let (key, name) = (wide(RUN_KEY), wide(RUN_VALUE));
    if !enable {
        // SAFETY: NUL-terminated UTF-16 strings.
        let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
        return if rc == ERROR_FILE_NOT_FOUND { Ok(()) } else { check(rc) };
    }
    let exe = std::env::current_exe()?;
    let command = wide(&format!("\"{}\"", exe.display()));
    let bytes = (command.len() * std::mem::size_of::<u16>()) as u32;
    // SAFETY: REG_SZ data with its NUL, of the size given.
    check(unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ, command.as_ptr() as *const c_void, bytes) })
}

/// Restarts: a new instance that waits for this one to exit (ADB Scan is a
/// single instance: started earlier it would hand over to the old one).
pub fn spawn_restart() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    Command::new(exe).env(WAIT_PID_VAR, std::process::id().to_string()).spawn().map(|_| ())
}

/// Start of `main`: if a previous instance asked for this restart, waits for
/// it to exit (10 s at most).
pub fn wait_for_previous_instance() {
    let Some(pid) = std::env::var(WAIT_PID_VAR).ok().and_then(|v| v.parse::<u32>().ok()) else { return };
    // There are no other threads yet: this is the start of `main`.
    std::env::remove_var(WAIT_PID_VAR);
    // SAFETY: the handle is checked and closed.
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !h.is_null() {
            WaitForSingleObject(h, 10_000);
            CloseHandle(h);
        }
    }
}

/// `%LOCALAPPDATA%\adb-scan\adb-scan.log`, to append to; it starts again when
/// it is too big.
pub fn open_log_file() -> Option<std::fs::File> {
    let path = paths::log_file();
    std::fs::create_dir_all(path.parent()?).ok()?;
    let too_big = std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES);
    std::fs::OpenOptions::new().create(true).append(!too_big).write(true).truncate(too_big).open(&path).ok()
}

/// Opens `program` in a console window of its own, with `tools_dir` first in
/// the PATH and the user's folder as the working directory.
pub fn open_console(program: &Path, args: &[&str], tools_dir: Option<&Path>) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new(program);
    cmd.args(args).creation_flags(CREATE_NEW_CONSOLE);
    if let Some(dir) = tools_dir {
        let mut dirs = vec![dir.to_path_buf()];
        if let Some(path) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&path));
        }
        if let Ok(joined) = std::env::join_paths(dirs) {
            cmd.env("PATH", joined);
        }
    }
    if let Some(home) = paths::home_dir() {
        cmd.current_dir(home);
    }
    cmd.spawn().map(|_| ())
}
