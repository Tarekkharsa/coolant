//! Spotlight probe: checks that the ".noindex" trick works on this machine.

use crate::config::Config;
use crate::run::Workspace;
use crate::util::*;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// Whether Spotlight indexed a file written to a location.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpotlightProbe {
    pub location: String,
    pub base: String,
    /// `mdutil -s <volume>` output.
    pub volume_status: String,
    /// mdfind found the probe file.
    pub indexed: bool,
    /// Time until it was found.
    #[serde(rename = "latency_s")]
    pub latency_sec: f64,
    #[serde(rename = "waited_s")]
    pub waited_sec: f64,
}

/// Writes a file with a unique name and content into a probe directory under
/// each location's base, then polls mdfind until the file shows up or
/// `timeout` passes (it stops waiting for the others a few seconds after the
/// first one is found). Probe directories are removed afterwards. A zero
/// timeout means 15 seconds.
pub fn probe_spotlight(cfg: &Config, cancel: &AtomicBool, timeout: Duration) -> Result<Vec<SpotlightProbe>> {
    let cfg = cfg.clone().with_defaults();
    let timeout = if timeout.is_zero() { Duration::from_secs(15) } else { timeout };
    let ws = Workspace::default();
    let token = format!("coolantprobe{}", hex(&random_bytes::<8>()));

    let mut probes = Vec::new();
    for loc in &cfg.locations {
        let base = cfg.base(loc);
        let d = ws.new_run_dir(Path::new(base))?;
        std::fs::write(d.join(format!("{token}.txt")), format!("spotlight probe {token}\n"))?;
        probes.push(SpotlightProbe {
            location: loc.clone(),
            base: base.to_string(),
            volume_status: volume_status(cancel, base),
            ..Default::default()
        });
    }
    let start = Instant::now();
    let mut first_found: Option<Instant> = None;
    loop {
        let mut pending = false;
        for p in &mut probes {
            if p.indexed {
                continue;
            }
            if mdfind_has(cancel, &p.base, &token) {
                p.indexed = true;
                p.latency_sec = start.elapsed().as_secs_f64();
                first_found.get_or_insert_with(Instant::now);
            } else {
                pending = true;
            }
        }
        if !pending || start.elapsed() > timeout || first_found.is_some_and(|f| f.elapsed() > Duration::from_secs(5)) {
            break;
        }
        sleep(cancel, Duration::from_millis(500));
        check(cancel)?;
    }
    for p in &mut probes {
        p.waited_sec = start.elapsed().as_secs_f64();
    }
    Ok(probes)
}

fn mdfind_has(cancel: &AtomicBool, dir: &str, token: &str) -> bool {
    // stderr is locale noise
    let out = output(cancel, Command::new("/usr/bin/mdfind").args(["-onlyin", dir, token]));
    out.is_ok_and(|o| o.status.success() && find_bytes(&o.stdout, token.as_bytes()).is_some())
}

/// Runs mdutil -s on the volume holding `path` (mdutil only understands
/// volume roots).
fn volume_status(cancel: &AtomicBool, path: &str) -> String {
    let mut vol = "/".to_string();
    if let Ok(out) = output(cancel, Command::new("/bin/df").arg(path))
        && out.status.success()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(f) = text.trim().lines().last().and_then(|l| l.split_whitespace().last()) {
            vol = f.to_string();
        }
    }
    match output(cancel, Command::new("/usr/bin/mdutil").args(["-s", &vol])) {
        Ok(out) => {
            // CombinedOutput: stdout and stderr together.
            let mut all = out.stdout;
            all.extend_from_slice(&out.stderr);
            let mut s = String::from_utf8_lossy(&all).split_whitespace().collect::<Vec<_>>().join(" ");
            if !out.status.success() {
                write!(s, " ({})", out.status).unwrap();
            }
            s
        }
        Err(e) => format!(" ({e})"),
    }
}

/// Renders probe results as a short Markdown section.
pub fn probe_markdown(ps: &[SpotlightProbe]) -> String {
    let mut b =
        String::from("## Spotlight check\n\n| location | base | volume | indexed by Spotlight? |\n|---|---|---|---|\n");
    for p in ps {
        let got = if p.indexed {
            format!("yes, after {:.1}s", p.latency_sec)
        } else {
            format!("no (waited {:.0}s)", p.waited_sec)
        };
        writeln!(b, "| {} | `{}` | {} | {} |", p.location, p.base, p.volume_status, got).unwrap();
    }
    b
}
