//! Carries out a doctor [`Action`]: what the app's Fix buttons do. Every
//! action goes through the same guarded functions as the CLI commands, so the
//! same safety rules apply (protected paths, folders in use, dry runs).

use crate::config::{Config, tilde};
use crate::doctor::Action;
use crate::fsutil::human;
use crate::{prewarm, sweep, tune, worktrees};
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::Command;

/// Worktrees idle this many days are swept (the CLI's default).
const SWEEP_IDLE_DAYS: f64 = 7.0;

/// The result of an action, and how to reverse it when that's possible.
#[derive(Debug, Clone)]
pub struct Done {
    pub message: String,
    pub undo: Option<Undo>,
}

#[derive(Debug, Clone)]
pub enum Undo {
    Tune(Vec<PathBuf>),
    ExcludeFromBackups(Vec<PathBuf>),
    StartService(String),
    MoveBack { from: PathBuf, to: PathBuf },
}

/// Applies `action`. Partial success is reported in the message; an error
/// means nothing was changed.
pub fn apply(cfg: &Config, action: &Action) -> Result<Done> {
    match action {
        Action::Tune { paths } => {
            let (mut moved, mut failed) = (Vec::new(), Vec::new());
            for p in paths {
                match tune::tune(cfg, p, false, false) {
                    Ok(tune::Outcome::Tuned { .. }) => moved.push(p.clone()),
                    Ok(_) => {}
                    Err(e) => failed.push(format!("{e:#}")),
                }
            }
            if moved.is_empty() && !failed.is_empty() {
                bail!("{}", failed.join("; "));
            }
            let mut message = format!("Moved {} folder(s) out of Spotlight; agents use the same paths.", moved.len());
            append_failures(&mut message, &failed);
            Ok(Done { message, undo: (!moved.is_empty()).then_some(Undo::Tune(moved)) })
        }
        Action::ExcludeFromBackups { paths } => {
            for p in paths {
                tune::set_backup_excluded(p, true).with_context(|| format!("excluding {}", tilde(p)))?;
            }
            Ok(Done {
                message: format!("Time Machine no longer backs up {} folder(s).", paths.len()),
                undo: Some(Undo::ExcludeFromBackups(paths.clone())),
            })
        }
        Action::Prewarm { worktrees, repos } => {
            let mut failed = Vec::new();
            let mut cloned = 0;
            for w in worktrees {
                match prewarm::prewarm(w, None, false) {
                    Ok(r) => cloned += r.cloned.len(),
                    Err(e) => failed.push(format!("{}: {e:#}", tilde(w))),
                }
            }
            let mut hooked = 0;
            for r in repos {
                match prewarm::install_hook(r) {
                    Ok(_) => hooked += 1,
                    // An existing hook of the user's own: leave it and say so.
                    Err(e) => failed.push(format!("{}: {}", tilde(r), e.to_string().lines().next().unwrap_or(""))),
                }
            }
            let mut message = format!(
                "Cloned {cloned} build folder(s) into {} worktree(s); new worktrees in {hooked} repo(s) are prewarmed automatically.",
                worktrees.len()
            );
            append_failures(&mut message, &failed);
            Ok(Done { message, undo: None })
        }
        Action::StopProcesses { procs } => {
            // Re-check every process: still the same program, and not one
            // serving a port shared through Tailscale or in a protected path.
            let inuse = crate::inuse::InUse::scan();
            let exposed = crate::exposed::exposed_pids();
            let mut stopped = 0;
            for (pid, command) in procs {
                let same = inuse.command(*pid) == Some(command.as_str());
                let protected = inuse.cwd(*pid).is_some_and(|c| cfg.is_protected(c));
                // SAFETY: kill(2) with a plain signal has no memory-safety concerns.
                if same && !protected && !exposed.contains(pid) && unsafe { libc::kill(*pid, libc::SIGTERM) } == 0 {
                    stopped += 1;
                }
            }
            if stopped == 0 {
                bail!("those processes have already exited or changed");
            }
            Ok(Done {
                message: format!("Stopped {stopped} extra cop{}.", if stopped == 1 { "y" } else { "ies" }),
                undo: None,
            })
        }
        Action::MoveAside { from, to } => {
            if to.exists() {
                bail!("{} already exists", tilde(to));
            }
            std::fs::rename(from, to).with_context(|| format!("moving {}", tilde(from)))?;
            Ok(Done {
                message: format!("Moved {} to {}.", tilde(from), tilde(to)),
                undo: Some(Undo::MoveBack { from: to.clone(), to: from.clone() }),
            })
        }
        Action::StopService { formula } => {
            brew(&["services", "stop", formula])?;
            Ok(Done {
                message: format!("Stopped {formula}; it won't start again at login."),
                undo: Some(Undo::StartService(formula.clone())),
            })
        }
        Action::Sweep => {
            let plans = sweep_plan(cfg, false);
            let freed = sweep::apply(&plans)?;
            Ok(Done { message: format!("Freed {} in {} worktree(s).", human(freed), plans.len()), undo: None })
        }
    }
}

/// Reverses an action.
pub fn undo(u: &Undo) -> Result<String> {
    match u {
        Undo::Tune(paths) => {
            for p in paths {
                tune::undo(p)?;
            }
            Ok(format!("Moved {} folder(s) back.", paths.len()))
        }
        Undo::ExcludeFromBackups(paths) => {
            for p in paths {
                tune::set_backup_excluded(p, false).with_context(|| format!("including {}", tilde(p)))?;
            }
            Ok(format!("Time Machine backs up {} folder(s) again.", paths.len()))
        }
        Undo::MoveBack { from, to } => {
            if to.exists() {
                bail!("{} exists again; not overwriting it", tilde(to));
            }
            std::fs::rename(from, to).with_context(|| format!("moving {} back", tilde(from)))?;
            Ok(format!("Moved {} back.", tilde(to)))
        }
        Undo::StartService(formula) => {
            brew(&["services", "start", formula])?;
            Ok(format!("Started {formula} again."))
        }
    }
}

/// What a sweep would delete: (worktrees, bytes). Measures folder sizes, so
/// it can take a few seconds.
pub fn sweep_preview(cfg: &Config) -> (usize, u64) {
    let plans = sweep_plan(cfg, true);
    let bytes = plans.iter().flat_map(|p| p.artifacts.iter().map(|a| a.1)).sum();
    (plans.len(), bytes)
}

fn sweep_plan(cfg: &Config, dry_run: bool) -> Vec<sweep::Plan> {
    let opts = sweep::Options { idle_days: SWEEP_IDLE_DAYS, remove_merged: false, dry_run };
    sweep::plan(cfg, worktrees(&cfg.roots()), &opts).0
}

/// Runs Homebrew, wherever it is installed.
fn brew(args: &[&str]) -> Result<()> {
    for bin in ["brew", "/opt/homebrew/bin/brew", "/usr/local/bin/brew"] {
        let Ok(out) = Command::new(bin).args(args).output() else { continue };
        if !out.status.success() {
            bail!("brew {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
        }
        return Ok(());
    }
    bail!("Homebrew not found")
}

fn append_failures(message: &mut String, failed: &[String]) {
    if !failed.is_empty() {
        message.push_str(&format!(" Skipped: {}", failed.join("; ")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tune_then_undo_restores_the_folder() {
        let base = std::env::temp_dir().join(format!("coolant-actions-{}", std::process::id()));
        let dir = base.join("worktrees");
        std::fs::create_dir_all(dir.join("wt")).unwrap();
        std::fs::write(dir.join("wt/file"), "x").unwrap();

        let done = apply(&Config::default(), &Action::Tune { paths: vec![dir.clone()] }).unwrap();
        assert!(std::fs::symlink_metadata(&dir).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(dir.join("wt/file")).unwrap(), "x");

        undo(done.undo.as_ref().expect("tune can be undone")).unwrap();
        assert!(!std::fs::symlink_metadata(&dir).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(dir.join("wt/file")).unwrap(), "x");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn move_aside_and_back() {
        let base = std::env::temp_dir().join(format!("coolant-move-{}", std::process::id()));
        let (from, to) = (base.join(".git"), base.join(".git-moved-by-coolant"));
        std::fs::create_dir_all(from.join("objects")).unwrap();
        let done = apply(&Config::default(), &Action::MoveAside { from: from.clone(), to: to.clone() }).unwrap();
        assert!(!from.exists() && to.join("objects").is_dir());
        undo(done.undo.as_ref().unwrap()).unwrap();
        assert!(from.join("objects").is_dir() && !to.exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
