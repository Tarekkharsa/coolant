//! Benchmark configuration and presets.

use coolant_platform::Backend;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Location names.
pub const INDEXED: &str = "indexed";
pub const NO_INDEX: &str = "noindex";

/// Scenario names.
pub const SC_SMALL_FILES: &str = "smallfiles";
pub const SC_SPAWN: &str = "spawn";
pub const SC_SPAWN_NEW_BINARY: &str = "spawn-new-binary";
pub const SC_GIT: &str = "git";
pub const SC_COPY_DEPS: &str = "copy-deps";
pub const SC_CLONEFILE_DEPS: &str = "clonefile-deps";

/// Every scenario in run order.
pub const ALL_SCENARIOS: [&str; 6] =
    [SC_SMALL_FILES, SC_SPAWN, SC_SPAWN_NEW_BINARY, SC_GIT, SC_COPY_DEPS, SC_CLONEFILE_DEPS];

/// Durations are stored as integer nanoseconds in JSON (`interval_ns`, ...).
mod ns {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_nanos().min(u64::MAX as u128) as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_nanos(u64::deserialize(d)?))
    }
}

/// Controls a benchmark run. Zero fields take the values from
/// [`Config::default`].
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Backend to profile with; defaults to `coolant_platform::current()`.
    #[serde(skip)]
    pub backend: Option<Arc<dyn Backend>>,

    /// `indexed_base` and `noindex_base` are the parent directories for the
    /// two locations. Each run creates (and later deletes) a run-* directory
    /// inside them and touches nothing else. `noindex_base` should end in
    /// ".noindex".
    pub indexed_base: String,
    pub noindex_base: String,

    /// Subset of [`INDEXED`], [`NO_INDEX`].
    pub locations: Vec<String>,
    /// Subset of [`ALL_SCENARIOS`].
    pub scenarios: Vec<String>,
    /// Parallelism sweep, e.g. 1,2,4.
    pub agents: Vec<usize>,
    /// Repetitions per case; the median is reported.
    pub reps: usize,

    // Per-agent workload sizes.
    /// Files in the node_modules-like tree.
    pub smallfiles: usize,
    /// Execs per agent (spawn, spawn-new-binary).
    pub spawns: usize,
    /// Distinct never-seen binaries per agent.
    pub new_binaries: usize,
    /// Files in the repo.
    pub git_files: usize,
    /// git status invocations.
    pub git_status: usize,
    /// Worktrees created and removed.
    pub git_worktrees: usize,
    /// Files in the deps tree copied by copy/clonefile-deps.
    pub deps_files: usize,

    /// Profiler sample interval.
    #[serde(rename = "interval_ns", with = "ns")]
    pub interval: Duration,
    /// Quiet period required before a case.
    #[serde(rename = "settle_window_ns", with = "ns")]
    pub settle_window: Duration,
    /// Give up waiting for quiet after this.
    #[serde(rename = "settle_max_ns", with = "ns")]
    pub settle_max: Duration,
    /// "Quiet" means fewer busy cores than this...
    pub settle_busy_cores: f64,
    /// ...and CPU temp within this many °C of the start.
    pub settle_temp_slack: f64,
    /// Keep recording after the workload ends.
    #[serde(rename = "tail_ns", with = "ns")]
    pub tail: Duration,

    /// Includes every full profiler `Run` in the results. They are large
    /// (one sample per interval with per-category maps).
    pub keep_profiles: bool,
}

impl Default for Config {
    /// The default configuration: realistic sizes, K=1,2,4, one repetition.
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let join = |name: &str| std::path::Path::new(&home).join(name).to_string_lossy().into_owned();
        Config {
            backend: None,
            indexed_base: join("coolant-bench"),
            noindex_base: join("coolant-bench.noindex"),
            locations: vec![INDEXED.into(), NO_INDEX.into()],
            scenarios: ALL_SCENARIOS.iter().map(|s| s.to_string()).collect(),
            agents: vec![1, 2, 4],
            reps: 1,
            smallfiles: 20000,
            spawns: 2000,
            new_binaries: 20,
            git_files: 10000,
            git_status: 50,
            git_worktrees: 3,
            deps_files: 20000,
            interval: Duration::from_secs(1),
            settle_window: Duration::from_secs(3),
            settle_max: Duration::from_secs(60),
            settle_busy_cores: 1.0,
            settle_temp_slack: 3.0,
            tail: Duration::from_secs(10),
            keep_profiles: false,
        }
    }
}

impl Config {
    /// The default configuration with three repetitions per case.
    pub fn full() -> Config {
        Config { reps: 3, ..Config::default() }
    }

    /// A small configuration (K=1,2, small trees, short settle and tail)
    /// that finishes in a few minutes.
    pub fn quick() -> Config {
        Config {
            agents: vec![1, 2],
            smallfiles: 3000,
            spawns: 300,
            new_binaries: 10,
            git_files: 1500,
            git_status: 20,
            git_worktrees: 2,
            deps_files: 3000,
            interval: Duration::from_millis(500),
            settle_window: Duration::from_secs(2),
            settle_max: Duration::from_secs(15),
            tail: Duration::from_secs(4),
            ..Config::default()
        }
    }

    /// Fills zero fields from the default configuration.
    pub(crate) fn with_defaults(mut self) -> Config {
        let d = Config::default();
        if self.indexed_base.is_empty() {
            self.indexed_base = d.indexed_base;
        }
        if self.noindex_base.is_empty() {
            self.noindex_base = d.noindex_base;
        }
        if self.locations.is_empty() {
            self.locations = d.locations;
        }
        if self.scenarios.is_empty() {
            self.scenarios = d.scenarios;
        }
        if self.agents.is_empty() {
            self.agents = d.agents;
        }
        let set_int = |v: &mut usize, def: usize| {
            if *v == 0 {
                *v = def;
            }
        };
        set_int(&mut self.reps, d.reps);
        set_int(&mut self.smallfiles, d.smallfiles);
        set_int(&mut self.spawns, d.spawns);
        set_int(&mut self.new_binaries, d.new_binaries);
        set_int(&mut self.git_files, d.git_files);
        set_int(&mut self.git_status, d.git_status);
        set_int(&mut self.git_worktrees, d.git_worktrees);
        set_int(&mut self.deps_files, d.deps_files);
        let set_dur = |v: &mut Duration, def: Duration| {
            if v.is_zero() {
                *v = def;
            }
        };
        set_dur(&mut self.interval, d.interval);
        set_dur(&mut self.settle_window, d.settle_window);
        set_dur(&mut self.settle_max, d.settle_max);
        set_dur(&mut self.tail, d.tail);
        if self.settle_busy_cores <= 0.0 {
            self.settle_busy_cores = d.settle_busy_cores;
        }
        if self.settle_temp_slack <= 0.0 {
            self.settle_temp_slack = d.settle_temp_slack;
        }
        self
    }

    /// The per-agent size parameter for a scenario (0 if unknown).
    pub(crate) fn scale(&self, sc: &str) -> usize {
        match sc {
            SC_SMALL_FILES => self.smallfiles,
            SC_SPAWN | SC_SPAWN_NEW_BINARY => self.spawns,
            SC_GIT => self.git_files,
            SC_COPY_DEPS | SC_CLONEFILE_DEPS => self.deps_files,
            _ => 0,
        }
    }

    pub(crate) fn base(&self, loc: &str) -> &str {
        if loc == NO_INDEX { &self.noindex_base } else { &self.indexed_base }
    }
}
