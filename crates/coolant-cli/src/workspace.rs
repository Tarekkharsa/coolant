//! Commands of the workspace layer: doctor, tune, prewarm, sweep, protect.

use anyhow::{Result, bail};
use coolant_workspace::config::{Config, canonical, tilde};
use coolant_workspace::{doctor as doc, prewarm as pw, sweep as sw, tune as tn, worktrees};
use std::path::PathBuf;
use std::time::Duration;

pub fn doctor(sample: u64, json: bool) -> Result<()> {
    let cfg = Config::load()?;
    if !json {
        eprintln!("looking for worktrees and watching processes for {sample}s…\n");
    }
    let wts = worktrees(&cfg.roots());
    let checks = doc::run(&cfg, &wts, Duration::from_secs(sample.max(1)));
    if json {
        println!("{}", serde_json::to_string_pretty(&checks)?);
    } else {
        print!("{}", doc::text(&checks));
    }
    Ok(())
}

pub fn tune(paths: Vec<PathBuf>, dry_run: bool, force: bool, undo: bool) -> Result<()> {
    if paths.is_empty() {
        bail!(
            "name the folders to tune, e.g. coolant tune ~/code/app/.claude/worktrees (coolant doctor suggests some)"
        );
    }
    let cfg = Config::load()?;
    for p in &paths {
        let res = if undo { tn::undo(p) } else { tn::tune(&cfg, p, dry_run, force) };
        match res {
            Ok(tn::Outcome::AlreadyExcluded) => println!("{}: already excluded from Spotlight", tilde(p)),
            Ok(tn::Outcome::Tuned { to, created }) => {
                let verb = match (dry_run, created) {
                    (true, _) => "would move to",
                    (false, true) => "created at",
                    (false, false) => "moved to",
                };
                println!("{}: {verb} {} (symlinked back)", tilde(p), tilde(&to));
            }
            Ok(tn::Outcome::Undone { from }) => println!("{}: moved back from {}", tilde(p), tilde(&from)),
            Err(e) => eprintln!("{}: {e:#}", tilde(p)),
        }
    }
    Ok(())
}

pub fn prewarm(worktree: Option<PathBuf>, from: Option<PathBuf>, dry_run: bool, hook: Option<PathBuf>) -> Result<()> {
    if let Some(repo) = hook {
        let path = pw::install_hook(&repo)?;
        println!("installed {}: new worktrees are prewarmed automatically", tilde(&path));
        return Ok(());
    }
    let wt = worktree.unwrap_or(std::env::current_dir()?);
    let report = pw::prewarm(&wt, from.as_deref(), dry_run)?;
    if report.cloned.is_empty() {
        println!("nothing to clone from {} (artifacts already present or none built)", tilde(&report.source));
    }
    for c in &report.cloned {
        let verb = if dry_run { "would clone" } else { "cloned" };
        println!("{verb} {} from {} ({:.1}s)", c.rel.display(), tilde(&report.source), c.took.as_secs_f64());
    }
    Ok(())
}

pub fn sweep(days: f64, remove_merged: bool, yes: bool) -> Result<()> {
    let cfg = Config::load()?;
    let opts = sw::Options { idle_days: days, remove_merged, dry_run: !yes };
    let (plans, skipped) = sw::plan(&cfg, worktrees(&cfg.roots()), &opts);
    if yes {
        sw::apply(&plans)?;
    }
    print!("{}", sw::describe(&plans, &skipped, !yes));
    Ok(())
}

pub fn protect(paths: Vec<PathBuf>, remove: bool) -> Result<()> {
    let mut cfg = Config::load()?;
    for p in &paths {
        let p = tilde(&canonical(p));
        if remove {
            cfg.protect.retain(|q| *q != p);
        } else if !cfg.protect.contains(&p) {
            cfg.protect.push(p);
        }
    }
    if !paths.is_empty() {
        cfg.save()?;
    }
    if cfg.protect.is_empty() {
        println!("no protected paths ({})", tilde(&Config::path()));
    }
    for p in &cfg.protect {
        println!("protected: {p}");
    }
    Ok(())
}
