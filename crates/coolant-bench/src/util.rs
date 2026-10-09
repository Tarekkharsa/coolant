//! Small helpers shared by the modules: cancellation, running commands, and
//! temporary-name generation.

use anyhow::{Result, anyhow};
use std::collections::hash_map::RandomState;
use std::ffi::OsStr;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

/// The error returned when the cancel flag was set.
#[derive(Debug)]
pub struct Canceled;

impl std::fmt::Display for Canceled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("benchmark canceled")
    }
}

impl std::error::Error for Canceled {}

pub(crate) fn is_canceled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Relaxed)
}

/// Errors with [`Canceled`] once the flag is set.
pub(crate) fn check(cancel: &AtomicBool) -> Result<()> {
    if is_canceled(cancel) { Err(Canceled.into()) } else { Ok(()) }
}

/// Sleeps for `d`, returning early if canceled.
pub(crate) fn sleep(cancel: &AtomicBool, d: Duration) {
    let end = Instant::now() + d;
    while !is_canceled(cancel) {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        std::thread::sleep(left.min(Duration::from_millis(50)));
    }
}

/// Runs a command with stdin closed and stdout/stderr captured. Setting the
/// cancel flag kills the child (the equivalent of `exec.CommandContext`).
pub(crate) fn output(cancel: &AtomicBool, cmd: &mut Command) -> Result<Output> {
    check(cancel)?;
    let child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let pid = child.id() as libc::pid_t;
    let (done, rx) = mpsc::channel::<()>();
    let out = std::thread::scope(|s| {
        s.spawn(move || {
            loop {
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Err(RecvTimeoutError::Timeout) if is_canceled(cancel) => {
                        // SAFETY: plain kill(2) on the child we just spawned,
                        // which is only reaped after `done` is dropped below.
                        unsafe { libc::kill(pid, libc::SIGKILL) };
                        return;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    _ => return,
                }
            }
        });
        let r = child.wait_with_output();
        drop(done);
        r
    })?;
    if is_canceled(cancel) && !out.status.success() {
        return Err(Canceled.into());
    }
    Ok(out)
}

/// Runs `cmd` and turns a failure into `"<what>: <status>: <output>"`.
/// Returns the combined stdout and stderr on success.
pub(crate) fn run_checked(cancel: &AtomicBool, cmd: &mut Command, what: &str) -> Result<Vec<u8>> {
    let out = output(cancel, cmd)?;
    let mut all = out.stdout;
    all.extend_from_slice(&out.stderr);
    if !out.status.success() {
        return Err(anyhow!("{what}: {}: {}", out.status, String::from_utf8_lossy(&all).trim()));
    }
    Ok(all)
}

/// Looks `name` up in PATH (like `exec.LookPath`).
pub(crate) fn which(name: impl AsRef<OsStr>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name.as_ref()))
        .find(|p| p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

/// Random bytes from the std hasher's per-instance random keys; good enough
/// for unique names and tags.
pub(crate) fn random_u64() -> u64 {
    RandomState::new().build_hasher().finish()
}

pub(crate) fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    for chunk in out.chunks_mut(8) {
        chunk.copy_from_slice(&random_u64().to_le_bytes()[..chunk.len()]);
    }
    out
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub(crate) fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Lexically cleans a path (like Go's `filepath.Clean`): drops `.`, resolves
/// `..` against preceding components.
pub(crate) fn clean(path: &Path) -> PathBuf {
    use std::path::Component::*;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            CurDir => {}
            ParentDir => {
                if !out.pop() && !path.is_absolute() {
                    out.push("..");
                }
            }
            c => out.push(c.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() { PathBuf::from(".") } else { out }
}

/// Local time as `YYYYMMDD-HHMMSS`.
pub(crate) fn timestamp() -> String {
    // SAFETY: localtime_r writes into the zeroed tm we pass.
    let tm = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}
