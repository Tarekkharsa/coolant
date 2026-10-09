# Findings

Measured on a Mac mini M4 (10 cores, 16 GB, macOS 27.0.1) in October 2026,
with coolant's own profiler and benchmarks. Raw results are in
[`results/`](results/). These are single-machine numbers. Treat the ratios as
the finding and the absolute values as illustrative.

## 1. Spotlight is the biggest hidden cost, and it's invisible to the agent

Benchmark: the same agent-like workloads in a normal folder under `~` and in a
folder whose name ends in `.noindex` (which Spotlight skips). CPU used by
background processes during the workload plus a 4 s tail:

| workload | indexed | `.noindex` |
|---|---|---|
| copy a 3,000-file deps tree (`cp -R`) | 20.9 CPU-s overhead | 0.7 |
| clone it with APFS clonefile (`cp -c`) | 17.4 | 0.6 |
| git: init, commit 1,500 files, 20× status, 2 worktrees | 4.0 | 1.2 |
| write / stat / read / delete 3,000 small files | 3.9 | 0.4 |

The workloads themselves took only 1–14% longer in the indexed folder, because
`mds_stores` and `mdworker_shared` do their work *after* the files are
written. An agent never sees it; the CPU, power draw and fan do.

**Fix:** keep agent worktrees out of Spotlight with `coolant tune <folder>`.
It moves the folder into `~/.coolant/spaces.noindex/` and symlinks it back.
Spotlight doesn't follow symlinks, so files written through the old path aren't
indexed; we verified this with `mdfind`. A `.metadata_never_index` file inside
a folder does **not** stop indexing on macOS 27; we tested that too.

## 2. Fresh binaries pay a security tax on first run

Running a newly built, ad-hoc-signed binary for the first time took
**~450 ms**, against **~2 ms** for later runs (and 0.7 ms for `/usr/bin/true`),
with the extra time spent in `syspolicyd`. Every `cargo build` / `cargo test`
produces new binaries, so Rust agents iterating on tests pay this again and
again.

## 3. Cold worktrees rebuild everything

A new git worktree has no `target/` or `node_modules`, so an agent's first
build or install in it starts from zero. Cloning the main checkout's artifacts
with copy-on-write before the first build (`coolant prewarm`):

| first `cargo build` in a fresh worktree of this repo | cold | prewarmed |
|---|---|---|
| `-p coolant-cli` (small dependency tree) | 25.4 CPU-s, 5.9 s | 5.4 CPU-s, 3.0 s |
| `-p coolant-app` (GPUI, ~400 crates) | 256.7 CPU-s, 39.7 s | 5.1 CPU-s, 15.9 s |

That's **50× less CPU** for the first build of the GUI app: about four minutes
of one core's work that every new worktree would otherwise spend at full
power. Cloning took about 4 s and uses no extra disk until files change.
Reproduce with `scripts/prewarm-bench.sh -p coolant-app`. Cargo's
dependency fingerprints don't depend on the workspace path, so only the
workspace's own crates rebuild. `coolant prewarm --install-hook <repo>` installs
a `post-checkout` hook that does this for every new worktree.

## 4. Leftovers keep burning

On the test machine, `coolant profile` found 24 dev servers (node API and web
servers, Caddy gateways) left running by earlier agent sessions in worktrees,
some for 5 days. Separately, Docker Desktop's VM used up to 1.9 cores. Neither
showed up as "agent" activity, but both heated the machine all day.

**Fix:** `coolant doctor` lists them grouped by project; `coolant-app` can stop
a group after confirmation. Processes serving ports exposed through Tailscale
Serve/Funnel, and paths you `coolant protect`, are never offered for stopping.

## 5. Merged worktrees keep gigabytes

`coolant sweep` found a merged agent worktree still holding 2 GB of
`node_modules`. Sweep removes artifact folders from idle or merged linked
worktrees that no process is using, as a dry run unless `--yes` is passed.

## Method notes

- CPU per category comes from sampling every process once per second
  (`/bin/ps` sees all users' processes), plus CPU of short-lived children
  recovered from their parents' reaped-children counters.
- Temperatures and power come from unprivileged sources: IOHID die sensors,
  SMC, IOReport GPU energy, and kernel per-coalition energy for CPU (the
  IOReport CPU energy counters don't change for unprivileged readers on
  macOS 27).
- Benchmarks let the machine settle (< 1 busy core, temperature near its
  starting value) before each case. Background noise on this machine (Docker,
  dev servers) affects absolute numbers; the indexed vs `.noindex` cases ran
  under the same conditions.
