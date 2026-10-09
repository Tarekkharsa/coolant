//! Unprivileged spawn counting. Without root there is no proc connector, so a
//! background thread diffs /proc listings. Processes that are born and die
//! between two polls are recovered from the PID allocator: Linux hands out
//! PIDs sequentially (wrapping at pid_max back to 300), and the fifth field of
//! /proc/loadavg is the most recently allocated PID, so every PID between the
//! previous cursor and the current one that is not alive was a process we
//! missed.
//!
//! Caveat: threads draw from the same PID space. A gap PID that still exists
//! as a thread (/proc/<pid> is present) is skipped, but a thread that was
//! created and exited within one poll interval is indistinguishable from a
//! short-lived process, so thread churn inflates the count. Gap-inferred
//! spawns carry no name or parent.
//!
//! Polling is adaptive like the macOS backend: fast while processes appear,
//! backing off when quiet. Listing /proc is cheap (one getdents pass).

use super::{names, parse_stat, read_lossy};
use crate::*;
use std::collections::HashSet;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// While processes are being created.
const INTERVAL: Duration = Duration::from_millis(50);
/// Back-off ceiling when nothing spawns.
const IDLE_MAX: Duration = Duration::from_millis(400);
/// Allocation restarts here after wrapping (the kernel's RESERVED_PIDS).
const PID_WRAP: i32 = 300;
/// Largest cursor advance we trust per poll; beyond it the PID space has
/// lapped (or we stalled for ages) and the delta is meaningless.
const WINDOW: i32 = 200_000;

/// Spawns shared between the polling thread and `drain`.
#[derive(Default)]
struct Buffers {
    /// Identified spawns awaiting drain.
    out: Vec<Spawn>,
    /// Found in the latest poll; re-identified next poll.
    pending: Vec<Spawn>,
}

struct Counter {
    shared: Arc<Mutex<Buffers>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

pub fn open() -> Result<Box<dyn SpawnCounter>> {
    let pid_max = read_lossy("/proc/sys/kernel/pid_max").and_then(|s| s.trim().parse().ok()).unwrap_or(32768);
    // Cursor first, listing second: anything allocated after the cursor is
    // then in the baseline or counted as new by the first poll.
    let cursor = last_pid().ok_or(Error::Unsupported)?;
    let mut p = Poller { prev: HashSet::new(), cur: HashSet::new(), cursor, pid_max };
    if !list_pids(&mut p.prev) {
        return Err(Error::Unsupported);
    }

    let shared = Arc::new(Mutex::new(Buffers::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let (shared, stop) = (shared.clone(), stop.clone());
        thread::Builder::new()
            .name("coolant-spawn".into())
            .spawn(move || p.run(&shared, &stop))
            .map_err(|e| Error::Other(format!("spawn counter thread: {e}")))?
    };
    Ok(Box::new(Counter { shared, stop, thread: Some(thread) }))
}

impl SpawnCounter for Counter {
    fn drain(&mut self) -> Vec<Spawn> {
        let mut b = self.shared.lock().unwrap();
        let mut out = std::mem::take(&mut b.out);
        out.append(&mut b.pending);
        out
    }
}

impl Drop for Counter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

/// State owned by the polling thread.
struct Poller {
    prev: HashSet<i32>,
    cur: HashSet<i32>,
    /// Most recently allocated PID as of the previous poll.
    cursor: i32,
    pid_max: i32,
}

impl Poller {
    fn run(&mut self, shared: &Mutex<Buffers>, stop: &AtomicBool) {
        let mut wait = INTERVAL;
        loop {
            let deadline = Instant::now() + wait;
            while !stop.load(Ordering::Acquire) {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                thread::park_timeout(left);
            }
            if stop.load(Ordering::Acquire) {
                return;
            }
            wait = if self.poll(shared) { INTERVAL } else { (2 * wait).min(IDLE_MAX) };
        }
    }

    /// Records spawns since the previous poll and reports whether any.
    fn poll(&mut self, shared: &Mutex<Buffers>) -> bool {
        // Cursor before listing: a process allocated in between is then in
        // the listing (counted as new) and, being past the cursor, cannot be
        // counted again from the gap next time because it is in `prev`.
        let Some(last) = last_pid() else { return false };
        if !list_pids(&mut self.cur) {
            return false;
        }
        let mut found = Vec::new();
        for &p in &self.cur {
            if !self.prev.contains(&p) {
                let mut s = Spawn { pid: p, ..Default::default() };
                identify(&mut s);
                found.push(s);
            }
        }
        // PIDs allocated since the cursor that are neither alive now nor were
        // before (and are not threads) belonged to processes that came and
        // went between polls.
        let span = self.advance(last);
        if (1..WINDOW).contains(&span) {
            let mut p = self.cursor;
            for _ in 0..span {
                p = if p + 1 >= self.pid_max { PID_WRAP } else { p + 1 };
                if !self.cur.contains(&p)
                    && !self.prev.contains(&p)
                    && !fs::exists(format!("/proc/{p}")).unwrap_or(false)
                {
                    found.push(Spawn { pid: p, ..Default::default() });
                }
            }
        }
        self.cursor = last;

        let mut b = shared.lock().unwrap();
        // Last poll's spawns may have exec'd since: refresh names of survivors.
        for s in &mut b.pending {
            if !s.name.is_empty() && self.cur.contains(&s.pid) {
                identify(s);
            }
        }
        let mut pending = std::mem::replace(&mut b.pending, found);
        b.out.append(&mut pending);
        let any = !b.pending.is_empty();
        drop(b);
        std::mem::swap(&mut self.prev, &mut self.cur);
        any
    }

    /// How many PIDs were allocated moving from the cursor to `last`.
    fn advance(&self, last: i32) -> i32 {
        pids_between(self.cursor, last, self.pid_max)
    }
}

/// Number of allocations from cursor `from` to cursor `to`, allowing for one
/// wrap past `pid_max` back to [`PID_WRAP`]. 0 means nothing happened.
fn pids_between(from: i32, to: i32, pid_max: i32) -> i32 {
    if to >= from { to - from } else { (pid_max - 1 - from) + (to - PID_WRAP + 1) }
}

/// The most recently allocated PID, from /proc/loadavg ("0.1 0.2 0.3 1/234 5678").
fn last_pid() -> Option<i32> {
    read_lossy("/proc/loadavg")?.split_ascii_whitespace().nth(4)?.parse().ok()
}

/// Fills `set` with the PIDs in /proc.
fn list_pids(set: &mut HashSet<i32>) -> bool {
    let Ok(dir) = fs::read_dir("/proc") else { return false };
    set.clear();
    set.extend(dir.flatten().filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok()));
    !set.is_empty()
}

/// Fills in name and parent of `s.pid`, if it is still alive.
fn identify(s: &mut Spawn) {
    let Some(st) = read_lossy(&format!("/proc/{}/stat", s.pid)).and_then(|t| parse_stat(&t)) else { return };
    let cmdline = fs::read(format!("/proc/{}/cmdline", s.pid)).unwrap_or_default();
    s.name = names(&st.comm, &cmdline).0;
    s.ppid = st.ppid;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn sees_short_lived_processes() {
        let mut sc = open().unwrap();
        thread::sleep(Duration::from_millis(100));
        sc.drain();

        const N: usize = 200;
        let mut want = HashSet::new();
        for _ in 0..N {
            let mut child = Command::new("/bin/true").spawn().unwrap();
            want.insert(child.id() as i32);
            child.wait().unwrap();
        }
        thread::sleep(3 * IDLE_MAX);

        let (mut seen, mut named, mut total) = (0, 0, 0);
        for s in sc.drain() {
            total += 1;
            if want.contains(&s.pid) {
                seen += 1;
                if s.name == "true" {
                    named += 1;
                }
            }
        }
        eprintln!("spawned {N}, counted {seen} of them ({named} named), {total} spawns system-wide");
        assert!(seen >= N * 9 / 10, "counted {seen} of {N} spawns, want >= 90%");
        assert!(sc.drain().len() <= total, "drain did not clear the buffer");
    }

    #[test]
    fn names_live_children() {
        let mut sc = open().unwrap();
        thread::sleep(Duration::from_millis(100));
        sc.drain();
        let mut child = Command::new("sleep").arg("2").spawn().unwrap();
        thread::sleep(Duration::from_millis(300));
        let got = sc.drain();
        let _ = child.kill();
        let _ = child.wait();
        let s = got.iter().find(|s| s.pid == child.id() as i32).expect("sleep not seen");
        assert_eq!(s.name, "sleep");
        assert_eq!(s.ppid, std::process::id() as i32);
    }

    #[test]
    fn pid_arithmetic_wraps() {
        assert_eq!(pids_between(100, 100, 32768), 0);
        assert_eq!(pids_between(100, 105, 32768), 5);
        // 32766, 32767 (last before pid_max), then 300, 301.
        assert_eq!(pids_between(32765, 301, 32768), 2 + 2);
        assert_eq!(pids_between(32767, 300, 32768), 1);
    }

    #[test]
    fn drop_stops_thread() {
        let t = Instant::now();
        drop(open().unwrap());
        assert!(t.elapsed() < INTERVAL, "drop took {:?}", t.elapsed());
    }
}
