//! Machine-wide things that make agents run hot or slow: security software
//! that checks every program start, several container VMs at once, and the
//! same dev server started more than once.

use coolant_core::classify::{self, Category};
use coolant_platform::Process;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Instant;

/// Endpoint security system extensions that are active, by display name
/// (e.g. "Falcon Sensor"). Every exec and file open waits for these.
pub fn endpoint_security_products() -> Vec<String> {
    let Ok(out) = Command::new("systemextensionsctl").arg("list").output() else { return Vec::new() };
    parse_system_extensions(&String::from_utf8_lossy(&out.stdout))
}

fn parse_system_extensions(text: &str) -> Vec<String> {
    let mut in_es = false;
    let mut names = Vec::new();
    for line in text.lines() {
        if let Some(section) = line.strip_prefix("--- ") {
            in_es = section.starts_with("com.apple.system_extension.endpoint_security");
            continue;
        }
        // enabled  active  teamID  bundleID (version)  name  [state]
        let cols: Vec<&str> = line.split('\t').collect();
        if in_es && cols.len() >= 6 && cols[5].contains("activated enabled") && !names.contains(&cols[4].to_string()) {
            names.push(cols[4].to_string());
        }
    }
    names
}

/// Median time to start and finish `/usr/bin/true`, in milliseconds. About
/// 0.7–1.5 ms on an Apple Silicon Mac without endpoint security software.
pub fn exec_latency_ms() -> Option<f64> {
    // The first starts are slower everywhere (caches); don't count them.
    for _ in 0..5 {
        Command::new("/usr/bin/true").status().ok()?;
    }
    let mut times: Vec<f64> = (0..15)
        .filter_map(|_| {
            let start = Instant::now();
            Command::new("/usr/bin/true").status().ok()?;
            Some(start.elapsed().as_secs_f64() * 1000.0)
        })
        .collect();
    if times.len() < 10 {
        return None;
    }
    times.sort_by(f64::total_cmp);
    Some(times[times.len() / 2])
}

/// Program start time measured only while the machine is quiet (1-minute
/// load under 2): under load it shows the load, not the security software
/// (7.5 ms vs 2.8 ms on the same Mac). The lowest seen by this process;
/// None until there has been a quiet moment.
pub fn quiet_exec_latency_ms() -> Option<f64> {
    static LOWEST: Mutex<Option<f64>> = Mutex::new(None);
    let mut lowest = LOWEST.lock().ok()?;
    let mut load = 0.0;
    // SAFETY: getloadavg writes at most one value into `load`.
    let quiet = unsafe { libc::getloadavg(&mut load, 1) } == 1 && load < 2.0;
    if quiet && let Some(ms) = exec_latency_ms() {
        *lowest = Some(lowest.map_or(ms, |l| l.min(ms)));
    }
    *lowest
}

/// Container/VM runtimes that are running; each keeps its own Linux VM.
pub fn container_runtimes(procs: &[Process]) -> Vec<&'static str> {
    const RUNTIMES: &[(&str, &str)] = &[
        ("Docker Desktop", "/Docker.app/Contents/MacOS/com.docker.backend"),
        ("OrbStack", "/OrbStack.app/"),
        ("Colima", "colima"),
        ("Rancher Desktop", "/Rancher Desktop.app/"),
        ("Podman", "podman machine"),
    ];
    let mut found: Vec<&'static str> = Vec::new();
    for (name, needle) in RUNTIMES {
        if procs.iter().any(|p| p.command.contains(needle)) && !found.contains(name) {
            found.push(name);
        }
    }
    found
}

/// Commands that start a long-running dev server or watcher.
const SERVERS: &[&str] = &[
    "vite",
    "next dev",
    "next-server",
    "webpack serve",
    "webpack-dev-server",
    "nodemon",
    "tsx watch",
    "ts-node-dev",
    "astro dev",
    "nuxt dev",
    "remix dev",
    "react-scripts start",
    "wrangler dev",
    "uvicorn",
    "flask run",
    "manage.py runserver",
    "rails server",
    "--watch",
    "--hot",
];

/// The same dev server or watcher running more than once in one folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Duplicate {
    pub dir: PathBuf,
    pub command: String,
    /// The copy to keep: the one started first.
    pub keep: i32,
    /// The later copies, (pid, command), oldest first.
    pub extra: Vec<(i32, String)>,
}

/// Dev servers and watchers run for a long time; test runners (vitest,
/// jest) start identical short-lived workers, which this leaves out.
const MIN_AGE_MS: u64 = 2 * 60 * 1000;

/// Whether `command` runs `server` as a word or path component: `vite`
/// matches `.bin/vite dev` and `vite/bin/vite.js` but not `vitest` or
/// `--config vite.config.ts`.
fn runs(command: &str, server: &str) -> bool {
    command.match_indices(server).any(|(i, _)| {
        let before = command[..i].chars().next_back().is_none_or(|c| c.is_whitespace() || c == '/' || c == '"');
        let rest = &command[i + server.len()..];
        let rest = [".js", ".mjs", ".cjs"].iter().find_map(|ext| rest.strip_prefix(ext)).unwrap_or(rest);
        let after = rest.chars().next().is_none_or(|c| c.is_whitespace() || c == '"');
        before && after
    })
}

/// Whether `command` starts a long-running dev server or watcher.
pub fn is_server(command: &str) -> bool {
    SERVERS.iter().any(|s| runs(command, s))
}

/// Groups this user's dev servers and watchers by (folder, command line).
/// `cwd` gives a process's working directory.
pub fn duplicate_servers(procs: &[Process], cwd: impl Fn(i32) -> Option<PathBuf>) -> Vec<Duplicate> {
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let now_ms =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let mut groups: BTreeMap<(PathBuf, String), Vec<&Process>> = BTreeMap::new();
    for p in procs {
        let dev = matches!(classify::of(&p.name, &p.command), Category::Runtime | Category::Build);
        let settled = p.start_ms > 0 && now_ms.saturating_sub(p.start_ms) >= MIN_AGE_MS;
        if p.uid != uid || !dev || !settled || !is_server(&p.command) {
            continue;
        }
        if let Some(dir) = cwd(p.pid) {
            groups.entry((dir, p.command.clone())).or_default().push(p);
        }
    }
    groups
        .into_iter()
        .filter(|(_, ps)| ps.len() > 1)
        .map(|((dir, command), mut ps)| {
            ps.sort_by_key(|p| (p.start_ms, p.pid));
            Duplicate {
                dir,
                keep: ps[0].pid,
                extra: ps[1..].iter().map(|p| (p.pid, p.command.clone())).collect(),
                command,
            }
        })
        .collect()
}

/// A short form of a command line for display: the program and up to three
/// arguments, paths shortened to their last component.
pub fn short_command(command: &str) -> String {
    command
        .split_whitespace()
        .take(4)
        .map(|w| if w.contains('/') { Path::new(w).file_name().map_or(w, |f| f.to_str().unwrap_or(w)) } else { w })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_active_endpoint_security_extensions() {
        let text = "3 extension(s)\n--- com.apple.system_extension.network_extension\nenabled\tactive\tteamID\tbundleID (version)\tname\t[state]\n*\t*\tW5364U7YZB\tio.tailscale.ipn (1.0)\tTailscale Network Extension\t[activated enabled]\n--- com.apple.system_extension.endpoint_security\nenabled\tactive\tteamID\tbundleID (version)\tname\t[state]\n*\t*\tX9E956P446\tcom.crowdstrike.falcon.Agent (7.10)\tFalcon Sensor\t[activated enabled]\n\t\tX9E956P446\tcom.crowdstrike.falcon.Agent (7.09)\tFalcon Sensor\t[terminated waiting to uninstall on reboot]\n";
        assert_eq!(parse_system_extensions(text), ["Falcon Sensor"]);
        assert!(parse_system_extensions("0 extension(s)\n").is_empty());
    }

    fn proc(pid: i32, start_ms: u64, command: &str) -> Process {
        Process {
            pid,
            ppid: 1,
            // SAFETY: getuid has no preconditions.
            uid: unsafe { libc::getuid() },
            name: "node".into(),
            command: command.into(),
            start_ms,
            ..Default::default()
        }
    }

    #[test]
    fn keeps_the_oldest_copy_of_a_dev_server() {
        let procs = [
            proc(30, 300, "node /app/node_modules/.bin/vite"),
            proc(10, 100, "node /app/node_modules/.bin/vite"),
            proc(20, 200, "node /app/node_modules/.bin/vite"),
            proc(40, 100, "node /other/node_modules/.bin/vite"),
            proc(50, 100, "node server.js"),
        ];
        let dups = duplicate_servers(&procs, |pid| Some(PathBuf::from(if pid == 40 { "/other" } else { "/app" })));
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].keep, 10);
        assert_eq!(dups[0].extra.iter().map(|e| e.0).collect::<Vec<_>>(), [20, 30]);
        assert_eq!(short_command(&dups[0].command), "node vite");
    }

    #[test]
    fn test_workers_and_new_processes_are_not_dev_servers() {
        assert!(runs("node /app/node_modules/vite/bin/vite.js --port 3000", "vite"));
        assert!(runs("node_modules/.bin/vite", "vite"));
        assert!(!runs("node /app/node_modules/.bin/vitest run", "vite"));
        assert!(!runs("node node_modules/vitest/dist/workers/forks.js --config vite.config.ts", "vite"));
        assert!(!runs("jest --watchAll", "--watch"));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        let young = [proc(10, now - 5_000, "node .bin/vite"), proc(20, now - 4_000, "node .bin/vite")];
        assert!(duplicate_servers(&young, |_| Some(PathBuf::from("/app"))).is_empty());
        let unknown = [proc(10, 0, "node .bin/vite"), proc(20, 0, "node .bin/vite")];
        assert!(duplicate_servers(&unknown, |_| Some(PathBuf::from("/app"))).is_empty());
    }
}
