//! Unprivileged spawn counting. Without root there is no exec/fork event feed
//! (no kdebug, no EndpointSecurity, no dtrace), so a background thread diffs
//! proc_listallpids snapshots. Processes that are born and die between two
//! polls are recovered from PID gaps: XNU hands out PIDs sequentially
//! (nextpid++, wrapping from 99999 back to 100 and skipping IDs still in use),
//! so every unseen PID between the previous allocation cursor and the newest
//! PID belonged to a process we missed.
//!
//! Interval trade-off: the gap logic keeps the count exact at any interval;
//! polling faster only raises the share of spawns that get a Name and PPID
//! (and lowers report latency). A cold proc_listallpids costs ~200 µs of CPU
//! on an idle machine, so polling stays at 50 ms only while processes are
//! appearing and backs off to 400 ms when quiet: measured ~0.07% of one core
//! idle, ~0.2% with a spawn every ~10 ms. Spawns too short to catch alive (most
//! git/rg children) are counted anonymously either way. Gap-inferred spawns
//! are reported once some later process is observed alive, which on a machine
//! running agents is usually within a poll or two.

use crate::*;
use std::mem::{MaybeUninit, size_of};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// While processes are being created.
const INTERVAL: Duration = Duration::from_millis(50);
/// Back-off ceiling when nothing spawns.
const IDLE_MAX: Duration = Duration::from_millis(400);
/// PID_MAX: allocation wraps when nextpid reaches it.
const PID_MAX: i32 = 99999;
/// ...and restarts here.
const PID_WRAP: i32 = 100;
/// Largest plausible cursor advance per poll.
const WINDOW: i32 = 30000;

/// Bitmap of live PIDs below [`PID_MAX`].
struct PidSet(Box<[u64; PID_MAX as usize / 64 + 1]>);

impl PidSet {
    fn new() -> Self {
        PidSet(Box::new([0; PID_MAX as usize / 64 + 1]))
    }

    fn has(&self, p: i32) -> bool {
        (0..PID_MAX).contains(&p) && self.0[p as usize / 64] & (1 << (p % 64)) != 0
    }

    fn add(&mut self, p: i32) {
        if (0..PID_MAX).contains(&p) {
            self.0[p as usize / 64] |= 1 << (p % 64);
        }
    }

    fn clear(&mut self) {
        self.0.fill(0);
    }
}

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
    let mut p = Poller { prev: PidSet::new(), cur: PidSet::new(), pids: vec![0; 4096], cursor: 0 };
    if list_pids(&mut p.pids, &mut p.prev).is_none() {
        return Err(Error::Unsupported);
    }
    p.cursor = newest_pid().ok_or(Error::Unsupported)?;

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
    prev: PidSet,
    cur: PidSet,
    pids: Vec<i32>,
    cursor: i32,
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
        let Some(n) = list_pids(&mut self.pids, &mut self.cur) else { return false };
        let mut found = Vec::new();
        let (mut newest, mut far) = (-1, 0);
        for &p in &self.pids[..n] {
            if self.prev.has(p) {
                continue;
            }
            let mut s = Spawn { pid: p, ..Default::default() };
            identify(&mut s);
            found.push(s);
            let d = ahead(self.cursor, p);
            if d > far && d < WINDOW {
                (newest, far) = (p, d);
            }
        }
        // PIDs allocated between the old cursor and the newest PID that are not
        // alive now and were not alive before: born and died between polls.
        if newest >= 0 {
            let mut p = self.cursor + 1;
            while p != newest {
                if p >= PID_MAX {
                    p = PID_WRAP;
                    if p == newest {
                        break;
                    }
                }
                if !self.cur.has(p) && !self.prev.has(p) {
                    found.push(Spawn { pid: p, ..Default::default() });
                }
                p += 1;
            }
            self.cursor = newest;
        }

        let mut b = shared.lock().unwrap();
        // Last poll's spawns may have exec'd since: refresh names of survivors.
        for s in &mut b.pending {
            if !s.name.is_empty() && self.cur.has(s.pid) {
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
}

/// How far PID `p` lies past `cursor` in allocation order.
fn ahead(cursor: i32, p: i32) -> i32 {
    if p > cursor {
        return p - cursor;
    }
    (PID_MAX - cursor) + (p - PID_WRAP)
}

/// Snapshots all PIDs into `set` and `pids[..n]`, returning n. `pids` only
/// ever grows, so its length is the capacity to offer the kernel.
fn list_pids(pids: &mut Vec<i32>, set: &mut PidSet) -> Option<usize> {
    loop {
        // SAFETY: the kernel writes at most `len * 4` bytes into the buffer.
        let n =
            unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * size_of::<i32>()) as libc::c_int) };
        if n <= 0 {
            return None;
        }
        let n = n as usize;
        if n < pids.len() {
            set.clear();
            for &p in &pids[..n] {
                set.add(p);
            }
            return Some(n);
        }
        pids.resize(2 * pids.len(), 0);
    }
}

/// Fills in name and parent of `s.pid`. PROC_PIDT_SHORTBSDINFO and
/// proc_pidpath work for other users' processes; proc_name and
/// PROC_PIDTBSDINFO do not.
fn identify(s: &mut Spawn) {
    let mut si = MaybeUninit::<libc::proc_bsdshortinfo>::zeroed();
    // SAFETY: the buffer is exactly one proc_bsdshortinfo.
    let n = unsafe {
        libc::proc_pidinfo(
            s.pid,
            libc::PROC_PIDT_SHORTBSDINFO,
            0,
            si.as_mut_ptr().cast(),
            size_of::<libc::proc_bsdshortinfo>() as libc::c_int,
        )
    };
    if n != size_of::<libc::proc_bsdshortinfo>() as libc::c_int {
        return;
    }
    // SAFETY: proc_pidinfo filled the whole struct.
    let si = unsafe { si.assume_init() };
    let mut path = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the buffer length is passed alongside the pointer.
    let len = unsafe { libc::proc_pidpath(s.pid, path.as_mut_ptr().cast(), path.len() as u32) };
    let comm;
    let full = if len > 0 {
        &path[..len as usize]
    } else {
        // Truncated to 16 bytes.
        comm = si.pbsi_comm.map(|c| c as u8);
        comm.split(|&b| b == 0).next().unwrap_or_default()
    };
    let base = full.rsplit(|&b| b == b'/').next().unwrap_or(full);
    s.name = String::from_utf8_lossy(base).into_owned();
    s.ppid = si.pbsi_ppid as i32;
}

/// The prefix of `struct kinfo_proc` (648 bytes on macOS) that we read.
#[repr(C)]
#[derive(Clone, Copy)]
struct KInfoProc {
    /// extern_proc.p_un.__p_starttime
    start_sec: i64,
    start_usec: i32,
    _pad0: [u8; 28],
    /// extern_proc.p_pid, at offset 40
    pid: i32,
    _rest: [u8; 604],
}

const _: () = assert!(size_of::<KInfoProc>() == 648);

/// Returns the PID of the most recently started process, which marks the
/// allocation cursor even after PIDs have wrapped.
fn newest_pid() -> Option<i32> {
    const SZ: usize = size_of::<KInfoProc>();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_ALL];
    let mut size = 0usize;
    // SAFETY: a null buffer asks only for the required size.
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) } != 0 {
        return None;
    }
    size += size / 4;
    let zero = KInfoProc { start_sec: 0, start_usec: 0, _pad0: [0; 28], pid: 0, _rest: [0; 604] };
    let mut kp = vec![zero; size / SZ];
    size = kp.len() * SZ;
    // SAFETY: `size` is the byte length of `kp`.
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, kp.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) } != 0 {
        return None;
    }
    let mut best = None;
    let mut bt = (0i64, 0i32);
    for k in &kp[..size / SZ] {
        let t = (k.start_sec, k.start_usec);
        if t >= bt {
            bt = t;
            best = Some(k.pid);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::process::Command;

    #[test]
    fn sees_short_lived_processes() {
        let mut sc = open().unwrap();
        thread::sleep(Duration::from_millis(100));
        sc.drain();

        const N: usize = 200;
        let mut want = HashSet::new();
        for _ in 0..N {
            let mut child = Command::new("/usr/bin/true").spawn().unwrap();
            want.insert(child.id() as i32);
            child.wait().unwrap();
        }
        // Spawns that die between polls are inferred from PID gaps, which close
        // once a later process is seen alive; start one so the tail is counted.
        let mut sentinel = Command::new("/bin/sleep").arg("1").spawn().unwrap();
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
        let _ = sentinel.kill();
        let _ = sentinel.wait();
        eprintln!("spawned {N}, counted {seen} of them ({named} named), {total} spawns system-wide");
        assert!(seen >= N * 9 / 10, "counted {seen} of {N} spawns, want >= 90%");
        assert!(sc.drain().len() <= total, "drain did not clear the buffer");
    }

    /// Needs the process to itself: run with `--ignored --test-threads=1`.
    #[test]
    #[ignore = "takes 3s and measures whole-process CPU"]
    fn overhead() {
        fn cpu() -> Duration {
            let mut ru = MaybeUninit::<libc::rusage>::zeroed();
            // SAFETY: getrusage fills the struct on success.
            let ru = unsafe {
                libc::getrusage(libc::RUSAGE_SELF, ru.as_mut_ptr());
                ru.assume_init()
            };
            let tv = |t: libc::timeval| Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1000);
            tv(ru.ru_utime) + tv(ru.ru_stime)
        }
        let before = cpu();
        let sc = open().unwrap();
        let d = Duration::from_secs(3);
        thread::sleep(d);
        drop(sc);
        let used = cpu() - before;
        let pct = 100.0 * used.as_secs_f64() / d.as_secs_f64();
        eprintln!("spawn counter used {used:?} CPU in {d:?} ({pct:.2}% of one core)");
        assert!(pct <= 1.0, "overhead {pct:.2}% of one core, want < 1%");
    }

    #[test]
    fn ahead_wraps() {
        for (p, want) in [(99995, 5), (100, 9), (105, 14), (99990, PID_MAX - 99990 + 99990 - PID_WRAP)] {
            assert_eq!(ahead(99990, p), want, "ahead({p})");
        }
    }

    #[test]
    fn drop_stops_thread() {
        let t = Instant::now();
        drop(open().unwrap());
        assert!(t.elapsed() < INTERVAL, "drop took {:?}", t.elapsed());
    }
}
