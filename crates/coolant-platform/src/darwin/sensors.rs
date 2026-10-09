//! Sensors without root. Every source is optional:
//!
//!   - Thermal pressure: the public notify(3) key behind OSThermalNotification.
//!   - Temperatures: IOHIDEventSystemClient (private) for CPU die sensors, and
//!     the AppleSMC user client for anything HID lacks (on M4 the GPU sensors
//!     exist only as SMC Tg* keys).
//!   - Power: IOReport "Energy Model" (private libIOReport, as used by macmon).
//!     On some systems (including the M4 / macOS 27 this was written on) the
//!     PMGR-backed CPU channels are frozen for unprivileged readers while the
//!     AGX "GPU Energy" channel stays live, so CPU energy falls back to summing
//!     the kernel's per-coalition energy accounting, and package power to the
//!     SMC PMU main-rail reading.
//!   - Fans: SMC F<n>Ac keys.

use super::cstr;
use crate::*;
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::size_of;
use std::ptr::null;
use std::time::Instant;

// ---- CoreFoundation ----

type CFTypeRef = *const c_void;
type CFIndex = isize;

const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8
const CF_NUMBER_INT_TYPE: CFIndex = 9; // kCFNumberIntType

/// Stand-in for the callback structs we only ever pass by address.
#[repr(C)]
struct Opaque([u8; 0]);

#[link(name = "CoreFoundation", kind = "framework")]
#[allow(non_upper_case_globals)]
unsafe extern "C" {
    static kCFTypeDictionaryKeyCallBacks: Opaque;
    static kCFTypeDictionaryValueCallBacks: Opaque;
    static kCFTypeArrayCallBacks: Opaque;

    fn CFRelease(cf: CFTypeRef);
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const c_char, enc: u32) -> CFTypeRef;
    fn CFStringGetLength(s: CFTypeRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(len: CFIndex, enc: u32) -> CFIndex;
    fn CFStringGetCString(s: CFTypeRef, buf: *mut c_char, size: CFIndex, enc: u32) -> u8;
    fn CFNumberCreate(alloc: CFTypeRef, ty: CFIndex, value: *const c_void) -> CFTypeRef;
    fn CFDictionaryCreate(
        alloc: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        n: CFIndex,
        key_cb: *const Opaque,
        value_cb: *const Opaque,
    ) -> CFTypeRef;
    fn CFDictionaryCreateMutableCopy(alloc: CFTypeRef, cap: CFIndex, d: CFTypeRef) -> CFTypeRef;
    fn CFDictionaryGetValue(d: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFDictionarySetValue(d: CFTypeRef, key: CFTypeRef, value: CFTypeRef);
    fn CFArrayCreateMutable(alloc: CFTypeRef, cap: CFIndex, cb: *const Opaque) -> CFTypeRef;
    fn CFArrayAppendValue(a: CFTypeRef, v: CFTypeRef);
    fn CFArrayGetCount(a: CFTypeRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(a: CFTypeRef, i: CFIndex) -> CFTypeRef;
}

/// An owned CF object (one we created or copied), released on drop. May be null.
struct Cf(CFTypeRef);

impl Cf {
    fn new(p: CFTypeRef) -> Option<Cf> {
        (!p.is_null()).then_some(Cf(p))
    }

    fn string(s: &str) -> Option<Cf> {
        let s = cstr(s);
        // SAFETY: `s` is NUL-terminated.
        Cf::new(unsafe { CFStringCreateWithCString(null(), s.as_ptr(), UTF8) })
    }

    fn int(v: i32) -> Option<Cf> {
        // SAFETY: kCFNumberIntType reads a C int from the pointer.
        Cf::new(unsafe { CFNumberCreate(null(), CF_NUMBER_INT_TYPE, (&raw const v).cast()) })
    }
}

impl Drop for Cf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: we own one reference to a non-null CF object.
            unsafe { CFRelease(self.0) };
        }
    }
}

/// Converts a CFString to a Rust string; None for null or non-string objects.
///
/// # Safety
/// `s` must be null or a valid CF object.
unsafe fn cf_string(s: CFTypeRef) -> Option<String> {
    // SAFETY: per the contract above; the buffer is sized by CF for the encoding.
    unsafe {
        if s.is_null() || CFGetTypeID(s) != CFStringGetTypeID() {
            return None;
        }
        let max = CFStringGetMaximumSizeForEncoding(CFStringGetLength(s), UTF8) + 1;
        let mut buf = vec![0u8; max as usize];
        if CFStringGetCString(s, buf.as_mut_ptr().cast(), max, UTF8) == 0 {
            return None;
        }
        let s = CStr::from_bytes_until_nul(&buf).ok()?;
        Some(s.to_string_lossy().into_owned())
    }
}

/// Borrowed elements of a CFArray.
///
/// # Safety
/// `a` must be a valid CFArray.
unsafe fn array_items(a: CFTypeRef) -> Vec<CFTypeRef> {
    // SAFETY: indices are within the count.
    unsafe { (0..CFArrayGetCount(a)).map(|i| CFArrayGetValueAtIndex(a, i)).collect() }
}

// ---- IOHIDEventSystemClient (private) ----

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDEventSystemClientCreate(alloc: CFTypeRef) -> CFTypeRef;
    fn IOHIDEventSystemClientSetMatching(client: CFTypeRef, matching: CFTypeRef) -> c_int;
    fn IOHIDEventSystemClientCopyServices(client: CFTypeRef) -> CFTypeRef;
    fn IOHIDServiceClientCopyEvent(service: CFTypeRef, ty: i64, a: i32, b: i64) -> CFTypeRef;
    fn IOHIDServiceClientCopyProperty(service: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn IOHIDEventGetFloatValue(event: CFTypeRef, field: i32) -> f64;

    fn IOServiceMatching(name: *const c_char) -> CFTypeRef;
    fn IOServiceGetMatchingService(main_port: u32, matching: CFTypeRef) -> u32;
    fn IOServiceOpen(service: u32, task: u32, ty: u32, conn: *mut u32) -> i32;
    fn IOServiceClose(conn: u32) -> i32;
    fn IOObjectRelease(obj: u32) -> i32;
    fn IOConnectCallStructMethod(
        conn: u32,
        selector: u32,
        input: *const c_void,
        input_size: usize,
        output: *mut c_void,
        output_size: *mut usize,
    ) -> i32;
}

const HID_EVENT_TEMPERATURE: i64 = 15; // kIOHIDEventTypeTemperature
const HID_FIELD_TEMPERATURE: i32 = (HID_EVENT_TEMPERATURE as i32) << 16; // IOHIDEventFieldBase(type)

struct Hid {
    // Declared services-first so they are released before the client.
    services: Cf,
    /// Kept alive for `services`.
    _client: Cf,
}

impl Hid {
    /// Matches AppleVendor (0xff00) / TemperatureSensor (5) services.
    fn open() -> Option<Hid> {
        // SAFETY: plain creation calls; every returned object is wrapped in Cf.
        unsafe {
            let client = Cf::new(IOHIDEventSystemClientCreate(null()))?;
            let (page, usage) = (Cf::int(0xff00)?, Cf::int(5)?);
            let (k_page, k_usage) = (Cf::string("PrimaryUsagePage")?, Cf::string("PrimaryUsage")?);
            let keys = [k_page.0, k_usage.0];
            let values = [page.0, usage.0];
            let m = Cf::new(CFDictionaryCreate(
                null(),
                keys.as_ptr(),
                values.as_ptr(),
                2,
                &raw const kCFTypeDictionaryKeyCallBacks,
                &raw const kCFTypeDictionaryValueCallBacks,
            ))?;
            IOHIDEventSystemClientSetMatching(client.0, m.0);
            let services = Cf::new(IOHIDEventSystemClientCopyServices(client.0))?;
            Some(Hid { services, _client: client })
        }
    }

    fn len(&self) -> CFIndex {
        // SAFETY: `services` is a valid CFArray.
        unsafe { CFArrayGetCount(self.services.0) }
    }

    fn service(&self, i: CFIndex) -> CFTypeRef {
        // SAFETY: callers pass i < len().
        unsafe { CFArrayGetValueAtIndex(self.services.0, i) }
    }

    fn name(&self, i: CFIndex) -> Option<String> {
        let key = Cf::string("Product")?;
        // SAFETY: the service is valid while `self` is; the copied property is owned by `name`.
        unsafe {
            let name = Cf::new(IOHIDServiceClientCopyProperty(self.service(i), key.0))?;
            cf_string(name.0)
        }
    }

    fn temp(&self, i: CFIndex) -> f64 {
        // SAFETY: the copied event is owned by `e`.
        unsafe {
            match Cf::new(IOHIDServiceClientCopyEvent(self.service(i), HID_EVENT_TEMPERATURE, 0, 0)) {
                Some(e) => IOHIDEventGetFloatValue(e.0, HID_FIELD_TEMPERATURE),
                None => -1000.0,
            }
        }
    }
}

// ---- AppleSMC user client ----

/// SMCParamStruct as used by AppleSMC's selector 2 (kSMCHandleYPCEvent).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SmcParam {
    key: u32,
    vers: [u8; 6],
    pad0: [u8; 2],
    plimit: [u8; 16],
    data_size: u32,
    data_type: u32,
    data_attributes: u8,
    pad1: [u8; 3],
    result: u8,
    status: u8,
    cmd: u8,
    pad2: u8,
    data32: u32,
    bytes: [u8; 32],
}

const _: () = assert!(size_of::<SmcParam>() == 80);

const SMC_CMD_READ: u8 = 5;
const SMC_CMD_KEY_AT: u8 = 8;
const SMC_CMD_INFO: u8 = 9;

#[derive(Clone, Copy)]
struct SmcKey {
    key: u32,
    size: u32,
    typ: [u8; 4],
}

struct Smc(u32);

impl Smc {
    fn open() -> Option<Smc> {
        // SAFETY: IOServiceGetMatchingService consumes the matching dictionary;
        // the service object is released once opened.
        unsafe {
            let m = IOServiceMatching(cstr("AppleSMC").as_ptr());
            if m.is_null() {
                return None;
            }
            let svc = IOServiceGetMatchingService(0, m); // kIOMainPortDefault
            if svc == 0 {
                return None;
            }
            let mut conn = 0;
            #[allow(deprecated)]
            let kr = IOServiceOpen(svc, libc::mach_task_self(), 0, &mut conn);
            IOObjectRelease(svc);
            (kr == 0 && conn != 0).then_some(Smc(conn))
        }
    }

    fn call(&self, input: &SmcParam) -> Option<SmcParam> {
        let mut out = SmcParam::default();
        let mut n = size_of::<SmcParam>();
        // SAFETY: both buffers are SmcParam-sized.
        let kr = unsafe {
            IOConnectCallStructMethod(
                self.0,
                2,
                (input as *const SmcParam).cast(),
                size_of::<SmcParam>(),
                (&raw mut out).cast(),
                &mut n,
            )
        };
        (kr == 0 && out.result == 0).then_some(out)
    }

    fn key_at(&self, i: u32) -> Option<u32> {
        let input = SmcParam { cmd: SMC_CMD_KEY_AT, data32: i, ..Default::default() };
        self.call(&input).map(|o| o.key)
    }

    fn info(&self, name: [u8; 4]) -> Option<SmcKey> {
        let key = u32::from_be_bytes(name);
        let input = SmcParam { key, cmd: SMC_CMD_INFO, ..Default::default() };
        let out = self.call(&input)?;
        if out.data_size == 0 || out.data_size > 32 {
            return None;
        }
        Some(SmcKey { key, size: out.data_size, typ: out.data_type.to_be_bytes() })
    }

    fn raw(&self, k: &SmcKey) -> Option<([u8; 32], usize)> {
        if k.size > 32 {
            return None;
        }
        let input = SmcParam { key: k.key, cmd: SMC_CMD_READ, data_size: k.size, ..Default::default() };
        Some((self.call(&input)?.bytes, k.size as usize))
    }

    /// Decodes the common numeric SMC types.
    fn value(&self, k: &SmcKey) -> Option<f64> {
        let (buf, n) = self.raw(k)?;
        let b = &buf[..n];
        match (&k.typ, n) {
            // native (little-endian) float on Apple Silicon
            (b"flt ", 4) => Some(f32::from_le_bytes(b.try_into().ok()?) as f64),
            // unsigned 14.2 fixed point (Intel fans)
            (b"fpe2", 2) => Some(u16::from_be_bytes(b.try_into().ok()?) as f64 / 4.0),
            // signed 7.8 fixed point (Intel temps)
            (b"sp78", 2) => Some(i16::from_be_bytes(b.try_into().ok()?) as f64 / 256.0),
            (b"ui8 ", 1) => Some(b[0] as f64),
            (b"ui16", 2) => Some(u16::from_be_bytes(b.try_into().ok()?) as f64),
            (b"ui32", 4) => Some(u32::from_be_bytes(b.try_into().ok()?) as f64),
            _ => None,
        }
    }

    /// Returns the plausible readings among keys. Sensors of power-gated
    /// blocks (e.g. an idle GPU) report placeholders such as -4.5 or 0.8.
    fn temps(&self, keys: &[SmcKey]) -> Vec<f64> {
        keys.iter().filter_map(|k| self.value(k)).filter(|&v| v > 10.0 && v < 130.0).collect()
    }
}

impl Drop for Smc {
    fn drop(&mut self) {
        // SAFETY: we own the connection.
        unsafe { IOServiceClose(self.0) };
    }
}

// ---- IOReport (private, libIOReport.dylib) ----

#[link(name = "IOReport")]
unsafe extern "C" {
    fn IOReportCopyChannelsInGroup(group: CFTypeRef, subgroup: CFTypeRef, a: u64, b: u64, c: u64) -> CFTypeRef;
    fn IOReportCreateSubscription(
        alloc: *const c_void,
        desired: CFTypeRef,
        subbed: *mut CFTypeRef,
        a: u64,
        b: CFTypeRef,
    ) -> CFTypeRef;
    fn IOReportCreateSamples(sub: CFTypeRef, subbed: CFTypeRef, a: CFTypeRef) -> CFTypeRef;
    fn IOReportCreateSamplesDelta(prev: CFTypeRef, cur: CFTypeRef, a: CFTypeRef) -> CFTypeRef;
    fn IOReportSimpleGetIntegerValue(ch: CFTypeRef, a: i32) -> i64;
    fn IOReportChannelGetChannelName(ch: CFTypeRef) -> CFTypeRef;
    fn IOReportChannelGetUnitLabel(ch: CFTypeRef) -> CFTypeRef;
}

/// Classifies an Energy Model channel: 0 CPU, 1 GPU, 2 ANE, 3 DRAM.
fn ior_kind(ch: CFTypeRef) -> Option<usize> {
    // SAFETY: `ch` is a channel dictionary; the name is borrowed from it.
    let name = unsafe { cf_string(IOReportChannelGetChannelName(ch)) }?;
    // "CPU Energy"/"GPU Energy" may carry a die prefix on multi-die chips.
    if name.ends_with("CPU Energy") {
        Some(0)
    } else if name.ends_with("GPU Energy") {
        Some(1)
    } else if name.starts_with("ANE") {
        Some(2)
    } else if name.starts_with("DRAM") {
        Some(3)
    } else {
        None
    }
}

fn joules(unit: Option<String>) -> f64 {
    match unit.as_deref() {
        Some("mJ") => 1e-3,
        Some("uJ") => 1e-6,
        Some("nJ") => 1e-9,
        _ => 0.0,
    }
}

struct Ior {
    // Released in this order, as the Go code did.
    prev: Cf,
    subbed: Cf,
    /// Kept alive for the subscription.
    _chans: Cf,
    sub: Cf,
    /// The "IOReportChannels" dictionary key.
    key: Cf,
}

impl Ior {
    fn open() -> Option<Ior> {
        let key = Cf::string("IOReportChannels")?;
        let group = Cf::string("Energy Model")?;
        // SAFETY: every returned object is wrapped in Cf; `all` is borrowed from `chans`
        // and only used before it is replaced.
        unsafe {
            let ch = Cf::new(IOReportCopyChannelsInGroup(group.0, null(), 0, 0, 0))?;
            let chans = Cf::new(CFDictionaryCreateMutableCopy(null(), 0, ch.0))?;
            drop(ch);
            // Subscribe only to the channels we sum; sampling all ~175 costs ~1.5 ms.
            let all = CFDictionaryGetValue(chans.0, key.0);
            if all.is_null() {
                return None;
            }
            let keep = Cf::new(CFArrayCreateMutable(null(), 0, &raw const kCFTypeArrayCallBacks))?;
            for c in array_items(all) {
                if ior_kind(c).is_some() {
                    CFArrayAppendValue(keep.0, c);
                }
            }
            CFDictionarySetValue(chans.0, key.0, keep.0);
            if CFArrayGetCount(keep.0) == 0 {
                return None;
            }
            drop(keep);
            let mut subbed: CFTypeRef = null();
            let sub = IOReportCreateSubscription(null(), chans.0, &mut subbed, 0, null());
            let subbed = Cf(subbed); // owned even if `sub` turns out null
            let sub = Cf::new(sub)?;
            if subbed.0.is_null() {
                return None;
            }
            let prev = Cf::new(IOReportCreateSamples(sub.0, subbed.0, null()))?;
            Some(Ior { prev, subbed, _chans: chans, sub, key })
        }
    }

    /// Takes a new sample and returns joules consumed since the previous
    /// one: [0] CPU, [1] GPU, [2] ANE, [3] DRAM. All zero on failure.
    fn sample(&mut self) -> [f64; 4] {
        let mut out = [0.0; 4];
        // SAFETY: all objects are valid and owned; channel dictionaries are
        // borrowed from `d`, which outlives the loop.
        unsafe {
            let Some(cur) = Cf::new(IOReportCreateSamples(self.sub.0, self.subbed.0, null())) else { return out };
            let d = Cf::new(IOReportCreateSamplesDelta(self.prev.0, cur.0, null()));
            self.prev = cur;
            let Some(d) = d else { return out };
            let chans = CFDictionaryGetValue(d.0, self.key.0);
            if chans.is_null() {
                return out;
            }
            for c in array_items(chans) {
                if let Some(k) = ior_kind(c) {
                    out[k] +=
                        IOReportSimpleGetIntegerValue(c, 0) as f64 * joules(cf_string(IOReportChannelGetUnitLabel(c)));
                }
            }
        }
        out
    }
}

// ---- Coalition energy ----
// The kernel accounts CPU energy (nJ, from the CLPC power model) per resource
// coalition; reading it needs no privileges. Not in the public SDK headers.

unsafe extern "C" {
    fn proc_listcoalitions(flavor: c_int, coaltype: c_int, buf: *mut c_void, bufsize: c_int) -> c_int;
    fn coalition_info_resource_usage(cid: u64, cru: *mut c_void, sz: usize) -> c_int;
    fn notify_register_check(name: *const c_char, token: *mut c_int) -> u32;
    fn notify_get_state(token: c_int, state: *mut u64) -> u32;
    fn notify_cancel(token: c_int) -> u32;
}

/// struct procinfo_coalinfo
#[repr(C)]
#[derive(Clone, Copy)]
struct CoalInfo {
    id: u64,
    kind: u32,
    tasks: u32,
}

const COALITION_TYPE_RESOURCE: u32 = 0;
const NOTIFY_STATUS_OK: u32 = 0;

/// Returns (id, cumulative energy in nJ) for every resource coalition.
fn coal_sample() -> Vec<(u64, u64)> {
    const SZ: usize = size_of::<CoalInfo>();
    let mut cap = 4096;
    let (buf, n) = loop {
        let mut buf = vec![CoalInfo { id: 0, kind: 0, tasks: 0 }; cap];
        let bytes = (cap * SZ) as c_int;
        // LISTCOALITIONS_ALL_COALS = 1; the kernel writes at most `bytes` bytes.
        let n = unsafe { proc_listcoalitions(1, 0, buf.as_mut_ptr().cast(), bytes) };
        if n < bytes || cap >= 1 << 20 {
            break (buf, n);
        }
        cap *= 2;
    };
    let n = if n > 0 { n as usize / SZ } else { 0 };
    let mut out = Vec::new();
    let mut cru = [0u64; 128]; // struct coalition_resource_usage, with headroom
    for c in &buf[..n] {
        if c.kind != COALITION_TYPE_RESOURCE {
            continue;
        }
        // SAFETY: the size passed is that of `cru`.
        if unsafe { coalition_info_resource_usage(c.id, cru.as_mut_ptr().cast(), size_of_val(&cru)) } != 0 {
            continue;
        }
        out.push((c.id, cru[11])); // .energy
    }
    out
}

// ---- Reader ----

struct Reader {
    thermal: Option<c_int>,

    hid: Option<Hid>,
    hid_cpu: Vec<CFIndex>,
    hid_gpu: Vec<CFIndex>,
    smc: Option<Smc>,
    smc_cpu: Vec<SmcKey>,
    smc_gpu: Vec<SmcKey>,
    smc_fans: Vec<SmcKey>,
    smc_soc: Option<SmcKey>,
    ior: Option<Ior>,
    coal_prev: HashMap<u64, u64>,
    coal_max: u64,
    has_coal: bool,
    last: Instant,
}

// SAFETY: the CF/IOKit objects are owned exclusively by the Reader and only
// touched through `&mut self`, so moving it to another thread is fine.
unsafe impl Send for Reader {}

pub fn open() -> Result<Box<dyn SensorReader>> {
    let mut r = Reader {
        thermal: None,
        hid: None,
        hid_cpu: Vec::new(),
        hid_gpu: Vec::new(),
        smc: None,
        smc_cpu: Vec::new(),
        smc_gpu: Vec::new(),
        smc_fans: Vec::new(),
        smc_soc: None,
        ior: None,
        coal_prev: HashMap::new(),
        coal_max: 0,
        has_coal: false,
        last: Instant::now(),
    };
    let name = cstr("com.apple.system.thermalpressurelevel"); // kOSThermalNotificationPressureLevelName
    let mut tok = 0;
    // SAFETY: `name` is NUL-terminated.
    if unsafe { notify_register_check(name.as_ptr(), &mut tok) } == NOTIFY_STATUS_OK {
        r.thermal = Some(tok);
    }
    r.open_hid();
    r.open_smc();
    r.ior = Ior::open();
    r.has_coal = r.sample_coalitions().is_some();
    r.last = Instant::now();

    if r.thermal.is_none()
        && r.hid_cpu.len() + r.hid_gpu.len() == 0
        && r.smc.is_none()
        && r.ior.is_none()
        && !r.has_coal
    {
        return Err(Error::Unsupported);
    }
    Ok(Box::new(r))
}

impl Reader {
    fn open_hid(&mut self) {
        let Some(hid) = Hid::open() else { return };
        let (mut mtr, mut tdie, mut gpu) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..hid.len() {
            let Some(name) = hid.name(i) else { continue };
            // M1–M3: per-cluster "pACC/eACC MTR Temp Sensor<n>" and "GPU MTR Temp Sensor<n>".
            if name.contains("ACC MTR Temp") {
                mtr.push(i);
            } else if name.starts_with("GPU MTR Temp") {
                gpu.push(i);
            // M4 exposes only PMGR die sensors: "PMU tdie<n>" and "PMU2 tdie<n>".
            } else if name.starts_with("PMU") && name.contains(" tdie") {
                tdie.push(i);
            }
        }
        self.hid_cpu = if mtr.is_empty() { tdie } else { mtr };
        self.hid_gpu = gpu;
        if self.hid_cpu.len() + self.hid_gpu.len() != 0 {
            self.hid = Some(hid);
        }
    }

    fn open_smc(&mut self) {
        let Some(smc) = Smc::open() else { return };
        let mut n = 0u64;
        if let Some(k) = smc.info(*b"#KEY")
            && let Some((b, len)) = smc.raw(&k)
            && len >= 4
        {
            n = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64;
        }
        for i in 0..n {
            let Some(key) = smc.key_at(i as u32) else { continue };
            let name = key.to_be_bytes();
            if name[0] != b'T' || !matches!(name[1], b'p' | b'e' | b'g') {
                continue;
            }
            let Some(k) = smc.info(name) else { continue };
            if &k.typ != b"flt " {
                continue;
            }
            if name[1] == b'g' {
                self.smc_gpu.push(k);
            } else {
                self.smc_cpu.push(k);
            }
        }
        if let Some(k) = smc.info(*b"FNum")
            && let Some(v) = smc.value(&k)
        {
            for i in 0..(v as usize).min(10) {
                if let Some(f) = smc.info([b'F', b'0' + i as u8, b'A', b'c']) {
                    self.smc_fans.push(f);
                }
            }
        }
        // PPMR tracks the sum of the SoC's PMU rails (CPU, GPU, SRAM, fabric…)
        // on M4: ~1 W idle, ~10 W with 4 P-cores busy, ~21 W with the GPU busy.
        if let Some(k) = smc.info(*b"PPMR")
            && &k.typ == b"flt "
        {
            self.smc_soc = Some(k);
        }
        self.smc = Some(smc);
    }

    fn hid_temps(&self, idx: &[CFIndex]) -> Vec<f64> {
        let Some(hid) = &self.hid else { return Vec::new() };
        idx.iter().map(|&i| hid.temp(i)).filter(|&v| v > 0.0 && v < 150.0).collect()
    }

    /// Returns CPU joules used since the previous call, or None if coalition
    /// accounting is unavailable.
    fn sample_coalitions(&mut self) -> Option<f64> {
        let list = coal_sample();
        if list.is_empty() {
            return None;
        }
        let first = self.coal_prev.is_empty();
        let mut cur = HashMap::with_capacity(list.len());
        let mut nj = 0u64;
        let mut max_id = self.coal_max;
        for (id, e) in list {
            cur.insert(id, e);
            max_id = max_id.max(id);
            if let Some(&prev) = self.coal_prev.get(&id) {
                if e >= prev {
                    nj += e - prev;
                }
            } else if !first && id > self.coal_max {
                nj += e; // coalition created since the last sample (IDs are monotonic)
            }
        }
        self.coal_prev = cur;
        self.coal_max = max_id;
        Some(nj as f64 / 1e9)
    }
}

impl SensorReader for Reader {
    fn read(&mut self) -> Result<Sensors> {
        let now = Instant::now();
        let mut s = Sensors::default();

        if let Some(tok) = self.thermal {
            let mut lvl = 0u64;
            // SAFETY: `tok` came from notify_register_check.
            if unsafe { notify_get_state(tok, &mut lvl) } == NOTIFY_STATUS_OK {
                s.pressure = pressure(lvl);
            }
        }

        let mut cpu = self.hid_temps(&self.hid_cpu);
        if let (true, Some(smc)) = (cpu.is_empty(), &self.smc) {
            cpu = smc.temps(&self.smc_cpu);
        }
        (s.cpu_temp_c, s.cpu_temp_max_c) = mean_max(&cpu);
        let mut gpu = self.hid_temps(&self.hid_gpu);
        if let (true, Some(smc)) = (gpu.is_empty(), &self.smc) {
            gpu = smc.temps(&self.smc_gpu);
        }
        s.gpu_temp_c = mean_max(&gpu).0;

        if let Some(smc) = &self.smc {
            for f in &self.smc_fans {
                if let Some(v) = smc.value(f)
                    && v >= 0.0
                {
                    s.fan_rpm.push(v);
                }
            }
        }

        let dt = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        let mut j = [0.0f64; 4]; // CPU, GPU, ANE, DRAM joules
        if let Some(ior) = &mut self.ior {
            j = ior.sample();
        }
        let coal_j = if self.has_coal { self.sample_coalitions() } else { None };
        if dt > 0.05 {
            if j[0] <= 0.0
                && let Some(c) = coal_j
            {
                // A busy-or-idle CPU never uses exactly zero energy: the
                // IOReport CPU channels are frozen, use coalition accounting.
                j[0] = c;
            }
            s.cpu_power_w = j[0] / dt;
            s.gpu_power_w = j[1] / dt;
            s.soc_power_w = (j[0] + j[1] + j[2] + j[3]) / dt;
        }
        if let (Some(smc), Some(k)) = (&self.smc, &self.smc_soc)
            && let Some(v) = smc.value(k)
            && v > s.soc_power_w
            && v < 1000.0
        {
            s.soc_power_w = v;
        }
        Ok(s)
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        if let Some(tok) = self.thermal.take() {
            // SAFETY: `tok` came from notify_register_check.
            unsafe { notify_cancel(tok) };
        }
        // hid, smc and ior release their own objects.
    }
}

fn pressure(mut lvl: u64) -> ThermalPressure {
    // macOS uses 0..4; embedded platforms use 0,10..50. Accept both.
    if lvl >= 10 {
        lvl = lvl / 10 - 1;
        if lvl == 0 {
            // "light" has no macOS equivalent
            lvl = 1;
        }
    }
    match lvl {
        0 => ThermalPressure::Nominal,
        1 => ThermalPressure::Moderate,
        2 => ThermalPressure::Heavy,
        3 => ThermalPressure::Trapping,
        4 => ThermalPressure::Sleeping,
        _ => ThermalPressure::Unknown,
    }
}

fn mean_max(v: &[f64]) -> (f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0);
    }
    let hi = v.iter().fold(0.0f64, |a, &x| a.max(x));
    (v.iter().sum::<f64>() / v.len() as f64, hi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn plausible() {
        let mut r = open().unwrap();
        r.read().unwrap();
        thread::sleep(Duration::from_secs(1));
        let s = r.read().unwrap();
        eprintln!("{s:?}");

        assert_ne!(s.pressure, ThermalPressure::Unknown, "thermal pressure unavailable");
        assert!(
            (20.0..=110.0).contains(&s.cpu_temp_c) && s.cpu_temp_max_c >= s.cpu_temp_c && s.cpu_temp_max_c <= 110.0,
            "implausible CPU temps: mean {:.1} max {:.1}",
            s.cpu_temp_c,
            s.cpu_temp_max_c
        );
        // The GPU sensors read 0 while the GPU is power-gated.
        assert!(
            s.gpu_temp_c == 0.0 || (20.0..=110.0).contains(&s.gpu_temp_c),
            "implausible GPU temp {:.1}",
            s.gpu_temp_c
        );
        assert!(s.cpu_power_w > 0.0 && s.cpu_power_w <= 200.0, "implausible CPU power {:.2} W", s.cpu_power_w);
        assert!(
            s.gpu_power_w >= 0.0 && s.soc_power_w >= s.cpu_power_w && s.soc_power_w <= 500.0,
            "implausible power: GPU {:.2} W, SoC {:.2} W",
            s.gpu_power_w,
            s.soc_power_w
        );
        for rpm in &s.fan_rpm {
            assert!((0.0..=10000.0).contains(rpm), "implausible fan speed {rpm:.0}");
        }
    }

    #[test]
    fn pressure_levels() {
        use ThermalPressure::*;
        for (lvl, want) in
            [(0, Nominal), (1, Moderate), (2, Heavy), (3, Trapping), (4, Sleeping), (30, Heavy), (99, Unknown)]
        {
            assert_eq!(pressure(lvl), want, "pressure({lvl})");
        }
    }
}
