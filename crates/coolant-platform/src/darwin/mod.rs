//! macOS backend, unprivileged.
//!
//! CPU and memory for every process come from `/bin/ps`, which is setuid root
//! on macOS and so can see system daemons such as mds_stores or syspolicyd.
//! Richer counters (disk I/O, wakeups, reaped-children CPU) come from
//! `proc_pid_rusage`, which only works for processes we own.

mod ffi;
mod sensors;
mod spawn;

use crate::*;
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Darwin {
    users: Mutex<HashMap<u32, String>>,
    /// Command lines never change, so cache them per (pid, start).
    args: Mutex<HashMap<(i32, u64), String>>,
}

impl Darwin {
    pub fn new() -> Self {
        Darwin { users: Mutex::default(), args: Mutex::default() }
    }

    fn user_name(&self, uid: u32) -> String {
        let mut users = self.users.lock().unwrap();
        users.entry(uid).or_insert_with(|| ffi::user_name(uid).unwrap_or_else(|| uid.to_string())).clone()
    }
}

impl Backend for Darwin {
    fn name(&self) -> &'static str {
        "darwin"
    }

    fn machine(&self) -> Machine {
        Machine {
            os: "darwin".into(),
            os_version: ffi::sysctl_string("kern.osproductversion").unwrap_or_default(),
            model: ffi::sysctl_string("hw.model").unwrap_or_default(),
            cpu: ffi::sysctl_string("machdep.cpu.brand_string").unwrap_or_default(),
            cores: ffi::sysctl_u64("hw.ncpu").unwrap_or(0) as u32,
            mem_bytes: ffi::sysctl_u64("hw.memsize").unwrap_or(0),
        }
    }

    fn cpu(&self) -> Result<CpuTimes> {
        ffi::cpu_times().ok_or_else(|| Error::Other("host_processor_info failed".into()))
    }

    fn sensors(&self) -> Result<Box<dyn SensorReader>> {
        sensors::open()
    }

    fn spawn_counter(&self) -> Result<Box<dyn SpawnCounter>> {
        spawn::open()
    }

    fn processes(&self) -> Result<Vec<Process>> {
        let out = Command::new("/bin/ps")
            .args(["-A", "-o", "pid=,ppid=,uid=,time=,rss=,comm="])
            .output()
            .map_err(|e| Error::Other(format!("ps: {e}")))?;
        let text = String::from_utf8_lossy(&out.stdout);
        let mut procs = Vec::with_capacity(1024);
        let mut seen = HashSet::new();
        let mut args = self.args.lock().unwrap();
        for line in text.lines() {
            let Some(mut p) = parse_ps(line) else { continue };
            p.user = self.user_name(p.uid);
            if add_rusage(&mut p) {
                let key = (p.pid, p.start_ms);
                seen.insert(key);
                let cmd = args.entry(key).or_insert_with(|| ffi::proc_args(p.pid).unwrap_or_default());
                if !cmd.is_empty() {
                    p.command = cmd.clone();
                }
            }
            procs.push(p);
        }
        args.retain(|k, _| seen.contains(k));
        Ok(procs)
    }
}

/// Parses one line of `pid ppid uid time rss comm`. `comm` comes last
/// because executable paths may contain spaces.
fn parse_ps(line: &str) -> Option<Process> {
    let mut rest = line.trim_start();
    let mut field = || {
        let end = rest.find(' ')?;
        let f = &rest[..end];
        rest = rest[end..].trim_start();
        Some(f)
    };
    let pid = field()?.parse().ok()?;
    let ppid = field()?.parse().ok()?;
    let uid = field()?.parse().ok()?;
    let cpu_time = parse_cpu_time(field()?)?;
    let rss: u64 = field()?.parse().ok()?;
    let comm = rest.trim_end();
    // ps shows exiting processes as "(name)".
    let comm = comm.strip_prefix('(').and_then(|c| c.strip_suffix(')')).unwrap_or(comm);
    if comm.is_empty() {
        return None;
    }
    Some(Process {
        pid,
        ppid,
        uid,
        name: comm.rsplit('/').next().unwrap_or(comm).to_string(),
        command: comm.to_string(),
        cpu_time,
        rss_bytes: rss * 1024,
        ..Default::default()
    })
}

/// Parses ps TIME values such as "0:00.09", "232:29.70" or "1-02:03:04.50".
fn parse_cpu_time(s: &str) -> Option<Duration> {
    let (days, hms) = match s.split_once('-') {
        Some((d, rest)) => (d.parse::<f64>().ok()?, rest),
        None => (0.0, s),
    };
    let mut secs = 0.0;
    for part in hms.split(':') {
        secs = secs * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(Duration::from_secs_f64(days * 86400.0 + secs))
}

/// Fills the detailed fields from proc_pid_rusage. Returns false for
/// processes we may not inspect.
fn add_rusage(p: &mut Process) -> bool {
    let Some(ri) = ffi::rusage(p.pid) else { return false };
    p.detailed = true;
    p.cpu_time = ffi::mach_dur(ri.ri_user_time + ri.ri_system_time); // finer than ps
    p.child_cpu_time = ffi::mach_dur(ri.ri_child_user_time + ri.ri_child_system_time);
    p.disk_read = ri.ri_diskio_bytesread;
    p.disk_written = ri.ri_diskio_byteswritten;
    p.wakeups = ri.ri_interrupt_wkups + ri.ri_pkg_idle_wkups;
    if ri.ri_phys_footprint > 0 {
        p.rss_bytes = ri.ri_phys_footprint;
    }
    // Anchor to a fixed reference so the same process always yields the
    // same start time: it is half of the key that tells processes apart.
    static BOOT_MS: OnceLock<u64> = OnceLock::new();
    let boot_ms = *BOOT_MS.get_or_init(|| {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        now.saturating_sub(ffi::mach_dur(ffi::mach_now())).as_millis() as u64
    });
    p.start_ms = boot_ms + ffi::mach_dur(ri.ri_proc_start_abstime).as_millis() as u64;
    true
}

pub(crate) fn cstr(s: &str) -> CString {
    CString::new(s).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_time_formats() {
        let s = |x: &str| parse_cpu_time(x).unwrap().as_secs_f64();
        assert!((s("0:00.09") - 0.09).abs() < 1e-9);
        assert!((s("232:29.70") - (232.0 * 60.0 + 29.70)).abs() < 1e-6);
        assert!((s("1-02:03:04.50") - (86400.0 + 7384.5)).abs() < 1e-6);
    }

    #[test]
    fn ps_line_with_spaces_and_parens() {
        let p = parse_ps("  123   1  501   0:01.50  2048 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
            .unwrap();
        assert_eq!((p.pid, p.ppid, p.uid, p.rss_bytes), (123, 1, 501, 2048 * 1024));
        assert_eq!(p.name, "Google Chrome");
        let p = parse_ps("99 1 0 0:00.00 0 (mdworker_shared)").unwrap();
        assert_eq!(p.name, "mdworker_shared");
    }

    #[test]
    fn live_snapshot() {
        let d = Darwin::new();
        let procs = d.processes().unwrap();
        assert!(procs.len() > 50);
        let me = procs.iter().find(|p| p.pid == std::process::id() as i32).unwrap();
        assert!(me.detailed && me.start_ms > 0 && !me.command.is_empty());
        assert!(d.cpu().unwrap().busy > Duration::ZERO);
        assert!(d.machine().mem_bytes > 0);
    }
}
