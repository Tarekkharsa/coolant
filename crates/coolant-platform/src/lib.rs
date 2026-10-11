//! The OS-independent view of a machine that the profiler and benchmarks
//! consume. Each OS implements [`Backend`] in its own module; [`current`]
//! picks one at compile time.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

#[cfg(target_os = "macos")]
mod darwin;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

/// Returns the backend for the OS this binary was built for.
pub fn current() -> Box<dyn Backend> {
    #[cfg(target_os = "macos")]
    return Box::new(darwin::Darwin::new());
    #[cfg(target_os = "linux")]
    return Box::new(linux::Linux::new());
    #[cfg(target_os = "windows")]
    return Box::new(windows::Windows);
}

#[derive(Debug)]
pub enum Error {
    /// The OS, or the current privilege level, can't provide this.
    Unsupported,
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported => f.write_str("not supported on this platform"),
            Error::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Everything the profiler needs from an operating system.
pub trait Backend: Send + Sync {
    /// Short identifier such as "darwin".
    fn name(&self) -> &'static str;
    /// Hardware and OS description, for reports.
    fn machine(&self) -> Machine;
    /// A snapshot of every visible process.
    fn processes(&self) -> Result<Vec<Process>>;
    /// Cumulative CPU time for the whole machine.
    fn cpu(&self) -> Result<CpuTimes>;
    /// Opens a reader for temperature, power and thermal pressure.
    fn sensors(&self) -> Result<Box<dyn SensorReader>>;
    /// Opens a tracker that counts processes created since it started,
    /// including ones too short-lived to appear in `processes`.
    fn spawn_counter(&self) -> Result<Box<dyn SpawnCounter>>;
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Machine {
    pub os: String,
    pub os_version: String,
    pub model: String,
    pub cpu: String,
    pub cores: u32,
    pub mem_bytes: u64,
}

/// One process at one instant. Cumulative counters only make sense as
/// deltas between two snapshots of the same process.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Process {
    pub pid: i32,
    pub ppid: i32,
    pub uid: u32,
    pub user: String,
    /// Short executable name.
    pub name: String,
    /// Full command line when visible, otherwise the executable path.
    pub command: String,
    /// User + system, cumulative.
    pub cpu_time: Duration,
    pub rss_bytes: u64,
    /// True when the fields below could be read. On macOS that means
    /// processes owned by the current user.
    pub detailed: bool,
    /// CPU of reaped descendants, cumulative.
    pub child_cpu_time: Duration,
    pub disk_read: u64,
    pub disk_written: u64,
    pub wakeups: u64,
    /// Process start, in milliseconds since the Unix epoch (0 if unknown).
    /// Together with the PID this identifies a process across snapshots.
    pub start_ms: u64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct CpuTimes {
    /// Summed across all cores.
    pub busy: Duration,
    pub idle: Duration,
}

/// Reads point-in-time sensor values. Power is averaged since the previous
/// read, so the first read may report zero power.
pub trait SensorReader: Send {
    fn read(&mut self) -> Result<Sensors>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThermalPressure {
    #[default]
    Unknown,
    Nominal,
    Moderate,
    Heavy,
    Trapping,
    Sleeping,
}

impl fmt::Display for ThermalPressure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Unknown => "unknown",
            Self::Nominal => "nominal",
            Self::Moderate => "moderate",
            Self::Heavy => "heavy",
            Self::Trapping => "trapping",
            Self::Sleeping => "sleeping",
        };
        f.write_str(s)
    }
}

/// The kernel's view of memory: whether it has started compressing, swapping
/// and asking apps to free memory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryPressure {
    #[default]
    Unknown,
    Normal,
    Warning,
    Critical,
}

impl fmt::Display for MemoryPressure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Unknown => "unknown",
            Self::Normal => "normal",
            Self::Warning => "warning",
            Self::Critical => "critical",
        };
        f.write_str(s)
    }
}

/// Whatever the platform could read; zero means unavailable.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Sensors {
    pub pressure: ThermalPressure,
    /// Mean of CPU die sensors.
    pub cpu_temp_c: f64,
    /// Hottest CPU die sensor.
    pub cpu_temp_max_c: f64,
    pub gpu_temp_c: f64,
    pub cpu_power_w: f64,
    pub gpu_power_w: f64,
    /// Whole package when available.
    pub soc_power_w: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fan_rpm: Vec<f64>,
    #[serde(default)]
    pub memory_pressure: MemoryPressure,
    /// Share of memory the kernel considers available, 0–100.
    #[serde(default)]
    pub memory_free_pct: f64,
    #[serde(default)]
    pub swap_used_bytes: u64,
}

/// Counts process creations since it was opened.
pub trait SpawnCounter: Send {
    /// Processes started since the previous call. `name` is empty when a
    /// process exited before it could be identified.
    fn drain(&mut self) -> Vec<Spawn>;
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Spawn {
    pub pid: i32,
    pub ppid: i32,
    pub name: String,
}
