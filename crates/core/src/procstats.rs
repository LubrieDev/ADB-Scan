//! Memory per app over the last hours (`dumpsys procstats --hours 24`,
//! read-only): how long each app stayed in memory and how much it held
//! (resident memory, RSS). Apps that sit in memory all day with hundreds of
//! megabytes are what pushes the others out and makes the phone reload them.
//!
//! Android keeps these statistics since the last start or the last time it
//! saved them (`Run time Stats`, the hours covered). PSS is often not sampled
//! (it shows `0.00`); RSS always is.

/// Read-only. Only the per-process totals and the hours covered.
pub const SCRIPT: &str = "dumpsys procstats --hours 24 2>&1 | grep -E '^  \\* |^ +TOTAL: |^ +S(On|Off)/'";

/// Memory of one app (its processes added up).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppMemory {
    pub package: String,
    /// Share of the period it stayed in memory, 0–100 (the most of its
    /// processes).
    pub time_pct: f64,
    /// Average resident memory while in memory, bytes (its processes added
    /// up).
    pub avg_rss: u64,
    pub max_rss: u64,
    pub processes: usize,
}

impl AppMemory {
    /// Average memory held over the whole period.
    pub fn weight(&self) -> f64 {
        self.avg_rss as f64 * self.time_pct / 100.0
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemoryReport {
    /// Time the statistics cover, ms.
    pub period_ms: u64,
    /// Most memory held first.
    pub apps: Vec<AppMemory>,
}

/// `473MB`, `12KB`, `1.2GB` → bytes.
fn size(text: &str) -> Option<u64> {
    let t = text.trim();
    let split = t.find(|c: char| c.is_ascii_alphabetic())?;
    let (num, unit) = t.split_at(split);
    let n: f64 = num.parse().ok()?;
    let mult = match unit {
        "B" => 1.0,
        "KB" | "K" => 1024.0,
        "MB" | "M" => 1024.0 * 1024.0,
        "GB" | "G" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((n * mult) as u64)
}

/// `TOTAL: 100% (pss min-avg-max/uss …/rss 473MB-474MB-505MB over 6)` →
/// (percent, rss avg, rss max).
fn parse_total(text: &str) -> Option<(f64, Option<(u64, u64)>)> {
    let rest = text.trim().strip_prefix("TOTAL:")?.trim();
    let pct: f64 = rest.split('%').next()?.trim().parse().ok()?;
    let rss = rest.split_once('(').and_then(|(_, r)| {
        let inner = r.split(" over").next()?;
        let rss = inner.split('/').nth(2)?;
        let v: Vec<&str> = rss.split('-').collect();
        if v.len() != 3 {
            return None;
        }
        Some((size(v[1])?, size(v[2])?))
    });
    Some((pct, rss))
}

pub fn parse(text: &str) -> MemoryReport {
    let mut report = MemoryReport::default();
    let mut current: Option<String> = None;
    let mut by_pkg: Vec<AppMemory> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        // `SOff/Norm: +1h16m34s374ms`
        if l.starts_with("SOn/") || l.starts_with("SOff/") {
            if let Some((_, v)) = l.split_once(": ") {
                report.period_ms += crate::battery_usage::parse_compact_duration_ms(v).unwrap_or(0);
            }
            continue;
        }
        // `* com.app:remote / u0a468 / v160000:`
        if let Some(rest) = l.strip_prefix("* ") {
            let name = rest.split(" / ").next().unwrap_or("").trim();
            current = Some(name.split(':').next().unwrap_or(name).to_string());
            continue;
        }
        let Some(pkg) = current.take() else { continue };
        let Some((pct, rss)) = parse_total(l) else { continue };
        let (avg, max) = rss.unwrap_or((0, 0));
        match by_pkg.iter_mut().find(|a| a.package == pkg) {
            Some(a) => {
                a.time_pct = a.time_pct.max(pct);
                a.avg_rss += avg;
                a.max_rss += max;
                a.processes += 1;
            }
            None => by_pkg.push(AppMemory { package: pkg, time_pct: pct, avg_rss: avg, max_rss: max, processes: 1 }),
        }
    }
    by_pkg.retain(|a| a.avg_rss > 0);
    by_pkg.sort_by(|a, b| b.weight().total_cmp(&a.weight()));
    report.apps = by_pkg;
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    // As Android 16 prints it after the filter (sample packages).
    const OUT: &str = "  * com.android.systemui / u0a468 / v160000:
         TOTAL: 100% (0.00-0.00-0.00/0.00-0.00-0.00/473MB-474MB-505MB over 6)
  * com.example.chat / u0a543 / v1:
         TOTAL: 40% (0.00-0.00-0.00/0.00-0.00-0.00/200MB-300MB-350MB over 4)
  * com.example.chat:push / u0a543 / v1:
         TOTAL: 100% (0.00-0.00-0.00/0.00-0.00-0.00/40MB-50MB-60MB over 7)
  * com.example.tiny / u0a600 / v3:
         TOTAL: 0.27%
  SOff/Norm: +1h16m34s374ms
   SOn/Norm: +3h3m50s512ms
";

    #[test]
    fn memory_per_app() {
        let r = parse(OUT);
        assert_eq!(r.period_ms, (3600 + 16 * 60 + 34) * 1000 + 374 + (3 * 3600 + 3 * 60 + 50) * 1000 + 512);
        assert_eq!(r.apps.len(), 2);
        assert_eq!(r.apps[0].package, "com.android.systemui");
        assert_eq!(r.apps[0].avg_rss, 474 * 1024 * 1024);
        let chat = &r.apps[1];
        assert_eq!((chat.processes, chat.time_pct, chat.avg_rss), (2, 100.0, 350 * 1024 * 1024));
        assert_eq!(size("1.5GB"), Some(1_610_612_736));
        assert!(SCRIPT.starts_with("dumpsys procstats"));
    }
}
