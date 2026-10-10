//! Deletes build artifacts from worktrees nobody is using anymore.
//!
//! A worktree is a candidate when it is a linked worktree (never the main
//! checkout) that is idle for `idle_days` or whose branch is merged, is not
//! protected, and no running process uses it. Only artifact folders
//! (target/, node_modules, .next, ...) are deleted; sources, branches and
//! history stay. Optionally, merged worktrees without local changes are
//! removed entirely with `git worktree remove`, which itself refuses to drop
//! uncommitted work.

use crate::config::{Config, tilde};
use crate::discover::{Worktree, git};
use crate::fsutil::{artifacts, disk_usage, human};
use crate::inuse::InUse;
use anyhow::Result;
use std::fmt::Write;
use std::path::PathBuf;

pub struct Options {
    pub idle_days: f64,
    /// Also `git worktree remove` merged, clean worktrees.
    pub remove_merged: bool,
    pub dry_run: bool,
}

#[derive(Debug)]
pub struct Plan {
    pub worktree: Worktree,
    pub reason: String,
    pub artifacts: Vec<(PathBuf, u64)>,
    pub remove: bool,
}

#[derive(Debug, Default)]
pub struct Skipped {
    pub in_use: Vec<(PathBuf, Vec<i32>)>,
    pub protected: Vec<PathBuf>,
    /// Worktrees of projects that are switched off.
    pub off: usize,
}

/// Decides what to clean, without touching anything.
pub fn plan(cfg: &Config, worktrees: Vec<Worktree>, opts: &Options) -> (Vec<Plan>, Skipped) {
    let inuse = InUse::scan();
    let mut plans = Vec::new();
    let mut skipped = Skipped::default();
    for wt in worktrees {
        if wt.main {
            continue;
        }
        let idle = wt.idle_days();
        let reason = match (wt.merged, idle >= opts.idle_days) {
            (true, _) => format!("branch {} is merged", wt.branch.as_deref().unwrap_or("?")),
            (false, true) => format!("idle for {idle:.0} days"),
            _ => continue,
        };
        if !cfg.manages_repo(&wt.repo) {
            skipped.off += 1;
            continue;
        }
        if cfg.is_protected(&wt.path) {
            skipped.protected.push(wt.path);
            continue;
        }
        let users = inuse.users_of(&wt.path);
        if !users.is_empty() {
            skipped.in_use.push((wt.path, users));
            continue;
        }
        let arts: Vec<(PathBuf, u64)> = artifacts(&wt.path)
            .into_iter()
            .map(|rel| {
                let size = disk_usage(&wt.path.join(&rel));
                (rel, size)
            })
            .collect();
        let remove = opts.remove_merged && wt.merged && is_clean(&wt);
        if arts.is_empty() && !remove {
            continue;
        }
        plans.push(Plan { worktree: wt, reason, artifacts: arts, remove });
    }
    (plans, skipped)
}

fn is_clean(wt: &Worktree) -> bool {
    git(&wt.path, &["status", "--porcelain", "--untracked-files=normal"]).is_some_and(|s| s.trim().is_empty())
}

/// Carries out a plan. Returns bytes freed (as measured before deleting).
pub fn apply(plans: &[Plan]) -> Result<u64> {
    let mut freed = 0;
    for p in plans {
        for (rel, size) in &p.artifacts {
            let path = p.worktree.path.join(rel);
            // remove_dir_all deletes a symlink itself, never its target.
            std::fs::remove_dir_all(&path).or_else(|_| std::fs::remove_file(&path))?;
            freed += size;
        }
        if p.remove {
            let path = p.worktree.path.to_string_lossy();
            git(&p.worktree.repo, &["worktree", "remove", &path]);
        }
    }
    Ok(freed)
}

pub fn describe(plans: &[Plan], skipped: &Skipped, dry_run: bool) -> String {
    let mut w = String::new();
    let total: u64 = plans.iter().flat_map(|p| p.artifacts.iter().map(|a| a.1)).sum();
    if plans.is_empty() {
        w.push_str("Nothing to sweep.\n");
    }
    for p in plans {
        let size: u64 = p.artifacts.iter().map(|a| a.1).sum();
        let _ = writeln!(w, "{}  ({}, {})", tilde(&p.worktree.path), p.reason, human(size));
        // List the big ones; fold the many tiny per-package folders.
        let (big, small): (Vec<_>, Vec<_>) = p.artifacts.iter().partition(|(_, s)| *s >= 1 << 20);
        for (rel, s) in &big {
            let _ = writeln!(w, "    {:<40} {:>9}", rel.display(), human(*s));
        }
        if !small.is_empty() {
            let size: u64 = small.iter().map(|a| a.1).sum();
            let _ = writeln!(w, "    {:<40} {:>9}", format!("+ {} smaller folders", small.len()), human(size));
        }
        if p.remove {
            let _ = writeln!(w, "    + remove the worktree (merged, no local changes)");
        }
    }
    for (path, pids) in &skipped.in_use {
        let _ = writeln!(w, "skipped {} (in use by pids {pids:?})", tilde(path));
    }
    if skipped.off > 0 {
        let _ = writeln!(w, "skipped {} worktree(s) of projects that are switched off (coolant projects)", skipped.off);
    }
    for path in &skipped.protected {
        let _ = writeln!(w, "skipped {} (protected)", tilde(path));
    }
    if !plans.is_empty() {
        let verb = if dry_run { "Would free" } else { "Freed" };
        let _ = writeln!(w, "\n{verb} about {} (clones share space, so actual savings can be lower).", human(total));
        if dry_run {
            w.push_str("Run again with --yes to delete.\n");
        }
    }
    w
}
