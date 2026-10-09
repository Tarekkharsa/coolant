//! Gives a fresh worktree the main checkout's build artifacts.
//!
//! A new worktree starts with no target/ or node_modules, so the agent's
//! first `cargo build` or `pnpm install` redoes everything: minutes of every
//! core at 100%, once per worktree. Cloning the main checkout's artifacts
//! with copy-on-write takes about a second, uses no extra disk until files
//! diverge, and turns that first build into an incremental one. Cargo keeps
//! dependency fingerprints independent of the workspace path, so only the
//! workspace's own crates rebuild.

use crate::config::tilde;
use crate::discover::repo_worktrees;
use crate::fsutil::{artifacts, clone_tree};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Cloned {
    pub rel: PathBuf,
    pub took: Duration,
}

#[derive(Debug, Default)]
pub struct Report {
    pub source: PathBuf,
    pub cloned: Vec<Cloned>,
    /// Present in the source but already in the worktree (left alone).
    pub skipped: Vec<PathBuf>,
}

/// Clones artifacts missing from `worktree` out of `source` (default: the
/// repository's main checkout).
pub fn prewarm(worktree: &Path, source: Option<&Path>, dry_run: bool) -> Result<Report> {
    let worktree = std::fs::canonicalize(worktree).with_context(|| format!("{} not found", worktree.display()))?;
    let source = match source {
        Some(s) => std::fs::canonicalize(s)?,
        None => main_checkout(&worktree)?,
    };
    if source == worktree {
        bail!("{} is the main checkout; pass --from to choose a source", tilde(&worktree));
    }
    let mut report = Report { source: source.clone(), ..Default::default() };
    for rel in artifacts(&source) {
        let (src, dst) = (source.join(&rel), worktree.join(&rel));
        // Symlinked artifacts in the source are shared on purpose; skip.
        if std::fs::symlink_metadata(&src).is_ok_and(|m| m.file_type().is_symlink()) {
            continue;
        }
        // Only where the worktree has the same project folder.
        let parent_exists = dst.parent().is_some_and(Path::is_dir);
        if std::fs::symlink_metadata(&dst).is_ok() || !parent_exists {
            report.skipped.push(rel);
            continue;
        }
        let start = Instant::now();
        if !dry_run {
            clone_tree(&src, &dst)?;
        }
        report.cloned.push(Cloned { rel, took: start.elapsed() });
    }
    Ok(report)
}

fn main_checkout(worktree: &Path) -> Result<PathBuf> {
    let wts = repo_worktrees(worktree);
    let main = wts.into_iter().find(|w| w.main).context("not inside a git repository")?;
    Ok(std::fs::canonicalize(&main.path)?)
}

/// A git post-checkout hook that prewarms every new worktree in the
/// background. `git worktree add` runs post-checkout with a null previous
/// HEAD, which is how a new worktree is told apart from a branch switch.
pub const HOOK: &str = r#"#!/bin/sh
# coolant: prewarm new worktrees with the main checkout's build artifacts.
case "$1" in
  *[!0]*) ;; # an ordinary checkout
  *) command -v coolant >/dev/null 2>&1 && (coolant prewarm "$(pwd)" >/dev/null 2>&1 &) ;;
esac
"#;

/// Installs [`HOOK`] in a repository unless it already has a post-checkout
/// hook (which we won't overwrite).
pub fn install_hook(repo: &Path) -> Result<PathBuf> {
    let hooks = crate::discover::git(repo, &["rev-parse", "--git-path", "hooks"]).context("not a git repository")?;
    let hooks = repo.join(hooks.trim());
    let path = hooks.join("post-checkout");
    if path.exists() {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current.contains("coolant prewarm") {
            return Ok(path);
        }
        bail!("{} already exists; add this to it:\n\n{HOOK}", tilde(&path));
    }
    std::fs::create_dir_all(&hooks)?;
    std::fs::write(&path, HOOK)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn clones_missing_artifacts_into_worktree() {
        let tmp = std::env::temp_dir().join(format!("coolant-prewarm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let main = tmp.join("main");
        std::fs::create_dir_all(main.join("apps/web")).unwrap();
        std::fs::write(main.join("apps/web/package.json"), "{}").unwrap();
        git(&main, &["init", "-q", "-b", "main"]);
        git(&main, &["add", "."]);
        git(&main, &["commit", "-q", "-m", "init"]);
        std::fs::create_dir_all(main.join("target/debug")).unwrap();
        std::fs::write(main.join("target/CACHEDIR.TAG"), "").unwrap();
        std::fs::create_dir_all(main.join("apps/web/node_modules/x")).unwrap();
        let wt = tmp.join("wt");
        git(&main, &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "feature"]);

        let report = prewarm(&wt, None, false).unwrap();
        let rels: Vec<_> = report.cloned.iter().map(|c| c.rel.clone()).collect();
        assert_eq!(rels, [PathBuf::from("apps/web/node_modules"), PathBuf::from("target")]);
        assert!(wt.join("target/CACHEDIR.TAG").exists() && wt.join("apps/web/node_modules/x").is_dir());
        // Running again changes nothing.
        assert!(prewarm(&wt, None, false).unwrap().cloned.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
