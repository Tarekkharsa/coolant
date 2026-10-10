//! The workloads: tree generation, many small files, process spawns, git
//! and dependency copies.

use crate::config::*;
use crate::util::*;
use anyhow::{Context, Result, anyhow, bail};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

type SetupFn = Box<dyn Fn(&AtomicBool, &Path) -> Result<()> + Send + Sync>;
type RunFn = Box<dyn Fn(&AtomicBool, &Path) -> Result<(usize, BTreeMap<String, f64>)> + Send + Sync>;

/// A scenario has an untimed setup phase and a timed run phase. Both get a
/// private per-agent directory. Run returns the number of operations done and
/// optional extra metrics (e.g. exec latency).
pub(crate) struct Scenario {
    pub name: &'static str,
    pub desc: String,
    pub setup: Option<SetupFn>,
    pub run: RunFn,
}

/// Builds the scenario table for a config. `tmpl` is the path of the
/// template binary for spawn-new-binary (empty if not needed).
pub(crate) fn scenarios(c: &Config, tmpl: &Path) -> HashMap<&'static str, Scenario> {
    let mut t = HashMap::new();

    let n = c.smallfiles;
    t.insert(SC_SMALL_FILES, Scenario {
        name: SC_SMALL_FILES,
        desc: format!("create a node_modules-like tree of {n} files (0.5–8 KB), stat-walk it, read every file, delete it; ops = file operations"),
        setup: None,
        run: Box::new(move |cancel, dir| Ok((small_files(cancel, dir, n)?, BTreeMap::new()))),
    });

    let n = c.spawns;
    t.insert(
        SC_SPAWN,
        Scenario {
            name: SC_SPAWN,
            desc: format!("exec /usr/bin/true {n} times sequentially; ops = execs"),
            setup: None,
            run: Box::new(move |cancel, dir| spawn_timed(cancel, dir, &[PathBuf::from("/usr/bin/true")], n)),
        },
    );

    let (nb, n) = (c.new_binaries, c.spawns);
    let tmpl = tmpl.to_path_buf();
    t.insert(
        SC_SPAWN_NEW_BINARY,
        Scenario {
            name: SC_SPAWN_NEW_BINARY,
            desc: format!(
                "exec {nb} freshly built, ad-hoc signed, never-seen binaries {n} times in total; ops = execs"
            ),
            setup: Some(Box::new(move |cancel, dir| {
                make_new_binaries(cancel, &tmpl, &dir.join("bin"), nb).map(|_| ())
            })),
            run: Box::new(move |cancel, dir| {
                let bins = list_binaries(&dir.join("bin"))?;
                spawn_timed(cancel, dir, &bins, n)
            }),
        },
    );

    let (files, status, wts) = (c.git_files, c.git_status, c.git_worktrees);
    t.insert(SC_GIT, Scenario {
        name: SC_GIT,
        desc: format!("git init a {files}-file repo, add+commit, {status}× git status (editing a file every 10), add and remove {wts} worktrees; ops = git commands"),
        setup: Some(Box::new(move |_, dir| write_tree(&dir.join("repo"), "src", files, 2).map(|_| ()))),
        run: Box::new(move |cancel, dir| Ok((git(cancel, dir, status, wts)?, BTreeMap::new()))),
    });

    t.insert(SC_COPY_DEPS, copy_scenario(c, false));
    t.insert(SC_CLONEFILE_DEPS, copy_scenario(c, true));
    t
}

fn copy_scenario(c: &Config, clone: bool) -> Scenario {
    let (name, how) = if clone { (SC_CLONEFILE_DEPS, CLONE_HOW) } else { (SC_COPY_DEPS, "cp -R") };
    let files = c.deps_files;
    Scenario {
        name,
        desc: format!("copy a {files}-file deps tree into a new worktree dir with {how}; ops = files"),
        setup: Some(Box::new(move |_, dir| write_tree(&dir.join("deps"), "node_modules", files, 3).map(|_| ()))),
        run: Box::new(move |cancel, dir| {
            copy_deps(
                cancel,
                &dir.join("deps").join("node_modules"),
                &dir.join("worktree").join("node_modules"),
                clone,
            )?;
            Ok((files, BTreeMap::new()))
        }),
    }
}

// --- tree generation ---------------------------------------------------------

static JS_CHUNK: LazyLock<Vec<u8>> = LazyLock::new(|| {
    let mut b = String::new();
    let mut i = 0;
    while b.len() < 9000 {
        write!(b, "export function helper{i}(value, options = {{}}) {{\n  if (!value) return null;\n  const out = Object.assign({{}}, options, {{ id: {i} }});\n  return typeof value === 'string' ? value.trim() + out.id : out;\n}}\n\n").unwrap();
        i += 1;
    }
    b.into_bytes()
});

const TREE_FILES: [&str; 17] = [
    "package.json",
    "README.md",
    "LICENSE",
    "index.js",
    "lib/index.js",
    "lib/utils.js",
    "lib/parse.js",
    "lib/format.js",
    "dist/esm/index.mjs",
    "dist/esm/utils.mjs",
    "dist/cjs/index.cjs",
    "dist/cjs/utils.cjs",
    "src/internal/core.ts",
    "src/internal/types.ts",
    "src/index.ts",
    "types/index.d.ts",
    "types/utils.d.ts",
];

/// A bit-exact port of Go's `math/rand/v2` PCG-DXSM generator and the parts
/// of `rand.Rand` we use, so trees match the Go benchmark's for a given seed.
pub(crate) struct Pcg {
    state: u128,
}

impl Pcg {
    pub(crate) fn new(seed1: u64, seed2: u64) -> Pcg {
        Pcg { state: (seed1 as u128) << 64 | seed2 as u128 }
    }

    fn next_u64(&mut self) -> u64 {
        const MUL: u128 = (2549297995355413924u128 << 64) | 4865540595714422341u128;
        const INC: u128 = (6364136223846793005u128 << 64) | 1442695040888963407u128;
        self.state = self.state.wrapping_mul(MUL).wrapping_add(INC);
        let (mut hi, lo) = ((self.state >> 64) as u64, self.state as u64);
        hi ^= hi >> 32;
        hi = hi.wrapping_mul(0xda942042e4dd58b5);
        hi ^= hi >> 48;
        hi.wrapping_mul(lo | 1)
    }

    /// Uniform in [0, n).
    pub(crate) fn int_n(&mut self, n: usize) -> usize {
        let n = n as u64;
        if n & (n - 1) == 0 {
            return (self.next_u64() & (n - 1)) as usize;
        }
        let m = |x: u64| (x as u128) * (n as u128);
        let mut p = m(self.next_u64());
        if (p as u64) < n {
            let thresh = n.wrapping_neg() % n;
            while (p as u64) < thresh {
                p = m(self.next_u64());
            }
        }
        (p >> 64) as usize
    }
}

/// Go's `filepath.Ext` for a '/'-separated name.
fn ext(name: &str) -> &str {
    for (i, ch) in name.char_indices().rev() {
        if ch == '/' {
            break;
        }
        if ch == '.' {
            return &name[i..];
        }
    }
    ""
}

/// Creates `files` files of 0.5–8 KB under dir/root, laid out like a
/// node_modules folder: plain and scoped packages, some with nested
/// node_modules, each with lib/, dist/, src/ and types/ subfolders. Contents
/// are JS-like text with a unique header so no two files are identical. The
/// layout is deterministic for a given seed. It returns the bytes written.
pub fn write_tree(dir: &Path, root: &str, files: usize, seed: u64) -> Result<u64> {
    let mut rng = Pcg::new(seed, 0xc001);
    let mut made: HashSet<PathBuf> = HashSet::new();
    let mut total = 0u64;
    let mut created = 0usize;
    let mut p = 0usize;
    while created < files {
        let pkg = if p.is_multiple_of(6) {
            Path::new(root).join(format!("@scope{}", p % 5)).join(format!("pkg{p}"))
        } else if p.is_multiple_of(9) {
            Path::new(root).join(format!("pkg{}", p - 1)).join("node_modules").join(format!("dep{p}"))
        } else {
            Path::new(root).join(format!("pkg{p}"))
        };
        p += 1;
        let n = 8 + rng.int_n(40);
        let mut i = 0;
        while i < n && created < files {
            let mut name = TREE_FILES[i % TREE_FILES.len()].to_string();
            if i >= TREE_FILES.len() {
                let e = ext(&name).to_string();
                name = format!("{}{}{}", &name[..name.len() - e.len()], i / TREE_FILES.len(), e);
            }
            let rel = pkg.join(&name);
            let path = dir.join(&rel);
            let parent = path.parent().unwrap().to_path_buf();
            if !made.contains(&parent) {
                fs::create_dir_all(&parent)?;
                made.insert(parent);
            }
            let size = 512 + rng.int_n(8192 - 512 + 1);
            let header = format!("// {} seed={seed} n={created}\n", rel.display());
            let mut body = Vec::with_capacity(size.max(header.len()));
            body.extend_from_slice(header.as_bytes());
            if size > body.len() {
                body.extend_from_slice(&JS_CHUNK[..size - body.len()]);
            }
            fs::write(&path, &body)?;
            total += body.len() as u64;
            created += 1;
            i += 1;
        }
    }
    Ok(total)
}

// --- smallfiles --------------------------------------------------------------

fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for e in fs::read_dir(dir)? {
        let e = e?;
        let md = e.metadata()?; // lstat
        if md.is_dir() {
            walk_files(&e.path(), out)?;
        } else if md.is_file() {
            out.push(e.path());
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn count_files(dir: &Path) -> Result<usize> {
    let mut v = Vec::new();
    walk_files(dir, &mut v)?;
    Ok(v.len())
}

/// Creates an n-file node_modules-like tree in dir, stat-walks it, reads
/// every file and deletes the tree. It returns the number of file operations
/// (4 per file).
pub fn small_files(cancel: &AtomicBool, dir: &Path, n: usize) -> Result<usize> {
    let tree = dir.join("work");
    write_tree(&tree, "node_modules", n, 1)?;
    check(cancel)?;
    let mut paths = Vec::new();
    walk_files(&tree, &mut paths)?;
    if paths.len() != n {
        bail!("smallfiles: walked {} files, want {n}", paths.len());
    }
    for (i, p) in paths.iter().enumerate() {
        if i % 256 == 0 {
            check(cancel)?;
        }
        fs::read(p)?;
    }
    fs::remove_dir_all(&tree)?;
    Ok(4 * n)
}

// --- spawn -------------------------------------------------------------------

/// Execs `bin` n times sequentially with `dir` as working directory.
pub fn spawn(cancel: &AtomicBool, dir: &Path, bin: &Path, n: usize) -> Result<usize> {
    Ok(spawn_timed(cancel, dir, &[bin.to_path_buf()], n)?.0)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Runs n execs spread over bins (each binary gets a contiguous run, so its
/// first exec is its first-ever exec). Extra metrics: mean latency of the
/// first exec of each binary and of the rest, in ms.
pub(crate) fn spawn_timed(
    cancel: &AtomicBool,
    dir: &Path,
    bins: &[PathBuf],
    n: usize,
) -> Result<(usize, BTreeMap<String, f64>)> {
    if bins.is_empty() {
        bail!("spawn: no binaries");
    }
    let (mut first, mut warm) = (Duration::ZERO, Duration::ZERO);
    let (mut n_first, mut n_warm) = (0u32, 0u32);
    for i in 0..n {
        let b = i * bins.len() / n;
        let is_first = i == 0 || (i - 1) * bins.len() / n != b;
        check(cancel)?;
        let t = Instant::now();
        let st = Command::new(&bins[b])
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let d = t.elapsed();
        match st {
            Ok(s) if s.success() => {}
            Ok(s) => {
                check(cancel)?;
                return Err(anyhow!("exec {}: {s}", bins[b].display()));
            }
            Err(e) => return Err(anyhow!("exec {}: {e}", bins[b].display())),
        }
        if is_first {
            first += d;
            n_first += 1;
        } else {
            warm += d;
            n_warm += 1;
        }
    }
    let mut extra = BTreeMap::new();
    if n_first > 0 {
        extra.insert("first_exec_ms".to_string(), ms(first) / n_first as f64);
    }
    if n_warm > 0 {
        extra.insert("warm_exec_ms".to_string(), ms(warm) / n_warm as f64);
    }
    Ok((n, extra))
}

pub(crate) const TAG_MARKER: &str = "COOLANT-TAG-";

const HELLO_C: &str = "#include <stdio.h>
volatile const char coolant_tag[] = \"COOLANT-TAG-00000000000000000000000000000000\";
int main(int argc, char **argv) { if (argc > 1) puts((const char *)coolant_tag); return 0; }
";

const HELLO_GO: &str = "package main

import \"os\"

var coolantTag = \"COOLANT-TAG-00000000000000000000000000000000\"

func main() {
	if len(os.Args) > 1 {
		os.Stdout.WriteString(coolantTag + \"\\n\")
	}
}
";

/// Compiles a tiny executable into `dir` containing a 32-byte tag that
/// [`make_new_binaries`] rewrites. It uses cc, falling back to go build.
pub fn build_template(cancel: &AtomicBool, dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let out = dir.join("coolant-hello");
    let mut errs: Vec<String> = Vec::new();
    if let Some(cc) = which("cc") {
        let src = dir.join("hello.c");
        fs::write(&src, HELLO_C)?;
        match run_checked(cancel, Command::new(cc).arg("-O2").arg("-o").arg(&out).arg(&src), "cc") {
            Ok(_) => {
                check_template(cancel, &out)?;
                return Ok(out);
            }
            Err(e) => errs.push(format!("{e:#}")),
        }
    }
    if let Some(gobin) = which("go") {
        let src = dir.join("hello.go");
        fs::write(&src, HELLO_GO)?;
        let mut cmd = Command::new(gobin);
        cmd.args(["build", "-trimpath", "-ldflags=-s -w", "-o"]).arg(&out).arg(&src).current_dir(dir);
        match run_checked(cancel, &mut cmd, "go build") {
            Ok(_) => {
                check_template(cancel, &out)?;
                return Ok(out);
            }
            Err(e) => errs.push(format!("{e:#}")),
        }
    }
    Err(anyhow!("cannot build template binary (need cc or go): {}", errs.join("\n")))
}

fn check_template(cancel: &AtomicBool, bin: &Path) -> Result<()> {
    let b = fs::read(bin)?;
    if find_bytes(&b, TAG_MARKER.as_bytes()).is_none() {
        bail!("template {} has no tag", bin.display());
    }
    let out = output(cancel, &mut Command::new(bin))?;
    if !out.status.success() {
        bail!("{}: {}", bin.display(), out.status);
    }
    Ok(())
}

/// Writes n copies of the template into `dir`, each with a random tag (so a
/// never-seen-before cdhash) and re-signed ad hoc. The copies are not
/// executed.
pub fn make_new_binaries(cancel: &AtomicBool, tmpl: &Path, dir: &Path, n: usize) -> Result<Vec<PathBuf>> {
    if tmpl.as_os_str().is_empty() {
        bail!("no template binary");
    }
    let mut data = fs::read(tmpl)?;
    let Some(at) = find_bytes(&data, TAG_MARKER.as_bytes()) else {
        bail!("template {} has no tag", tmpl.display());
    };
    let at = at + TAG_MARKER.len();
    fs::create_dir_all(dir)?;
    let mut bins = Vec::new();
    for i in 0..n {
        let tag = hex(&random_bytes::<16>());
        data[at..at + 32].copy_from_slice(tag.as_bytes());
        let p = dir.join(format!("coolant-hello-{i:03}"));
        write_executable(&p, &data)?;
        run_checked(
            cancel,
            Command::new("/usr/bin/codesign").args(["-f", "-s", "-"]).arg(&p),
            &format!("codesign {}", p.display()),
        )?;
        bins.push(p);
    }
    Ok(bins)
}

fn write_executable(p: &Path, data: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o755).open(p)?;
    f.write_all(data)?;
    Ok(())
}

fn list_binaries(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir)? {
        let e = e?;
        if e.file_name().to_string_lossy().starts_with("coolant-hello-") {
            out.push(e.path());
        }
    }
    out.sort();
    Ok(out)
}

// --- git ---------------------------------------------------------------------

/// Runs git in `dir`, isolated from the user's global and system config
/// (hooks, fsmonitor, signing, LFS filters) so runs are comparable across
/// machines.
fn git_cmd(cancel: &AtomicBool, dir: &Path, args: &[&str]) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(dir).envs([
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_AUTHOR_NAME", "coolant"),
        ("GIT_AUTHOR_EMAIL", "bench@coolant.invalid"),
        ("GIT_COMMITTER_NAME", "coolant"),
        ("GIT_COMMITTER_EMAIL", "bench@coolant.invalid"),
    ]);
    run_checked(cancel, &mut cmd, &format!("git {}", args.join(" "))).map(|_| ())
}

/// Expects a file tree in dir/repo. It runs git init, add, commit, `status`
/// `status_n` times (appending to a file before every 10th, as an agent
/// editing code would), then adds and removes `worktrees` worktrees next to
/// the repo. It returns the number of git commands run.
pub fn git(cancel: &AtomicBool, dir: &Path, status_n: usize, worktrees: usize) -> Result<usize> {
    let repo = dir.join("repo");
    let mut ops = 0;
    let mut run = |args: &[&str]| {
        ops += 1;
        git_cmd(cancel, &repo, args)
    };
    run(&["init", "-q", "-b", "main"])?;
    run(&["add", "-A"])?;
    run(&["commit", "-q", "-m", "initial import"])?;
    let edit = repo.join("NOTES.md");
    for i in 0..status_n {
        if i % 10 == 0 {
            let mut f = fs::OpenOptions::new().create(true).append(true).open(&edit)?;
            writeln!(f, "edit {i}")?;
        }
        run(&["status", "--porcelain"])?;
    }
    for i in 0..worktrees {
        let wt = dir.join(format!("wt-{i}"));
        run(&["worktree", "add", "-q", "-b", &format!("agent-{i}"), &wt.to_string_lossy()])?;
    }
    for i in 0..worktrees {
        let wt = dir.join(format!("wt-{i}"));
        run(&["worktree", "remove", "--force", &wt.to_string_lossy()])?;
    }
    Ok(ops)
}

// --- copy / clonefile ----------------------------------------------------------

#[cfg(target_os = "macos")]
const CLONE_FLAG: &str = "-c";
#[cfg(target_os = "macos")]
const CLONE_HOW: &str = "cp -c -R (APFS clonefile)";
#[cfg(not(target_os = "macos"))]
const CLONE_FLAG: &str = "--reflink=auto";
#[cfg(not(target_os = "macos"))]
const CLONE_HOW: &str = "cp --reflink=auto -R (reflinks where the filesystem supports them)";

/// Copies the tree at `src` to `dst` with /bin/cp -R, or with copy-on-write
/// clones (APFS clonefile, Linux reflinks) when `clone` is set.
pub fn copy_deps(cancel: &AtomicBool, src: &Path, dst: &Path, clone: bool) -> Result<()> {
    fs::create_dir_all(dst.parent().context("copy: destination has no parent")?)?;
    let mut args: Vec<&std::ffi::OsStr> = vec!["-R".as_ref(), src.as_ref(), dst.as_ref()];
    if clone {
        args.insert(0, CLONE_FLAG.as_ref());
    }
    let what = format!("cp {}", args.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
    run_checked(cancel, Command::new("/bin/cp").args(&args), &what).map(|_| ())
}
