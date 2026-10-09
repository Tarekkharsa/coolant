use super::*;
use crate::run::{Workspace, background_categories, median};
use crate::scenarios::*;
use crate::util::which;
use coolant_core::classify::Category;
use coolant_core::profile::{Proc, Run, Usage};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

/// A temp directory removed on drop (the equivalent of Go's `t.TempDir`).
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "coolant-bench-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            walk(&e.path(), out);
        } else {
            out.push(e.path());
        }
    }
}

#[test]
fn write_tree_is_deterministic_and_node_modules_like() {
    let (d1, d2) = (TempDir::new(), TempDir::new());
    let n1 = write_tree(d1.path(), "node_modules", 300, 7).unwrap();
    let n2 = write_tree(d2.path(), "node_modules", 300, 7).unwrap();
    assert_eq!(n1, n2, "not deterministic");
    let mut files = Vec::new();
    walk(d1.path(), &mut files);
    let (mut nested, mut scoped) = (0, 0);
    for p in &files {
        let size = p.metadata().unwrap().len();
        assert!((512..=8192).contains(&size), "{}: size {size} out of range", p.display());
        let s = p.to_string_lossy();
        if s.matches("node_modules").count() > 1 {
            nested += 1;
        }
        if s.contains("@scope") {
            scoped += 1;
        }
    }
    assert_eq!(files.len(), 300);
    assert!(nested > 0 && scoped > 0, "want nested and scoped packages, got nested={nested} scoped={scoped}");
}

/// The tree layout matches the Go benchmark's: these are the first values
/// Go's `rand.New(rand.NewPCG(7, 0xc001))` yields for IntN(40), IntN(7681).
#[test]
fn rng_matches_go_pcg() {
    let mut r = Pcg::new(7, 0xc001);
    let want = [(4, 905), (1, 4590), (35, 1491), (15, 2735)];
    for (a, b) in want {
        assert_eq!((r.int_n(40), r.int_n(8192 - 512 + 1)), (a, b));
    }
}

#[test]
fn small_files_cleans_up() {
    let d = TempDir::new();
    let ops = small_files(&no_cancel(), d.path(), 200).unwrap();
    assert_eq!(ops, 800);
    assert!(!d.path().join("work").exists(), "tree not deleted");
}

#[test]
fn spawn_runs_binary() {
    let ops = spawn(&no_cancel(), TempDir::new().path(), Path::new("/usr/bin/true"), 5).unwrap();
    assert_eq!(ops, 5);
}

#[test]
fn spawn_honors_cancel() {
    let c = AtomicBool::new(true);
    let err = spawn(&c, TempDir::new().path(), Path::new("/usr/bin/true"), 5).unwrap_err();
    assert!(err.downcast_ref::<Canceled>().is_some());
}

#[test]
fn new_binaries_are_unique() {
    if which("codesign").is_none() {
        eprintln!("no codesign; skipping");
        return;
    }
    let cancel = no_cancel();
    let d = TempDir::new();
    let tmpl = match build_template(&cancel, &d.path().join("tmpl")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e:#}; skipping");
            return;
        }
    };
    let bins = make_new_binaries(&cancel, &tmpl, &d.path().join("bin"), 3).unwrap();
    let mut seen = HashSet::new();
    for b in &bins {
        let out = Command::new(b).arg("tag").output().unwrap_or_else(|e| panic!("{}: {e}", b.display())).stdout;
        let out = String::from_utf8_lossy(&out).into_owned();
        assert!(out.starts_with(TAG_MARKER) && seen.insert(out.clone()), "{}: tag {out:?} not unique", b.display());
    }
    let (ops, extra) = spawn_timed(&cancel, d.path(), &bins, 9).unwrap();
    assert_eq!(ops, 9);
    assert!(extra["first_exec_ms"] > 0.0 && extra["warm_exec_ms"] > 0.0, "extra = {extra:?}");
}

#[test]
fn git_scenario() {
    if which("git").is_none() {
        eprintln!("no git; skipping");
        return;
    }
    let d = TempDir::new();
    write_tree(&d.path().join("repo"), "src", 50, 2).unwrap();
    let ops = git(&no_cancel(), d.path(), 3, 1).unwrap();
    assert_eq!(ops, 3 + 3 + 2);
    assert!(!d.path().join("wt-0").exists(), "worktree not removed");
}

#[test]
fn copy_deps_copies_every_file() {
    for clone in [false, true] {
        let d = TempDir::new();
        let src = d.path().join("deps");
        write_tree(&src, "node_modules", 120, 3).unwrap();
        let dst = d.path().join("wt").join("node_modules");
        copy_deps(&no_cancel(), &src.join("node_modules"), &dst, clone).unwrap();
        assert_eq!(count_files(&dst).unwrap(), 120, "clone={clone}");
    }
}

#[test]
fn workspace_refuses_outside() {
    let tmp = TempDir::new();
    let base = tmp.path().join("base");
    let ws = Workspace::default();
    let rd = ws.new_run_dir(&base).unwrap();
    let outside = TempDir::new();
    assert!(ws.remove(outside.path()).is_err(), "removed a dir outside the run dir");
    let mut sibling = rd.clone().into_os_string();
    sibling.push("-sibling");
    assert!(ws.remove(Path::new(&sibling)).is_err(), "removed a sibling with a shared prefix");
    assert!(ws.remove(&rd.join("..").join("..")).is_err(), "removed a parent via ..");
    assert!(ws.remove(&rd.join("..").join(outside.path().file_name().unwrap())).is_err());
    std::fs::create_dir_all(rd.join("x").join("y")).unwrap();
    ws.cleanup().unwrap();
    assert!(!base.exists(), "created base dir left behind");
    assert!(outside.path().exists(), "outside dir touched");
}

fn m(wall: f64, cats: &[(&str, f64)]) -> Metrics {
    Metrics {
        wall_sec: wall,
        categories: cats.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        ..Default::default()
    }
}

#[test]
fn median_of_reps() {
    let got = median(&[m(3.0, &[("indexer", 1.0)]), m(1.0, &[]), m(2.0, &[("indexer", 3.0)])]);
    assert_eq!(got.wall_sec, 2.0);
    assert_eq!(got.categories["indexer"], 1.0);
}

#[test]
fn background_categories_subtracts_own_tree() {
    let me = std::process::id() as i32;
    let usage = |key: &str, cpu_s| Usage { key: key.into(), cpu_s, ..Default::default() };
    let proc =
        |pid, name: &str, cat, cpu_s| Proc { pid, name: name.into(), category: Some(cat), cpu_s, ..Default::default() };
    let run = Run {
        categories: vec![usage("other", 5.0), usage("vcs", 2.0), usage("indexer", 1.0)],
        processes: vec![
            proc(me, "bench", Category::Other, 1.0),
            proc(99999, "git", Category::Vcs, 2.0),
            proc(99998, "mds_stores", Category::Indexer, 1.0),
        ],
        ..Default::default()
    };
    // Tree used 4s: 2s visible git, 2s in our process / hidden children.
    let workload: HashSet<String> = ["bench", "git"].into_iter().map(String::from).collect();
    let got = background_categories(&run, 4.0, &workload);
    let g = |k: &str| got.get(k).copied().unwrap_or(0.0);
    assert!(g("other") == 3.0 && g("vcs") == 0.0 && g("indexer") == 1.0, "got {got:?}");
}

#[test]
fn markdown_report() {
    let mk = |sc: &str, loc: &str, k: usize, wall: f64, ovh: f64| {
        let med = Metrics {
            wall_sec: wall,
            ops: 100,
            ops_per_sec: 100.0 / wall,
            workload_cpu_sec: 1.0,
            overhead_cpu_sec: ovh,
            categories: BTreeMap::from([("indexer".to_string(), ovh)]),
            extra: BTreeMap::from([("first_exec_ms".to_string(), 3.0), ("warm_exec_ms".to_string(), 1.0)]),
            ..Default::default()
        };
        let top =
            |name: &str, category: &str, cpu_sec| ProcCpu { name: name.into(), category: category.into(), cpu_sec };
        BenchResult {
            scenario: sc.into(),
            location: loc.into(),
            agents: k,
            median: med.clone(),
            reps: vec![med],
            top_background: vec![top("mds_stores", "indexer", ovh), top("mystery", "other", 0.5)],
            ..Default::default()
        }
    };
    let mut rs = Vec::new();
    for k in [1, 2] {
        for loc in [INDEXED, NO_INDEX] {
            let ovh = if loc == INDEXED { 1.0 } else { 0.1 };
            rs.push(mk(SC_SMALL_FILES, loc, k, 2.0, ovh));
            rs.push(mk(SC_COPY_DEPS, loc, k, 2.0, ovh));
            rs.push(mk(SC_CLONEFILE_DEPS, loc, k, 0.5, ovh));
            rs.push(mk(SC_SPAWN, loc, k, 1.0, 0.0));
            rs.push(mk(SC_SPAWN_NEW_BINARY, loc, k, 1.0, 0.0));
        }
    }
    let md = markdown(&rs);
    for want in [
        "## smallfiles",
        "copy vs clonefile",
        "4.0×",
        "largest overhead category: indexer",
        "mystery",
        "What this shows",
        "first exec",
    ] {
        assert!(md.contains(want), "markdown missing {want:?}:\n{md}");
    }
    serde_json::to_string(&rs).unwrap();
    assert!(markdown(&[]).contains("No results."));
}

#[test]
fn config_presets_and_json() {
    let q = Config::quick();
    assert_eq!(q.agents, vec![1, 2]);
    assert_eq!(Config::full().reps, 3);
    assert!(Config::default().noindex_base.ends_with("coolant-bench.noindex"));
    let j = serde_json::to_value(&q).unwrap();
    assert_eq!(j["interval_ns"], 500_000_000u64);
    let back: Config = serde_json::from_value(j).unwrap();
    assert_eq!(back.interval, q.interval);
    let sparse: Config = serde_json::from_str(r#"{"reps": 0}"#).unwrap();
    assert_eq!(sparse.with_defaults().reps, 1);
}

/// Runs the whole pipeline at tiny sizes in temp dirs. It takes ~30s because
/// of settling and tails, so it only runs with COOLANT_BENCH_SMOKE=1.
#[test]
fn run_smoke() {
    if std::env::var_os("COOLANT_BENCH_SMOKE").is_none_or(|v| v.is_empty()) {
        eprintln!("set COOLANT_BENCH_SMOKE=1");
        return;
    }
    let tmp = TempDir::new();
    let mut cfg = Config::quick();
    cfg.indexed_base = tmp.path().join("bench").to_string_lossy().into_owned();
    cfg.noindex_base = tmp.path().join("bench.noindex").to_string_lossy().into_owned();
    cfg.agents = vec![1, 2];
    (cfg.smallfiles, cfg.spawns, cfg.new_binaries, cfg.git_files, cfg.git_status, cfg.git_worktrees, cfg.deps_files) =
        (100, 10, 2, 50, 2, 1, 100);
    (cfg.settle_window, cfg.settle_max, cfg.tail) =
        (Duration::from_secs(1), Duration::from_secs(2), Duration::from_millis(500));
    let mut log = Vec::new();
    let rs = run(&cfg, &no_cancel(), &mut log);
    eprintln!("{}", String::from_utf8_lossy(&log));
    let rs = rs.unwrap();
    assert_eq!(rs.len(), ALL_SCENARIOS.len() * 2 * 2);
    for r in &rs {
        assert!(
            r.error.is_empty() && r.median.ops != 0,
            "{}/{} k={}: ops={} err={}",
            r.scenario,
            r.location,
            r.agents,
            r.median.ops,
            r.error
        );
    }
    for b in [&cfg.indexed_base, &cfg.noindex_base] {
        assert!(!Path::new(b).exists(), "{b} left behind");
    }
    eprintln!("{}", markdown(&rs));
}
