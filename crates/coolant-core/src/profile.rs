//! Samples a [`Backend`] over time and attributes CPU, disk I/O and process
//! spawns to categories and agent sessions.

use crate::classify::{self, Category, Labeled};
use coolant_platform::{Backend, CpuTimes, Machine, SensorReader, Sensors, SpawnCounter, ThermalPressure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Callback invoked after every sample.
pub type OnSample = Box<dyn FnMut(&Sample) + Send>;

pub struct Options {
    pub interval: Duration,
    /// Keep at most this many samples on the timeline (oldest dropped), for
    /// long-running sessions such as a UI. `None` keeps everything.
    pub max_samples: Option<usize>,
    /// Called after every sample, e.g. for live output.
    pub on_sample: Option<OnSample>,
}

impl Default for Options {
    fn default() -> Self {
        Options { interval: Duration::from_secs(1), max_samples: None, on_sample: None }
    }
}

/// The result of one profiling session; what gets saved as JSON and
/// rendered by [`crate::report`]. CPU figures are in seconds.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Run {
    pub machine: Machine,
    /// Unix milliseconds.
    pub start_ms: u64,
    pub duration_s: f64,
    pub interval_s: f64,
    /// Free-form, e.g. the benchmark case that produced the run.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Total CPU time used by the machine during the run.
    pub busy_cpu_s: f64,
    /// Busy CPU we couldn't tie to a process: kernel work, interrupts,
    /// other users' short-lived processes.
    pub unattributed_cpu_s: f64,
    pub categories: Vec<Usage>,
    pub sessions: Vec<Usage>,
    /// Top processes by CPU.
    pub processes: Vec<Proc>,
    /// Our own dev processes that outlived their parent.
    pub orphans: Vec<Proc>,
    /// Programs running many times at once, e.g. one tsserver per worktree.
    pub duplicates: Vec<Duplicate>,
    pub spawns: u64,
    pub thermal: Thermal,
    pub samples: Vec<Sample>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl Run {
    /// Average cores used: CPU seconds per wall-clock second.
    pub fn cores(&self, cpu_s: f64) -> f64 {
        if self.duration_s > 0.0 { cpu_s / self.duration_s } else { 0.0 }
    }

    pub fn category(&self, c: Category) -> Usage {
        self.categories.iter().find(|u| u.key == c.as_str()).cloned().unwrap_or_default()
    }
}

/// Resource use for a category or an agent session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub key: String,
    pub cpu_s: f64,
    pub disk_read: u64,
    pub disk_written: u64,
    pub spawns: u64,
    /// Sessions only: where the session's CPU went.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_category: BTreeMap<Category, f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Proc {
    pub pid: i32,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    pub user: String,
    pub category: Option<Category>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session: String,
    pub cpu_s: f64,
    pub rss_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Duplicate {
    pub name: String,
    pub count: u32,
    pub cpu_s: f64,
    pub rss_bytes: u64,
}

/// One point on the timeline.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Sample {
    /// Seconds since the run started.
    pub t: f64,
    /// Average cores busy since the previous sample.
    pub busy_cores: f64,
    /// Cores per category.
    pub categories: BTreeMap<Category, f64>,
    pub spawns: u64,
    pub sensors: Sensors,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Thermal {
    pub cpu_temp_start_c: f64,
    pub cpu_temp_avg_c: f64,
    pub cpu_temp_max_c: f64,
    pub cpu_power_avg_w: f64,
    pub cpu_power_max_w: f64,
    pub soc_power_avg_w: f64,
    pub fan_max_rpm: f64,
    /// Seconds spent at each non-nominal pressure level.
    pub pressure_s: BTreeMap<String, f64>,
}

/// A profiling session in progress. Dropping it without calling
/// [`Recorder::stop`] stops sampling and discards the run.
pub struct Recorder {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<Run>>,
}

impl Recorder {
    /// Takes a baseline and starts sampling on a background thread.
    pub fn start(backend: Arc<dyn Backend>, opts: Options) -> coolant_platform::Result<Recorder> {
        let mut state = Session::new(backend, opts)?;
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("coolant-profile".into())
            .spawn(move || {
                let mut next = Instant::now() + state.opts.interval;
                loop {
                    let wait = next.saturating_duration_since(Instant::now());
                    match rx.recv_timeout(wait) {
                        Err(RecvTimeoutError::Timeout) => {
                            state.sample();
                            next += state.opts.interval;
                        }
                        // Stop requested (or the Recorder was dropped):
                        // take one last sample to capture the tail end.
                        _ => {
                            state.sample();
                            return state.finish();
                        }
                    }
                }
            })
            .map_err(|e| coolant_platform::Error::Other(e.to_string()))?;
        Ok(Recorder { stop: Some(tx), thread: Some(thread) })
    }

    /// Ends the session and returns the finished run.
    pub fn stop(mut self) -> Run {
        self.stop.take();
        self.thread.take().unwrap().join().expect("profiler thread panicked")
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Identifies a process across snapshots. The start time guards against PID
/// reuse and, unlike the name, survives exec. It is 0 when unknown.
type Key = (i32, u64);

fn key(l: &Labeled) -> Key {
    (l.process.pid, l.process.start_ms)
}

/// A profiling session driven by the caller: call [`Session::sample`] on
/// your own schedule and [`Session::snapshot`] whenever you want results.
/// [`Recorder`] wraps one on a background thread.
pub struct Session {
    backend: Arc<dyn Backend>,
    opts: Options,
    sensors: Option<Box<dyn SensorReader>>,
    spawns: Option<Box<dyn SpawnCounter>>,
    run: Run,
    started: Instant,
    prev: HashMap<Key, Labeled>,
    prev_cpu: CpuTimes,
    prev_time: Instant,
    cats: HashMap<Category, Usage>,
    sessions: HashMap<String, Usage>,
    procs: HashMap<Key, Proc>,
    attributed: Duration,
}

impl Session {
    /// Takes a baseline; the first [`Session::sample`] measures from here.
    pub fn new(backend: Arc<dyn Backend>, opts: Options) -> coolant_platform::Result<Session> {
        let mut warnings = Vec::new();
        let mut sensors = backend.sensors().map_err(|e| warnings.push(format!("sensors unavailable: {e}"))).ok();
        let mut spawns =
            backend.spawn_counter().map_err(|e| warnings.push(format!("spawn counter unavailable: {e}"))).ok();
        let procs = backend.processes()?;
        let prev_cpu = backend.cpu()?;
        if let Some(s) = &mut sensors {
            let _ = s.read(); // prime power averaging
        }
        if let Some(s) = &mut spawns {
            s.drain();
        }
        let now = Instant::now();
        let run = Run {
            machine: backend.machine(),
            start_ms: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
            interval_s: opts.interval.as_secs_f64(),
            warnings,
            ..Default::default()
        };
        Ok(Session {
            backend,
            opts,
            sensors,
            spawns,
            run,
            started: now,
            prev: classify::label(procs).into_iter().map(|l| (key(&l), l)).collect(),
            prev_cpu,
            prev_time: now,
            cats: HashMap::new(),
            sessions: HashMap::new(),
            procs: HashMap::new(),
            attributed: Duration::ZERO,
        })
    }

    /// Takes one sample. Returns false if the platform could not be read.
    pub fn sample(&mut self) -> bool {
        let (Ok(procs), Ok(cpu)) = (self.backend.processes(), self.backend.cpu()) else { return false };
        let now = Instant::now();
        let prev = std::mem::take(&mut self.prev);
        let cur: HashMap<Key, Labeled> = classify::label(procs).into_iter().map(|l| (key(&l), l)).collect();
        let by_pid: HashMap<i32, &Labeled> = cur.values().map(|l| (l.process.pid, l)).collect();

        let elapsed = now.duration_since(self.prev_time).as_secs_f64().max(1e-3);
        let busy = cpu.busy.saturating_sub(self.prev_cpu.busy);
        self.run.busy_cpu_s += busy.as_secs_f64();
        let mut s = Sample {
            t: now.duration_since(self.started).as_secs_f64(),
            busy_cores: busy.as_secs_f64() / elapsed,
            ..Default::default()
        };

        for (k, l) in &cur {
            let p = &l.process;
            let old = prev.get(k);
            let d = match old {
                Some(old) if p.cpu_time >= old.process.cpu_time => p.cpu_time - old.process.cpu_time,
                _ => p.cpu_time, // new (or PID reused): everything it used is new
            };
            self.add(&mut s, elapsed, l, l.category, d);
            let entry = self.procs.entry(*k).or_insert_with(|| Proc {
                pid: p.pid,
                name: p.name.clone(),
                command: p.command.clone(),
                user: p.user.clone(),
                category: Some(l.category),
                session: l.session.clone(),
                ..Default::default()
            });
            entry.cpu_s += d.as_secs_f64();
            entry.rss_bytes = entry.rss_bytes.max(p.rss_bytes);
            if p.detailed {
                let (rd, wr) = match old {
                    Some(o) if o.process.detailed => (
                        p.disk_read.saturating_sub(o.process.disk_read),
                        p.disk_written.saturating_sub(o.process.disk_written),
                    ),
                    Some(_) => (0, 0),
                    None => (p.disk_read, p.disk_written),
                };
                for u in [Some(self.cats.entry(l.category).or_default()), session_usage(&mut self.sessions, &l.session)]
                    .into_iter()
                    .flatten()
                {
                    u.disk_read += rd;
                    u.disk_written += wr;
                }
            }
        }

        // Short-lived children. A parent's reaped-children CPU grows by each
        // child's whole lifetime; whatever we didn't already see a child use
        // was spent by processes that came and went between samples.
        let mut gone: HashMap<i32, Duration> = HashMap::new();
        for (k, l) in &prev {
            if !cur.contains_key(k) {
                *gone.entry(l.process.ppid).or_default() += l.process.cpu_time + l.process.child_cpu_time;
            }
        }
        let spawns = self.spawns.as_mut().map(|c| c.drain()).unwrap_or_default();
        s.spawns = spawns.len() as u64;
        self.run.spawns += s.spawns;
        let mut spawned: HashMap<i32, HashMap<Category, u32>> = HashMap::new();
        for sp in &spawns {
            let cat = if sp.name.is_empty() { Category::Other } else { classify::of(&sp.name, "") };
            self.cats.entry(cat).or_default().spawns += 1;
            if let Some(parent) = by_pid.get(&sp.ppid)
                && let Some(u) = session_usage(&mut self.sessions, &parent.session)
            {
                u.spawns += 1;
            }
            *spawned.entry(sp.ppid).or_default().entry(cat).or_default() += 1;
        }
        for (k, l) in &cur {
            let Some(old) = prev.get(k) else { continue };
            if !l.process.detailed || !old.process.detailed || l.process.child_cpu_time < old.process.child_cpu_time {
                continue;
            }
            let grown = l.process.child_cpu_time - old.process.child_cpu_time;
            let hidden = grown.saturating_sub(gone.get(&l.process.pid).copied().unwrap_or_default());
            if hidden.is_zero() {
                continue;
            }
            // Split by what the parent spawned; fall back to the parent's
            // own category when the children went unnamed.
            let named: Vec<(Category, u32)> = spawned
                .get(&l.process.pid)
                .map(|m| m.iter().filter(|(c, _)| **c != Category::Other).map(|(c, n)| (*c, *n)).collect())
                .unwrap_or_default();
            let total: u32 = named.iter().map(|(_, n)| n).sum();
            if total == 0 {
                self.add(&mut s, elapsed, l, l.category, hidden);
            } else {
                for (c, n) in named {
                    self.add(&mut s, elapsed, l, c, hidden * n / total);
                }
            }
        }

        if let Some(r) = &mut self.sensors
            && let Ok(sn) = r.read()
        {
            s.sensors = sn;
        }
        if let Some(f) = &mut self.opts.on_sample {
            f(&s);
        }
        self.run.samples.push(s);
        if let Some(max) = self.opts.max_samples {
            let excess = self.run.samples.len().saturating_sub(max);
            self.run.samples.drain(..excess);
        }
        self.prev = cur;
        self.prev_cpu = cpu;
        self.prev_time = now;
        true
    }

    fn add(&mut self, s: &mut Sample, elapsed: f64, l: &Labeled, cat: Category, d: Duration) {
        if d.is_zero() {
            return;
        }
        self.attributed += d;
        let secs = d.as_secs_f64();
        *s.categories.entry(cat).or_default() += secs / elapsed;
        self.cats.entry(cat).or_default().cpu_s += secs;
        if let Some(u) = session_usage(&mut self.sessions, &l.session) {
            u.cpu_s += secs;
            *u.by_category.entry(cat).or_default() += secs;
        }
    }

    /// Ends the session.
    pub fn finish(self) -> Run {
        self.snapshot()
    }

    /// Results so far, without stopping the session.
    pub fn snapshot(&self) -> Run {
        let mut run = self.run.clone();
        run.duration_s = self.prev_time.duration_since(self.started).as_secs_f64();
        run.unattributed_cpu_s = (run.busy_cpu_s - self.attributed.as_secs_f64()).max(0.0);

        run.categories = self.cats.iter().map(|(c, u)| Usage { key: c.as_str().into(), ..u.clone() }).collect();
        run.sessions = self.sessions.iter().map(|(k, u)| Usage { key: k.clone(), ..u.clone() }).collect();
        let by_cpu = |a: &Usage, b: &Usage| b.cpu_s.total_cmp(&a.cpu_s);
        run.categories.sort_by(by_cpu);
        run.sessions.sort_by(by_cpu);

        let mut all: Vec<Proc> = self.procs.values().cloned().collect();
        all.sort_by(|a, b| b.cpu_s.total_cmp(&a.cpu_s));
        all.truncate(40);
        run.processes = all;

        // Orphans: our own dev processes (and agents) reparented to launchd,
        // which usually means the agent, terminal or app that started them
        // has gone away.
        for (k, l) in &self.prev {
            let p = &l.process;
            let dev = matches!(
                l.category,
                Category::Runtime
                    | Category::Build
                    | Category::LanguageServer
                    | Category::PackageManager
                    | Category::Agent
            );
            let app = p.command.starts_with("/Applications/") || p.command.starts_with("/System/");
            if p.ppid == 1
                && p.detailed
                && dev
                && !app
                && let Some(pr) = self.procs.get(k)
            {
                run.orphans.push(pr.clone());
            }
        }
        run.orphans.sort_by(|a, b| b.cpu_s.total_cmp(&a.cpu_s));

        let mut dups: BTreeMap<String, Duplicate> = BTreeMap::new();
        for (k, l) in &self.prev {
            if l.category != Category::LanguageServer {
                continue;
            }
            let name =
                if l.process.command.contains("tsserver") { "tsserver".to_string() } else { l.process.name.clone() };
            let d = dups.entry(name.clone()).or_insert_with(|| Duplicate { name, ..Default::default() });
            d.count += 1;
            d.rss_bytes += l.process.rss_bytes;
            d.cpu_s += self.procs.get(k).map_or(0.0, |p| p.cpu_s);
        }
        run.duplicates = dups.into_values().filter(|d| d.count > 1).collect();
        run.duplicates.sort_by_key(|d| std::cmp::Reverse(d.count));

        run.thermal = summarize(&run.samples);
        run
    }
}

fn session_usage<'a>(m: &'a mut HashMap<String, Usage>, session: &str) -> Option<&'a mut Usage> {
    if session.is_empty() {
        return None;
    }
    Some(m.entry(session.to_string()).or_default())
}

fn summarize(samples: &[Sample]) -> Thermal {
    let mut t = Thermal::default();
    let (mut n, mut np) = (0.0, 0.0);
    let mut prev_t = 0.0;
    for s in samples {
        let sn = &s.sensors;
        if sn.cpu_temp_c > 0.0 {
            if n == 0.0 {
                t.cpu_temp_start_c = sn.cpu_temp_c;
            }
            n += 1.0;
            t.cpu_temp_avg_c += sn.cpu_temp_c;
            t.cpu_temp_max_c = t.cpu_temp_max_c.max(sn.cpu_temp_max_c).max(sn.cpu_temp_c);
        }
        if sn.cpu_power_w > 0.0 || sn.soc_power_w > 0.0 {
            np += 1.0;
            t.cpu_power_avg_w += sn.cpu_power_w;
            t.soc_power_avg_w += sn.soc_power_w;
            t.cpu_power_max_w = t.cpu_power_max_w.max(sn.cpu_power_w);
        }
        for &f in &sn.fan_rpm {
            t.fan_max_rpm = t.fan_max_rpm.max(f);
        }
        if !matches!(sn.pressure, ThermalPressure::Unknown | ThermalPressure::Nominal) {
            *t.pressure_s.entry(sn.pressure.to_string()).or_default() += s.t - prev_t;
        }
        prev_t = s.t;
    }
    if n > 0.0 {
        t.cpu_temp_avg_c /= n;
    }
    if np > 0.0 {
        t.cpu_power_avg_w /= np;
        t.soc_power_avg_w /= np;
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use coolant_platform::{Error, Process, Result};
    use std::sync::Mutex;

    /// Replays scripted snapshots; each call to cpu() advances one step.
    struct Fake {
        steps: Vec<Vec<Process>>,
        cpu: Vec<u64>,
        i: Mutex<usize>,
    }

    impl Backend for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn machine(&self) -> Machine {
            Machine::default()
        }
        fn processes(&self) -> Result<Vec<Process>> {
            let i = *self.i.lock().unwrap();
            Ok(self.steps[i.min(self.steps.len() - 1)].clone())
        }
        fn cpu(&self) -> Result<CpuTimes> {
            let mut i = self.i.lock().unwrap();
            let ms = self.cpu[(*i).min(self.cpu.len() - 1)];
            *i += 1;
            Ok(CpuTimes { busy: Duration::from_millis(ms), idle: Duration::ZERO })
        }
        fn sensors(&self) -> Result<Box<dyn SensorReader>> {
            Err(Error::Unsupported)
        }
        fn spawn_counter(&self) -> Result<Box<dyn SpawnCounter>> {
            Err(Error::Unsupported)
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn attribution_includes_unseen_children() {
        let agent = |cpu, child| Process {
            pid: 100,
            ppid: 1,
            name: "claude".into(),
            command: "claude".into(),
            detailed: true,
            cpu_time: ms(cpu),
            child_cpu_time: ms(child),
            start_ms: 5,
            ..Default::default()
        };
        let git = Process {
            pid: 200,
            ppid: 100,
            name: "git".into(),
            command: "git status".into(),
            detailed: true,
            cpu_time: ms(30),
            start_ms: 7,
            ..Default::default()
        };
        let mds =
            |cpu| Process { pid: 300, ppid: 1, name: "mds_stores".into(), cpu_time: ms(cpu), ..Default::default() };
        let fake = Arc::new(Fake {
            steps: vec![
                vec![agent(1000, 0), mds(5000)],              // baseline
                vec![agent(1100, 0), git.clone(), mds(5200)], // agent +100, git new 30, mds +200
                vec![agent(1150, 500), mds(5300)],            // git reaped; 470 ms of children we never saw
            ],
            cpu: vec![0, 400, 1100],
            i: Mutex::new(0),
        });
        let rec = Recorder::start(fake.clone(), Options { interval: ms(20), ..Default::default() }).unwrap();
        while *fake.i.lock().unwrap() < 3 {
            std::thread::sleep(ms(5));
        }
        let run = rec.stop();
        let cpu = |c: Category| (run.category(c).cpu_s * 1000.0).round();
        assert_eq!(cpu(Category::Agent), 620.0); // 100 + 50 + (500 - 30)
        assert_eq!(cpu(Category::Vcs), 30.0);
        assert_eq!(cpu(Category::Indexer), 300.0);
        assert_eq!(run.sessions.len(), 1);
        assert_eq!(run.sessions[0].key, "claude:100");
        assert_eq!((run.sessions[0].cpu_s * 1000.0).round(), 650.0);
    }
}
