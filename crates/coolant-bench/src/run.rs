//! Runs the cases: settle, profile, workload, tail, and turn the profile
//! into per-case metrics.

use crate::config::*;
use crate::scenarios::build_template;
use crate::scenarios::{Scenario, scenarios};
use crate::util::*;
use anyhow::{Context, Result, anyhow};
use coolant_core::classify::Category;
use coolant_core::profile::{Options, Recorder, Run, Sample};
use coolant_platform::{Backend, Machine};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One measurement of one case. CPU figures are in seconds and cover the
/// whole recording window (workload + settle tail) unless noted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Metrics {
    /// Workload only.
    #[serde(rename = "wall_s")]
    pub wall_sec: f64,
    /// Workload + tail.
    #[serde(rename = "window_s")]
    pub window_sec: f64,
    pub ops: usize,
    #[serde(rename = "ops_per_s")]
    pub ops_per_sec: f64,

    /// Whole machine.
    #[serde(rename = "busy_cpu_s")]
    pub busy_cpu_sec: f64,
    /// CPU used by the benchmark process and its children during the
    /// workload (getrusage), minus the profiler's own cost.
    #[serde(rename = "workload_cpu_s")]
    pub workload_cpu_sec: f64,
    /// Busy CPU minus everything the benchmark process tree used: what the
    /// rest of the machine did during the window.
    #[serde(rename = "background_cpu_s")]
    pub background_cpu_sec: f64,
    /// Background CPU minus the idle background rate measured while
    /// settling, i.e. extra work our workload caused.
    #[serde(rename = "background_excess_s")]
    pub background_excess_sec: f64,

    /// Profiler CPU per category with the benchmark's own process tree
    /// subtracted, so "other" means other processes.
    pub categories: BTreeMap<String, f64>,
    /// Sums categories where `Category::is_overhead()` is true (indexer,
    /// security, fs-daemon, backup-sync).
    #[serde(rename = "overhead_cpu_s")]
    pub overhead_cpu_sec: f64,
    /// Overhead minus the overhead rate seen while settling.
    #[serde(rename = "overhead_excess_s")]
    pub overhead_excess_sec: f64,
    /// Overhead CPU as a percentage of workload CPU.
    pub overhead_pct: f64,

    /// While settling.
    pub baseline_busy_cores: f64,
    #[serde(rename = "settle_s")]
    pub settle_sec: f64,

    pub spawns: u64,
    #[serde(rename = "cpu_temp_start_c")]
    pub cpu_temp_start: f64,
    #[serde(rename = "cpu_temp_max_c")]
    pub cpu_temp_max: f64,
    #[serde(rename = "cpu_power_avg_w")]
    pub cpu_power_avg: f64,
    #[serde(rename = "soc_power_avg_w")]
    pub soc_power_avg: f64,
    #[serde(rename = "fan_max_rpm")]
    pub fan_max: f64,

    /// Scenario-specific numbers such as first_exec_ms.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, f64>,
}

/// CPU used by a (non-benchmark) process name during a case, averaged over
/// repetitions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcCpu {
    pub name: String,
    pub category: String,
    #[serde(rename = "cpu_s")]
    pub cpu_sec: f64,
}

/// One (scenario, location, agents) case.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BenchResult {
    pub scenario: String,
    pub description: String,
    pub location: String,
    /// Base dir of the location.
    pub dir: String,
    pub agents: usize,
    /// Per-agent size parameter.
    pub scale: usize,
    pub machine: Machine,
    pub median: Metrics,
    pub reps: Vec<Metrics>,
    /// The processes outside the benchmark that used the most CPU, to spot
    /// what reacts to the workload (and misclassified "other" processes).
    pub top_background: Vec<ProcCpu>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// Only with `Config::keep_profiles`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<Run>,
}

/// Executes every (scenario, agents, location) case `cfg` asks for and
/// returns one result per case. All directories it creates live in run-*
/// folders under `cfg.indexed_base` and `cfg.noindex_base` and are removed
/// before it returns, including on error or cancellation. Progress lines go
/// to `progress`. Setting `cancel` stops the run with a [`Canceled`] error.
pub fn run(cfg: &Config, cancel: &AtomicBool, progress: &mut dyn Write) -> Result<Vec<BenchResult>> {
    let (results, err) = run_partial(cfg, cancel, progress);
    match err {
        Some(e) => Err(e),
        None => Ok(results),
    }
}

/// Like [`run`], but also returns the results of the cases completed before
/// an error or cancellation.
pub fn run_partial(
    cfg: &Config,
    cancel: &AtomicBool,
    progress: &mut dyn Write,
) -> (Vec<BenchResult>, Option<anyhow::Error>) {
    let cfg = cfg.clone().with_defaults();
    // Create run-* dirs and make sure they go away whatever happens.
    let ws = Workspace::default();
    let mut results = Vec::new();
    let mut err = run_cases(&cfg, cancel, progress, &ws, &mut results).err();
    if let Err(cerr) = ws.cleanup() {
        err = err.or(Some(cerr));
    }
    for r in &mut results {
        r.median = median(&r.reps);
    }
    (results, err)
}

fn run_cases(
    cfg: &Config,
    cancel: &AtomicBool,
    progress: &mut dyn Write,
    ws: &Workspace,
    results: &mut Vec<BenchResult>,
) -> Result<()> {
    let mut logf = |s: String| {
        let _ = writeln!(progress, "{s}");
    };
    let backend: Arc<dyn Backend> = cfg.backend.clone().unwrap_or_else(|| Arc::from(coolant_platform::current()));
    for sc in &cfg.scenarios {
        if cfg.scale(sc) == 0 {
            return Err(anyhow!("unknown scenario {sc:?}"));
        }
    }
    for loc in &cfg.locations {
        if loc != INDEXED && loc != NO_INDEX {
            return Err(anyhow!("unknown location {loc:?}"));
        }
    }
    if !clean(Path::new(&cfg.noindex_base)).to_string_lossy().ends_with(".noindex") {
        logf(format!("warning: noindex base {} does not end in .noindex; Spotlight will index it", cfg.noindex_base));
    }

    let mut run_dirs: HashMap<&str, PathBuf> = HashMap::new();
    for loc in &cfg.locations {
        run_dirs.insert(loc, ws.new_run_dir(Path::new(cfg.base(loc)))?);
    }

    let mut tmpl = PathBuf::new();
    if cfg.scenarios.iter().any(|s| s == SC_SPAWN_NEW_BINARY) {
        // Build in the noindex location if present, so building the
        // template doesn't itself trigger indexing.
        let loc = cfg.locations.last().unwrap();
        tmpl = build_template(cancel, &run_dirs[loc.as_str()].join("template"))?;
    }
    let table = scenarios(cfg, &tmpl);
    let machine = backend.machine();

    let mut by_key: HashMap<(String, String, usize), usize> = HashMap::new();
    let mut b = Bencher::new(cfg.clone(), backend);
    let mut case_n = 0;
    for sc_name in &cfg.scenarios {
        let sc = &table[sc_name.as_str()];
        for &k in &cfg.agents {
            let mut locs = cfg.locations.clone();
            if case_n % 2 == 1 {
                // alternate which location goes first
                locs.reverse();
            }
            case_n += 1;
            for rep in 0..cfg.reps {
                for loc in &locs {
                    check(cancel)?;
                    let idx = *by_key.entry((sc_name.clone(), loc.clone(), k)).or_insert_with(|| {
                        results.push(BenchResult {
                            scenario: sc_name.clone(),
                            description: sc.desc.clone(),
                            location: loc.clone(),
                            dir: cfg.base(loc).to_string(),
                            agents: k,
                            scale: cfg.scale(sc_name),
                            machine: machine.clone(),
                            ..Default::default()
                        });
                        results.len() - 1
                    });
                    let res = &mut results[idx];
                    let dir = run_dirs[loc.as_str()].join(format!("{sc_name}-k{k}-r{rep}"));
                    logf(format!("[{loc}] {sc_name} k={k} rep={}/{} ...", rep + 1, cfg.reps));
                    let mut warns = Vec::new();
                    let mut outcome = b.run_case(cancel, sc, &dir, k, &mut warns);
                    if let Err(rerr) = ws.remove(&dir)
                        && outcome.is_ok()
                    {
                        outcome = Err(rerr);
                    }
                    append_unique(&mut res.warnings, warns);
                    match outcome {
                        Err(e) => {
                            res.error = format!("{e:#}");
                            logf(format!("  error: {e:#}"));
                            if is_canceled(cancel) {
                                return Err(Canceled.into());
                            }
                        }
                        Ok((m, run)) => {
                            logf(format!(
                                "  wall {:.2}s  {:.0} ops/s  workload {:.2} cpu-s  overhead {:.2} cpu-s ({:.1}%)  bg excess {:.2} cpu-s",
                                m.wall_sec,
                                m.ops_per_sec,
                                m.workload_cpu_sec,
                                m.overhead_cpu_sec,
                                m.overhead_pct,
                                m.background_excess_sec
                            ));
                            res.reps.push(m);
                            let top = top_background(&run, &b.workload);
                            res.top_background =
                                merge_top(std::mem::take(&mut res.top_background), top, res.reps.len());
                            if cfg.keep_profiles {
                                res.profiles.push(run);
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

type OnSample = Box<dyn FnMut(&Sample) + Send>;

struct Bencher {
    cfg: Config,
    backend: Arc<dyn Backend>,
    /// First CPU temperature seen, for settling.
    temp_base: f64,
    /// Executables the benchmark itself runs; they are excluded from
    /// background process lists.
    workload: HashSet<String>,
}

/// The machine's idle behaviour just before a case.
#[derive(Default)]
struct Baseline {
    busy_cores: f64,
    /// The profiler's own cost (it runs in our process).
    self_cores: f64,
    cats: HashMap<Category, f64>,
    dur: Duration,
    settled: bool,
}

impl Bencher {
    fn new(cfg: Config, backend: Arc<dyn Backend>) -> Bencher {
        let me = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let workload = [me.as_str(), "ps", "true", "git", "cp", "codesign"].into_iter().map(String::from).collect();
        Bencher { cfg, backend, temp_base: 0.0, workload }
    }

    fn start_profiler(&self, on_sample: Option<OnSample>) -> Result<Recorder> {
        Ok(Recorder::start(
            self.backend.clone(),
            Options { interval: self.cfg.interval, on_sample, ..Default::default() },
        )?)
    }

    fn run_case(
        &mut self,
        cancel: &AtomicBool,
        sc: &Scenario,
        dir: &Path,
        k: usize,
        warns: &mut Vec<String>,
    ) -> Result<(Metrics, Run)> {
        let agent_dirs: Vec<PathBuf> = (0..k).map(|i| dir.join(format!("agent{i}"))).collect();
        for d in &agent_dirs {
            std::fs::create_dir_all(d)?;
        }
        if let Some(setup) = &sc.setup {
            parallel(k, |i| setup(cancel, &agent_dirs[i])).context("setup")?;
        }

        let base = self.settle(cancel)?;
        if !base.settled {
            warns.push(format!(
                "machine did not settle below {:.1} busy cores within {:?}",
                self.cfg.settle_busy_cores, self.cfg.settle_max
            ));
        }

        let rec = self.start_profiler(None)?;
        let ru0 = tree_cpu();
        let t0 = Instant::now();
        let werr = parallel(k, |i| (sc.run)(cancel, &agent_dirs[i]));
        let wall = t0.elapsed().as_secs_f64();
        let ru_work = tree_cpu() - ru0;
        if werr.is_ok() {
            sleep(cancel, self.cfg.tail);
        }
        let mut run = rec.stop();
        let ru_window = tree_cpu() - ru0;
        run.label = format!("{} k={k} {}", sc.name, dir.display());
        warns.extend(run.warnings.iter().cloned());
        let outs = werr?;
        check(cancel)?;

        let window = run.duration_s;
        let mut m = Metrics {
            wall_sec: wall,
            window_sec: window,
            busy_cpu_sec: run.busy_cpu_s,
            baseline_busy_cores: base.busy_cores,
            settle_sec: base.dur.as_secs_f64(),
            spawns: run.spawns,
            cpu_temp_start: run.thermal.cpu_temp_start_c,
            cpu_temp_max: run.thermal.cpu_temp_max_c,
            cpu_power_avg: run.thermal.cpu_power_avg_w,
            soc_power_avg: run.thermal.soc_power_avg_w,
            fan_max: run.thermal.fan_max_rpm,
            extra: merge_extras(outs.iter().map(|(_, e)| e)),
            ..Default::default()
        };
        m.ops = outs.iter().map(|(n, _)| n).sum();
        if wall > 0.0 {
            m.ops_per_sec = m.ops as f64 / wall;
        }
        m.workload_cpu_sec = (ru_work - base.self_cores * wall).max(0.0);
        m.background_cpu_sec = (run.busy_cpu_s - ru_window).max(0.0);
        m.background_excess_sec = m.background_cpu_sec - (base.busy_cores - base.self_cores) * window;

        m.categories = background_categories(&run, ru_window, &self.workload);
        let base_overhead: f64 = base.cats.iter().filter(|(c, _)| c.is_overhead()).map(|(_, v)| v).sum();
        m.overhead_cpu_sec = m.categories.iter().filter(|(c, _)| is_overhead_key(c)).map(|(_, v)| v).sum();
        m.overhead_excess_sec = m.overhead_cpu_sec - base_overhead * window;
        if m.workload_cpu_sec >= MIN_WORKLOAD_CPU {
            m.overhead_pct = 100.0 * m.overhead_cpu_sec / m.workload_cpu_sec;
        }
        Ok((m, run))
    }

    /// Samples the machine until it has been quiet (fewer than
    /// `settle_busy_cores` busy, and CPU temperature near where the
    /// benchmark started) for `settle_window`, or `settle_max` passes. It
    /// returns the idle rates seen over the final window.
    fn settle(&mut self, cancel: &AtomicBool) -> Result<Baseline> {
        let (tx, samples) = mpsc::sync_channel::<Sample>(1024);
        let rec = self.start_profiler(Some(Box::new(move |s: &Sample| {
            let _ = tx.try_send(s.clone());
        })))?;
        let start = Instant::now();
        let ru0 = tree_cpu();
        let need = ((self.cfg.settle_window.as_nanos() / self.cfg.interval.as_nanos()) as usize).max(1);
        let mut win: Vec<Sample> = Vec::new();
        let mut bl = Baseline::default();
        loop {
            if is_canceled(cancel) {
                rec.stop();
                return Err(Canceled.into());
            }
            let left = self.cfg.settle_max.saturating_sub(start.elapsed());
            if left.is_zero() {
                break;
            }
            let s = match samples.recv_timeout(left.min(Duration::from_millis(100))) {
                Ok(s) => s,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            let t = s.sensors.cpu_temp_c;
            if t > 0.0 && self.temp_base == 0.0 {
                self.temp_base = t;
            }
            win.push(s);
            if win.len() > need {
                win.remove(0);
            }
            if win.len() < need {
                continue;
            }
            let busy = win.iter().map(|w| w.busy_cores).sum::<f64>() / win.len() as f64;
            let t = win.last().unwrap().sensors.cpu_temp_c;
            if busy < self.cfg.settle_busy_cores
                && (t == 0.0 || self.temp_base == 0.0 || t <= self.temp_base + self.cfg.settle_temp_slack)
            {
                bl.settled = true;
                break;
            }
        }
        rec.stop();
        bl.dur = start.elapsed();
        bl.self_cores = (tree_cpu() - ru0) / bl.dur.as_secs_f64();
        let n = win.len() as f64;
        for w in &win {
            bl.busy_cores += w.busy_cores / n;
            for (c, v) in &w.categories {
                *bl.cats.entry(*c).or_default() += v / n;
            }
        }
        Ok(bl)
    }
}

fn is_overhead_key(key: &str) -> bool {
    Category::ALL.iter().any(|c| c.as_str() == key && c.is_overhead())
}

/// Workload CPU below which overhead percentages are noise (e.g.
/// spawn-new-binary mostly waits on the first-exec check).
pub(crate) const MIN_WORKLOAD_CPU: f64 = 0.05;

/// Returns CPU seconds per category with the benchmark's own process tree
/// removed: visible children (git, cp, ...) are subtracted from their
/// categories, and the rest of the tree's CPU (our process plus children
/// too short-lived to be sampled) from our own category.
pub(crate) fn background_categories(run: &Run, tree: f64, workload: &HashSet<String>) -> BTreeMap<String, f64> {
    let mut cats: BTreeMap<String, f64> = BTreeMap::new();
    for u in &run.categories {
        *cats.entry(u.key.clone()).or_default() += u.cpu_s;
    }
    let me = std::process::id() as i32;
    let mut self_cat = Category::Other.as_str().to_string();
    let mut children = 0.0;
    for p in &run.processes {
        let cat = p.category.unwrap_or(Category::Other).as_str().to_string();
        if p.pid == me {
            self_cat = cat;
        } else if workload.contains(&p.name) {
            *cats.entry(cat).or_default() -= p.cpu_s;
            children += p.cpu_s;
        }
    }
    *cats.entry(self_cat).or_default() -= (tree - children).max(0.0);
    cats.retain(|_, v| *v > 1e-9);
    cats
}

fn top_background(run: &Run, workload: &HashSet<String>) -> Vec<ProcCpu> {
    let me = std::process::id() as i32;
    let mut by_name: HashMap<String, ProcCpu> = HashMap::new();
    for p in &run.processes {
        if p.pid == me || workload.contains(&p.name) || p.cpu_s <= 0.0 {
            continue;
        }
        by_name
            .entry(p.name.clone())
            .or_insert_with(|| ProcCpu {
                name: p.name.clone(),
                category: p.category.unwrap_or(Category::Other).as_str().into(),
                cpu_sec: 0.0,
            })
            .cpu_sec += p.cpu_s;
    }
    let mut out: Vec<ProcCpu> = by_name.into_values().collect();
    sort_procs(&mut out);
    out.truncate(12);
    out
}

/// Folds one repetition's top list into the running mean over n reps.
fn merge_top(acc: Vec<ProcCpu>, next: Vec<ProcCpu>, n: usize) -> Vec<ProcCpu> {
    let n = n as f64;
    let mut m: HashMap<String, ProcCpu> = HashMap::new();
    for mut p in acc {
        p.cpu_sec *= (n - 1.0) / n;
        m.insert(p.name.clone(), p);
    }
    for p in next {
        m.entry(p.name.clone())
            .or_insert_with(|| ProcCpu { name: p.name.clone(), category: p.category.clone(), cpu_sec: 0.0 })
            .cpu_sec += p.cpu_sec / n;
    }
    let mut out: Vec<ProcCpu> = m.into_values().collect();
    sort_procs(&mut out);
    out.truncate(12);
    out
}

pub(crate) fn sort_procs(ps: &mut [ProcCpu]) {
    ps.sort_by(|a, b| b.cpu_sec.total_cmp(&a.cpu_sec).then_with(|| a.name.cmp(&b.name)));
}

/// User+system CPU seconds of this process plus its reaped children.
fn tree_cpu() -> f64 {
    let get = |who| {
        // SAFETY: getrusage fills the zeroed rusage we pass.
        let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(who, &mut ru) };
        let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
        tv(ru.ru_utime) + tv(ru.ru_stime)
    };
    get(libc::RUSAGE_SELF) + get(libc::RUSAGE_CHILDREN)
}

/// Runs `f(0..n)` on n threads, returning all results or every error joined
/// by newlines.
fn parallel<T: Send>(n: usize, f: impl Fn(usize) -> Result<T> + Sync) -> Result<Vec<T>> {
    let results: Vec<Result<T>> = std::thread::scope(|s| {
        let f = &f;
        let handles: Vec<_> = (0..n).map(|i| s.spawn(move || f(i))).collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err(anyhow!("agent thread panicked")))).collect()
    });
    let (mut oks, mut errs) = (Vec::new(), Vec::new());
    for r in results {
        match r {
            Ok(v) => oks.push(v),
            Err(e) => errs.push(e),
        }
    }
    match errs.len() {
        0 => Ok(oks),
        1 => Err(errs.pop().unwrap()),
        _ => {
            if errs.iter().any(|e| e.downcast_ref::<Canceled>().is_some()) {
                return Err(Canceled.into());
            }
            Err(anyhow!(errs.iter().map(|e| format!("{e:#}")).collect::<Vec<_>>().join("\n")))
        }
    }
}

fn merge_extras<'a>(es: impl Iterator<Item = &'a BTreeMap<String, f64>>) -> BTreeMap<String, f64> {
    let mut sum: BTreeMap<String, (f64, usize)> = BTreeMap::new();
    for e in es {
        for (k, v) in e {
            let s = sum.entry(k.clone()).or_default();
            s.0 += v;
            s.1 += 1;
        }
    }
    sum.into_iter().map(|(k, (s, n))| (k, s / n as f64)).collect()
}

fn append_unique(xs: &mut Vec<String>, add: Vec<String>) {
    for a in add {
        if !xs.contains(&a) {
            xs.push(a);
        }
    }
}

// --- workspace: the only code that creates or deletes directories --------------

#[derive(Default)]
struct WorkspaceState {
    /// run-* dirs we created.
    run_dirs: Vec<PathBuf>,
    /// Base dirs we created (removed again if left empty).
    made_base: Vec<PathBuf>,
}

#[derive(Default)]
pub(crate) struct Workspace {
    state: Mutex<WorkspaceState>,
}

impl Workspace {
    pub(crate) fn new_run_dir(&self, base: &Path) -> Result<PathBuf> {
        let base = clean(&std::path::absolute(base)?);
        if !base.exists() {
            std::fs::create_dir_all(&base)?;
            self.state.lock().unwrap().made_base.push(base.clone());
        }
        let prefix = format!("run-{}-", timestamp());
        let d = loop {
            let d = base.join(format!("{prefix}{}", random_u64() % 1_000_000_000));
            match create_private_dir(&d) {
                Ok(()) => break d,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        };
        self.state.lock().unwrap().run_dirs.push(d.clone());
        Ok(d)
    }

    /// Deletes `path`, which must be one of our run dirs or inside one.
    pub(crate) fn remove(&self, path: &Path) -> Result<()> {
        let st = self.state.lock().unwrap();
        Self::remove_locked(&st, path)
    }

    fn remove_locked(st: &WorkspaceState, path: &Path) -> Result<()> {
        let path = clean(path);
        if st.run_dirs.iter().any(|rd| path.starts_with(rd)) {
            return match std::fs::remove_dir_all(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            };
        }
        Err(anyhow!("refusing to remove {}: not inside a bench run dir", path.display()))
    }

    pub(crate) fn cleanup(&self) -> Result<()> {
        let st = self.state.lock().unwrap();
        let mut errs = Vec::new();
        for rd in &st.run_dirs {
            if !rd.file_name().is_some_and(|n| n.to_string_lossy().starts_with("run-")) {
                continue;
            }
            if let Err(e) = Self::remove_locked(&st, rd) {
                errs.push(format!("{e:#}"));
            }
        }
        for b in &st.made_base {
            let _ = std::fs::remove_dir(b); // only succeeds if empty
        }
        if errs.is_empty() { Ok(()) } else { Err(anyhow!(errs.join("\n"))) }
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn create_private_dir(d: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(d)
}

// --- medians -------------------------------------------------------------------

pub(crate) fn median(ms: &[Metrics]) -> Metrics {
    match ms {
        [] => return Metrics::default(),
        [one] => return one.clone(),
        _ => {}
    }
    let f = |get: fn(&Metrics) -> f64| med(ms.iter().map(get).collect());
    Metrics {
        wall_sec: f(|m| m.wall_sec),
        window_sec: f(|m| m.window_sec),
        ops: f(|m| m.ops as f64) as usize,
        ops_per_sec: f(|m| m.ops_per_sec),
        busy_cpu_sec: f(|m| m.busy_cpu_sec),
        workload_cpu_sec: f(|m| m.workload_cpu_sec),
        background_cpu_sec: f(|m| m.background_cpu_sec),
        background_excess_sec: f(|m| m.background_excess_sec),
        overhead_cpu_sec: f(|m| m.overhead_cpu_sec),
        overhead_excess_sec: f(|m| m.overhead_excess_sec),
        overhead_pct: f(|m| m.overhead_pct),
        baseline_busy_cores: f(|m| m.baseline_busy_cores),
        settle_sec: f(|m| m.settle_sec),
        spawns: f(|m| m.spawns as f64) as u64,
        cpu_temp_start: f(|m| m.cpu_temp_start),
        cpu_temp_max: f(|m| m.cpu_temp_max),
        cpu_power_avg: f(|m| m.cpu_power_avg),
        soc_power_avg: f(|m| m.soc_power_avg),
        fan_max: f(|m| m.fan_max),
        categories: med_map(ms, |m| &m.categories),
        extra: med_map(ms, |m| &m.extra),
    }
}

fn med_map(ms: &[Metrics], get: fn(&Metrics) -> &BTreeMap<String, f64>) -> BTreeMap<String, f64> {
    let keys: std::collections::BTreeSet<&String> = ms.iter().flat_map(|m| get(m).keys()).collect();
    keys.into_iter()
        .map(|k| (k.clone(), med(ms.iter().map(|m| get(m).get(k).copied().unwrap_or(0.0)).collect())))
        .collect() // missing counts as zero
}

fn med(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}
