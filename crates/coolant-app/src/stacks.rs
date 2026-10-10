//! Leftover dev processes grouped by project, with a guarded Stop action.
//!
//! Orphans (dev servers and watchers whose parent session is gone) are
//! grouped by the git project their working directory belongs to. Groups
//! inside a protected path, or serving a port Tailscale exposes to other
//! people, are shown locked and can't be stopped from here. Ones in projects
//! that are switched off aren't shown at all.

use coolant_core::profile::Run;
use coolant_workspace::config::{Config, tilde};
use coolant_workspace::exposed::exposed_pids;
use coolant_workspace::inuse::InUse;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Stack {
    /// Display label and identity: the project folder.
    pub key: String,
    /// The project folder, when the processes' working directory is known.
    pub root: Option<PathBuf>,
    pub procs: Vec<StackProc>,
    pub cpu_s: f64,
    pub rss_bytes: u64,
    /// Why it can't be stopped from the app, if it can't.
    pub locked: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub struct StackProc {
    pub pid: i32,
    pub name: String,
}

/// Groups the run's orphans. Runs lsof and reads the config, so call it
/// from the sampler thread, not the UI.
pub fn stacks(run: &Run) -> Vec<Stack> {
    if run.orphans.is_empty() {
        return Vec::new();
    }
    let cfg = Config::load().unwrap_or_default();
    let inuse = InUse::scan();
    let exposed = exposed_pids();
    let mut groups: BTreeMap<String, Stack> = BTreeMap::new();
    for o in &run.orphans {
        let cwd = inuse.cwd(o.pid).map(Path::to_path_buf);
        let protected = cwd.as_deref().is_some_and(|c| cfg.is_protected(c));
        if !protected && !cfg.may_touch_cwd(cwd.as_deref()) {
            continue;
        }
        // Group by protected path first (it may not be a git project), then
        // by the git project the process runs in.
        let root = cwd.as_deref().map(|c| cfg.protected_root(c).unwrap_or_else(|| project_root(c)));
        let key = root.as_deref().map(tilde).unwrap_or_else(|| o.name.clone());
        let s = groups.entry(key.clone()).or_insert_with(|| Stack {
            key,
            root: root.clone(),
            procs: Vec::new(),
            cpu_s: 0.0,
            rss_bytes: 0,
            locked: None,
        });
        if protected {
            s.locked = Some("protected");
        } else if exposed.contains(&o.pid) && s.locked.is_none() {
            s.locked = Some("exposed via Tailscale");
        }
        s.cpu_s += o.cpu_s;
        s.rss_bytes += o.rss_bytes;
        s.procs.push(StackProc { pid: o.pid, name: o.name.clone() });
    }
    let mut v: Vec<Stack> = groups.into_values().collect();
    v.sort_by(|a, b| b.cpu_s.total_cmp(&a.cpu_s));
    v
}

/// Adds the stack's project to the protected paths (like `coolant protect`),
/// so its processes are kept and no longer reported as leftovers.
pub fn keep(stack: &Stack) -> Result<(), String> {
    let root = stack.root.as_deref().ok_or("its folder is unknown")?;
    let mut cfg = Config::load().map_err(|e| e.to_string())?;
    let p = tilde(&coolant_workspace::config::canonical(root));
    if !cfg.protect.contains(&p) {
        cfg.protect.push(p);
        cfg.save().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The nearest folder holding a `.git` (file or directory).
fn project_root(cwd: &Path) -> PathBuf {
    cwd.ancestors().find(|d| d.join(".git").exists()).unwrap_or(cwd).to_path_buf()
}

/// Sends SIGTERM to every process of a stack, re-checking protection and
/// exposure first so a stale UI can't stop something it shouldn't.
pub fn stop(stack: &Stack) -> Result<usize, String> {
    if let Some(why) = stack.locked {
        return Err(format!("{} is {why}", stack.key));
    }
    let cfg = Config::load().unwrap_or_default();
    let inuse = InUse::scan();
    let exposed = exposed_pids();
    let mut stopped = 0;
    for p in &stack.procs {
        let locked = exposed.contains(&p.pid) || !cfg.may_touch_cwd(inuse.cwd(p.pid));
        // The PID must still be the same program, not a reused number.
        let same = inuse.command(p.pid).is_some_and(|c| c.contains(&p.name));
        if locked || !same {
            continue;
        }
        // SAFETY: kill(2) with a plain signal has no memory-safety concerns.
        if unsafe { libc::kill(p.pid, libc::SIGTERM) } == 0 {
            stopped += 1;
        }
    }
    Ok(stopped)
}
