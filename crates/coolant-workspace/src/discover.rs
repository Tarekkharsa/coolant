//! Finds git repositories and their worktrees.

use crate::config::canonical;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: PathBuf,
    /// The repository's main checkout.
    pub repo: PathBuf,
    /// True for the main checkout itself.
    pub main: bool,
    pub branch: Option<String>,
    /// Latest git activity (index, HEAD reflog) in this worktree.
    pub last_active: SystemTime,
    /// Branch is fully merged into the default branch.
    pub merged: bool,
}

impl Worktree {
    pub fn idle_days(&self) -> f64 {
        SystemTime::now().duration_since(self.last_active).map_or(0.0, |d| d.as_secs_f64() / 86400.0)
    }
}

/// Every worktree of every repository under `roots`.
pub fn worktrees(roots: &[PathBuf]) -> Vec<Worktree> {
    let mut repos = BTreeSet::new();
    for root in roots {
        // The home folder is searched shallowly; code folders a bit deeper.
        let depth = if *root == crate::config::home() { 1 } else { 3 };
        find_repos(root, depth, &mut repos);
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for repo in repos {
        for wt in repo_worktrees(&repo) {
            if seen.insert(canonical(&wt.path)) {
                out.push(wt);
            }
        }
    }
    out
}

/// Directories never worth descending into while looking for repositories.
const SKIP: &[&str] = &["node_modules", "target", "Library", "Applications", "Movies", "Music", "Pictures", ".Trash"];

fn find_repos(dir: &Path, depth: usize, out: &mut BTreeSet<PathBuf>) {
    // Only a .git *directory* marks a main checkout; linked worktrees have a
    // .git file and are found through their repository. A stray repository
    // at ~ isn't a project (doctor flags it); look inside it as usual.
    if dir.join(".git").is_dir() && dir != crate::config::home() {
        out.insert(dir.to_path_buf());
        return;
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || SKIP.contains(&name.as_ref()) {
            continue;
        }
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            find_repos(&e.path(), depth - 1, out);
        }
    }
}

pub fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The worktrees of one repository, main checkout first.
pub fn repo_worktrees(repo: &Path) -> Vec<Worktree> {
    let Some(list) = git(repo, &["worktree", "list", "--porcelain"]) else { return Vec::new() };
    let default = default_branch(repo);
    let merged: BTreeSet<String> = default
        .as_deref()
        .and_then(|d| git(repo, &["branch", "--merged", d, "--format=%(refname:short)"]))
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default();

    let mut out = Vec::new();
    for (i, block) in list.split("\n\n").filter(|b| !b.trim().is_empty()).enumerate() {
        let mut path = None;
        let mut branch = None;
        let mut prunable = false;
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p));
            } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
                branch = Some(b.to_string());
            } else if line.starts_with("prunable") {
                prunable = true;
            }
        }
        let Some(path) = path else { continue };
        if prunable || !path.exists() {
            continue;
        }
        let is_merged = match (&branch, &default) {
            (Some(b), Some(d)) => b != d && merged.contains(b),
            _ => false,
        };
        out.push(Worktree {
            last_active: last_active(&path),
            repo: repo.to_path_buf(),
            main: i == 0,
            merged: is_merged,
            branch,
            path,
        });
    }
    out
}

fn default_branch(repo: &Path) -> Option<String> {
    if let Some(s) = git(repo, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) {
        return s.trim().strip_prefix("origin/").map(str::to_string);
    }
    ["main", "master"]
        .into_iter()
        .find(|b| git(repo, &["rev-parse", "--verify", "--quiet", b]).is_some())
        .map(str::to_string)
}

/// The latest of the worktree's index and HEAD reflog modification times.
fn last_active(path: &Path) -> SystemTime {
    let gitdir = git(path, &["rev-parse", "--absolute-git-dir"]).map(|s| PathBuf::from(s.trim()));
    let mut latest = SystemTime::UNIX_EPOCH;
    if let Some(g) = gitdir {
        for f in ["index", "logs/HEAD", "HEAD"] {
            if let Ok(t) = std::fs::metadata(g.join(f)).and_then(|m| m.modified()) {
                latest = latest.max(t);
            }
        }
    }
    latest
}
