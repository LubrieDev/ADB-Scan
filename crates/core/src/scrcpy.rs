//! scrcpy: screen mirroring, recording and control.
//!
//! The official binary is launched with arguments built and validated here. The
//! clipboard is synchronised natively by scrcpy.

use crate::{i18n::tr, t};
use std::path::{Path, PathBuf};

use crate::adb::{find_tool, tool_version, Tool};
use crate::error::{Error, Result};
use crate::exec::{self, StreamEvent, StreamHandle, StreamOptions};
use crate::settings::ScrcpyPreset;

#[derive(Debug, Clone)]
pub struct Scrcpy {
    path: PathBuf,
    /// (major, minor) if the version could be read.
    pub version: Option<(u32, u32)>,
}

#[derive(Debug, Clone, Default)]
pub struct SessionOptions {
    pub serial: String,
    pub title: String,
    /// .mp4/.mkv file to record to.
    pub record: Option<PathBuf>,
    /// Record only, without a window.
    pub record_only: bool,
    /// Time limit in seconds (0 = no limit).
    pub time_limit: u32,
    /// Use the camera instead of the screen (scrcpy ≥ 2.2).
    pub camera: bool,
    /// Send the video to a v4l2loopback virtual camera (`/dev/videoN`).
    pub v4l2_sink: Option<String>,
}

pub fn parse_version(text: &str) -> Option<(u32, u32)> {
    let word = text.split_whitespace().find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    let mut parts = word.split('.');
    Some((parts.next()?.parse().ok()?, parts.next().unwrap_or("0").trim_end_matches(|c: char| !c.is_ascii_digit()).parse().unwrap_or(0)))
}

impl Scrcpy {
    pub fn locate(configured: Option<&str>) -> Result<Self> {
        let path = find_tool(Tool::Scrcpy, configured).ok_or(Error::ToolNotFound { tool: "scrcpy".into() })?;
        let version = tool_version(&path, Tool::Scrcpy).and_then(|v| parse_version(&v));
        Ok(Self { path, version })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn at_least(&self, major: u32, minor: u32) -> bool {
        self.version.map_or(true, |v| v >= (major, minor))
    }

    pub fn build_args(&self, preset: &ScrcpyPreset, opts: &SessionOptions) -> Result<Vec<String>> {
        if opts.serial.is_empty() {
            return Err(Error::Invalid(tr("no device selected").into()));
        }
        let mut a = vec!["-s".to_string(), opts.serial.clone()];
        if !opts.title.is_empty() {
            a.push(format!("--window-title={}", ascii_title(&opts.title)));
        }
        if preset.max_size > 0 {
            a.push(format!("--max-size={}", preset.max_size.clamp(240, 8192)));
        }
        if preset.max_fps > 0 {
            a.push(format!("--max-fps={}", preset.max_fps.clamp(1, 240)));
        }
        if preset.bitrate_mbps > 0 {
            a.push(format!("--video-bit-rate={}M", preset.bitrate_mbps.clamp(1, 200)));
        }
        match preset.codec.as_str() {
            "h264" | "" => {}
            c @ ("h265" | "av1") => a.push(format!("--video-codec={c}")),
            other => return Err(Error::Invalid(t!("invalid codec: {}", other))),
        }
        if !preset.audio && self.at_least(2, 0) {
            a.push("--no-audio".into());
        }
        if preset.stay_awake {
            a.push("--stay-awake".into());
        }
        if preset.turn_screen_off {
            a.push("--turn-screen-off".into());
        }
        if preset.show_touches {
            a.push("--show-touches".into());
        }
        if preset.always_on_top {
            a.push("--always-on-top".into());
        }
        if preset.fullscreen {
            a.push("--fullscreen".into());
        }
        if preset.borderless {
            a.push("--window-borderless".into());
        }
        if preset.no_control {
            a.push("--no-control".into());
        }
        if preset.keyboard_uhid && self.at_least(2, 4) {
            a.push("--keyboard=uhid".into());
        }
        match preset.rotation {
            0 => {}
            r @ (90 | 180 | 270) => {
                if self.at_least(3, 0) {
                    a.push(format!("--orientation={r}"));
                } else {
                    a.push(format!("--lock-video-orientation={}", r / 90));
                }
            }
            _ => return Err(Error::Invalid(tr("invalid rotation").into())),
        }
        if opts.camera {
            a.push("--video-source=camera".into());
        }
        if let Some(dev) = &opts.v4l2_sink {
            if !dev.starts_with("/dev/video") || !dev["/dev/video".len()..].chars().all(|c| c.is_ascii_digit()) {
                return Err(Error::Invalid(tr("invalid video device").into()));
            }
            a.push(format!("--v4l2-sink={dev}"));
        }
        if let Some(rec) = &opts.record {
            let ext = rec.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !["mp4", "mkv"].contains(&ext) {
                return Err(Error::Invalid(tr("the recording must be .mp4 or .mkv").into()));
            }
            a.push(format!("--record={}", rec.display()));
            if opts.record_only {
                a.push(if self.at_least(3, 0) { "--no-window".into() } else { "--no-playback".into() });
            }
        }
        if opts.time_limit > 0 {
            a.push(format!("--time-limit={}", opts.time_limit));
        }
        Ok(a)
    }

    pub fn launch<F>(&self, preset: &ScrcpyPreset, opts: &SessionOptions, adb: Option<&Path>, on_event: F) -> Result<StreamHandle>
    where
        F: Fn(StreamEvent) + Send + Sync + 'static,
    {
        let args = self.build_args(preset, opts)?;
        // scrcpy uses the ADB variable to find adb. On Windows this matters
        // even more: scrcpy's zip carries its own adb.exe, of another version,
        // and two different adb servers keep restarting each other.
        let env = adb.map(|p| vec![("ADB".to_string(), p.display().to_string())]).unwrap_or_default();
        exec::stream(&self.path, &args, StreamOptions { with_stdin: false, env, user_locale: true }, on_event)
    }
}

/// Estimate of a recording's size (MB) from the bitrate.
pub fn estimate_recording_mb(bitrate_mbps: u32, seconds: u32, audio: bool) -> f64 {
    let video = bitrate_mbps as f64 * seconds as f64 / 8.0;
    let audio = if audio { 128.0 / 1000.0 * seconds as f64 / 8.0 } else { 0.0 };
    video + audio
}

/// The PC's v4l2loopback virtual cameras: (`/dev/videoN`, name). Real webcams
/// are excluded. `sys` is `/sys/class/video4linux`.
pub fn v4l2_loopback_devices(sys: &std::path::Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(sys)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let node = e.file_name().to_string_lossy().into_owned();
            let name = std::fs::read_to_string(e.path().join("name")).ok()?.trim().to_string();
            // v4l2loopback hangs off the “v4l2loopback” platform device and is
            // called “Dummy video device” or whatever its card_label says.
            let dev = e.path().join("device");
            let via_module = std::fs::canonicalize(&dev).map(|p| p.to_string_lossy().contains("v4l2loopback")).unwrap_or(false);
            let is_loop = !dev.exists() || via_module || name.to_lowercase().contains("loopback") || name.starts_with("Dummy video device");
            is_loop.then(|| (format!("/dev/{node}"), name))
        })
        .collect();
    out.sort();
    out
}

/// scrcpy (SDL) on X11 stores the title only as Latin-1 text: “·”, “—” or “ñ”
/// come out as “Â·”, “â€””, “Ã±”. It is converted to readable ASCII.
pub fn ascii_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    for c in title.chars() {
        match c {
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            '·' | '—' | '–' | '•' | '|' => out.push('-'),
            'á' | 'à' | 'ä' | 'â' => out.push('a'),
            'é' | 'è' | 'ë' | 'ê' => out.push('e'),
            'í' | 'ì' | 'ï' | 'î' => out.push('i'),
            'ó' | 'ò' | 'ö' | 'ô' => out.push('o'),
            'ú' | 'ù' | 'ü' | 'û' => out.push('u'),
            'Á' | 'À' | 'Ä' | 'Â' => out.push('A'),
            'É' | 'È' | 'Ë' | 'Ê' => out.push('E'),
            'Í' | 'Ì' | 'Ï' | 'Î' => out.push('I'),
            'Ó' | 'Ò' | 'Ö' | 'Ô' => out.push('O'),
            'Ú' | 'Ù' | 'Ü' | 'Û' => out.push('U'),
            'ñ' => out.push('n'),
            'Ñ' => out.push('N'),
            'ç' => out.push('c'),
            'Ç' => out.push('C'),
            c if c.is_whitespace() => out.push(' '),
            _ => {}
        }
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        "ADB Scan".into()
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: Option<(u32, u32)>) -> Scrcpy {
        Scrcpy { path: PathBuf::from("/usr/bin/scrcpy"), version: v }
    }

    #[test]
    fn version_parsing() {
        assert_eq!(parse_version("scrcpy 3.3.1 <https://github.com/Genymobile/scrcpy>"), Some((3, 3)));
        assert_eq!(parse_version("scrcpy 1.25"), Some((1, 25)));
        assert_eq!(parse_version("nada"), None);
    }

    #[test]
    fn args_for_preset() {
        let p = ScrcpyPreset { rotation: 90, codec: "h265".into(), ..ScrcpyPreset::balanced() };
        let o = SessionOptions { serial: "ABC".into(), title: "Café phone".into(), ..Default::default() };
        let a = s(Some((3, 1))).build_args(&p, &o).unwrap();
        assert_eq!(&a[..2], ["-s", "ABC"]);
        assert!(a.contains(&"--max-size=1600".to_string()));
        assert!(a.contains(&"--video-codec=h265".to_string()));
        assert!(a.contains(&"--orientation=90".to_string()));
        assert!(a.contains(&"--window-title=Cafe phone".to_string()));
        assert_eq!(ascii_title("REDMAGIC 11S Pro · Screen — ñ"), "REDMAGIC 11S Pro - Screen - n");
        assert_eq!(ascii_title("📱"), "ADB Scan");
        let o = SessionOptions { serial: "ABC".into(), v4l2_sink: Some("/dev/video9".into()), ..Default::default() };
        assert!(s(Some((3, 1))).build_args(&ScrcpyPreset::balanced(), &o).unwrap().contains(&"--v4l2-sink=/dev/video9".to_string()));
        let bad = SessionOptions { serial: "ABC".into(), v4l2_sink: Some("/tmp/x; rm".into()), ..Default::default() };
        assert!(s(Some((3, 1))).build_args(&ScrcpyPreset::balanced(), &bad).is_err());
        let dir = std::env::temp_dir().join(format!("adbscan-v4l-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("video0/device")).unwrap();
        std::fs::create_dir_all(dir.join("video7")).unwrap();
        std::fs::write(dir.join("video0/name"), "Integrated Camera: Integrated C\n").unwrap();
        std::fs::write(dir.join("video7/name"), "Dummy video device (0x0000)\n").unwrap();
        assert_eq!(v4l2_loopback_devices(&dir), vec![("/dev/video7".to_string(), "Dummy video device (0x0000)".to_string())]);
        let _ = std::fs::remove_dir_all(dir);
        let old = s(Some((1, 25))).build_args(&p, &o).unwrap();
        assert!(old.contains(&"--lock-video-orientation=1".to_string()));
        assert!(!old.contains(&"--no-audio".to_string()));
    }

    #[test]
    fn recording_validation() {
        let p = ScrcpyPreset::balanced();
        let bad = SessionOptions { serial: "A".into(), record: Some("/tmp/x.avi".into()), ..Default::default() };
        assert!(s(None).build_args(&p, &bad).is_err());
        let ok = SessionOptions { serial: "A".into(), record: Some("/tmp/x.mp4".into()), record_only: true, ..Default::default() };
        assert!(s(Some((3, 0))).build_args(&p, &ok).unwrap().contains(&"--no-window".to_string()));
        assert!((estimate_recording_mb(8, 60, false) - 60.0).abs() < 1e-9);
    }
}
