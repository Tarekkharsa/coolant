//! Keeps a folder out of Spotlight (and Time Machine) without changing its
//! path.
//!
//! Spotlight skips anything inside a folder whose name ends in `.noindex`,
//! and it doesn't follow symlinks. So the folder's contents move into
//! `~/.coolant/spaces.noindex/<name>` and a symlink takes its place: tools
//! keep reading and writing the old path, while the files physically live
//! where Spotlight won't index them. (A `.metadata_never_index` marker file
//! does *not* work for folders on current macOS; we tested.)

use crate::config::{Config, canonical, home, tilde};
use crate::inuse::InUse;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where tuned folders physically live.
pub fn spaces_dir() -> PathBuf {
    home().join(".coolant/spaces.noindex")
}

/// Whether Spotlight would skip `p`: some component of its real path ends in
/// `.noindex`.
pub fn excluded_from_spotlight(p: &Path) -> bool {
    canonical(p).components().any(|c| c.as_os_str().to_string_lossy().ends_with(".noindex"))
}

#[derive(Debug)]
pub enum Outcome {
    AlreadyExcluded,
    /// Moved (or, if it didn't exist yet, created) at `to`, symlinked back.
    Tuned {
        to: PathBuf,
        created: bool,
    },
    Undone {
        from: PathBuf,
    },
}

/// Tunes `dir`. With `dry_run`, only checks that it would work.
pub fn tune(cfg: &Config, dir: &Path, dry_run: bool, force: bool) -> Result<Outcome> {
    let dir = absolute(dir)?;
    if excluded_from_spotlight(&dir) {
        return Ok(Outcome::AlreadyExcluded);
    }
    if cfg.is_protected(&dir) {
        bail!("{} is protected in {}", tilde(&dir), tilde(&Config::path()));
    }
    if std::fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("{} is already a symlink; not touching it", tilde(&dir));
    }
    if !force && let Some(why) = too_broad(cfg, &dir) {
        bail!("not moving {}: {why} (pass --force if you really mean it)", tilde(&dir));
    }
    let target = spaces_dir().join(space_name(&dir));
    if target.exists() {
        bail!("{} already exists", tilde(&target));
    }
    let exists = dir.exists();
    if exists && !force {
        let users = InUse::scan().users_of(&dir);
        if !users.is_empty() {
            bail!(
                "{} is in use by {} process(es) (pids {:?}); stop them first or pass --force",
                tilde(&dir),
                users.len(),
                users
            );
        }
    }
    if dry_run {
        return Ok(Outcome::Tuned { to: target, created: !exists });
    }

    std::fs::create_dir_all(spaces_dir())?;
    if exists {
        // A rename is atomic and instant, but only within one volume.
        std::fs::rename(&dir, &target).with_context(|| {
            format!("moving {} to {} (both must be on the same volume)", tilde(&dir), tilde(&target))
        })?;
    } else {
        std::fs::create_dir_all(&target)?;
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
    }
    if let Err(e) = std::os::unix::fs::symlink(&target, &dir) {
        // Put things back rather than leave the folder missing.
        if exists {
            let _ = std::fs::rename(&target, &dir);
        }
        return Err(e).with_context(|| format!("symlinking {}", tilde(&dir)));
    }
    exclude_from_backups(&target);
    Ok(Outcome::Tuned { to: target, created: !exists })
}

/// Folders whose children are all linked worktrees (or empty), like
/// `<repo>/.claude/worktrees`. Tuning one of these covers future worktrees.
pub fn dedicated_container(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    entries.flatten().all(|e| {
        let p = e.path();
        e.file_name().to_string_lossy().starts_with('.') || p.join(".git").is_file()
    })
}

/// Folders that are too big a change to move without --force: the home
/// folder, configured roots, and anything holding a main checkout.
fn too_broad(cfg: &Config, dir: &Path) -> Option<&'static str> {
    if dir == home() || cfg.roots().iter().any(|r| r == dir) {
        return Some("it is your home folder or a code root");
    }
    if dir.join(".git").is_dir() {
        return Some("it is a main git checkout");
    }
    let holds_repo =
        std::fs::read_dir(dir).map(|es| es.flatten().any(|e| e.path().join(".git").is_dir())).unwrap_or(false);
    holds_repo.then_some("it contains main git checkouts")
}

/// Reverses [`tune`]: moves the contents back and removes the symlink.
pub fn undo(dir: &Path) -> Result<Outcome> {
    let dir = absolute(dir)?;
    let target = std::fs::read_link(&dir).with_context(|| format!("{} is not a tuned symlink", tilde(&dir)))?;
    if !target.starts_with(spaces_dir()) {
        bail!("{} points to {}, which coolant didn't create", tilde(&dir), tilde(&target));
    }
    std::fs::remove_file(&dir)?;
    std::fs::rename(&target, &dir).with_context(|| format!("moving {} back", tilde(&target)))?;
    Ok(Outcome::Undone { from: target })
}

/// Marks a folder as excluded from Time Machine (sticky, no root needed).
/// Harmless when Time Machine isn't set up.
fn exclude_from_backups(p: &Path) {
    if cfg!(target_os = "macos") {
        let _ = Command::new("tmutil").arg("addexclusion").arg(p).output();
    }
}

/// A readable, unique folder name: the home-relative path with `/` → `%`.
fn space_name(dir: &Path) -> String {
    let rel = dir.strip_prefix(home()).unwrap_or(dir);
    rel.to_string_lossy().trim_matches('/').replace('/', "%")
}

fn absolute(p: &Path) -> Result<PathBuf> {
    let p = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir()?.join(p) };
    // Normalize `..` without resolving symlinks (the last component may be one).
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_flat() {
        let d = home().join("Documents/GitHub/app/.claude/worktrees");
        assert_eq!(space_name(&d), "Documents%GitHub%app%.claude%worktrees");
    }

    #[test]
    fn noindex_detection() {
        assert!(excluded_from_spotlight(Path::new("/tmp/x.noindex/a/b")));
        assert!(!excluded_from_spotlight(Path::new("/tmp/x/a/b")));
    }
}
