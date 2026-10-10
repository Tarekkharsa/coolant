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
    for dir in [home().join("Library/Logs/DiagnosticReports"), PathBuf::from("/Library/Logs/DiagnosticReports")] {
        for (path, modified) in ips_files(&dir) {
            if now.duration_since(modified).unwrap_or_default() > WINDOW {
                continue;
            }
            if let Some(r) = std::fs::read_to_string(&path).ok().and_then(|t| parse(&t)) {
                reports.push((r, modified));
            }
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
    Some(Report { name: name.to_string(), label, system, reason })
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
