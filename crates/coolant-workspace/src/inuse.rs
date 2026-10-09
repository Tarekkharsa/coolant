//! Which processes are using a folder: running in it, or naming it on their
//! command line (a dev server started with `--env-file=/that/folder/...`).

use crate::config::canonical;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct InUse {
    cwds: Vec<(i32, PathBuf)>,
    commands: Vec<(i32, String)>,
}

impl InUse {
    /// Snapshots every visible process's working directory and command line.
    pub fn scan() -> InUse {
        InUse { cwds: cwds(), commands: commands() }
    }

    /// A process's working directory, if visible.
    pub fn cwd(&self, pid: i32) -> Option<&Path> {
        self.cwds.iter().find(|(p, _)| *p == pid).map(|(_, c)| c.as_path())
    }

    /// A process's command line, if visible.
    pub fn command(&self, pid: i32) -> Option<&str> {
        self.commands.iter().find(|(p, _)| *p == pid).map(|(_, c)| c.as_str())
    }

    /// PIDs of processes using `dir`.
    pub fn users_of(&self, dir: &Path) -> Vec<i32> {
        let real = canonical(dir);
        let given = dir.to_string_lossy();
        let real_s = real.to_string_lossy();
        let mut pids: Vec<i32> = self
            .cwds
            .iter()
            .filter(|(_, cwd)| cwd.starts_with(&real) || cwd.starts_with(dir))
            .map(|(p, _)| *p)
            .chain(self.commands.iter().filter(|(_, c)| mentions(c, &given) || mentions(c, &real_s)).map(|(p, _)| *p))
            .filter(|p| *p != std::process::id() as i32)
            .collect();
        pids.sort_unstable();
        pids.dedup();
        pids
    }
}

/// Whether a command line contains `dir` as a whole path prefix (so that
/// `/a/b` doesn't match `/a/bc`).
fn mentions(cmd: &str, dir: &str) -> bool {
    let dir = dir.trim_end_matches('/');
    cmd.match_indices(dir).any(|(i, _)| {
        let rest = &cmd[i + dir.len()..];
        rest.is_empty() || rest.starts_with('/') || rest.starts_with(' ') || rest.starts_with('"')
    })
}

#[cfg(target_os = "linux")]
fn cwds() -> Vec<(i32, PathBuf)> {
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let pid: i32 = e.file_name().to_str()?.parse().ok()?;
            Some((pid, std::fs::read_link(e.path().join("cwd")).ok()?))
        })
        .collect()
}

/// `lsof -d cwd -Fpn` prints `p<pid>` then `n<path>` for each process.
#[cfg(not(target_os = "linux"))]
fn cwds() -> Vec<(i32, PathBuf)> {
    let Ok(out) = Command::new("lsof").args(["-n", "-d", "cwd", "-Fpn"]).output() else { return Vec::new() };
    let mut res = Vec::new();
    let mut pid = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(p) = line.strip_prefix('p') {
            pid = p.parse().ok();
        } else if let (Some(n), Some(p)) = (line.strip_prefix('n'), pid) {
            res.push((p, PathBuf::from(n)));
        }
    }
    res
}

fn commands() -> Vec<(i32, String)> {
    let Ok(out) = Command::new("ps").args(["-A", "-o", "pid=,command="]).output() else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let l = l.trim_start();
            let (pid, cmd) = l.split_once(' ')?;
            Some((pid.parse().ok()?, cmd.trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mention_is_path_prefix() {
        assert!(mentions("node --env-file=/a/b/.env src/main.ts", "/a/b"));
        assert!(mentions("vite /a/b", "/a/b"));
        assert!(!mentions("node /a/bc/x.js", "/a/b"));
    }

    #[test]
    fn finds_own_cwd() {
        let dir = std::env::current_dir().unwrap();
        let mut child = Command::new("sleep").arg("5").current_dir(&dir).spawn().unwrap();
        let users = InUse::scan().users_of(&dir);
        child.kill().unwrap();
        let _ = child.wait();
        assert!(users.contains(&(child.id() as i32)), "{users:?}");
    }
}
