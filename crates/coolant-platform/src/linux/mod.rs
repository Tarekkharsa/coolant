//! Linux backend, unprivileged, built on /proc and /sys.
//!
//! Every process comes from `/proc/[pid]/{stat,status,cmdline}`. The richer
//! counters (`/proc/[pid]/io`) are readable only for processes we own (or as
//! root), which is what `detailed` means here. Reaped-children CPU
//! (`cutime + cstime`) is in `stat` and so is filled in for every process.
//!
//! `wakeups` is not a real wakeup count (Linux has no per-process equivalent
//! of macOS interrupt wakeups); it is `voluntary_ctxt_switches +
//! nonvoluntary_ctxt_switches` from `status`, a cumulative scheduling-activity
//! counter that is readable for every process.

mod sensors;
mod spawn;

use crate::*;
use std::collections::HashMap;
use std::ffi::CStr;
use std::fs;
use std::sync::{Mutex, OnceLock};

pub struct Linux {
    users: Mutex<HashMap<u32, String>>,
}

impl Linux {
    pub fn new() -> Self {
        Linux { users: Mutex::default() }
    }

    fn user_name(&self, uid: u32) -> String {
        let mut users = self.users.lock().unwrap();
        users.entry(uid).or_insert_with(|| lookup_user(uid).unwrap_or_else(|| uid.to_string())).clone()
    }

    /// Reads one process; None if it vanished (or was never readable).
    fn read_process(&self, pid: i32) -> Option<Process> {
        let dir = format!("/proc/{pid}/");
        let st = parse_stat(&read_lossy(&format!("{dir}stat"))?)?;
        let status = parse_status(&read_lossy(&format!("{dir}status"))?);
        let (name, command) = names(&st.comm, &fs::read(format!("{dir}cmdline")).unwrap_or_default());

        let mut p = Process {
            pid,
            ppid: st.ppid,
            uid: status.uid,
            user: self.user_name(status.uid),
            name,
            command,
            cpu_time: ticks(st.utime + st.stime),
            rss_bytes: st.rss_pages * page_size(),
            child_cpu_time: ticks(st.cutime + st.cstime),
            wakeups: status.ctxt_switches,
            start_ms: boot_ms() + st.starttime.saturating_mul(1000) / clk_tck(),
            ..Default::default()
        };
        // Opening is what fails for other users' processes; a process that
        // exits mid-read still yields the counters it had.
        if let Some(io) = read_lossy(&format!("{dir}io")) {
            p.detailed = true;
            let (r, w) = parse_io(&io);
            p.disk_read = r;
            p.disk_written = w;
        }
        Some(p)
    }
}

impl Backend for Linux {
    fn name(&self) -> &'static str {
        "linux"
    }

    fn machine(&self) -> Machine {
        Machine {
            os: "linux".into(),
            os_version: os_version(),
            model: model(),
            cpu: cpu_name(),
            cores: std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(0),
            mem_bytes: mem_total(),
        }
    }

    fn cpu(&self) -> Result<CpuTimes> {
        let stat = read_lossy("/proc/stat").ok_or_else(|| Error::Other("cannot read /proc/stat".into()))?;
        parse_cpu(&stat).ok_or_else(|| Error::Other("malformed /proc/stat".into()))
    }

    fn sensors(&self) -> Result<Box<dyn SensorReader>> {
        sensors::open()
    }

    fn spawn_counter(&self) -> Result<Box<dyn SpawnCounter>> {
        spawn::open()
    }

    fn processes(&self) -> Result<Vec<Process>> {
        let dir = fs::read_dir("/proc").map_err(|e| Error::Other(format!("/proc: {e}")))?;
        let mut procs = Vec::with_capacity(512);
        for ent in dir.flatten() {
            let Some(pid) = ent.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
            if let Some(p) = self.read_process(pid) {
                procs.push(p);
            }
        }
        Ok(procs)
    }
}

// ---- parsing ----

/// The /proc/[pid]/stat fields we use.
#[derive(Debug, Default, PartialEq, Eq)]
struct Stat {
    comm: String,
    ppid: i32,
    utime: u64,
    stime: u64,
    cutime: u64,
    cstime: u64,
    starttime: u64,
    rss_pages: u64,
}

/// Parses /proc/[pid]/stat. `comm` is parenthesised and may itself contain
/// spaces and parentheses, so it spans from the first '(' to the LAST ')'.
fn parse_stat(s: &str) -> Option<Stat> {
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    if close < open {
        return None;
    }
    // After the comm: state ppid pgrp session tty tpgid flags minflt cminflt
    // majflt cmajflt utime stime cutime cstime priority nice threads
    // itrealvalue starttime vsize rss ...
    let f: Vec<&str> = s[close + 1..].split_ascii_whitespace().collect();
    let num = |i: usize| f.get(i)?.parse::<i64>().ok();
    let unsigned = |i: usize| num(i).map(|v| v.max(0) as u64);
    Some(Stat {
        comm: s[open + 1..close].to_string(),
        ppid: num(1)? as i32,
        utime: unsigned(11)?,
        stime: unsigned(12)?,
        cutime: unsigned(13)?,
        cstime: unsigned(14)?,
        starttime: unsigned(19)?,
        rss_pages: unsigned(21)?,
    })
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Status {
    /// Real UID.
    uid: u32,
    ctxt_switches: u64,
}

fn parse_status(s: &str) -> Status {
    let mut st = Status::default();
    for line in s.lines() {
        let Some((key, val)) = line.split_once(':') else { continue };
        match key {
            "Uid" => st.uid = val.split_ascii_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0),
            "voluntary_ctxt_switches" | "nonvoluntary_ctxt_switches" => {
                st.ctxt_switches += val.trim().parse::<u64>().unwrap_or(0)
            }
            _ => {}
        }
    }
    st
}

/// (read_bytes, write_bytes): what actually hit the storage layer.
fn parse_io(s: &str) -> (u64, u64) {
    let (mut r, mut w) = (0, 0);
    for line in s.lines() {
        match line.split_once(':') {
            Some(("read_bytes", v)) => r = v.trim().parse().unwrap_or(0),
            Some(("write_bytes", v)) => w = v.trim().parse().unwrap_or(0),
            _ => {}
        }
    }
    (r, w)
}

/// Aggregate CPU time from the first line of /proc/stat, in USER_HZ ticks.
fn parse_cpu(s: &str) -> Option<CpuTimes> {
    let mut it = s.lines().next()?.split_ascii_whitespace();
    if it.next()? != "cpu" {
        return None;
    }
    let v: Vec<u64> = it.map_while(|x| x.parse().ok()).collect();
    if v.len() < 4 {
        return None;
    }
    // user nice system idle iowait irq softirq steal (guest time is already
    // part of user/nice).
    let at = |i: usize| v.get(i).copied().unwrap_or(0);
    let busy = at(0) + at(1) + at(2) + at(5) + at(6) + at(7);
    let idle = at(3) + at(4);
    Some(CpuTimes { busy: ticks(busy), idle: ticks(idle) })
}

/// Splits a raw cmdline (NUL-separated) into `(name, command)`.
///
/// `comm` is truncated to 15 bytes by the kernel, so when it looks truncated
/// and argv[0]'s basename extends it, the longer name wins (this keeps names
/// like "tracker-miner-fs-3" intact). Kernel threads have no cmdline and are
/// shown as `[comm]`, as ps does.
fn names(comm: &str, cmdline: &[u8]) -> (String, String) {
    let raw = cmdline.strip_suffix(&[0]).unwrap_or(cmdline);
    if raw.is_empty() {
        return (comm.to_string(), format!("[{comm}]"));
    }
    let command = String::from_utf8_lossy(raw).replace('\0', " ");
    let argv0 = String::from_utf8_lossy(raw.split(|&b| b == 0).next().unwrap_or_default()).into_owned();
    let base = argv0.rsplit('/').next().unwrap_or("");
    let name = if comm.len() >= 15 && base.starts_with(comm) { base } else { comm };
    (name.to_string(), command)
}

// ---- system constants ----

pub(super) fn clk_tck() -> u64 {
    static HZ: OnceLock<u64> = OnceLock::new();
    *HZ.get_or_init(|| {
        // SAFETY: sysconf has no preconditions.
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if hz > 0 { hz as u64 } else { 100 }
    })
}

fn page_size() -> u64 {
    static SZ: OnceLock<u64> = OnceLock::new();
    *SZ.get_or_init(|| {
        // SAFETY: sysconf has no preconditions.
        let sz = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if sz > 0 { sz as u64 } else { 4096 }
    })
}

fn ticks(n: u64) -> Duration {
    Duration::from_nanos((n as u128 * 1_000_000_000 / clk_tck() as u128) as u64)
}

/// Boot time in Unix milliseconds. Read once and cached: `btime` can be
/// re-derived from the wall clock and drift when the clock is stepped, which
/// would make `start_ms` (half of a process's identity) change between
/// snapshots.
fn boot_ms() -> u64 {
    static BOOT: OnceLock<u64> = OnceLock::new();
    *BOOT.get_or_init(|| {
        let btime = read_lossy("/proc/stat")
            .and_then(|s| s.lines().find_map(|l| l.strip_prefix("btime ")?.trim().parse::<u64>().ok()));
        if let Some(b) = btime {
            return b * 1000;
        }
        let up = read_lossy("/proc/uptime").and_then(|s| s.split_ascii_whitespace().next()?.parse::<f64>().ok());
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        now.as_millis() as u64 - (up.unwrap_or(0.0) * 1000.0) as u64
    })
}

// ---- machine description ----

pub(super) fn read_lossy(path: &str) -> Option<String> {
    Some(String::from_utf8_lossy(&fs::read(path).ok()?).into_owned())
}

fn first_line(path: &str) -> Option<String> {
    let s = read_lossy(path)?;
    let l = s.split(['\n', '\0']).next()?.trim();
    (!l.is_empty()).then(|| l.to_string())
}

fn uname() -> Option<(String, String)> {
    // SAFETY: utsname is plain bytes, and uname fills it on success.
    let u = unsafe {
        let mut u: libc::utsname = std::mem::zeroed();
        if libc::uname(&mut u) != 0 {
            return None;
        }
        u
    };
    // SAFETY: uname NUL-terminates each field.
    let s = |f: &[libc::c_char]| unsafe { CStr::from_ptr(f.as_ptr()) }.to_string_lossy().into_owned();
    Some((s(&u.release), s(&u.machine)))
}

fn os_version() -> String {
    let pretty = read_lossy("/etc/os-release").and_then(|s| {
        s.lines().find_map(|l| Some(l.strip_prefix("PRETTY_NAME=")?.trim().trim_matches(['"', '\'']).to_string()))
    });
    match pretty {
        Some(p) if !p.is_empty() => p,
        _ => uname().map(|(release, _)| release).unwrap_or_default(),
    }
}

fn model() -> String {
    first_line("/sys/class/dmi/id/product_name").or_else(|| first_line("/proc/device-tree/model")).unwrap_or_default()
}

fn cpu_name() -> String {
    let info = read_lossy("/proc/cpuinfo").unwrap_or_default();
    let find = |key: &str| {
        info.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == key && !v.trim().is_empty()).then(|| v.trim().to_string())
        })
    };
    // "model name" on x86; "Model" on Raspberry Pi; "Hardware" on older ARM.
    find("model name")
        .or_else(|| find("Model"))
        .or_else(|| find("Hardware"))
        .or_else(|| uname().map(|(_, machine)| machine))
        .unwrap_or_default()
}

fn mem_total() -> u64 {
    read_lossy("/proc/meminfo")
        .and_then(|s| {
            let kb = s.lines().find_map(|l| l.strip_prefix("MemTotal:"))?;
            kb.split_ascii_whitespace().next()?.parse::<u64>().ok()
        })
        .map_or(0, |kb| kb * 1024)
}

// ---- users ----

fn lookup_user(uid: u32) -> Option<String> {
    let mut buf = vec![0u8; 1024];
    loop {
        // SAFETY: passwd holds only integers and pointers, for which zero is valid.
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut res: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: the buffer pointer and length describe `buf`; `res` is set
        // to &pwd on success and null if there is no entry.
        let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr().cast(), buf.len(), &mut res) };
        if rc == libc::ERANGE && buf.len() < (1 << 20) {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if rc != 0 || res.is_null() {
            return None;
        }
        // SAFETY: pw_name points into `buf` and is NUL-terminated.
        return Some(unsafe { CStr::from_ptr(pwd.pw_name) }.to_string_lossy().into_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 52 fields as the kernel writes them, with `comm` spliced in.
    fn stat_line(comm: &str) -> String {
        format!(
            "4242 ({comm}) S 7 4242 4242 0 -1 4194560 100 200 3 4 \
             150 50 11 9 20 0 1 0 987654 1000000 321 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 3 0 0 0 0 0"
        )
    }

    #[test]
    fn stat_with_nasty_comm() {
        for comm in ["bash", "Web Content", ") (", "a) (b c) d", "(x)", " "] {
            let st = parse_stat(&stat_line(comm)).unwrap_or_else(|| panic!("failed on {comm:?}"));
            assert_eq!(st.comm, comm);
            assert_eq!((st.ppid, st.utime, st.stime, st.cutime, st.cstime), (7, 150, 50, 11, 9));
            assert_eq!((st.starttime, st.rss_pages), (987654, 321));
        }
        assert!(parse_stat("garbage").is_none());
        assert!(parse_stat("1 (short) S 1").is_none());
    }

    #[test]
    fn status_and_io() {
        let st = parse_status(
            "Name:\tx\nUid:\t1000\t0\t0\t0\nvoluntary_ctxt_switches:\t10\nnonvoluntary_ctxt_switches:\t5\n",
        );
        assert_eq!(st, Status { uid: 1000, ctxt_switches: 15 });
        assert_eq!(
            parse_io("rchar: 9\nwchar: 9\nread_bytes: 4096\nwrite_bytes: 8192\ncancelled_write_bytes: 0\n"),
            (4096, 8192)
        );
    }

    #[test]
    fn cpu_line() {
        let c = parse_cpu("cpu  10 20 30 400 50 6 7 8 9 10\ncpu0 1 2 3 4 5 6 7 8 9 10\n").unwrap();
        assert_eq!(c.busy, ticks(10 + 20 + 30 + 6 + 7 + 8));
        assert_eq!(c.idle, ticks(450));
        assert!(parse_cpu("intr 1 2 3").is_none());
    }

    #[test]
    fn command_and_name() {
        assert_eq!(names("bash", b"bash\0-c\0echo hi\0"), ("bash".into(), "bash -c echo hi".into()));
        assert_eq!(names("kworker/0:1", b""), ("kworker/0:1".into(), "[kworker/0:1]".into()));
        // comm is cut at 15 bytes; argv[0] has the full name.
        let (n, _) = names("tracker-miner-f", b"/usr/libexec/tracker-miner-fs-3\0");
        assert_eq!(n, "tracker-miner-fs-3");
        // ...but a short comm is authoritative (e.g. a renamed process).
        assert_eq!(names("worker", b"/usr/bin/python3\0app.py\0").0, "worker");
    }

    #[test]
    fn live_snapshot() {
        let l = Linux::new();
        let me = std::process::id() as i32;
        let find = |ps: Vec<Process>| ps.into_iter().find(|p| p.pid == me).unwrap();

        let a = l.processes().unwrap();
        assert!(!a.is_empty());
        let a = find(a);
        assert!(a.detailed, "own process must have readable io");
        assert!(a.start_ms > 0 && !a.command.is_empty() && !a.name.is_empty());
        assert!(a.rss_bytes > 0);
        assert!(!a.user.is_empty());
        // SAFETY: getuid cannot fail.
        assert_eq!(a.uid, unsafe { libc::getuid() });
        assert_eq!(a.ppid, std::os::unix::process::parent_id() as i32);

        // Burn a little CPU so that deltas are visible, then re-snapshot.
        let t = std::time::Instant::now();
        while t.elapsed() < Duration::from_millis(30) {
            std::hint::black_box(0u64.wrapping_sub(1));
        }
        let b = find(l.processes().unwrap());
        assert_eq!(a.start_ms, b.start_ms, "start_ms must be stable across snapshots");
        assert!(b.cpu_time >= a.cpu_time && b.disk_read >= a.disk_read);
    }

    #[test]
    fn live_cpu_and_machine() {
        let l = Linux::new();
        let c = l.cpu().unwrap();
        assert!(c.busy > Duration::ZERO && c.idle >= Duration::ZERO);
        let m = l.machine();
        assert_eq!(m.os, "linux");
        assert!(m.mem_bytes > 0 && m.cores > 0 && !m.cpu.is_empty() && !m.os_version.is_empty());
    }

    #[test]
    fn child_cpu_is_tracked() {
        let l = Linux::new();
        let me = std::process::id() as i32;
        let before = l.processes().unwrap().into_iter().find(|p| p.pid == me).unwrap().child_cpu_time;
        // A reaped child that burns ~50ms of CPU.
        let st = std::process::Command::new("sh")
            .args(["-c", "i=0; while [ $i -lt 300000 ]; do i=$((i+1)); done"])
            .status()
            .unwrap();
        assert!(st.success());
        let after = l.processes().unwrap().into_iter().find(|p| p.pid == me).unwrap().child_cpu_time;
        assert!(after > before, "child_cpu_time did not grow: {before:?} -> {after:?}");
    }
}
