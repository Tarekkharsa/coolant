//! Checks the machine for the setups and processes that make agents run hot,
//! each with a concrete fix.

use crate::config::{Config, home, tilde};
use crate::discover::Worktree;
use crate::fsutil::artifacts;
use crate::tune::excluded_from_spotlight;
use coolant_core::classify::Category;
use coolant_core::profile::{Options, Run, Session};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub status: Status,
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fix: String,
    /// What the app's Fix button does, when the fix can be automated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
}

/// A fix coolant can apply itself (see [`crate::actions`]).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Open Spotlight's Search Privacy settings and show these folders in
    /// Finder, to be dragged in: excluded where they are. (Moving them
    /// behind a symlink broke Claude Code, which follows it and then refuses
    /// a worktree outside its repository.)
    SpotlightPrivacy { paths: Vec<PathBuf> },
    /// Move folders an earlier version moved into `~/.coolant/spaces.noindex`
    /// back to these paths.
    RestoreMoved { paths: Vec<PathBuf> },
    /// Exclude these folders from Time Machine.
    ExcludeFromBackups { paths: Vec<PathBuf> },
    /// Clone build artifacts into these worktrees, and install the
    /// post-checkout hook in their repositories for future ones.
    Prewarm { worktrees: Vec<PathBuf>, repos: Vec<PathBuf> },
    /// Delete build artifacts in idle or merged worktrees with [`crate::sweep`].
    Sweep,
    /// `brew services stop` a crash-looping Homebrew service.
    StopService { formula: String },
    /// Stop these processes (pid, command line), after checking each is
    /// still the same program: extra copies of a dev server.
    StopProcesses { procs: Vec<(i32, String)> },
    /// Rename `from` to `to` (a stray `~/.git`); Undo renames it back.
    MoveAside { from: PathBuf, to: PathBuf },
}

fn check(status: Status, title: impl Into<String>, detail: impl Into<String>, fix: impl Into<String>) -> Check {
    Check { status, title: title.into(), detail: detail.into(), fix: fix.into(), action: None }
}

impl Check {
    fn with(mut self, action: Action) -> Check {
        self.action = Some(action);
        self
    }
}

/// Runs every check. `sample` is how long to watch running processes.
pub fn run(cfg: &Config, worktrees: &[Worktree], sample: Duration) -> Vec<Check> {
    let mut out = setup(cfg, worktrees);
    processes(cfg, sample, &mut out);
    out
}

/// The checks of folders and settings, without watching processes (the app
/// watches those itself).
pub fn setup(cfg: &Config, worktrees: &[Worktree]) -> Vec<Check> {
    let mut out = Vec::new();
    let linked: Vec<&Worktree> = worktrees.iter().filter(|w| !w.main).collect();
    out.push(check(
        Status::Info,
        format!(
            "{} repositories, {} agent/linked worktrees",
            worktrees.iter().filter(|w| w.main).count(),
            linked.len()
        ),
        "",
        "",
    ));
    moved_earlier(&mut out);
    spotlight(&linked, &mut out);
    time_machine(&linked, &mut out);
    icloud(cfg, &mut out);
    stale(cfg, &linked, &mut out);
    prewarm(&linked, worktrees, &mut out);
    crash_loops(&mut out);
    big_repos(&mut out);
    endpoint_security(&mut out);
    let procs = coolant_platform::current().processes().unwrap_or_default();
    container_vms(&procs, &mut out);
    duplicate_servers(cfg, &procs, &mut out);
    out
}

/// A git repository at ~ (or ~/Documents, ~/Desktop) puts every folder
/// under it in one huge working tree: any tool that checks git status in a
/// folder without a repository of its own scans all of it. On the M1 that
/// was 20 s and 8 CPU-s per check, about twice a minute.
fn big_repos(out: &mut Vec<Check>) {
    for dir in [home(), home().join("Documents"), home().join("Desktop")] {
        let git = dir.join(".git");
        if !git.is_dir() || ignores_everything(&dir) {
            continue;
        }
        let mut to = dir.join(".git-moved-by-coolant");
        let mut n = 2;
        while to.exists() {
            to = dir.join(format!(".git-moved-by-coolant-{n}"));
            n += 1;
        }
        out.push(
            check(
                Status::Warn,
                format!("{} is a git repository", tilde(&dir)),
                format!(
                    "Every folder under {} is part of one huge working tree, so editors, agent hosts and git tools that check a folder without its own repository scan all of it (on one Mac: 20 s and 8 CPU-s per check, twice a minute).{}",
                    tilde(&dir),
                    repo_use(&dir)
                ),
                format!("If you don't use it, move it aside: mv {} {}", tilde(&git), tilde(&to)),
            )
            .with(Action::MoveAside { from: git, to }),
        );
    }
}

/// How much a repository is used, to help decide whether it can go: " It
/// tracks 12 files; the last commit was 3 years ago."
fn repo_use(dir: &Path) -> String {
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let files = git(&["ls-files"]).map_or(0, |o| o.lines().count());
    let last = git(&["log", "-1", "--format=%cr"]).map(|o| o.trim().to_string()).filter(|s| !s.is_empty());
    let files = match files {
        0 => "It tracks no files".to_string(),
        1 => "It tracks 1 file".to_string(),
        n => format!("It tracks {n} files"),
    };
    match last {
        Some(when) => format!(" {files}; the last commit was {when}."),
        None => format!(" {files} and has no commits."),
    }
}

/// Whether a repository at `dir` ignores everything (`*` or `/*`), the usual
/// dotfiles setup, in which case git doesn't walk the tree.
fn ignores_everything(dir: &Path) -> bool {
    [dir.join(".gitignore"), dir.join(".git/info/exclude")]
        .iter()
        .any(|f| std::fs::read_to_string(f).is_ok_and(|t| t.lines().any(|l| matches!(l.trim(), "*" | "/*"))))
}

/// Median `/usr/bin/true` start-to-exit time above which program starts
/// count as slow. Unmanaged Apple Silicon Macs measure about 0.8–1.5 ms;
/// a MacBook Pro with CrowdStrike Falcon measured 2.8 ms here (4.0 ms in
/// the benchmark), so 2.5 ms would sit too close to it.
const SLOW_EXEC_MS: f64 = 2.0;

fn endpoint_security(out: &mut Vec<Check>) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let Some(ms) = crate::host::exec_latency_ms() else { return };
    if ms < SLOW_EXEC_MS {
        return;
    }
    let products = crate::host::endpoint_security_products();
    let who = if products.is_empty() {
        "Something checks every program before it runs: security software, a VPN or a device-management agent."
            .to_string()
    } else {
        format!("{} checks every program before it runs.", products.join(", "))
    };
    out.push(check(
        Status::Info,
        format!("Starting a program takes {ms:.1} ms on this Mac (typical: about 1 ms)"),
        format!("Agents start thousands of short programs (git, rg, node, compilers), so every tool call costs more. {who}"),
        "Nothing to change from here. If IT manages this Mac, ask whether developer tools and agent worktrees can be excluded from scanning; prewarming worktrees helps more than usual.",
    ));
}

fn container_vms(procs: &[coolant_platform::Process], out: &mut Vec<Check>) {
    let running = crate::host::container_runtimes(procs);
    if running.len() < 2 {
        return;
    }
    out.push(check(
        Status::Warn,
        format!("{} are all running", running.join(" and ")),
        "Each keeps its own Linux VM running, using CPU and memory even with no containers.",
        "Quit the ones you don't use (keep the one your projects need).",
    ));
}

fn duplicate_servers(cfg: &Config, procs: &[coolant_platform::Process], out: &mut Vec<Check>) {
    let inuse = crate::inuse::InUse::scan();
    let exposed = crate::exposed::exposed_pids();
    let mut dups = crate::host::duplicate_servers(procs, |pid| inuse.cwd(pid).map(Path::to_path_buf));
    dups.retain(|d| !cfg.is_protected(&d.dir));
    for d in &mut dups {
        d.extra.retain(|(pid, _)| !exposed.contains(pid));
    }
    dups.retain(|d| !d.extra.is_empty());
    dups.sort_by_key(|d| std::cmp::Reverse(d.extra.len()));
    for d in dups.into_iter().take(3) {
        let pids = d.extra.iter().map(|(p, _)| p.to_string()).collect::<Vec<_>>().join(" ");
        out.push(
            check(
                Status::Warn,
                format!(
                    "{} copies of `{}` are running in {}",
                    d.extra.len() + 1,
                    crate::host::short_command(&d.command),
                    tilde(&d.dir)
                ),
                format!(
                    "Agents often start a dev server or watcher again without stopping the first, and every copy rebuilds on each change. The first one (pid {}) would keep running.",
                    d.keep
                ),
                format!("kill {pids}"),
            )
            .with(Action::StopProcesses { procs: d.extra }),
        );
    }
}

/// Folders that hold linked worktrees, e.g. `<repo>/.claude/worktrees`.
fn containers(linked: &[&Worktree]) -> BTreeMap<PathBuf, usize> {
    let mut m = BTreeMap::new();
    for w in linked {
        if let Some(parent) = w.path.parent() {
            *m.entry(parent.to_path_buf()).or_default() += 1;
        }
    }
    m
}

fn spotlight(linked: &[&Worktree], out: &mut Vec<Check>) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let status = Command::new("mdutil").args(["-s", "/System/Volumes/Data"]).output();
    let enabled = status.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("enabled"));
    if !enabled {
        out.push(check(Status::Ok, "Spotlight indexing is off for the data volume", "", ""));
        return;
    }
    let mut indexed = 0;
    for (dir, n) in containers(linked) {
        if excluded_from_spotlight(&dir) || !crate::tune::spotlight_indexes(&dir) {
            continue;
        }
        indexed += n;
        // Exclude a folder that only holds worktrees (so future ones are
        // covered too); otherwise the worktrees one by one.
        let targets: Vec<PathBuf> = if crate::tune::dedicated_container(&dir) {
            vec![dir.clone()]
        } else {
            linked.iter().filter(|w| w.path.parent() == Some(&dir)).map(|w| w.path.clone()).collect()
        };
        let fix = targets.iter().map(|t| shell_quote(&tilde(t))).collect::<Vec<_>>().join(" ");
        out.push(
            check(
                Status::Warn,
                format!("Spotlight indexes {n} worktree(s) in {}", tilde(&dir)),
                "Every file agents write there (dependencies, build output) gets re-indexed: in our benchmark ~30× more background CPU than an excluded folder.",
                format!("System Settings → Spotlight → Search Privacy…, then add {fix}"),
            )
            .with(Action::SpotlightPrivacy { paths: targets }),
        );
    }
    if indexed == 0 && !linked.is_empty() {
        out.push(check(Status::Ok, "Spotlight isn't indexing agent worktrees", "", ""));
    }
}

/// Folders an earlier version of coolant moved into `~/.coolant/spaces.noindex`
/// behind a symlink. Claude Code resolves the link and then refuses to
/// create worktrees there, so put them back.
fn moved_earlier(out: &mut Vec<Check>) {
    let moved = crate::tune::moved_worktrees();
    if moved.is_empty() {
        return;
    }
    let list = moved.iter().map(|p| tilde(p)).collect::<Vec<_>>().join(", ");
    out.push(
        check(
            Status::Warn,
            format!("{} worktree folder(s) moved by an earlier Coolant: {list}", moved.len()),
            "An earlier version moved them into ~/.coolant/spaces.noindex and left a link in their place. Claude Code follows the link, finds a path outside the repository and refuses to create agent worktrees there.",
            format!("coolant tune --undo {}", moved.iter().map(|p| shell_quote(&tilde(p))).collect::<Vec<_>>().join(" ")),
        )
        .with(Action::RestoreMoved { paths: moved }),
    );
}

fn time_machine(linked: &[&Worktree], out: &mut Vec<Check>) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let Ok(info) = Command::new("tmutil").arg("destinationinfo").output() else { return };
    if String::from_utf8_lossy(&info.stdout).contains("No destinations") || !info.status.success() {
        return; // Time Machine isn't set up.
    }
    for dir in containers(linked).into_keys() {
        let excluded = Command::new("tmutil")
            .arg("isexcluded")
            .arg(&dir)
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("[Excluded]"));
        if !excluded {
            out.push(
                check(
                    Status::Warn,
                    format!("Time Machine backs up {}", tilde(&dir)),
                    "Backups re-read every changed file in agent worktrees.",
                    format!("tmutil addexclusion {}", shell_quote(&dir.display().to_string())),
                )
                .with(Action::ExcludeFromBackups { paths: vec![dir] }),
            );
        }
    }
}

fn icloud(cfg: &Config, out: &mut Vec<Check>) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let cloud = home().join("Library/Mobile Documents/com~apple~CloudDocs");
    for synced in ["Documents", "Desktop"] {
        if !cloud.join(synced).is_dir() {
            continue;
        }
        let under: Vec<_> = cfg.roots().into_iter().filter(|r| r.starts_with(home().join(synced))).collect();
        if !under.is_empty() {
            out.push(check(
                Status::Warn,
                format!("iCloud Drive syncs ~/{synced}, which holds your code"),
                "fileproviderd/bird upload build output and dependencies as agents write them.",
                "Move repositories out of ~/Documents and ~/Desktop, or turn off Desktop & Documents in iCloud settings.",
            ));
        }
    }
}

fn stale(cfg: &Config, linked: &[&Worktree], out: &mut Vec<Check>) {
    let stale: Vec<&&Worktree> =
        linked.iter().filter(|w| (w.merged || w.idle_days() >= 7.0) && !cfg.is_protected(&w.path)).collect();
    let with_artifacts = stale.iter().filter(|w| !artifacts(&w.path).is_empty()).count();
    if with_artifacts > 0 {
        out.push(
            check(
                Status::Info,
                format!("{with_artifacts} merged or idle worktree(s) still hold build artifacts"),
                "target/ and node_modules in finished worktrees take disk space and keep being scanned.",
                "coolant sweep",
            )
            .with(Action::Sweep),
        );
    }
}

fn prewarm(linked: &[&Worktree], all: &[Worktree], out: &mut Vec<Check>) {
    let (mut cold, mut repos) = (Vec::new(), Vec::new());
    for w in linked.iter().filter(|w| w.idle_days() < 7.0) {
        let Some(main) = all.iter().find(|m| m.main && m.repo == w.repo) else { continue };
        let missing = artifacts(&main.path)
            .iter()
            .any(|rel| !w.path.join(rel).exists() && w.path.join(rel).parent().is_some_and(Path::is_dir));
        if missing {
            cold.push(w.path.clone());
            if !repos.contains(&main.path) {
                repos.push(main.path.clone());
            }
        }
    }
    if !cold.is_empty() {
        out.push(
            check(
                Status::Info,
                format!("{} active worktree(s) lack build artifacts the main checkout has", cold.len()),
                "Their first build or install starts from scratch: every core busy for minutes, per worktree.",
                "coolant prewarm <worktree>   (or coolant prewarm --install-hook <repo> to do it automatically)",
            )
            .with(Action::Prewarm { worktrees: cold, repos }),
        );
    }
}

/// Programs that keep crashing; at most three, most crashes first. One
/// that stopped crashing a while ago is only mentioned.
fn crash_loops(out: &mut Vec<Check>) {
    let loaded = loaded_jobs();
    for c in crate::crashes::recent().into_iter().take(3) {
        let ago = c.last.elapsed().unwrap_or_default().as_secs();
        let ago = if ago < 3600 { format!("{} min", ago / 60) } else { format!("{} h", ago / 3600) };
        let reason = if c.reason.is_empty() { String::new() } else { format!(" ({})", c.reason) };
        let active = c.active();
        let title = if active {
            format!("{} crashed {} times in the last 24 hours", c.name, c.count)
        } else {
            format!("{} crashed {} times in the last 24 hours, but not since {ago} ago", c.name, c.count)
        };
        let mut detail = format!(
            "Each crash makes ReportCrash write a report, often seconds of CPU each, and a service that restarts crashes again. Last crash {ago} ago{reason}."
        );
        // A missing library usually means Homebrew updated a dependency
        // under it; reinstalling links it against the new one.
        let reinstall = (c.reason == "Library missing").then(|| crate::crashes::homebrew_formula_of(&c.name)).flatten();
        if let (Some(f), Some(lib)) = (&reinstall, &c.missing) {
            detail.push_str(&format!(
                " It can't find {lib}, probably removed by a Homebrew update; {f} needs reinstalling."
            ));
        }
        let fix_crash = |stop: String| match &reinstall {
            Some(f) => format!("brew reinstall {f}   (or {stop})"),
            None => format!("Fix what makes it crash, or {stop}"),
        };
        let check = match (&c.label, c.brew_formula()) {
            // Stopped since (only this user's launchd jobs are listed, so a
            // root service started with `sudo brew services` lands here too).
            (Some(label), _) if !loaded.contains(label) => continue,
            _ if !active => {
                let fix = match &reinstall {
                    Some(f) => format!("Before it runs again: brew reinstall {f}"),
                    None => String::new(),
                };
                check(Status::Info, title, detail, fix)
            }
            (Some(_), Some(f)) => check(
                Status::Warn,
                title,
                detail,
                fix_crash(format!("stop it: brew services stop {f}   (start it again with brew services start {f})")),
            )
            .with(Action::StopService { formula: f.to_string() }),
            (Some(label), None) => check(
                Status::Warn,
                title,
                detail,
                // SAFETY: getuid has no preconditions.
                fix_crash(format!("stop it: launchctl bootout gui/{}/{label}", unsafe { libc::getuid() })),
            ),
            (None, _) if c.system => {
                check(Status::Info, title, detail, "This is part of macOS; restarting the Mac usually clears it.")
            }
            (None, _) => {
                check(Status::Warn, title, detail, fix_crash(format!("stop whatever keeps starting {}", c.name)))
            }
        };
        out.push(check);
    }
}

/// Labels of the current user's launchd jobs.
fn loaded_jobs() -> std::collections::BTreeSet<String> {
    let Ok(out) = Command::new("launchctl").arg("list").output() else { return Default::default() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .filter_map(|l| l.split('\t').nth(2))
        .map(str::to_string)
        .collect()
}

fn processes(cfg: &Config, sample: Duration, out: &mut Vec<Check>) {
    let backend = Arc::from(coolant_platform::current());
    let Ok(mut session) = Session::new(backend, Options::default()) else { return };
    std::thread::sleep(sample);
    session.sample();
    let run = session.finish();
    orphans(cfg, &run, out);
    for d in run.duplicates.iter().filter(|d| d.count >= 3) {
        out.push(check(
            Status::Info,
            format!("{} copies of {} are running", d.count, d.name),
            format!(
                "{} of memory together; usually one per worktree or editor window.",
                crate::fsutil::human(d.rss_bytes)
            ),
            "Close editor windows and agent sessions for finished worktrees.",
        ));
    }
    let vm = run.cores(run.category(Category::ContainerVm).cpu_s);
    if vm >= 0.5 {
        out.push(check(
            Status::Info,
            format!("A container/VM is using {vm:.1} cores right now"),
            "Docker Desktop, OrbStack and similar keep a Linux VM running; idle containers still cost CPU.",
            "Stop containers you aren't using (keep the ones your projects need).",
        ));
    }
    let idx = run.cores(run.category(Category::Indexer).cpu_s);
    if idx >= 0.3 {
        out.push(check(Status::Warn, format!("Spotlight is busy right now ({idx:.1} cores)"), "", ""));
    }
}

fn orphans(cfg: &Config, run: &Run, out: &mut Vec<Check>) {
    if run.orphans.is_empty() {
        return;
    }
    let inuse = crate::inuse::InUse::scan();
    let exposed = crate::exposed::exposed_pids();
    let mut groups: BTreeMap<String, usize> = BTreeMap::new();
    let (mut n, mut shared) = (0, 0);
    for o in &run.orphans {
        let cwd = inuse.cwd(o.pid).map(Path::to_path_buf);
        let protected = cwd.as_deref().is_some_and(|c| cfg.is_protected(c))
            || cfg.protect.iter().any(|p| o.command.contains(&*crate::config::expand(p).to_string_lossy()));
        if protected {
            continue;
        }
        if exposed.contains(&o.pid) {
            shared += 1;
            continue;
        }
        n += 1;
        let label = cwd.map(|c| short(&c)).unwrap_or_else(|| o.name.clone());
        *groups.entry(label).or_default() += 1;
    }
    if shared > 0 {
        out.push(check(
            Status::Info,
            format!("{shared} long-running process(es) serve ports exposed through Tailscale; left alone"),
            "",
            "",
        ));
    }
    if n == 0 {
        return;
    }
    let list = groups.iter().map(|(k, c)| format!("{c}× {k}")).collect::<Vec<_>>().join(", ");
    out.push(check(
        Status::Warn,
        format!("{n} dev processes have outlived the session that started them"),
        list,
        "Stop the ones you don't need (coolant-app, or your project's stop script). Protect the rest: coolant protect <path>",
    ));
}

/// The first few components of a home-relative path.
fn short(p: &Path) -> String {
    tilde(p).split('/').take(6).collect::<Vec<_>>().join("/")
}

fn shell_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_alphanumeric() || "/._-~%".contains(c)) { s.into() } else { format!("'{s}'") }
}

pub fn text(checks: &[Check]) -> String {
    let mut w = String::new();
    for c in checks {
        let mark = match c.status {
            Status::Ok => "  ok  ",
            Status::Warn => " warn ",
            Status::Info => " info ",
        };
        w.push_str(&format!("[{mark}] {}\n", c.title));
        if !c.detail.is_empty() {
            w.push_str(&format!("         {}\n", c.detail));
        }
        if !c.fix.is_empty() {
            w.push_str(&format!("         → {}\n", c.fix));
        }
    }
    w
}
