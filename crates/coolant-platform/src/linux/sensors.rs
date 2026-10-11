//! Sensors from sysfs, without root. Every source is optional:
//!
//!   - CPU temperature: hwmon chips for the CPU (coretemp, k10temp, zenpower,
//!     cpu_thermal, soc_thermal), falling back to thermal zones.
//!   - GPU temperature and power: amdgpu / nouveau hwmon.
//!   - Fans: any hwmon `fan*_input`.
//!   - Power: RAPL `energy_uj` counters, differenced between reads. Recent
//!     kernels make these root-only (CVE-2020-8694), in which case power
//!     stays zero.
//!
//! Thermal pressure has no Linux equivalent and is always `Unknown`.

use super::read_lossy;
use crate::*;
use std::fs;
use std::time::Instant;

const HWMON: &str = "/sys/class/hwmon";
const THERMAL: &str = "/sys/class/thermal";
const POWERCAP: &str = "/sys/class/powercap";

/// hwmon chip names whose temperature inputs describe the CPU.
const CPU_CHIPS: &[&str] = &["coretemp", "k10temp", "zenpower", "cpu_thermal", "soc_thermal"];
const GPU_CHIPS: &[&str] = &["amdgpu", "nouveau"];

pub fn open() -> Result<Box<dyn SensorReader>> {
    let mut r = Reader::discover();
    let first = r.read()?;
    let any = first.cpu_temp_c > 0.0 || first.gpu_temp_c > 0.0 || !first.fan_rpm.is_empty() || !r.rapl_pkg.is_empty();
    if !any {
        return Err(Error::Unsupported);
    }
    Ok(Box::new(r))
}

/// What a CPU temperature input measures, from its label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Package,
    Core,
    Tctl,
    Tdie,
    /// Unlabelled (cpu_thermal, thermal zones) or anything else.
    Other,
}

impl Kind {
    fn from_label(label: &str) -> Option<Kind> {
        let l = label.to_ascii_lowercase();
        Some(if l.starts_with("package") || l.starts_with("physical id") {
            Kind::Package
        } else if l.starts_with("core") {
            Kind::Core
        } else if l == "tctl" {
            Kind::Tctl
        } else if l == "tdie" {
            Kind::Tdie
        } else if l.is_empty() || l.starts_with("cpu") || l.starts_with("soc") {
            Kind::Other
        } else {
            return None; // e.g. Tccd1, SVI2_*, Composite
        })
    }
}

/// (mean, max) over CPU temperature readings. Tctl is dropped when Tdie is
/// present (Tctl can carry a vendor offset), and the package reading is left
/// out of the mean when per-core readings exist, but still counts for the max.
fn aggregate(vals: &[(Kind, f64)]) -> (f64, f64) {
    let has = |k: Kind| vals.iter().any(|v| v.0 == k);
    let vals: Vec<(Kind, f64)> = vals.iter().copied().filter(|v| !(v.0 == Kind::Tctl && has(Kind::Tdie))).collect();
    let max = vals.iter().map(|v| v.1).fold(0.0, f64::max);
    let for_mean: Vec<f64> = vals.iter().filter(|v| !(v.0 == Kind::Package && has(Kind::Core))).map(|v| v.1).collect();
    if for_mean.is_empty() {
        return (0.0, 0.0);
    }
    (for_mean.iter().sum::<f64>() / for_mean.len() as f64, max)
}

/// A RAPL energy counter.
struct Domain {
    path: String,
    /// Counter range; it wraps to zero past this.
    max_uj: u64,
    last: Option<(u64, Instant)>,
}

impl Domain {
    fn open(dir: &str) -> Option<Domain> {
        let path = format!("{dir}/energy_uj");
        read_u64(&path)?; // fails with EACCES when not root
        let max_uj = read_u64(&format!("{dir}/max_energy_range_uj")).unwrap_or(0);
        Some(Domain { path, max_uj, last: None })
    }

    /// Watts since the previous call; None on the first.
    fn sample(&mut self, now: Instant) -> Option<f64> {
        let e = read_u64(&self.path)?;
        let prev = self.last.replace((e, now))?;
        let dt = now.duration_since(prev.1).as_secs_f64();
        let delta = if e >= prev.0 {
            e - prev.0
        } else if self.max_uj > prev.0 {
            self.max_uj - prev.0 + e // wrapped
        } else {
            return None;
        };
        (dt > 0.0).then(|| delta as f64 / 1e6 / dt)
    }
}

struct Reader {
    /// (kind, path of temp*_input)
    cpu_temps: Vec<(Kind, String)>,
    /// Thermal-zone temp files, used only when hwmon has no CPU sensor.
    zones: Vec<String>,
    gpu_temp: Option<String>,
    gpu_power: Option<String>,
    fans: Vec<String>,
    rapl_pkg: Vec<Domain>,
    rapl_core: Vec<Domain>,
}

impl Reader {
    fn discover() -> Reader {
        let mut r = Reader {
            cpu_temps: vec![],
            zones: vec![],
            gpu_temp: None,
            gpu_power: None,
            fans: vec![],
            rapl_pkg: vec![],
            rapl_core: vec![],
        };
        for dir in list_dirs(HWMON, "hwmon") {
            let name = read_trimmed(&format!("{dir}/name")).unwrap_or_default();
            let files = list_files(&dir);
            let inputs = |prefix: &str| -> Vec<String> {
                let mut v: Vec<String> = files
                    .iter()
                    .filter(|f| f.starts_with(prefix) && f.ends_with("_input"))
                    .map(|f| f.trim_end_matches("_input").to_string())
                    .collect();
                v.sort_by_key(|s| s.trim_start_matches(char::is_alphabetic).parse::<u32>().unwrap_or(0));
                v
            };
            if CPU_CHIPS.contains(&name.as_str()) {
                for t in inputs("temp") {
                    let label = read_trimmed(&format!("{dir}/{t}_label")).unwrap_or_default();
                    if let Some(kind) = Kind::from_label(&label) {
                        r.cpu_temps.push((kind, format!("{dir}/{t}_input")));
                    }
                }
            } else if GPU_CHIPS.contains(&name.as_str()) && r.gpu_temp.is_none() {
                // temp1 is the edge sensor on amdgpu and the die on nouveau.
                r.gpu_temp = inputs("temp").first().map(|t| format!("{dir}/{t}_input"));
                r.gpu_power = ["power1_average", "power1_input"]
                    .iter()
                    .map(|f| format!("{dir}/{f}"))
                    .find(|p| read_u64(p).is_some());
            }
            for f in inputs("fan") {
                r.fans.push(format!("{dir}/{f}_input"));
            }
        }

        for dir in list_dirs(THERMAL, "thermal_zone") {
            let ty = read_trimmed(&format!("{dir}/type")).unwrap_or_default().to_ascii_lowercase();
            if ty == "x86_pkg_temp" || ty.starts_with("cpu") || ty.starts_with("soc") {
                r.zones.push(format!("{dir}/temp"));
            }
        }

        // Package domains are "intel-rapl:N" (one colon); "intel-rapl:N:M"
        // are their subdomains, of which "core" is the CPU cores alone.
        for dir in list_dirs(POWERCAP, "intel-rapl:") {
            let depth = dir.rsplit('/').next().unwrap_or("").matches(':').count();
            let name = read_trimmed(&format!("{dir}/name")).unwrap_or_default();
            if depth == 1 && name.starts_with("package") {
                r.rapl_pkg.extend(Domain::open(&dir));
            } else if depth == 2 && name == "core" {
                r.rapl_core.extend(Domain::open(&dir));
            }
        }
        r
    }
}

impl SensorReader for Reader {
    fn read(&mut self) -> Result<Sensors> {
        let mut s = Sensors::default();

        let mut temps: Vec<(Kind, f64)> = self.cpu_temps.iter().filter_map(|(k, p)| Some((*k, celsius(p)?))).collect();
        if temps.is_empty() {
            temps = self.zones.iter().filter_map(|p| Some((Kind::Other, celsius(p)?))).collect();
        }
        (s.cpu_temp_c, s.cpu_temp_max_c) = aggregate(&temps);

        s.gpu_temp_c = self.gpu_temp.as_deref().and_then(celsius).unwrap_or(0.0);
        s.gpu_power_w = self.gpu_power.as_deref().and_then(read_u64).map_or(0.0, |uw| uw as f64 / 1e6);
        s.fan_rpm = self.fans.iter().filter_map(|p| read_u64(p)).map(|v| v as f64).collect();
        if let Some(m) = read_lossy("/proc/meminfo") {
            let kb = |key: &str| -> Option<u64> {
                let v = m.lines().find_map(|l| l.strip_prefix(key))?;
                v.split_ascii_whitespace().next()?.parse().ok()
            };
            if let (Some(total), Some(avail)) = (kb("MemTotal:"), kb("MemAvailable:"))
                && total > 0
            {
                s.memory_free_pct = 100.0 * avail as f64 / total as f64;
            }
            if let (Some(total), Some(free)) = (kb("SwapTotal:"), kb("SwapFree:")) {
                s.swap_used_bytes = total.saturating_sub(free) * 1024;
            }
        }

        let now = Instant::now();
        let watts = |domains: &mut Vec<Domain>| -> Option<f64> {
            // Every domain must advance its baseline, so no short-circuiting.
            let w: Vec<Option<f64>> = domains.iter_mut().map(|d| d.sample(now)).collect();
            (!w.is_empty() && w.iter().all(Option::is_some)).then(|| w.into_iter().flatten().sum())
        };
        let pkg = watts(&mut self.rapl_pkg);
        let core = watts(&mut self.rapl_core);
        s.soc_power_w = pkg.unwrap_or(0.0);
        s.cpu_power_w = core.or(pkg).unwrap_or(0.0);
        Ok(s)
    }
}

/// Reads a millidegree file; None unless the value is a plausible CPU/GPU
/// temperature (some sensors report 0 or -273 when absent).
fn celsius(path: &str) -> Option<f64> {
    let v = read_i64(path)? as f64 / 1000.0;
    (v > 0.0 && v < 150.0).then_some(v)
}

fn read_trimmed(path: &str) -> Option<String> {
    Some(read_lossy(path)?.trim().to_string())
}

fn read_i64(path: &str) -> Option<i64> {
    read_trimmed(path)?.parse().ok()
}

fn read_u64(path: &str) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

/// Full paths of entries in `dir` whose names start with `prefix`.
fn list_dirs(dir: &str, prefix: &str) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(prefix))
        .map(|n| format!("{dir}/{n}"))
        .collect();
    v.sort();
    v
}

fn list_files(dir: &str) -> Vec<String> {
    fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| e.file_name().into_string().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(Kind::from_label("Package id 0"), Some(Kind::Package));
        assert_eq!(Kind::from_label("Core 3"), Some(Kind::Core));
        assert_eq!(Kind::from_label("Tctl"), Some(Kind::Tctl));
        assert_eq!(Kind::from_label(""), Some(Kind::Other));
        assert_eq!(Kind::from_label("Tccd1"), None);
    }

    #[test]
    fn aggregation() {
        use Kind::*;
        // Cores drive the mean; the package still counts for the max.
        let (mean, max) = aggregate(&[(Package, 60.0), (Core, 40.0), (Core, 50.0)]);
        assert_eq!((mean, max), (45.0, 60.0));
        // Tdie wins over Tctl.
        assert_eq!(aggregate(&[(Tctl, 80.0), (Tdie, 55.0)]), (55.0, 55.0));
        assert_eq!(aggregate(&[(Tctl, 80.0)]), (80.0, 80.0));
        assert_eq!(aggregate(&[]), (0.0, 0.0));
    }

    #[test]
    fn rapl_wraps() {
        let dir = std::env::temp_dir().join(format!("coolant-rapl-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("energy_uj");
        fs::write(&path, "900000").unwrap();
        let mut d = Domain { path: path.to_str().unwrap().into(), max_uj: 1_000_000, last: None };
        let t0 = Instant::now();
        assert_eq!(d.sample(t0), None);
        fs::write(&path, "100000").unwrap(); // wrapped: 100000 + 200000 = 0.3 J
        let w = d.sample(t0 + Duration::from_secs(1)).unwrap();
        assert!((w - 0.2).abs() < 1e-9, "got {w}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn live_is_unsupported_or_plausible() {
        match open() {
            Err(Error::Unsupported) => {}
            Err(e) => panic!("unexpected error: {e}"),
            Ok(mut r) => {
                let s = r.read().unwrap();
                assert_eq!(s.pressure, ThermalPressure::Unknown);
                assert!(s.cpu_temp_c == 0.0 || (1.0..150.0).contains(&s.cpu_temp_c), "{s:?}");
                assert!(s.cpu_temp_max_c >= s.cpu_temp_c);
                assert!(s.cpu_power_w >= 0.0 && s.soc_power_w >= 0.0 && s.cpu_power_w < 2000.0);
                assert!(s.gpu_temp_c == 0.0 || (1.0..150.0).contains(&s.gpu_temp_c));
                assert!(s.fan_rpm.iter().all(|&f| (0.0..100_000.0).contains(&f)));
            }
        }
    }
}
