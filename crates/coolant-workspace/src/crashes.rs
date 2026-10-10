//! Programs that keep crashing. Every crash makes `ReportCrash` symbolicate
//! and write a report (often seconds of CPU), and a service that launchd
//! restarts crashes again: on one test machine three crash-looping Homebrew
//! services cost 20 CPU-s of ReportCrash in five minutes.
//!
//! Reads the crash reports macOS keeps in `~/Library/Logs/DiagnosticReports`
//! (and the system-wide folder): no privileges needed.

use crate::config::home;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Crashes within [`WINDOW`] before a program counts as crash-looping.
pub const THRESHOLD: usize = 10;
pub const WINDOW: Duration = Duration::from_secs(24 * 3600);

/// One crash report.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub name: String,
    /// The launchd job that started it (e.g. `homebrew.mxcl.php`), if any.
    pub label: Option<String>,
    /// Part of macOS rather than something the user installed.
    pub system: bool,
    /// Signal or termination reason, e.g. "SIGABRT" or "Library missing".
    pub reason: String,
    /// When it crashed, from the report's header. (The file's modification
    /// time is no good: macOS rewrites old reports.)
    pub at: Option<SystemTime>,
    /// Identifies the crash across copies of its report.
    pub incident: String,
}

/// A program that crashed at least [`THRESHOLD`] times.
#[derive(Debug, Clone, PartialEq)]
pub struct Crashing {
    pub name: String,
    pub count: usize,
    pub label: Option<String>,
    pub system: bool,
    pub reason: String,
    pub last: SystemTime,
}

impl Crashing {
    /// The Homebrew formula, for services started with `brew services`.
    pub fn brew_formula(&self) -> Option<&str> {
        self.label.as_deref()?.strip_prefix("homebrew.mxcl.")
    }
}

/// Programs that crashed at least [`THRESHOLD`] times in the last [`WINDOW`],
/// most crashes first.
pub fn recent() -> Vec<Crashing> {
    let now = SystemTime::now();
    let mut reports = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in [home().join("Library/Logs/DiagnosticReports"), PathBuf::from("/Library/Logs/DiagnosticReports")] {
        for (path, modified) in ips_files(&dir) {
            // A report is never modified before its crash, so old files can
            // be skipped without reading them.
            if now.duration_since(modified).unwrap_or_default() > WINDOW {
                continue;
            }
            let Some(r) = std::fs::read_to_string(&path).ok().and_then(|t| parse(&t)) else { continue };
            let at = r.at.unwrap_or(modified);
            if now.duration_since(at).unwrap_or_default() > WINDOW
                || (!r.incident.is_empty() && !seen.insert(r.incident.clone()))
            {
                continue;
            }
            reports.push((r, at));
        }
    }
    summarize(reports)
}

fn ips_files(dir: &Path) -> Vec<(PathBuf, SystemTime)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "ips"))
        .filter_map(|e| Some((e.path(), e.metadata().ok()?.modified().ok()?)))
        .collect()
}

/// Parses an `.ips` crash report: a one-line JSON header, then a JSON body.
/// Other report types (hangs, jetsam, …) have no `procName` and are skipped.
pub fn parse(text: &str) -> Option<Report> {
    let (header, body) = text.split_once('\n')?;
    let header: Value = serde_json::from_str(header).ok()?;
    let body: Value = serde_json::from_str(body).ok()?;
    let name = body.get("procName").and_then(Value::as_str).or_else(|| header.get("name")?.as_str())?;
    // A process launchd started belongs to the coalition named after its job.
    let label = (body.get("parentProc").and_then(Value::as_str) == Some("launchd"))
        .then(|| body.get("coalitionName").and_then(Value::as_str))
        .flatten()
        .map(str::to_string);
    let path = body.get("procPath").and_then(Value::as_str).unwrap_or("");
    let system = ["/System/", "/usr/libexec/", "/usr/sbin/", "/usr/bin/", "/sbin/"].iter().any(|p| path.starts_with(p));
    let reason = body
        .pointer("/termination/indicator")
        .or_else(|| body.pointer("/exception/signal"))
        .or_else(|| body.pointer("/exception/type"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let at = header.get("timestamp").and_then(Value::as_str).and_then(parse_timestamp);
    let incident = header.get("incident_id").and_then(Value::as_str).unwrap_or("").to_string();
    Some(Report { name: name.to_string(), label, system, reason, at, incident })
}

/// Parses a report timestamp like "2026-10-10 12:13:30.00 +0400".
fn parse_timestamp(s: &str) -> Option<SystemTime> {
    let mut parts = s.split(' ');
    let (date, time, zone) = (parts.next()?, parts.next()?, parts.next()?);
    let d: Vec<i64> = date.split('-').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    let t: Vec<f64> = time.split(':').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    if d.len() != 3 || t.len() != 3 || zone.len() != 5 {
        return None;
    }
    let sign = if zone.starts_with('-') { -1 } else { 1 };
    let offset = sign * (zone[1..3].parse::<i64>().ok()? * 3600 + zone[3..5].parse::<i64>().ok()? * 60);
    let secs = days_from_civil(d[0], d[1], d[2]) * 86400 + (t[0] * 3600.0 + t[1] * 60.0) as i64 - offset;
    let secs = secs as f64 + t[2];
    (secs >= 0.0).then(|| SystemTime::UNIX_EPOCH + Duration::from_secs_f64(secs))
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's
/// algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn summarize(reports: Vec<(Report, SystemTime)>) -> Vec<Crashing> {
    let mut by_name: BTreeMap<String, Crashing> = BTreeMap::new();
    for (r, at) in reports {
        let c = by_name.entry(r.name.clone()).or_insert_with(|| Crashing {
            name: r.name.clone(),
            count: 0,
            label: None,
            system: r.system,
            reason: String::new(),
            last: SystemTime::UNIX_EPOCH,
        });
        c.count += 1;
        if at >= c.last {
            c.last = at;
            c.reason = r.reason;
        }
        if c.label.is_none() {
            c.label = r.label;
        }
    }
    let mut out: Vec<Crashing> = by_name.into_values().filter(|c| c.count >= THRESHOLD).collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.count));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ips(name: &str, parent: &str, coalition: &str, path: &str) -> String {
        format!(
            "{{\"app_name\":\"{name}\",\"bug_type\":\"309\",\"name\":\"{name}\"}}\n{{\"procName\":\"{name}\",\"procPath\":\"{path}\",\"parentProc\":\"{parent}\",\"coalitionName\":\"{coalition}\",\"exception\":{{\"type\":\"EXC_CRASH\",\"signal\":\"SIGABRT\"}}}}"
        )
    }

    #[test]
    fn parses_a_homebrew_service_crash() {
        let r = parse(&ips("php-fpm", "launchd", "homebrew.mxcl.php", "/opt/homebrew/*/php-fpm")).unwrap();
        assert_eq!(r.name, "php-fpm");
        assert_eq!(r.label.as_deref(), Some("homebrew.mxcl.php"));
        assert!(!r.system);
        assert_eq!(r.reason, "SIGABRT");
    }

    #[test]
    fn crash_time_comes_from_the_header() {
        let at = parse_timestamp("2026-10-10 12:13:30.00 +0400").unwrap();
        // 2026-10-10 08:13:30 UTC
        assert_eq!(at.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(), 1_791_620_010);
        assert_eq!(parse_timestamp("1970-01-01 00:00:00.00 +0000"), Some(SystemTime::UNIX_EPOCH));
        assert_eq!(parse_timestamp("not a date"), None);
        let r = parse(&format!(
            "{{\"name\":\"x\",\"timestamp\":\"2026-10-10 12:13:30.00 +0400\",\"incident_id\":\"ABC\"}}\n{}",
            r#"{"procName":"x"}"#
        ))
        .unwrap();
        assert_eq!(r.at, Some(at));
        assert_eq!(r.incident, "ABC");
    }

    #[test]
    fn a_shell_child_has_no_launchd_label() {
        let r = parse(&ips("rust-objcopy", "zsh", "com.openssh.sshd.X", "/Users/USER/*/rust-objcopy")).unwrap();
        assert_eq!(r.label, None);
    }

    #[test]
    fn non_crash_reports_are_skipped() {
        assert_eq!(parse("{\"bug_type\":\"298\"}\n{\"memoryStatus\":{}}"), None);
        assert_eq!(parse("not json"), None);
    }

    #[test]
    fn only_repeat_crashers_are_reported() {
        let now = SystemTime::now();
        let php = parse(&ips("php-fpm", "launchd", "homebrew.mxcl.php", "/opt/homebrew/php-fpm")).unwrap();
        let once = parse(&ips("swift-frontend", "zsh", "x", "/Library/x")).unwrap();
        let mut reports: Vec<_> = (0..THRESHOLD).map(|_| (php.clone(), now)).collect();
        reports.push((once, now));
        let out = summarize(reports);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].count, THRESHOLD);
        assert_eq!(out[0].brew_formula(), Some("php"));
    }
}
