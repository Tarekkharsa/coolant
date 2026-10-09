//! Renders a [`Run`] for humans and derives findings: concrete, actionable
//! explanations of where the heat came from.

use crate::classify::Category;
use crate::profile::Run;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    High,
    Medium,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fix: String,
}

fn sev(v: f64, high: f64) -> Severity {
    if v >= high { Severity::High } else { Severity::Medium }
}

/// Applies heuristics to a run. Thresholds are in cores: CPU seconds per
/// wall-clock second.
pub fn findings(run: &Run) -> Vec<Finding> {
    let mut fs = Vec::new();
    if run.duration_s <= 0.0 {
        return fs;
    }
    let cores = |c: Category| run.cores(run.category(c).cpu_s);
    let mut push = |severity, title: String, detail: &str, fix: &str| {
        fs.push(Finding { severity, title, detail: detail.into(), fix: fix.into() })
    };

    let c = cores(Category::Indexer);
    if c >= 0.25 {
        push(
            sev(c, 0.5),
            format!("Spotlight indexing is using {c:.2} cores"),
            "mds_stores/mdworker re-index files as agents write them: dependencies, build output, worktrees.",
            "Keep agent worktrees in a folder ending in .noindex, or add them under System Settings > Spotlight > Search Privacy.",
        );
    }
    let c = cores(Category::Security);
    if c >= 0.15 {
        push(
            sev(c, 0.4),
            format!("Security scanning is using {c:.2} cores"),
            "syspolicyd/XProtect/amfid (or an EDR agent) check executables and files as they are created and launched. Agents launch thousands of short processes.",
            "Reduce process churn (long-lived tool servers instead of one process per command); ask IT to exclude agent workspaces from EDR scanning.",
        );
    }
    let c = cores(Category::FsDaemon);
    if c >= 0.15 {
        push(
            sev(c, 0.4),
            format!("File system daemons are using {c:.2} cores"),
            "fseventsd and friends fan out change events to every watcher; many agents plus editors watching the same trees multiply this.",
            "",
        );
    }
    let c = cores(Category::BackupSync);
    if c >= 0.1 {
        push(
            sev(c, 0.3),
            format!("Backup/sync is using {c:.2} cores"),
            "Time Machine or iCloud/Dropbox is processing files the agents changed.",
            "Keep repositories out of ~/Documents and ~/Desktop when iCloud Drive syncs them; exclude workspaces from Time Machine.",
        );
    }
    let rate = run.spawns as f64 / run.duration_s;
    if rate >= 10.0 {
        push(
            sev(rate, 50.0),
            format!("{rate:.0} processes launched per second"),
            "Every launch costs fork/exec and dynamic linking, plus code-signing checks on macOS. Short-lived git/rg/node processes add up.",
            "",
        );
    }
    if !run.orphans.is_empty() {
        let cpu: f64 = run.orphans.iter().map(|o| o.cpu_s).sum();
        let names: Vec<String> = run.orphans.iter().take(5).map(|o| format!("{} (pid {})", o.name, o.pid)).collect();
        push(
            sev(run.cores(cpu), 0.5),
            format!("{} orphaned dev processes using {:.2} cores", run.orphans.len(), run.cores(cpu)),
            &format!(
                "Dev servers, watchers or builds whose parent (often an agent or terminal) is gone: {}",
                names.join(", ")
            ),
            "Stop them if they're leftovers from finished agent sessions.",
        );
    }
    for d in run.duplicates.iter().filter(|d| d.count >= 3) {
        push(
            Severity::Medium,
            format!("{} copies of {} running", d.count, d.name),
            &format!(
                "Together {} of memory and {:.2} cores. Usually one per worktree or editor window.",
                bytes(d.rss_bytes),
                run.cores(d.cpu_s)
            ),
            "",
        );
    }
    for (p, secs) in &run.thermal.pressure_s {
        push(
            Severity::High,
            format!("Thermal pressure was {p} for {secs:.0}s"),
            "macOS was throttling to keep temperatures in check.",
            "",
        );
    }
    fs.sort_by_key(|f| f.severity);
    fs
}

/// A terminal-friendly report.
pub fn text(run: &Run) -> String {
    let mut w = String::new();
    if run.duration_s <= 0.0 {
        return "run too short to report\n".into();
    }
    let m = &run.machine;
    let _ = writeln!(
        w,
        "coolant profile: {} {} ({}, {} cores), {} {}",
        m.model,
        m.cpu,
        bytes(m.mem_bytes),
        m.cores,
        m.os,
        m.os_version
    );
    let _ = writeln!(w, "{:.0}s recorded, {:.2} cores busy on average\n", run.duration_s, run.cores(run.busy_cpu_s));

    let t = &run.thermal;
    if t.cpu_temp_max_c > 0.0 || t.cpu_power_avg_w > 0.0 {
        let _ = write!(
            w,
            "Thermals: CPU {:.0}°C → max {:.0}°C (avg {:.0}°C)",
            t.cpu_temp_start_c, t.cpu_temp_max_c, t.cpu_temp_avg_c
        );
        if t.cpu_power_avg_w > 0.0 {
            let _ = write!(w, ", CPU power avg {:.1} W / max {:.1} W", t.cpu_power_avg_w, t.cpu_power_max_w);
        }
        if t.soc_power_avg_w > 0.0 {
            let _ = write!(w, ", SoC avg {:.1} W", t.soc_power_avg_w);
        }
        if t.fan_max_rpm > 0.0 {
            let _ = write!(w, ", fan max {:.0} rpm", t.fan_max_rpm);
        }
        w.push_str("\n\n");
    }

    let fs = findings(run);
    if !fs.is_empty() {
        w.push_str("Findings\n");
        for f in &fs {
            let sev = match f.severity {
                Severity::High => "high",
                Severity::Medium => "medium",
                Severity::Info => "info",
            };
            let _ = writeln!(w, "  [{sev}] {}\n      {}", f.title, f.detail);
            if !f.fix.is_empty() {
                let _ = writeln!(w, "      fix: {}", f.fix);
            }
        }
        w.push('\n');
    }

    w.push_str("Where the CPU went\n");
    let mut rows = vec![row(["category", "cores", "share", "disk written", "spawns"])];
    for u in &run.categories {
        if u.cpu_s < 0.01 && u.spawns == 0 {
            continue;
        }
        let overhead = Category::ALL.iter().any(|c| c.as_str() == u.key && c.is_overhead());
        rows.push(row([
            format!("{}{}", u.key, if overhead { " *" } else { "" }),
            format!("{:.2}", run.cores(u.cpu_s)),
            format!("{:.0}%", pct(u.cpu_s, run.busy_cpu_s)),
            bytes(u.disk_written),
            u.spawns.to_string(),
        ]));
    }
    rows.push(row([
        "unattributed".into(),
        format!("{:.2}", run.cores(run.unattributed_cpu_s)),
        format!("{:.0}%", pct(run.unattributed_cpu_s, run.busy_cpu_s)),
        String::new(),
        String::new(),
    ]));
    table(&mut w, &rows);
    w.push_str("  * OS overhead triggered by file and process activity\n\n");

    if !run.sessions.is_empty() {
        w.push_str("Agent sessions (agent + everything it launched)\n");
        let mut rows = vec![row(["session", "cores", "disk written", "spawns", "top categories"])];
        for u in &run.sessions {
            rows.push(row([
                u.key.clone(),
                format!("{:.2}", run.cores(u.cpu_s)),
                bytes(u.disk_written),
                u.spawns.to_string(),
                top_categories(&u.by_category, 3),
            ]));
        }
        table(&mut w, &rows);
        w.push('\n');
    }

    w.push_str("Top processes\n");
    let mut rows = vec![row(["pid", "name", "category", "cores", "memory", "session"])];
    for p in run.processes.iter().take(15) {
        let cat = p.category.map(|c| c.as_str()).unwrap_or("");
        rows.push(row([
            p.pid.to_string(),
            trunc(&p.name, 32),
            cat.into(),
            format!("{:.2}", run.cores(p.cpu_s)),
            bytes(p.rss_bytes),
            p.session.clone(),
        ]));
    }
    table(&mut w, &rows);
    for warn in &run.warnings {
        let _ = writeln!(w, "\nwarning: {warn}");
    }
    w
}

fn row<const N: usize, S: Into<String>>(cells: [S; N]) -> Vec<String> {
    cells.into_iter().map(Into::into).collect()
}

/// Writes rows as a table: first column left-aligned, the rest right-aligned.
fn table(w: &mut String, rows: &[Vec<String>]) {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> =
        (0..cols).map(|i| rows.iter().filter_map(|r| r.get(i)).map(|c| c.chars().count()).max().unwrap_or(0)).collect();
    for r in rows {
        w.push_str("  ");
        for (i, c) in r.iter().enumerate() {
            if i == 0 {
                let _ = write!(w, "{c:<width$}", width = widths[i]);
            } else {
                let _ = write!(w, "  {c:>width$}", width = widths[i]);
            }
        }
        w.truncate(w.trim_end().len());
        w.push('\n');
    }
}

fn top_categories(m: &BTreeMap<Category, f64>, n: usize) -> String {
    let total: f64 = m.values().sum();
    let mut v: Vec<_> = m.iter().collect();
    v.sort_by(|a, b| b.1.total_cmp(a.1));
    v.iter().take(n).map(|(c, s)| format!("{c} {:.0}%", pct(**s, total))).collect::<Vec<_>>().join(", ")
}

fn pct(a: f64, b: f64) -> f64 {
    if b > 0.0 { 100.0 * a / b } else { 0.0 }
}

pub fn bytes(n: u64) -> String {
    let f = n as f64;
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", f / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.0} MB", f / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.0} KB", f / 1024.0),
        _ => format!("{n} B"),
    }
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    s.chars().take(n - 1).chain(std::iter::once('…')).collect()
}
