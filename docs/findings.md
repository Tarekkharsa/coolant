# Findings

Measured on a Mac mini M4 (10 cores, 16 GB, macOS 27.0.1) in October 2026,
with coolant's own profiler and benchmarks. Raw results are in
[`results/`](results/). These are single-machine numbers. Treat the ratios as
the finding and the absolute values as illustrative. Quick-preset runs on a
MacBook Pro 13" M1 (8 cores, 8 GB, macOS 26.5.2) and on a company-managed
MacBook Pro 14" M2 Pro (10 cores, 16 GB, macOS 27.0, CrowdStrike Falcon) are
noted below where they differ, as are 2-minute profiles of the M1 at idle and
of the M2 Pro with an agent at work.

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

The gap holds at realistic sizes. In the Default preset (20,000-file trees, a
10,000-file git repo, 1, 2 and 4 agents in parallel):

| workload, 1 agent | indexed | `.noindex` |
|---|---|---|
| copy a 20,000-file deps tree (`cp -R`) | 50.1 CPU-s overhead | 6.0 |
| clone it with APFS clonefile (`cp -c`) | 46.3 | 3.2 |
| git: 10,000-file repo, 50× status, 3 worktrees | 20.4 | 8.7 |
| 20,000 small files | 12.4 | 2.8 |

With 4 agents doing the git workload at once, the indexed folder cost
95 CPU-s of overhead against 29 in `.noindex`, on top of 85 CPU-s of actual
work. The remaining overhead in `.noindex` is mostly `fseventsd`, which no
folder setting avoids.

The workloads themselves took only 1–14% longer in the indexed folder, because
`mds_stores` and `mdworker_shared` do their work *after* the files are
written. An agent never sees it; the CPU, power draw and fan do.

The gap held on the M1: 16.3 vs 1.6 CPU-s for `cp -R`, 11.2 vs 1.1 for
clonefile, and 15.3 vs 2.8 for git with 2 agents (a second run: 15.2 vs 1.4,
13.6 vs 1.2, and 22.2 vs 3.0). On the M2 Pro: 14.1 vs 1.6,
12.1 vs 0.8, and 19.2 vs 3.1.

**Fix:** add the agent worktree folder (e.g. `<repo>/.claude/worktrees`) to
*System Settings → Spotlight → Search Privacy…*, which excludes it in place.
A `.metadata_never_index` file inside a folder does **not** stop indexing on
macOS 27; we tested that.

We first moved worktree folders into `~/.coolant/spaces.noindex/` behind a
symlink. Spotlight skipped them (verified with `mdfind`), but **it broke
Claude Code**: it resolves the link, finds the worktree outside its repository
and refuses to create agent worktrees. `coolant tune` now refuses worktrees,
and `coolant doctor` offers to move back any it moved before.

## 2. Fresh binaries pay a security tax on first run

Running a newly built, ad-hoc-signed binary for the first time took
**320–450 ms** across runs, against **1–2 ms** for later runs (and 0.7 ms for `/usr/bin/true`),
with the extra time spent in `syspolicyd`. Every `cargo build` / `cargo test`
produces new binaries, so Rust agents iterating on tests pay this again and
again.

On the M1 the tax was heavier: about 1.9 CPU-s in security daemons per 10 new
binaries (M4: 0.4). With 2 agents running new binaries at once, first runs took
about 830 ms and throughput barely improved over 1 agent (57% parallel
efficiency, against 100% on the M4). The two machines also run different macOS
versions (26.5 vs 27), and the security scanners change between releases, so
part of the difference may be the OS rather than the chip. A second run gave
the same picture: 1.8–3.9 CPU-s per 10 new binaries, first runs of 407–517 ms
with 1 agent and 808–834 ms with 2, and 62% parallel efficiency. The M2 Pro
scaled at 98% once its other agents were quiet; the M1 was never quiet (see
finding 7).

On the M2 Pro, first runs took 870–920 ms and later runs 7 ms. See finding 6
for why.

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

Crash loops are a quieter version of the same problem. On the M1, three
Homebrew services (`php-fpm`, `mysqld`, `watchman`) kept crashing and being
restarted, and `ReportCrash` spent 20 CPU-s writing their crash reports during
a five-minute benchmark. `coolant doctor` now flags any program with 10 or
more crashes in a day and, for a Homebrew service, offers `brew services stop`.

**Fix:** `coolant doctor` lists them grouped by project; `coolant-app` can stop
a group after confirmation. Processes serving ports exposed through Tailscale
Serve/Funnel, and paths you `coolant protect`, are never offered for stopping.

## 5. Merged worktrees keep gigabytes

`coolant sweep` found a merged agent worktree still holding 2 GB of
`node_modules`. Sweep removes artifact folders from idle or merged linked
worktrees that no process is using, as a dry run unless `--yes` is passed.

## 6. A Mac with an endpoint security agent was 2–6× slower at starting processes

The M2 Pro runs CrowdStrike Falcon, which company-managed Macs often have. It
was slower than the M1 and the M4 at everything that starts a process or
touches a file, although it runs macOS 27 like the M4:

| Quick preset, 1 agent, `.noindex` | M2 Pro + Falcon | M1 | M4 |
|---|---|---|---|
| `/usr/bin/true` execs per second | 267 | 1,019 | 1,502 |
| warm exec of a new binary | 7.1 ms | 1.5 ms | 2.0 ms |
| first exec of a new binary | 870 ms | 461 ms | 449 ms |
| copy a 3,000-file tree (`cp -R`) | 2.3 s | 1.0 s | 0.6 s |
| git workload, commands per second | 9 | 13 | 21 |

The extra CPU lands in the agent's own processes (the copy itself used 1.8
CPU-s, against 0.9 on the M1), because Endpoint Security makes each exec and
file open wait for Falcon's verdict. Falcon's own processes report 0:00 CPU
time to `ps`, so coolant can't attribute the cost to them. We could not turn
Falcon off to confirm it is the cause. Agents spawn thousands of short
processes (`git`, `rg`, `node`, compilers), so on a machine like this every
tool call costs more. There is nothing to fix from user space; prewarming and
fewer cold builds help more here than on an unmanaged Mac.

A second run on the same Mac, with no other agents busy, gave the same
numbers (244–250 execs per second, 780–930 ms for a first exec), and new
binaries then scaled to 2 agents at 98% efficiency. `coolant doctor` now
measures program start time and names any active endpoint security extension.

## 7. An idle Mac can be busier than the agents

With no agent working and no benchmark running, the M1 kept **1.63 cores**
busy on average over 2 minutes. Spotlight and security were 2% of it. Most of
the rest came from three sources, none of them agent work:

- **A git repository at `~`.** The home folder had a stray `.git` with one
  commit, so every folder under it sat inside a 1-million-file working tree.
  The agent host polls the git state of a thread's folder, which meant
  `git ls-files --others` at the root of the home folder: 20 s and 8.2 CPU-s
  per run, about twice a minute, 854 MB read from disk in 2 minutes. In this
  repository the same command takes 0.01 s.
- **`coreaudiod` at a quarter of a core**, with nothing playing. A dictation
  app had held the microphone for 2 days, which accounts for almost all of
  `coreaudiod`'s 696 CPU-minutes since boot. We didn't quit the app to confirm.
- **Crash loops** (finding 4): `ReportCrash` used 0.14 cores at idle.

Every number in the M1 runs carries this background load, which is why some of
its cells flip direction between runs. Details are in
[`results/2026-10-10-m1-macbook-pro-idle.md`](results/2026-10-10-m1-macbook-pro-idle.md).

## 8. One agent session can keep most of the Mac busy

A 2-minute profile of the M2 Pro while one Claude Code session (with its
subagents) worked on a TypeScript monorepo:

- **6.77 cores busy on average**, most of it under that session:
  `vite build`, two `tsc` processes (2.8 and 3.1 GB), three `tsgolint`
  (2.5–2.7 GB each) and `git`. Subagents run inside the session's process, so
  the profile can't split the work between them.
- **The Mac got warm.** CPU 58–61 °C, 14.8 W on average (22.5 W at most), fans
  up to 4,915 rpm. During the benchmarks the day before it stayed at 35–38 °C
  under 7 W with the fans off.
- **155 process launches per second.** At about 4 ms per launch on this Mac
  (finding 6), that is about 0.6 s of start-up time every second. 89% of the
  launches had no name, so the profile files them under "other".
- **The agent's work dwarfs the OS overhead here.** Spotlight, security and
  file-system daemons together used under 0.2 cores, because the session
  worked in folders it had already built.

Apple's `XProtectRemediator*` malware scans ran during the profile (0.6 cores,
unrelated to the agent). A third `claude` process, `claude --chrome-native-host`,
is the Claude in Chrome bridge, not an agent session. The category totals in
this profile overcount; see the method notes. Details are in
[`results/2026-10-11-m2-pro-macbook-pro-busy.md`](results/2026-10-11-m2-pro-macbook-pro-busy.md).

A second profile after the overcount fix found **8.75 cores** busy, with
memory pressure at "warning" for 24 s (38% available at the lowest, no swap)
while three `tsc` and two `tsgolint` ran at once. The busiest single process
was the Coolant app itself, at 0.77 cores, deleting a `node_modules` tree in
an idle worktree; `fseventsd` used another 0.75 cores passing on the change
events. Details are in
[`results/2026-10-12-m2-pro-macbook-pro-busy-rerun.md`](results/2026-10-12-m2-pro-macbook-pro-busy-rerun.md).

## Method notes

- CPU per category comes from sampling every process once per second
  (`/bin/ps` sees all users' processes), plus CPU of short-lived children
  recovered from their parents' reaped-children counters.
- That recovery overcounts when a process and its own children exit in the
  same sample interval: the children's CPU reaches the surviving ancestor but
  isn't subtracted, so it is counted twice, including CPU they used before the
  profile started. In the M2 Pro busy profile one sample added 1,108 CPU-s to
  `agent`, and the categories summed to 1,972 CPU-s against 813 measured by the
  host counters. The busy-core totals come from the host counters and are
  correct; per-category and per-session totals in profiles with build tools
  in them are not. Fixed after 0.2.0-beta.8: a vanished process is credited
  to its nearest living ancestor, and a sample can't add more hidden CPU than
  the host counters say was busy.
- One case remains: when a process's details can't be read for one sample
  (usually as it exits), its start time reads as 0, it looks like a new
  process, and its lifetime CPU is counted again in that sample (36.5 CPU-s in
  the second M2 Pro busy profile). The cap on hidden CPU doesn't cover newly
  seen processes, so that category is too high by the same amount. Fixed
  after 0.2.0-beta.10: a process without a start time keeps the key it had,
  and processes seen for the first time can't add more than the machine had
  left in that sample.
- Temperatures and power come from unprivileged sources: IOHID die sensors,
  SMC, IOReport GPU energy, and kernel per-coalition energy for CPU (the
  IOReport CPU energy counters don't change for unprivileged readers on
  macOS 27).
- Benchmarks let the machine settle (< 1 busy core, temperature near its
  starting value) before each case. Background noise on this machine (Docker,
  dev servers) affects absolute numbers; the indexed vs `.noindex` cases ran
  under the same conditions.
