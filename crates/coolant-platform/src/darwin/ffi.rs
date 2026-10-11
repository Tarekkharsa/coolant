//! Thin safe wrappers over the libc/mach calls the process sampler needs.

// libc marks its mach bindings deprecated in favour of the mach2 crate; they
// are still correct and save a dependency.
#![allow(deprecated)]

use super::cstr;
use crate::CpuTimes;
use std::ffi::CStr;
use std::mem::{MaybeUninit, size_of};
use std::sync::OnceLock;
use std::time::Duration;

pub fn sysctl_string(name: &str) -> Option<String> {
    let name = cstr(name);
    let mut buf = [0u8; 256];
    let mut size = buf.len();
    let r = unsafe { libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) };
    if r != 0 {
        return None;
    }
    let s = CStr::from_bytes_until_nul(&buf[..size]).ok()?;
    Some(s.to_string_lossy().into_owned())
}

/// Reads an integer sysctl of either 4 or 8 bytes.
pub fn sysctl_u64(name: &str) -> Option<u64> {
    let name = cstr(name);
    let mut v = 0u64;
    let mut size = size_of::<u64>();
    let r = unsafe { libc::sysctlbyname(name.as_ptr(), (&raw mut v).cast(), &mut size, std::ptr::null_mut(), 0) };
    match (r, size) {
        (0, 4) => Some(v & 0xffff_ffff), // little-endian: the low half was filled
        (0, 8) => Some(v),
        _ => None,
    }
}

/// Bytes of swap in use.
pub fn swap_used() -> Option<u64> {
    let name = cstr("vm.swapusage");
    let mut v = MaybeUninit::<libc::xsw_usage>::zeroed();
    let mut size = size_of::<libc::xsw_usage>();
    let r = unsafe { libc::sysctlbyname(name.as_ptr(), v.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) };
    // SAFETY: zero-initialised and filled by the kernel on success.
    (r == 0).then(|| unsafe { v.assume_init() }.xsu_used)
}

pub fn user_name(uid: u32) -> Option<String> {
    let mut pw = MaybeUninit::<libc::passwd>::uninit();
    let mut buf = vec![0 as libc::c_char; 1024];
    let mut out = std::ptr::null_mut();
    let r = unsafe { libc::getpwuid_r(uid, pw.as_mut_ptr(), buf.as_mut_ptr(), buf.len(), &mut out) };
    if r != 0 || out.is_null() {
        return None;
    }
    Some(unsafe { CStr::from_ptr((*out).pw_name) }.to_string_lossy().into_owned())
}

/// Ticks reported by host_processor_info are 1/100 s.
const CLK_TCK: u64 = 100;

/// Sums per-core tick counters into busy and idle totals.
pub fn cpu_times() -> Option<CpuTimes> {
    let mut n: libc::natural_t = 0;
    let mut info: libc::processor_info_array_t = std::ptr::null_mut();
    let mut count: libc::mach_msg_type_number_t = 0;
    let (host, task) = unsafe { (libc::mach_host_self(), libc::mach_task_self()) };
    let r = unsafe { libc::host_processor_info(host, libc::PROCESSOR_CPU_LOAD_INFO, &mut n, &mut info, &mut count) };
    if r != libc::KERN_SUCCESS {
        return None;
    }
    let ticks = unsafe { std::slice::from_raw_parts(info as *const u32, count as usize) };
    let (mut busy, mut idle) = (0u64, 0u64);
    for cpu in ticks.as_chunks::<{ libc::CPU_STATE_MAX as usize }>().0 {
        busy += (cpu[libc::CPU_STATE_USER as usize]
            + cpu[libc::CPU_STATE_SYSTEM as usize]
            + cpu[libc::CPU_STATE_NICE as usize]) as u64;
        idle += cpu[libc::CPU_STATE_IDLE as usize] as u64;
    }
    unsafe {
        libc::vm_deallocate(task, info as libc::vm_address_t, count as usize * size_of::<i32>());
    }
    let tick = |t: u64| Duration::from_millis(t * 1000 / CLK_TCK);
    Some(CpuTimes { busy: tick(busy), idle: tick(idle) })
}

pub fn rusage(pid: i32) -> Option<libc::rusage_info_v4> {
    let mut ri = MaybeUninit::<libc::rusage_info_v4>::zeroed();
    let r = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, ri.as_mut_ptr().cast()) };
    (r == 0).then(|| unsafe { ri.assume_init() })
}

pub fn mach_now() -> u64 {
    unsafe { libc::mach_absolute_time() }
}

/// Converts mach absolute time units (used by rusage on Apple Silicon) to a
/// Duration.
pub fn mach_dur(t: u64) -> Duration {
    static TIMEBASE: OnceLock<(u64, u64)> = OnceLock::new();
    let &(numer, denom) = TIMEBASE.get_or_init(|| {
        let mut tb = libc::mach_timebase_info { numer: 0, denom: 0 };
        unsafe { libc::mach_timebase_info(&mut tb) };
        (tb.numer as u64, tb.denom.max(1) as u64)
    });
    Duration::from_nanos((t as u128 * numer as u128 / denom as u128) as u64)
}

/// Reads a process's argv via KERN_PROCARGS2, which works for processes
/// owned by the current user.
pub fn proc_args(pid: i32) -> Option<String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut buf = vec![0u8; 256 * 1024];
    let mut size = buf.len();
    let r = unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) };
    if r != 0 || size < 4 {
        return None;
    }
    let buf = &buf[..size];
    let argc = i32::from_ne_bytes(buf[..4].try_into().ok()?) as usize;
    // Layout: argc, exec path, NUL padding, then argv strings.
    let rest = &buf[4..];
    let after_path = rest.iter().position(|&b| b == 0)?;
    let start = after_path + rest[after_path..].iter().position(|&b| b != 0)?;
    let args: Vec<_> = rest[start..].split(|&b| b == 0).take(argc).map(String::from_utf8_lossy).collect();
    Some(args.join(" "))
}
