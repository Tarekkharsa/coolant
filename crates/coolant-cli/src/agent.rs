//! `coolant agent`: what the calling coding agent is doing to the Mac, and
//! whether it should start more heavy work. Meant to be run by the agent.

use anyhow::Result;
use coolant_core::classify::{self, Category};
use coolant_core::profile::{Options, Session};
use coolant_core::report::bytes;
use coolant_platform::{MemoryPressure, ThermalPressure};
use coolant_workspace::config::tilde;
use coolant_workspace::host;
use coolant_workspace::inuse::InUse;
use serde_json::json;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Servers younger than this are probably test runs, not leftovers.
const SERVER_MIN_AGE_MS: u64 = 2 * 60 * 1000;

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
    let others: f64 = run.sessions.iter().filter(|u| u.key != mine).map(|u| u.cpu_s / elapsed).sum();
    let other_agents = run.sessions.iter().filter(|u| u.key != mine).count();

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

fn age_text(ms: u64) -> String {
    match ms / 60_000 {
        m if m < 60 => format!("{m} min"),
        m => format!("{} h", m / 60),
    }
}

fn round(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}
