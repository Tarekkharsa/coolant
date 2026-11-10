//! `coolant agent`: what the calling coding agent is doing to the Mac, and
//! whether it should start more heavy work. Meant to be run by the agent.

use anyhow::Result;
use coolant_core::classify::{self, Category, Labeled};
use coolant_core::profile::{Options, Session};
use coolant_core::report::bytes;
use coolant_platform::{MemoryPressure, ThermalPressure};
use coolant_workspace::config::tilde;
use coolant_workspace::host;
use coolant_workspace::inuse::InUse;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Servers younger than this are probably test runs, not leftovers.
const SERVER_MIN_AGE_MS: u64 = 2 * 60 * 1000;

/// A process without a parent has to be this old, and its tree this busy,
/// to be reported as a leftover.
const LEFTOVER_MIN_AGE_MS: u64 = 10 * 60 * 1000;
const LEFTOVER_MIN_CORES: f64 = 0.5;

pub fn run(secs: u64, as_json: bool) -> Result<()> {
    let backend: Arc<dyn coolant_platform::Backend> = Arc::from(coolant_platform::current());
    let mut session = Session::new(backend.clone(), Options::default())?;
    std::thread::sleep(Duration::from_secs(secs.max(1)));
    session.sample();
    let run = session.snapshot();
    let procs = classify::label(backend.processes()?);
    let me = std::process::id() as i32;
    let mine = procs.iter().find(|l| l.process.pid == me).map(|l| l.session.clone()).unwrap_or_default();

    let elapsed = run.duration_s.max(1e-3);
    let busy = run.busy_cpu_s / elapsed;
    let cores = run.machine.cores.max(1) as f64;
    let sn = run.samples.last().map(|s| s.sensors.clone()).unwrap_or_default();

    // This agent: CPU over the sample (with children that came and went),
    // memory of everything it is running now (without this command).
    let ours: Vec<_> = procs
        .iter()
        .filter(|l| !mine.is_empty() && l.session == mine && l.process.pid != me && l.process.ppid != me)
        .collect();
    let agent_cores = run.sessions.iter().find(|u| u.key == mine).map_or(0.0, |u| u.cpu_s / elapsed);
    let agent_rss: u64 = ours.iter().map(|l| l.process.rss_bytes).sum();
    let mut heavy: Vec<_> =
        ours.iter().filter(|l| l.category != Category::Agent && l.process.rss_bytes >= 100 << 20).collect();
    heavy.sort_by_key(|l| std::cmp::Reverse(l.process.rss_bytes));
    heavy.truncate(5);

    // Servers and watchers running in this project, whoever started them.
    let dir = project_dir();
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let users = InUse::scan().users_of(&dir);
    let servers: Vec<_> = procs
        .iter()
        .filter(|l| users.contains(&l.process.pid) && host::is_server(&l.process.command))
        .filter(|l| l.process.start_ms > 0 && now_ms.saturating_sub(l.process.start_ms) >= SERVER_MIN_AGE_MS)
        .map(|l| {
            let who = if !mine.is_empty() && l.session == mine {
                "this agent"
            } else if l.session.is_empty() {
                "no agent (its parent exited)"
            } else {
                "another agent"
            };
            (l.process.pid, host::short_command(&l.process.command), now_ms - l.process.start_ms, who)
        })
        .collect();

    // Busy dev processes nobody is running anymore: they make the Mac busy
    // without being anyone's work.
    let mut cpu_s: HashMap<i32, f64> = run.processes.iter().map(|p| (p.pid, p.cpu_s)).collect();
    // An agent's CPU includes children that came and went: use its session's.
    for u in &run.sessions {
        if let Some(pid) = u.key.rsplit(':').next().and_then(|p| p.parse().ok()) {
            cpu_s.insert(pid, u.cpu_s);
        }
    }
    let left = leftovers(&procs, &cpu_s, &mine, elapsed, now_ms);
    let left_cores: f64 = left.iter().map(|l| l.cores).sum();
    // An agent whose parent exited is a leftover, not another agent at work.
    let working = |key: &str| key != mine && !left.iter().any(|l| l.session == key);
    let others: f64 = run.sessions.iter().filter(|u| working(&u.key)).map(|u| u.cpu_s / elapsed).sum();
    let other_agents = run.sessions.iter().filter(|u| working(&u.key)).count();

    let thermal_bad =
        !matches!(sn.pressure, ThermalPressure::Nominal | ThermalPressure::Moderate | ThermalPressure::Unknown);
    let memory_bad = sn.memory_pressure == MemoryPressure::Critical
        || (sn.memory_pressure == MemoryPressure::Warning && sn.memory_free_pct < 25.0);
    let (verdict, advice) = if thermal_bad || sn.cpu_temp_c >= 85.0 || memory_bad {
        let why = if memory_bad { "short of memory" } else { "hot" };
        (
            "pause",
            format!(
                "The Mac is {why}. Don't start anything heavy until what's running finishes; stop servers and watchers you no longer need."
            ),
        )
    } else if busy >= 0.6 * cores
        || sn.pressure == ThermalPressure::Moderate
        || sn.memory_pressure == MemoryPressure::Warning
        || (sn.memory_free_pct > 0.0 && sn.memory_free_pct < 25.0)
        || sn.cpu_temp_c >= 75.0
    {
        ("busy", "Don't add parallel work: run builds, type checks and tests one at a time.".to_string())
    } else {
        ("ok", "Go ahead. Type checkers and builds take gigabytes each, so still prefer one at a time.".to_string())
    };
    let advice = if left.is_empty() || verdict == "ok" {
        advice
    } else {
        format!(
            "{advice} {left_cores:.1} of the busy cores are leftover processes, not anyone's work: ask the user whether to stop them."
        )
    };

    if as_json {
        let v = json!({
            "verdict": verdict,
            "advice": advice,
            "mac": {
                "busy_cores": round(busy), "cores": run.machine.cores, "cpu_temp_c": round(sn.cpu_temp_c),
                "thermal": sn.pressure.to_string(), "memory": sn.memory_pressure.to_string(),
                "memory_free_pct": round(sn.memory_free_pct), "swap_bytes": sn.swap_used_bytes,
            },
            "agent": (!mine.is_empty()).then(|| json!({
                "session": mine, "cores": round(agent_cores), "memory_bytes": agent_rss, "processes": ours.len(),
                "heaviest": heavy.iter().map(|l| json!({
                    "pid": l.process.pid, "name": l.process.name, "memory_bytes": l.process.rss_bytes,
                })).collect::<Vec<_>>(),
            })),
            "other_agents": { "sessions": other_agents, "cores": round(others) },
            "leftovers": left.iter().map(|l| json!({
                "pid": l.pid, "command": l.command, "running_s": l.age_ms / 1000,
                "cores": round(l.cores), "memory_bytes": l.rss_bytes,
            })).collect::<Vec<_>>(),
            "project": tilde(&dir),
            "servers": servers.iter().map(|(pid, cmd, age, who)| json!({
                "pid": pid, "command": cmd, "running_s": age / 1000, "started_by": who,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }

    println!(
        "Mac: {busy:.1} of {} cores busy · {:.0}°C · thermal {} · memory {} ({:.0}% free) · swap {}",
        run.machine.cores,
        sn.cpu_temp_c,
        sn.pressure,
        sn.memory_pressure,
        sn.memory_free_pct,
        bytes(sn.swap_used_bytes)
    );
    if mine.is_empty() {
        println!("Not running under a coding agent; showing the whole Mac.");
    } else {
        println!("This agent ({mine}): {agent_cores:.1} cores, {} in {} processes", bytes(agent_rss), ours.len());
        if !heavy.is_empty() {
            let list: Vec<String> = heavy
                .iter()
                .map(|l| format!("{} {} (pid {})", l.process.name, bytes(l.process.rss_bytes), l.process.pid))
                .collect();
            println!("  heaviest: {}", list.join(", "));
        }
    }
    if other_agents > 0 {
        println!("Other agent sessions: {other_agents}, {others:.1} cores");
    }
    if !left.is_empty() {
        println!("Leftover processes (their parent exited): {left_cores:.1} cores");
        for l in &left {
            println!(
                "  pid {:<7} {:<30} running {}, {:.1} cores, {}",
                l.pid,
                l.command,
                age_text(l.age_ms),
                l.cores,
                bytes(l.rss_bytes)
            );
        }
    }
    if !servers.is_empty() {
        println!("Servers and watchers in {}:", tilde(&dir));
        for (pid, cmd, age, who) in &servers {
            println!("  pid {pid:<7} {cmd:<30} running {}, started by {who}", age_text(*age));
        }
    }
    println!("Verdict: {verdict}. {advice}");
    Ok(())
}

/// The git worktree this was run from, or the current folder.
fn project_dir() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_default();
    Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&cwd)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .unwrap_or(cwd)
}

/// A dev process or agent whose parent exited, with everything under it.
#[derive(Debug, PartialEq)]
struct Leftover {
    pid: i32,
    command: String,
    age_ms: u64,
    /// Cores the whole tree used over the sample.
    cores: f64,
    rss_bytes: u64,
    /// The agent session this is, when the leftover is an agent.
    session: String,
}

/// Dev processes and agents that were handed to launchd/init when their
/// parent exited, have run for a while and are still busy: a test server
/// left spinning for days, say, or an agent whose terminal is gone. Never
/// the calling agent (`mine`). `cpu_s` is each process's CPU over the
/// sample (an agent's includes its session), which lasted `elapsed`
/// seconds. Busiest first.
fn leftovers(procs: &[Labeled], cpu_s: &HashMap<i32, f64>, mine: &str, elapsed: f64, now_ms: u64) -> Vec<Leftover> {
    let mut children: HashMap<i32, Vec<&Labeled>> = HashMap::new();
    for l in procs {
        children.entry(l.process.ppid).or_default().push(l);
    }
    let mut out: Vec<Leftover> = procs
        .iter()
        .filter(|l| {
            let p = &l.process;
            let dev = matches!(
                l.category,
                Category::Runtime | Category::Build | Category::LanguageServer | Category::PackageManager
            );
            let agent = l.category == Category::Agent && l.session != mine;
            let app = p.command.starts_with("/Applications/") || p.command.starts_with("/System/");
            p.ppid == 1
                && p.detailed
                && (agent || dev && l.session.is_empty())
                && !app
                && p.start_ms > 0
                && now_ms.saturating_sub(p.start_ms) >= LEFTOVER_MIN_AGE_MS
        })
        .filter_map(|root| {
            // The root and everything under it (an agent's session is
            // already counted in the agent's CPU).
            let is_agent = root.category == Category::Agent;
            let (mut cpu, mut rss) = (0.0, 0);
            let mut todo = vec![root];
            let mut seen = 0;
            while let Some(l) = todo.pop() {
                seen += 1;
                if seen > 4096 || l.session != root.session {
                    continue;
                }
                if !is_agent || l.process.pid == root.process.pid {
                    cpu += cpu_s.get(&l.process.pid).copied().unwrap_or(0.0);
                }
                rss += l.process.rss_bytes;
                todo.extend(
                    children.get(&l.process.pid).into_iter().flatten().filter(|c| c.process.pid != l.process.pid),
                );
            }
            let cores = cpu / elapsed;
            (cores >= LEFTOVER_MIN_CORES).then(|| Leftover {
                pid: root.process.pid,
                command: host::short_command(&root.process.command),
                age_ms: now_ms - root.process.start_ms,
                cores,
                rss_bytes: rss,
                session: root.session.clone(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.cores.total_cmp(&a.cores));
    out.truncate(5);
    out
}

fn age_text(ms: u64) -> String {
    match ms / 60_000 {
        m if m < 60 => format!("{m} min"),
        m if m < 48 * 60 => format!("{} h", m / 60),
        m => format!("{} days", m / (24 * 60)),
    }
}

fn round(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use coolant_platform::Process;

    const NOW: u64 = 1_000_000_000;
    const DAY: u64 = 24 * 60 * 60 * 1000;

    fn p(pid: i32, ppid: i32, command: &str, start_ms: u64) -> Process {
        let name = command.split(' ').next().unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
        Process {
            pid,
            ppid,
            name,
            command: command.into(),
            detailed: true,
            start_ms,
            rss_bytes: 100,
            ..Default::default()
        }
    }

    #[test]
    fn busy_old_orphans_are_leftovers() {
        let procs = classify::label(vec![
            // A server whose parent exited, spinning for three days.
            p(10, 1, "node /tmp/a/.output/server/index.mjs", NOW - 3 * DAY),
            // A wrapper whose parent exited, with the busy server under it.
            p(20, 1, "node scripts/serve.mjs", NOW - 3 * DAY),
            p(21, 20, "node /tmp/b/.output/server/index.mjs", NOW - 3 * DAY),
            // An agent whose parent exited, still building.
            p(70, 1, "claude --verbose", NOW - 3 * DAY),
            p(71, 70, "node /repo/build.mjs", NOW - DAY),
            // Not leftovers: idle, just started, an app, an agent someone
            // runs, and the agent asking.
            p(30, 1, "node /srv/idle.mjs", NOW - 3 * DAY),
            p(40, 1, "node /tmp/new.mjs", NOW - 60_000),
            p(50, 1, "/Applications/Foo.app/Contents/MacOS/node x.js", NOW - 3 * DAY),
            p(5, 1, "Terminal", NOW - 3 * DAY),
            p(60, 5, "claude --verbose", NOW - 3 * DAY),
            p(61, 60, "node /repo/build.mjs", NOW - 3 * DAY),
            p(80, 1, "claude --verbose", NOW - 3 * DAY),
        ]);
        // Agents (60, 70, 80) carry their session's CPU.
        let cpu: HashMap<i32, f64> =
            [(10, 3.0), (21, 2.4), (40, 3.0), (50, 3.0), (60, 3.0), (61, 3.0), (70, 6.0), (71, 5.0), (80, 9.0)].into();
        let got = leftovers(&procs, &cpu, "claude:80", 3.0, NOW);
        let pids: Vec<i32> = got.iter().map(|l| l.pid).collect();
        assert_eq!(pids, [70, 10, 20]);
        assert_eq!((got[0].cores, got[0].session.as_str(), got[0].rss_bytes), (2.0, "claude:70", 200));
        assert_eq!(got[1].command, "node index.mjs");
        assert_eq!((got[2].cores * 10.0).round(), 8.0);
        assert_eq!(got[2].rss_bytes, 200);
    }

    #[test]
    fn ages() {
        assert_eq!(age_text(5 * 60_000), "5 min");
        assert_eq!(age_text(3 * 60 * 60_000), "3 h");
        assert_eq!(age_text(3 * DAY + 16 * 60 * 60_000), "3 days");
    }
}
