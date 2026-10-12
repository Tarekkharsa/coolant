//! Build-artifact discovery, sizes and copy-on-write cloning.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Build output folders that are safe to delete and expensive to recreate.
/// `marker` is a file that must exist inside, so a source folder that happens
/// to share the name isn't mistaken for build output.
pub struct Artifact {
    pub name: &'static str,
    pub marker: Option<&'static str>,
}

pub const ARTIFACTS: &[Artifact] = &[
    Artifact { name: "target", marker: Some("CACHEDIR.TAG") }, // cargo writes this
    Artifact { name: "node_modules", marker: None },
    Artifact { name: ".next", marker: None },
    Artifact { name: ".turbo", marker: None },
    Artifact { name: ".nuxt", marker: None },
    Artifact { name: ".svelte-kit", marker: None },
    Artifact { name: ".parcel-cache", marker: None },
];

fn is_artifact(dir: &Path) -> bool {
    let Some(name) = dir.file_name().and_then(|n| n.to_str()) else { return false };
    ARTIFACTS.iter().any(|a| a.name == name && a.marker.is_none_or(|m| dir.join(m).exists()))
}

/// Artifact folders inside `root`, as paths relative to it. Looks a few
/// levels deep for monorepos (apps/web/node_modules) but never inside an
/// artifact or .git.
pub fn artifacts(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, root, 4, &mut out);
    out.sort();
    out
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let path = e.path();
        // Symlinked artifacts (some tools share node_modules this way) are
        // reported too, but never followed.
        if (ft.is_dir() || ft.is_symlink()) && is_artifact(&path) {
            out.push(path.strip_prefix(root).unwrap().to_path_buf());
        } else if ft.is_dir() && depth > 1 && e.file_name() != ".git" {
            walk(root, &path, depth - 1, out);
        }
    }
}

/// Bytes allocated on disk under `path`, not following symlinks. Clones
/// share blocks with their source, so this overstates what deleting frees.
pub fn disk_usage(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = std::fs::symlink_metadata(path) else { return 0 };
    let own = meta.blocks() * 512;
    if !meta.is_dir() {
        return own;
    }
    let Ok(entries) = std::fs::read_dir(path) else { return own };
    own + entries.flatten().map(|e| disk_usage(&e.path())).sum::<u64>()
}

/// Copies a directory tree with copy-on-write where the file system allows
/// it: APFS clonefile on macOS, reflinks on Linux (btrfs, XFS).
pub fn clone_tree(src: &Path, dst: &Path) -> anyhow::Result<()> {
    let status = if cfg!(target_os = "macos") {
        Command::new("cp").arg("-c").arg("-R").arg(src).arg(dst).status()?
    } else {
        Command::new("cp").args(["-a", "--reflink=auto"]).arg(src).arg(dst).status()?
    };
    anyhow::ensure!(status.success(), "cp {} → {}: {status}", src.display(), dst.display());
    Ok(())
}

/// Deletes a folder without competing with the agents. It is renamed out of
/// the way first (instant, and a `.noindex` name so Spotlight drops it), then
/// deleted on a thread at background priority: on macOS that throttles its
/// CPU and disk use, on Linux it runs at the lowest nice level. A file or
/// symlink is just removed; a symlink's target is never touched.
pub fn remove_quietly(path: &Path) -> std::io::Result<()> {
    if !std::fs::symlink_metadata(path)?.is_dir() {
        return std::fs::remove_file(path);
    }
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    let trash = path.with_file_name(format!(".{name}.coolant-trash-{}.noindex", std::process::id()));
    let target = if std::fs::rename(path, &trash).is_ok() { trash } else { path.to_path_buf() };
    std::thread::spawn(move || {
        background_priority();
        std::fs::remove_dir_all(&target)
    })
    .join()
    .unwrap_or_else(|_| Err(std::io::Error::other("delete thread panicked")))
}

#[cfg(target_os = "macos")]
fn background_priority() {
    unsafe { libc::setpriority(libc::PRIO_DARWIN_THREAD, 0, libc::PRIO_DARWIN_BG) };
}

#[cfg(target_os = "linux")]
fn background_priority() {
    unsafe { libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as _, 19) };
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn background_priority() {}

pub fn human(n: u64) -> String {
    let f = n as f64;
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", f / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.0} MB", f / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.0} KB", f / 1024.0),
        _ => format!("{n} B"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_artifacts_with_markers() {
        let tmp = std::env::temp_dir().join(format!("coolant-fsutil-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        for d in ["node_modules/x", "apps/web/node_modules", "target", "src/target", ".git/node_modules"] {
            std::fs::create_dir_all(tmp.join(d)).unwrap();
        }
        std::fs::write(tmp.join("src/target/CACHEDIR.TAG"), "").unwrap();
        let found = artifacts(&tmp);
        let _ = std::fs::remove_dir_all(&tmp);
        // `target` without CACHEDIR.TAG is not cargo output; .git is skipped.
        let want: Vec<PathBuf> =
            ["apps/web/node_modules", "node_modules", "src/target"].iter().map(PathBuf::from).collect();
        assert_eq!(found, want);
    }

    #[test]
    fn removes_quietly_without_leftovers() {
        let tmp = std::env::temp_dir().join(format!("coolant-quiet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("wt/node_modules/a/b")).unwrap();
        std::fs::write(tmp.join("wt/node_modules/a/b/f.js"), "x").unwrap();
        std::fs::create_dir_all(tmp.join("keep")).unwrap();
        std::fs::write(tmp.join("keep/f"), "x").unwrap();
        std::os::unix::fs::symlink(tmp.join("keep"), tmp.join("wt/link")).unwrap();
        remove_quietly(&tmp.join("wt/node_modules")).unwrap();
        remove_quietly(&tmp.join("wt/link")).unwrap();
        let left: Vec<_> = std::fs::read_dir(tmp.join("wt")).unwrap().collect();
        let kept = tmp.join("keep/f").exists();
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(left.is_empty(), "{left:?}");
        assert!(kept, "a symlink's target must survive");
    }
}
