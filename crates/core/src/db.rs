//! Local history in SQLite (WAL mode): sessions, logcat captures and events,
//! plus tables for the audit log, battery samples and security results.

use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

use crate::error::Result;
use crate::models::now_secs;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    serial TEXT NOT NULL,
    model TEXT,
    connected_at REAL NOT NULL,
    disconnected_at REAL,
    last_seen REAL
);
CREATE INDEX IF NOT EXISTS idx_sessions_serial ON sessions(serial);
CREATE INDEX IF NOT EXISTS idx_sessions_connected ON sessions(connected_at DESC);

CREATE TABLE IF NOT EXISTS logcat_sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    serial TEXT NOT NULL,
    model TEXT,
    filename TEXT NOT NULL,
    started_at REAL NOT NULL,
    ended_at REAL,
    lines INTEGER DEFAULT 0,
    filters TEXT DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_logcat_started ON logcat_sessions(started_at DESC);

CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp REAL NOT NULL,
    level TEXT NOT NULL,
    category TEXT NOT NULL,
    message TEXT NOT NULL,
    serial TEXT
);
CREATE INDEX IF NOT EXISTS idx_events_time ON events(timestamp DESC);

-- Log of operations that modify the device.
CREATE TABLE IF NOT EXISTS audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp REAL NOT NULL,
    serial TEXT,
    action TEXT NOT NULL,
    target TEXT,
    ok INTEGER NOT NULL,
    detail TEXT
);
CREATE INDEX IF NOT EXISTS idx_audit_time ON audit(timestamp DESC);

-- Battery samples (at most one per minute) for the history.
CREATE TABLE IF NOT EXISTS battery_samples (
    serial TEXT NOT NULL,
    timestamp REAL NOT NULL,
    level REAL,
    temperature REAL,
    voltage_mv INTEGER,
    charging INTEGER
);
CREATE INDEX IF NOT EXISTS idx_battery_serial_time ON battery_samples(serial, timestamp DESC);

-- Security audit reports (JSON).
CREATE TABLE IF NOT EXISTS security_reports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    serial TEXT NOT NULL,
    timestamp REAL NOT NULL,
    score INTEGER,
    report TEXT NOT NULL
);
"#;

#[derive(Debug, Clone, PartialEq)]
pub struct SessionRecord {
    pub id: i64,
    pub serial: String,
    pub model: String,
    pub connected_at: f64,
    pub disconnected_at: Option<f64>,
    pub last_seen: Option<f64>,
}

/// Phone that has already been through ADB Scan (recognised by its serial
/// number).
#[derive(Debug, Clone, PartialEq)]
pub struct KnownDevice {
    pub serial: String,
    pub model: String,
    pub last_seen: Option<f64>,
    pub sessions: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogcatSessionRecord {
    pub id: i64,
    pub serial: String,
    pub model: String,
    pub filename: String,
    pub started_at: f64,
    pub ended_at: Option<f64>,
    pub lines: i64,
    pub filters: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventRecord {
    pub id: i64,
    pub timestamp: f64,
    pub level: String,
    pub category: String,
    pub message: String,
    pub serial: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditRecord {
    pub id: i64,
    pub timestamp: f64,
    pub serial: Option<String>,
    pub action: String,
    pub target: Option<String>,
    pub ok: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BatterySample {
    pub timestamp: f64,
    pub level: Option<f64>,
    pub temperature: Option<f64>,
    pub voltage_mv: Option<i64>,
    pub charging: bool,
}

pub struct History {
    conn: Mutex<Connection>,
}

const MAX_EVENTS: i64 = 4000;
const MAX_AUDIT: i64 = 10_000;

impl History {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_default() -> Result<Self> {
        Self::open(&crate::paths::database_file())
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(3))?;
        conn.execute_batch(SCHEMA)?;
        let db = Self { conn: Mutex::new(conn) };
        db.close_orphans()?;
        Ok(db)
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        Ok(f(&conn)?)
    }

    /// Closes sessions left open if the app ended abruptly.
    pub fn close_orphans(&self) -> Result<usize> {
        self.with(|c| c.execute("UPDATE sessions SET disconnected_at=COALESCE(last_seen, connected_at) WHERE disconnected_at IS NULL", []))
    }

    // -- sessions ------------------------------------------------------------

    pub fn start_session(&self, serial: &str, model: &str) -> Result<i64> {
        let now = now_secs();
        self.with(|c| {
            let open: Option<i64> = c.query_row("SELECT id FROM sessions WHERE serial=? AND disconnected_at IS NULL ORDER BY id DESC LIMIT 1", [serial], |r| r.get(0)).optional()?;
            if let Some(id) = open {
                c.execute("UPDATE sessions SET last_seen=? WHERE id=?", params![now, id])?;
                return Ok(id);
            }
            c.execute("INSERT INTO sessions(serial, model, connected_at, last_seen) VALUES (?,?,?,?)", params![serial, model, now, now])?;
            Ok(c.last_insert_rowid())
        })
    }

    pub fn touch_session(&self, serial: &str) -> Result<()> {
        let now = now_secs();
        self.with(|c| c.execute("UPDATE sessions SET last_seen=? WHERE serial=? AND disconnected_at IS NULL", params![now, serial]))?;
        Ok(())
    }

    pub fn end_session(&self, serial: &str) -> Result<()> {
        let now = now_secs();
        self.with(|c| c.execute("UPDATE sessions SET disconnected_at=?, last_seen=? WHERE serial=? AND disconnected_at IS NULL", params![now, now, serial]))?;
        Ok(())
    }

    pub fn sessions(&self, limit: u32) -> Result<Vec<SessionRecord>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id, serial, COALESCE(model,''), connected_at, disconnected_at, last_seen FROM sessions ORDER BY connected_at DESC LIMIT ?")?;
            let rows =
                st.query_map([limit], |r| Ok(SessionRecord { id: r.get(0)?, serial: r.get(1)?, model: r.get(2)?, connected_at: r.get(3)?, disconnected_at: r.get(4)?, last_seen: r.get(5)? }))?;
            rows.collect()
        })
    }

    /// What the local history knows about a serial number: last recorded name,
    /// last time seen and number of connections. Only data ADB Scan saved in
    /// previous connections with authorised debugging.
    pub fn known_device(&self, serial: &str) -> Result<Option<KnownDevice>> {
        if serial.is_empty() {
            return Ok(None);
        }
        self.with(|c| {
            c.query_row(
                "SELECT COALESCE((SELECT model FROM sessions WHERE serial=?1 AND COALESCE(model,'')<>'' ORDER BY connected_at DESC, id DESC LIMIT 1),''), \
                 MAX(COALESCE(last_seen, disconnected_at, connected_at)), COUNT(*) FROM sessions WHERE serial=?1",
                [serial],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<f64>>(1)?, r.get::<_, i64>(2)?)),
            )
            .map(|(model, last_seen, sessions)| (sessions > 0).then_some(KnownDevice { serial: serial.to_string(), model, last_seen, sessions }))
        })
    }

    // -- logcat ----------------------------------------------------------------

    pub fn start_logcat(&self, serial: &str, model: &str, filename: &str, filters: &str) -> Result<i64> {
        let now = now_secs();
        self.with(|c| {
            c.execute("INSERT INTO logcat_sessions(serial, model, filename, started_at, filters) VALUES (?,?,?,?,?)", params![serial, model, filename, now, filters])?;
            Ok(c.last_insert_rowid())
        })
    }

    pub fn update_logcat(&self, id: i64, lines: i64, finished: bool) -> Result<()> {
        self.with(|c| {
            if finished {
                c.execute("UPDATE logcat_sessions SET lines=?, ended_at=? WHERE id=?", params![lines, now_secs(), id])
            } else {
                c.execute("UPDATE logcat_sessions SET lines=? WHERE id=?", params![lines, id])
            }
        })?;
        Ok(())
    }

    pub fn logcat_sessions(&self, limit: u32) -> Result<Vec<LogcatSessionRecord>> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT id, serial, COALESCE(model,''), filename, started_at, ended_at, COALESCE(lines,0), COALESCE(filters,'') \
                 FROM logcat_sessions ORDER BY started_at DESC LIMIT ?",
            )?;
            let rows = st.query_map([limit], |r| {
                Ok(LogcatSessionRecord { id: r.get(0)?, serial: r.get(1)?, model: r.get(2)?, filename: r.get(3)?, started_at: r.get(4)?, ended_at: r.get(5)?, lines: r.get(6)?, filters: r.get(7)? })
            })?;
            rows.collect()
        })
    }

    pub fn delete_logcat(&self, id: i64) -> Result<()> {
        self.with(|c| c.execute("DELETE FROM logcat_sessions WHERE id=?", [id]))?;
        Ok(())
    }

    // -- events ----------------------------------------------------------------

    pub fn log_event(&self, category: &str, message: &str, level: &str, serial: Option<&str>) -> Result<()> {
        self.with(|c| {
            c.execute("INSERT INTO events(timestamp, level, category, message, serial) VALUES (?,?,?,?,?)", params![now_secs(), level, category, message, serial])?;
            c.execute("DELETE FROM events WHERE id < (SELECT MAX(id) - ? FROM events)", [MAX_EVENTS])
        })?;
        Ok(())
    }

    pub fn events(&self, limit: u32) -> Result<Vec<EventRecord>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id, timestamp, level, category, message, serial FROM events ORDER BY id DESC LIMIT ?")?;
            let rows = st.query_map([limit], |r| Ok(EventRecord { id: r.get(0)?, timestamp: r.get(1)?, level: r.get(2)?, category: r.get(3)?, message: r.get(4)?, serial: r.get(5)? }))?;
            rows.collect()
        })
    }

    // -- audit ------------------------------------------------------------------

    pub fn audit(&self, serial: Option<&str>, action: &str, target: Option<&str>, ok: bool, detail: Option<&str>) -> Result<()> {
        self.with(|c| {
            c.execute("INSERT INTO audit(timestamp, serial, action, target, ok, detail) VALUES (?,?,?,?,?,?)", params![now_secs(), serial, action, target, ok as i64, detail])?;
            c.execute("DELETE FROM audit WHERE id < (SELECT MAX(id) - ? FROM audit)", [MAX_AUDIT])
        })?;
        Ok(())
    }

    pub fn audit_log(&self, limit: u32) -> Result<Vec<AuditRecord>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id, timestamp, serial, action, target, ok, detail FROM audit ORDER BY id DESC LIMIT ?")?;
            let rows = st.query_map([limit], |r| {
                Ok(AuditRecord { id: r.get(0)?, timestamp: r.get(1)?, serial: r.get(2)?, action: r.get(3)?, target: r.get(4)?, ok: r.get::<_, i64>(5)? != 0, detail: r.get(6)? })
            })?;
            rows.collect()
        })
    }

    // -- battery ----------------------------------------------------------------

    /// Saves a sample if the previous one is more than `min_interval` seconds
    /// old.
    pub fn add_battery_sample(&self, serial: &str, s: &BatterySample, min_interval: f64) -> Result<bool> {
        self.with(|c| {
            let last: Option<f64> = c.query_row("SELECT MAX(timestamp) FROM battery_samples WHERE serial=?", [serial], |r| r.get(0)).optional()?.flatten();
            if last.is_some_and(|t| s.timestamp - t < min_interval) {
                return Ok(false);
            }
            c.execute(
                "INSERT INTO battery_samples(serial, timestamp, level, temperature, voltage_mv, charging) VALUES (?,?,?,?,?,?)",
                params![serial, s.timestamp, s.level, s.temperature, s.voltage_mv, s.charging as i64],
            )?;
            Ok(true)
        })
    }

    pub fn battery_samples(&self, serial: &str, since: f64) -> Result<Vec<BatterySample>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT timestamp, level, temperature, voltage_mv, charging FROM battery_samples WHERE serial=? AND timestamp>=? ORDER BY timestamp")?;
            let rows = st.query_map(params![serial, since], |r| {
                Ok(BatterySample { timestamp: r.get(0)?, level: r.get(1)?, temperature: r.get(2)?, voltage_mv: r.get(3)?, charging: r.get::<_, i64>(4)? != 0 })
            })?;
            rows.collect()
        })
    }

    // -- security ---------------------------------------------------------------

    pub fn save_security_report(&self, serial: &str, score: i64, json: &str) -> Result<i64> {
        self.with(|c| {
            c.execute("INSERT INTO security_reports(serial, timestamp, score, report) VALUES (?,?,?,?)", params![serial, now_secs(), score, json])?;
            Ok(c.last_insert_rowid())
        })
    }

    pub fn security_reports(&self, serial: &str, limit: u32) -> Result<Vec<(i64, f64, i64, String)>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id, timestamp, score, report FROM security_reports WHERE serial=? ORDER BY timestamp DESC LIMIT ?")?;
            let rows = st.query_map(params![serial, limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect()
        })
    }

    /// Deletes a device's saved security reports; returns how many there were.
    pub fn delete_security_reports(&self, serial: &str) -> Result<usize> {
        self.with(|c| c.execute("DELETE FROM security_reports WHERE serial=?", [serial]))
    }

    // -- maintenance ------------------------------------------------------------

    pub fn prune(&self, days: u32) -> Result<()> {
        let cutoff = now_secs() - days as f64 * 86_400.0;
        self.with(|c| {
            c.execute("DELETE FROM sessions WHERE connected_at < ?", [cutoff])?;
            c.execute("DELETE FROM logcat_sessions WHERE started_at < ?", [cutoff])?;
            c.execute("DELETE FROM events WHERE timestamp < ?", [cutoff])?;
            c.execute("DELETE FROM battery_samples WHERE timestamp < ?", [cutoff])?;
            c.execute("DELETE FROM audit WHERE timestamp < ?", [cutoff])
        })?;
        Ok(())
    }

    pub fn clear(&self, include_logcat: bool) -> Result<()> {
        self.with(|c| {
            c.execute_batch("DELETE FROM sessions; DELETE FROM events; DELETE FROM battery_samples; DELETE FROM audit;")?;
            if include_logcat {
                c.execute("DELETE FROM logcat_sessions", [])?;
            }
            Ok(())
        })
    }
}

/// Spreads samples over `buckets` equal bands between `since` and `until`
/// (average level and temperature per band; NaN if the band is empty). That way
/// an index-based chart stays proportional to time and the gaps (phone
/// disconnected) show up as breaks in the line.
pub fn bucket_samples(samples: &[BatterySample], since: f64, until: f64, buckets: usize) -> (Vec<f64>, Vec<f64>) {
    let n = buckets.max(1);
    let span = (until - since).max(1.0);
    let mut sums = vec![(0.0f64, 0usize, 0.0f64, 0usize); n];
    for s in samples {
        if s.timestamp < since || s.timestamp > until {
            continue;
        }
        let i = (((s.timestamp - since) / span) * n as f64).floor().min((n - 1) as f64) as usize;
        if let Some(l) = s.level.filter(|v| v.is_finite()) {
            sums[i].0 += l;
            sums[i].1 += 1;
        }
        if let Some(t) = s.temperature.filter(|v| v.is_finite()) {
            sums[i].2 += t;
            sums[i].3 += 1;
        }
    }
    let avg = |sum: f64, k: usize| if k == 0 { f64::NAN } else { sum / k as f64 };
    (sums.iter().map(|b| avg(b.0, b.1)).collect(), sums.iter().map(|b| avg(b.2, b.3)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_time_proportional() {
        let s = |t: f64, l: f64| BatterySample { timestamp: t, level: Some(l), temperature: Some(30.0), voltage_mv: None, charging: false };
        // Samples at the beginning and end of an hour; a gap in between.
        let samples = vec![s(0.0, 50.0), s(60.0, 52.0), s(3540.0, 80.0)];
        let (levels, temps) = bucket_samples(&samples, 0.0, 3600.0, 30);
        assert_eq!(levels.len(), 30);
        assert_eq!(levels[0], 51.0);
        assert!(levels[10].is_nan(), "the gap stays empty");
        assert_eq!(levels[29], 80.0);
        assert_eq!(temps[0], 30.0);
    }

    #[test]
    fn sessions_lifecycle() {
        let h = History::in_memory().unwrap();
        let id = h.start_session("S1", "Pixel").unwrap();
        assert_eq!(h.start_session("S1", "Pixel").unwrap(), id, "does not duplicate open sessions");
        h.touch_session("S1").unwrap();
        h.end_session("S1").unwrap();
        let s = h.sessions(10).unwrap();
        assert_eq!(s.len(), 1);
        assert!(s[0].disconnected_at.is_some());
        assert_ne!(h.start_session("S1", "Pixel").unwrap(), id);
    }

    #[test]
    fn known_device_by_serial() {
        let h = History::in_memory().unwrap();
        assert_eq!(h.known_device("S1").unwrap(), None);
        assert_eq!(h.known_device("").unwrap(), None);
        h.start_session("S1", "NX809J").unwrap();
        h.end_session("S1").unwrap();
        h.start_session("S1", "REDMAGIC 11S Pro").unwrap();
        h.start_session("S2", "Pixel").unwrap();
        let k = h.known_device("S1").unwrap().unwrap();
        assert_eq!(k.sessions, 2);
        assert_eq!(k.model, "REDMAGIC 11S Pro", "the most recent name");
        assert!(k.last_seen.is_some());
    }

    #[test]
    fn security_reports_are_deleted_per_device() {
        let h = History::in_memory().unwrap();
        h.save_security_report("S1", 84, "{}").unwrap();
        h.save_security_report("S1", 90, "{}").unwrap();
        h.save_security_report("S2", 70, "{}").unwrap();
        assert_eq!(h.delete_security_reports("S1").unwrap(), 2);
        assert!(h.security_reports("S1", 10).unwrap().is_empty());
        // Those of another phone are kept.
        assert_eq!(h.security_reports("S2", 10).unwrap().len(), 1);
        assert_eq!(h.delete_security_reports("S1").unwrap(), 0);
    }

    #[test]
    fn events_are_bounded() {
        let h = History::in_memory().unwrap();
        for i in 0..(MAX_EVENTS + 50) {
            h.log_event("t", &format!("e{i}"), "info", None).unwrap();
        }
        assert!(h.events(10_000).unwrap().len() as i64 <= MAX_EVENTS + 1);
    }

    #[test]
    fn logcat_and_audit() {
        let h = History::in_memory().unwrap();
        let id = h.start_logcat("S", "M", "/tmp/x.log", "level:W").unwrap();
        h.update_logcat(id, 42, true).unwrap();
        let l = h.logcat_sessions(5).unwrap();
        assert_eq!(l[0].lines, 42);
        assert!(l[0].ended_at.is_some());
        h.audit(Some("S"), "uninstall", Some("com.x.y"), false, Some("Failure")).unwrap();
        let a = h.audit_log(5).unwrap();
        assert_eq!(a[0].action, "uninstall");
        assert!(!a[0].ok);
    }

    #[test]
    fn battery_sampling_interval() {
        let h = History::in_memory().unwrap();
        let s = |t: f64| BatterySample { timestamp: t, level: Some(50.0), temperature: Some(30.0), voltage_mv: Some(4000), charging: true };
        assert!(h.add_battery_sample("S", &s(1000.0), 60.0).unwrap());
        assert!(!h.add_battery_sample("S", &s(1030.0), 60.0).unwrap());
        assert!(h.add_battery_sample("S", &s(1061.0), 60.0).unwrap());
        assert_eq!(h.battery_samples("S", 0.0).unwrap().len(), 2);
    }

    #[test]
    fn reads_python_database_if_present() {
        // If the real history database exists, it must open without a
        // migration.
        let real = crate::paths::database_file();
        if real.exists() {
            let copy = std::env::temp_dir().join(format!("adbscan-db-{}.db", std::process::id()));
            std::fs::copy(&real, &copy).unwrap();
            let h = History::open(&copy).unwrap();
            h.sessions(5).unwrap();
            h.events(5).unwrap();
            let _ = std::fs::remove_file(copy);
        }
    }
}
