//! Markdown rendering of benchmark results.

use crate::config::*;
use crate::run::{BenchResult, MIN_WORKLOAD_CPU, Metrics, ProcCpu, sort_procs};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

type CaseKey<'a> = (&'a str, &'a str, usize);
type Index<'a> = HashMap<CaseKey<'a>, &'a BenchResult>;

/// Renders results as comparison tables (indexed vs noindex side by side, as
/// "indexed / noindex" cells) followed by a short auto-generated list of what
/// the numbers show.
pub fn markdown(results: &[BenchResult]) -> String {
    let mut b = String::from("# coolant benchmark\n\n");
    if results.is_empty() {
        b.push_str("No results.\n");
        return b;
    }
    let m = &results[0].machine;
    write!(
        b,
        "Machine: {}, {}, {} cores, {} {}. Median of {} repetition(s).\n\n",
        m.model,
        m.cpu,
        m.cores,
        m.os,
        m.os_version,
        results[0].reps.len()
    )
    .unwrap();
    b.push_str(
        "Cells are `indexed / noindex`. CPU is in CPU-seconds over the workload plus the settle tail; \
         categories exclude the benchmark's own processes. *Overhead* = indexer + security + fs-daemon + backup-sync; \
         *overhead %* is relative to the workload's own CPU. *bg excess* is CPU used by everything else on the machine \
         above its idle rate just before the case.\n\n",
    );

    let idx = index(results);
    for sc in scenario_order(results) {
        let desc = results.iter().find(|r| r.scenario == sc).map(|r| r.description.as_str()).unwrap_or("");
        write!(b, "## {sc}\n\n{desc}.\n\n").unwrap();
        b.push_str("| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |\n");
        b.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
        for k in agents_for(results, sc) {
            let (a, n) = (idx.get(&(sc, INDEXED, k)).copied(), idx.get(&(sc, NO_INDEX, k)).copied());
            let pair =
                |f: &dyn Fn(&Metrics) -> f64, prec: usize| format!("{} / {}", cell(a, f, prec), cell(n, f, prec));
            let cat = |c: &'static str| move |m: &Metrics| m.categories.get(c).copied().unwrap_or(0.0);
            writeln!(
                b,
                "| {k} | {} | {} | {} | {} | {} / {} | {} | {} | {} | {} | {} | {} | {} / {} | {} | {} |",
                pair(&|m| m.wall_sec, 2),
                pair(&|m| m.ops_per_sec, 0),
                pair(&|m| m.workload_cpu_sec, 2),
                pair(&|m| m.overhead_cpu_sec, 2),
                ovh_pct(a),
                ovh_pct(n),
                pair(&cat("indexer"), 2),
                pair(&cat("security"), 2),
                pair(&cat("fs-daemon"), 2),
                pair(&cat("other"), 2),
                pair(&|m| m.background_excess_sec, 2),
                pair(&|m| m.spawns as f64, 0),
                temp_pair(a),
                temp_pair(n),
                pair(&|m| m.cpu_power_avg, 1),
                pair(&|m| m.fan_max, 0),
            )
            .unwrap();
        }
        if sc == SC_SPAWN || sc == SC_SPAWN_NEW_BINARY {
            b.push_str("\n| K | first exec ms | warm exec ms |\n|---|---|---|\n");
            for k in agents_for(results, sc) {
                let (a, n) = (idx.get(&(sc, INDEXED, k)).copied(), idx.get(&(sc, NO_INDEX, k)).copied());
                let ex = |key: &str| {
                    let f = |m: &Metrics| m.extra.get(key).copied().unwrap_or(0.0);
                    format!("{} / {}", cell(a, f, 2), cell(n, f, 2))
                };
                writeln!(b, "| {k} | {} | {} |", ex("first_exec_ms"), ex("warm_exec_ms")).unwrap();
            }
        }
        b.push('\n');
    }

    if has_scenario(results, SC_COPY_DEPS) && has_scenario(results, SC_CLONEFILE_DEPS) {
        b.push_str("## copy vs clonefile\n\n| location | K | cp -R wall s | cp -c -R wall s | speedup | cp -R workload CPU | clonefile workload CPU | cp -R overhead | clonefile overhead |\n|---|---|---|---|---|---|---|---|---|\n");
        for loc in [INDEXED, NO_INDEX] {
            for k in agents_for(results, SC_COPY_DEPS) {
                let (Some(c), Some(cl)) = (idx.get(&(SC_COPY_DEPS, loc, k)), idx.get(&(SC_CLONEFILE_DEPS, loc, k)))
                else {
                    continue;
                };
                writeln!(
                    b,
                    "| {loc} | {k} | {:.2} | {:.2} | {} | {:.2} | {:.2} | {:.2} | {:.2} |",
                    c.median.wall_sec,
                    cl.median.wall_sec,
                    ratio(c.median.wall_sec, cl.median.wall_sec),
                    c.median.workload_cpu_sec,
                    cl.median.workload_cpu_sec,
                    c.median.overhead_cpu_sec,
                    cl.median.overhead_cpu_sec
                )
                .unwrap();
            }
        }
        b.push('\n');
    }

    let top = aggregate_top(results);
    if !top.is_empty() {
        b.push_str(
            "## Busiest background processes (all cases, summed)\n\n| process | category | CPU s |\n|---|---|---|\n",
        );
        for p in top.iter().take(15) {
            writeln!(b, "| {} | {} | {:.2} |", p.name, p.category, p.cpu_sec).unwrap();
        }
        b.push('\n');
    }

    b.push_str("## What this shows\n\n");
    for f in findings(results, &idx, &top) {
        b.push_str("- ");
        b.push_str(&f);
        b.push('\n');
    }
    b
}

fn index(rs: &[BenchResult]) -> Index<'_> {
    rs.iter()
        .filter(|r| !r.reps.is_empty())
        .map(|r| ((r.scenario.as_str(), r.location.as_str(), r.agents), r))
        .collect()
}

fn scenario_order(rs: &[BenchResult]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for r in rs {
        if !out.contains(&r.scenario.as_str()) {
            out.push(&r.scenario);
        }
    }
    out
}

fn agents_for(rs: &[BenchResult], sc: &str) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for r in rs {
        if r.scenario == sc && !out.contains(&r.agents) {
            out.push(r.agents);
        }
    }
    out.sort();
    out
}

fn has_scenario(rs: &[BenchResult], sc: &str) -> bool {
    rs.iter().any(|r| r.scenario == sc && !r.reps.is_empty())
}

fn cell(r: Option<&BenchResult>, f: impl Fn(&Metrics) -> f64, prec: usize) -> String {
    match r {
        None => "–".into(),
        Some(r) => format!("{:.*}", prec, f(&r.median)),
    }
}

fn temp_pair(r: Option<&BenchResult>) -> String {
    match r {
        Some(r) if r.median.cpu_temp_max != 0.0 => {
            format!("{:.0}→{:.0}", r.median.cpu_temp_start, r.median.cpu_temp_max)
        }
        _ => "n/a".into(),
    }
}

fn ratio(a: f64, b: f64) -> String {
    if b <= 0.0 { "–".into() } else { format!("{:.1}×", a / b) }
}

fn aggregate_top(rs: &[BenchResult]) -> Vec<ProcCpu> {
    let mut m: BTreeMap<&str, ProcCpu> = BTreeMap::new();
    for r in rs {
        for p in &r.top_background {
            m.entry(&p.name)
                .or_insert_with(|| ProcCpu { name: p.name.clone(), category: p.category.clone(), cpu_sec: 0.0 })
                .cpu_sec += p.cpu_sec;
        }
    }
    let mut out: Vec<ProcCpu> = m.into_values().collect();
    sort_procs(&mut out);
    out
}

/// Derives a few plain-language observations from the medians.
fn findings(rs: &[BenchResult], idx: &Index<'_>, top: &[ProcCpu]) -> Vec<String> {
    let mut out = Vec::new();
    for sc in scenario_order(rs) {
        let (mut wa, mut wn, mut oa, mut on, mut work_a, mut work_n) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        let mut pairs = 0;
        for k in agents_for(rs, sc) {
            let (Some(a), Some(n)) = (idx.get(&(sc, INDEXED, k)), idx.get(&(sc, NO_INDEX, k))) else { continue };
            pairs += 1;
            wa += a.median.wall_sec;
            wn += n.median.wall_sec;
            oa += a.median.overhead_cpu_sec;
            on += n.median.overhead_cpu_sec;
            work_a += a.median.workload_cpu_sec;
            work_n += n.median.workload_cpu_sec;
        }
        if pairs == 0 {
            continue;
        }
        let mut s = format!("**{sc}**: indexed location took {} wall time vs noindex", pct(wa, wn));
        s += &format!(
            "; OS overhead {oa:.2} vs {on:.2} CPU-s ({} vs {} of workload CPU)",
            pct_of(oa, work_a),
            pct_of(on, work_n)
        );
        let (c, v) = biggest_overhead_category(rs, sc);
        if v >= 0.05 {
            s += &format!("; largest overhead category: {c} ({v:.2} CPU-s summed over cases)");
        }
        out.push(s + ".");
    }
    for loc in [INDEXED, NO_INDEX] {
        let (mut cp, mut cl) = (0.0, 0.0);
        for k in agents_for(rs, SC_COPY_DEPS) {
            if let (Some(c), Some(l)) = (idx.get(&(SC_COPY_DEPS, loc, k)), idx.get(&(SC_CLONEFILE_DEPS, loc, k))) {
                cp += c.median.wall_sec;
                cl += l.median.wall_sec;
            }
        }
        if cp > 0.0 && cl > 0.0 {
            out.push(format!(
                "Cloning deps with clonefile (`cp -c -R`) was {} faster than a plain copy in the {loc} location.",
                ratio(cp, cl)
            ));
        }
    }
    for sc in scenario_order(rs) {
        let ks = agents_for(rs, sc);
        if ks.len() < 2 {
            continue;
        }
        let (k0, k1) = (ks[0], ks[ks.len() - 1]);
        let pick = |loc| (idx.get(&(sc, loc, k0)), idx.get(&(sc, loc, k1)));
        let (lo, hi) = match pick(NO_INDEX) {
            (Some(lo), Some(hi)) => (lo, hi),
            _ => match pick(INDEXED) {
                (Some(lo), Some(hi)) => (lo, hi),
                _ => continue,
            },
        };
        if lo.median.ops_per_sec == 0.0 {
            continue;
        }
        let eff = hi.median.ops_per_sec / lo.median.ops_per_sec / (k1 as f64 / k0 as f64);
        out.push(format!(
            "{sc}: throughput at K={k1} is {:.1}× K={k0} ({:.0}% parallel efficiency).",
            hi.median.ops_per_sec / lo.median.ops_per_sec,
            100.0 * eff
        ));
    }
    if let (Some(s), Some(n)) = (idx.get(&(SC_SPAWN, NO_INDEX, 1)), idx.get(&(SC_SPAWN_NEW_BINARY, NO_INDEX, 1))) {
        let g = |m: &Metrics, k: &str| m.extra.get(k).copied().unwrap_or(0.0);
        let c = |m: &Metrics, k: &str| m.categories.get(k).copied().unwrap_or(0.0);
        out.push(format!(
            "First exec of a never-seen binary took {:.2} ms vs {:.2} ms warm (system /usr/bin/true: {:.2} ms); security CPU {:.2} vs {:.2} CPU-s.",
            g(&n.median, "first_exec_ms"),
            g(&n.median, "warm_exec_ms"),
            g(&s.median, "warm_exec_ms"),
            c(&n.median, "security"),
            c(&s.median, "security")
        ));
    }
    let mut others = Vec::new();
    for p in top {
        if p.category == "other" && p.cpu_sec >= 0.1 && others.len() < 8 {
            others.push(format!("{} ({:.2}s)", p.name, p.cpu_sec));
        }
    }
    if !others.is_empty() {
        out.push(format!(
            "Background processes classified as \"other\" that used noticeable CPU (classification candidates): {}.",
            others.join(", ")
        ));
    }
    let temps = rs.iter().any(|r| r.median.cpu_temp_max > 0.0);
    let spawns = rs.iter().any(|r| r.median.spawns > 0);
    if !temps {
        out.push("No temperature/power/fan data: the sensor backend was unavailable for this run.".into());
    }
    if !spawns {
        out.push("Spawn counts are zero: the spawn-counter backend was unavailable for this run.".into());
    }
    for r in rs {
        if !r.error.is_empty() {
            out.push(format!("{}/{} K={} failed: {}", r.scenario, r.location, r.agents, r.error));
        }
    }
    out
}

const OVERHEAD_CATEGORIES: [&str; 4] = ["indexer", "security", "fs-daemon", "backup-sync"];

fn biggest_overhead_category(rs: &[BenchResult], sc: &str) -> (&'static str, f64) {
    let mut sum = [0.0; 4];
    for r in rs.iter().filter(|r| r.scenario == sc) {
        for (s, c) in sum.iter_mut().zip(OVERHEAD_CATEGORIES) {
            *s += r.median.categories.get(c).copied().unwrap_or(0.0);
        }
    }
    let (mut best, mut v) = ("", 0.0);
    for (s, c) in sum.into_iter().zip(OVERHEAD_CATEGORIES) {
        if s > v {
            (best, v) = (c, s);
        }
    }
    (best, v)
}

fn pct(a: f64, b: f64) -> String {
    if b <= 0.0 {
        return "n/a".into();
    }
    let d = 100.0 * (a - b) / b;
    if d.abs() < 0.5 {
        "the same".into()
    } else if d > 0.0 {
        format!("{d:.0}% more")
    } else {
        format!("{:.0}% less", -d)
    }
}

fn pct_of(a: f64, b: f64) -> String {
    if b < MIN_WORKLOAD_CPU { "n/a".into() } else { format!("{:.0}%", 100.0 * a / b) }
}

fn ovh_pct(r: Option<&BenchResult>) -> String {
    match r {
        None => "–".into(),
        Some(r) if r.median.workload_cpu_sec < MIN_WORKLOAD_CPU => "n/a".into(),
        Some(r) => format!("{:.1}", r.median.overhead_pct),
    }
}
