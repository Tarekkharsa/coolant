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
    /// Move these folders out of Spotlight's reach with [`crate::tune`].
    Tune { paths: Vec<PathBuf> },
    /// Exclude these folders from Time Machine.
    ExcludeFromBackups { paths: Vec<PathBuf> },
    /// Clone build artifacts into these worktrees, and install the
    /// post-checkout hook in their repositories for future ones.
    Prewarm { worktrees: Vec<PathBuf>, repos: Vec<PathBuf> },
    /// Delete build artifacts in idle or merged worktrees with [`crate::sweep`].
    Sweep,
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
    spotlight(&linked, &mut out);
    time_machine(&linked, &mut out);
    icloud(cfg, &mut out);
    stale(cfg, &linked, &mut out);
    prewarm(&linked, worktrees, &mut out);
    out
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
        if excluded_from_spotlight(&dir) {
            continue;
        }
        indexed += n;
        // Tune a folder that only holds worktrees (so future ones land there
        // too); otherwise the worktrees one by one.
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
                format!("coolant tune {fix}"),
            )
            .with(Action::Tune { paths: targets }),
        );
    }
    if indexed == 0 && !linked.is_empty() {
        out.push(check(Status::Ok, "Agent worktrees are excluded from Spotlight", "", ""));
    }
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
