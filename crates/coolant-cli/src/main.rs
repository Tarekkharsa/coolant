//! coolant: explains (and eventually reduces) the heat and CPU load of
//! running many AI coding agents on one machine.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use coolant_core::classify::Category;
use coolant_core::profile::{OnSample, Options, Recorder, Run, Sample};
use coolant_core::report;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "coolant", version, about = "Why is my machine hot while agents run?")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Record system load and explain where it went
    Profile {
        /// Seconds to record (0 = until ctrl-c)
        #[arg(short, long, default_value_t = 60)]
        duration: u64,
        /// Sampling interval in milliseconds
        #[arg(short, long, default_value_t = 1000)]
        interval: u64,
        /// Save the raw run as JSON
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Don't print live samples
        #[arg(short, long)]
        quiet: bool,
    },
    /// Re-render a saved profile
    Report { file: PathBuf },
    /// Run agent-like workloads under the profiler and compare locations
    #[cfg(unix)]
    Bench {
        /// quick (~5 min), default, or full (3 repetitions)
        #[arg(short, long, default_value = "quick")]
        preset: String,
        /// Comma-separated scenarios (default: all)
        #[arg(short, long, value_delimiter = ',')]
        scenarios: Vec<String>,
        /// Comma-separated parallel agent counts, e.g. 1,2,4
        #[arg(short, long, value_delimiter = ',')]
        agents: Vec<usize>,
        /// Directory for results (markdown + JSON)
        #[arg(short, long, default_value = "runs")]
        out: PathBuf,
    },
    /// Print temperature, power and fan readings
    Sensors {
        #[arg(short = 'n', long, default_value_t = 5)]
        count: u32,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Profile { duration, interval, out, quiet } => profile(duration, interval, out, quiet),
        Cmd::Report { file } => {
            let run: Run = serde_json::from_slice(&std::fs::read(&file)?)
                .with_context(|| format!("reading {}", file.display()))?;
            print!("{}", report::text(&run));
            Ok(())
        }
        #[cfg(unix)]
        Cmd::Bench { preset, scenarios, agents, out } => bench(&preset, scenarios, agents, &out),
        Cmd::Sensors { count } => sensors(count),
    }
}

fn profile(duration: u64, interval: u64, out: Option<PathBuf>, quiet: bool) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = tx.send(());
    })?;
    let opts = Options {
        interval: Duration::from_millis(interval.max(100)),
        on_sample: (!quiet).then(|| Box::new(live) as OnSample),
    };
    let what =
        if duration == 0 { "until ctrl-c".to_string() } else { format!("for {duration}s (ctrl-c to stop early)") };
    eprintln!("recording {what}…");
    let rec = Recorder::start(Arc::from(coolant_platform::current()), opts)?;
    let _ = if duration == 0 {
        rx.recv().map_err(|_| ())
    } else {
        rx.recv_timeout(Duration::from_secs(duration)).map_err(|_| ())
    };
    let run = rec.stop();
    eprintln!();
    if let Some(path) = out {
        std::fs::write(&path, serde_json::to_vec_pretty(&run)?)?;
        eprintln!("saved {}\n", path.display());
    }
    print!("{}", report::text(&run));
    Ok(())
}

/// One compact line per sample: load, the biggest categories, temperature
/// and power.
fn live(s: &Sample) {
    let mut line = format!("{:>6.0}s  busy {:4.1} cores", s.t, s.busy_cores);
    let mut cats: Vec<(Category, f64)> =
        s.categories.iter().map(|(c, v)| (*c, *v)).filter(|(_, v)| *v >= 0.1).collect();
    cats.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (c, v) in cats.iter().take(4) {
        let _ = write!(line, "  {c} {v:.1}");
    }
    if s.spawns > 0 {
        let _ = write!(line, "  spawns {}", s.spawns);
    }
    if s.sensors.cpu_temp_c > 0.0 {
        let _ = write!(line, "  {:.0}°C", s.sensors.cpu_temp_c);
    }
    if s.sensors.cpu_power_w > 0.0 {
        let _ = write!(line, "  {:.1}W", s.sensors.cpu_power_w);
    }
    eprintln!("{line}");
}

#[cfg(unix)]
fn bench(preset: &str, scenarios: Vec<String>, agents: Vec<usize>, out: &std::path::Path) -> Result<()> {
    let mut cfg = match preset {
        "quick" => coolant_bench::Config::quick(),
        "default" => coolant_bench::Config::default(),
        "full" => coolant_bench::Config::full(),
        other => anyhow::bail!("unknown preset {other:?} (quick, default, full)"),
    };
    if !scenarios.is_empty() {
        cfg.scenarios = scenarios;
    }
    if !agents.is_empty() {
        cfg.agents = agents;
    }
    use std::sync::atomic::{AtomicBool, Ordering};
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst))?;

    let (results, err) = coolant_bench::run_partial(&cfg, &cancel, &mut std::io::stderr());
    if !results.is_empty() {
        std::fs::create_dir_all(out)?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
        let md = coolant_bench::markdown(&results);
        let base = out.join(format!("bench-{preset}-{stamp}"));
        std::fs::write(base.with_extension("md"), &md)?;
        std::fs::write(base.with_extension("json"), serde_json::to_vec_pretty(&results)?)?;
        println!("{md}");
        eprintln!("saved {}.{{md,json}}", base.display());
    }
    err.map_or(Ok(()), Err)
}

fn sensors(count: u32) -> Result<()> {
    let mut r = coolant_platform::current().sensors()?;
    let _ = r.read(); // power is averaged since the previous read
    for _ in 0..count {
        std::thread::sleep(Duration::from_secs(1));
        let s = r.read()?;
        println!(
            "cpu {:.1}°C (max {:.1})  gpu {:.1}°C  cpu {:.2}W  gpu {:.2}W  soc {:.2}W  fans {:?}  pressure {}",
            s.cpu_temp_c,
            s.cpu_temp_max_c,
            s.gpu_temp_c,
            s.cpu_power_w,
            s.gpu_power_w,
            s.soc_power_w,
            s.fan_rpm,
            s.pressure
        );
    }
    Ok(())
}
