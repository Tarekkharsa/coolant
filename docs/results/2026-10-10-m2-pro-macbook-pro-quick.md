# coolant benchmark

Machine: Mac14,9, Apple M2 Pro, 10 cores, darwin 27.0. Median of 1 repetition(s).

Cells are `indexed / noindex`. CPU is in CPU-seconds over the workload plus the settle tail; categories exclude the benchmark's own processes. *Overhead* = indexer + security + fs-daemon + backup-sync; *overhead %* is relative to the workload's own CPU. *bg excess* is CPU used by everything else on the machine above its idle rate just before the case.

## smallfiles

create a node_modules-like tree of 3000 files (0.5–8 KB), stat-walk it, read every file, delete it; ops = file operations.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 0.98 / 0.94 | 12245 / 12785 | 0.74 / 0.72 | 2.86 / 0.78 | 388.7 / 108.4 | 2.48 / 0.30 | 0.01 / 0.04 | 0.37 / 0.44 | 0.34 / 1.50 | 4.65 / 6.10 | 32 / 389 | 35→36 / 35→36 | 1.6 / 2.8 | 0 / 0 |
| 2 | 1.26 / 1.24 | 19036 / 19397 | 2.05 / 2.02 | 4.16 / 1.44 | 203.2 / 71.6 | 3.30 / 0.47 | 0.00 / 0.07 | 0.85 / 0.90 | 0.22 / 4.17 | 9.60 / 10.50 | 906 / 905 | 35→36 / 35→36 | 2.5 / 3.3 | 0 / 0 |

## spawn

exec /usr/bin/true 300 times sequentially; ops = execs.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 1.14 / 1.12 | 264 / 267 | 0.46 / 0.44 | 0.03 / 0.01 | 6.7 / 2.1 | 0.02 / 0.01 | 0.00 / 0.00 | 0.01 / 0.00 | 0.00 / 0.00 | 1.03 / 1.33 | 318 / 311 | 35→36 / 35→36 | 0.6 / 0.6 | 0 / 0 |
| 2 | 0.95 / 0.95 | 632 / 630 | 0.81 / 0.81 | 0.32 / 0.00 | 39.6 / 0.1 | 0.31 / 0.00 | 0.00 / 0.00 | 0.01 / 0.00 | 0.00 / 0.00 | 2.14 / 1.72 | 625 / 613 | 35→36 / 35→36 | 1.3 / 1.3 | 0 / 0 |

| K | first exec ms | warm exec ms |
|---|---|---|
| 1 | 11.64 / 8.64 | 3.76 / 3.73 |
| 2 | 12.82 / 11.47 | 3.13 / 3.14 |

## spawn-new-binary

exec 10 freshly built, ad-hoc signed, never-seen binaries 300 times in total; ops = execs.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 11.15 / 10.77 | 27 / 28 | 0.40 / 0.52 | 2.09 / 0.54 | 528.3 / 102.5 | 1.12 / 0.03 | 0.93 / 0.48 | 0.04 / 0.02 | 3.87 / 0.98 | 5.21 / 6.53 | 1198 / 1237 | 35→36 / 35→36 | 1.5 / 0.6 | 0 / 0 |
| 2 | 12.57 / 16.14 | 48 / 37 | 0.81 / 0.90 | 1.00 / 0.84 | 123.3 / 93.1 | 0.23 / 0.12 | 0.70 / 0.68 | 0.07 / 0.03 | 172.87 / 1.02 | 5.96 / 5.56 | 1249 / 651 | 35→35 / 35→35 | 1.5 / 0.7 | 0 / 0 |

| K | first exec ms | warm exec ms |
|---|---|---|
| 1 | 917.72 / 869.92 | 6.80 / 7.14 |
| 2 | 1076.56 / 1422.14 | 6.22 / 6.61 |

## git

git init a 1500-file repo, add+commit, 20× git status (editing a file every 10), add and remove 2 worktrees; ops = git commands.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 3.22 / 3.18 | 8 / 9 | 2.65 / 2.57 | 9.98 / 1.45 | 377.1 / 56.4 | 6.14 / 0.79 | 0.02 / 0.02 | 0.70 / 0.64 | 0.00 / 0.00 | -5.84 / 6.45 | 415 / 407 | 35→36 / 35→36 | 3.3 / 2.8 | 0 / 0 |
| 2 | 4.12 / 4.08 | 13 / 13 | 6.93 / 6.77 | 19.15 / 3.07 | 276.3 / 45.3 | 17.55 / 1.62 | 0.08 / 0.03 | 1.51 / 1.42 | 0.00 / 0.00 | 5.76 / 13.90 | 1715 / 798 | 36→37 / 35→36 | 6.2 / 5.0 | 0 / 0 |

## copy-deps

copy a 3000-file deps tree into a new worktree dir with cp -R; ops = files.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 2.42 / 2.32 | 1241 / 1294 | 1.82 / 1.76 | 14.07 / 1.58 | 774.1 / 90.1 | 13.50 / 1.02 | 0.00 / 0.02 | 0.57 / 0.54 | 0.52 / 2.87 | 1.11 / 5.36 | 45 / 18 | 36→37 / 36→37 | 2.9 / 2.4 | 0 / 0 |
| 2 | 2.82 / 2.95 | 2129 / 2032 | 4.46 / 4.70 | 11.09 / 2.42 | 248.8 / 51.5 | 10.00 / 1.17 | 0.00 / 0.06 | 1.09 / 1.19 | 0.41 / 1.40 | 5.44 / 9.89 | 37 / 73 | 37→38 / 36→37 | 4.9 / 5.8 | 0 / 0 |

## clonefile-deps

copy a 3000-file deps tree into a new worktree dir with cp -c -R (APFS clonefile); ops = files.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 1.15 / 1.10 | 2612 / 2729 | 0.90 / 0.85 | 12.08 / 0.79 | 1336.5 / 92.8 | 11.52 / 0.19 | 0.00 / 0.02 | 0.55 / 0.58 | 0.37 / 0.21 | 1.68 / 0.11 | 28 / 14 | 37→37 / 37→38 | 2.0 / 1.5 | 0 / 0 |
| 2 | 1.45 / 1.41 | 4132 / 4248 | 2.30 / 2.29 | 12.74 / 1.71 | 554.0 / 75.0 | 11.57 / 0.57 | 0.00 / 0.01 | 1.17 / 1.13 | 0.46 / 0.39 | 3.81 / 3.46 | 35 / 19 | 37→38 / 37→37 | 3.3 / 2.8 | 0 / 0 |

## copy vs clonefile

| location | K | cp -R wall s | cp -c -R wall s | speedup | cp -R workload CPU | clonefile workload CPU | cp -R overhead | clonefile overhead |
|---|---|---|---|---|---|---|---|---|
| indexed | 1 | 2.42 | 1.15 | 2.1× | 1.82 | 0.90 | 14.07 | 12.08 |
| indexed | 2 | 2.82 | 1.45 | 1.9× | 4.46 | 2.30 | 11.09 | 12.74 |
| noindex | 1 | 2.32 | 1.10 | 2.1× | 1.76 | 0.85 | 1.58 | 0.79 |
| noindex | 2 | 2.95 | 1.41 | 2.1× | 4.70 | 2.29 | 2.42 | 1.71 |

## Busiest background processes (all cases, summed)

| process | category | CPU s |
|---|---|---|
| 2.1.295 | other | 158.29 |
| mdworker_shared | indexer | 51.25 |
| mds | indexer | 14.96 |
| fseventsd | fs-daemon | 13.46 |
| mds_stores | indexer | 13.06 |
| 2.1.296 | other | 11.19 |
| Finder | desktop | 4.56 |
| spindump | other | 3.69 |
| launchd | kernel | 3.41 |
| mediaanalysisd | backup-sync | 3.12 |
| system_profiler | other | 2.39 |
| contextstored | desktop | 2.20 |
| com.apple.DriverKit-AppleBCMWLAN | kernel | 1.62 |
| claude | agent | 1.62 |
| syspolicyd | security | 1.59 |

## What this shows

- **smallfiles**: indexed location took 3% more wall time vs noindex; OS overhead 7.02 vs 2.22 CPU-s (252% vs 81% of workload CPU); largest overhead category: indexer (6.56 CPU-s summed over cases).
- **spawn**: indexed location took 1% more wall time vs noindex; OS overhead 0.35 vs 0.01 CPU-s (28% vs 1% of workload CPU); largest overhead category: indexer (0.34 CPU-s summed over cases).
- **spawn-new-binary**: indexed location took 12% less wall time vs noindex; OS overhead 3.10 vs 1.37 CPU-s (256% vs 97% of workload CPU); largest overhead category: security (2.80 CPU-s summed over cases).
- **git**: indexed location took 1% more wall time vs noindex; OS overhead 29.13 vs 4.52 CPU-s (304% vs 48% of workload CPU); largest overhead category: indexer (26.11 CPU-s summed over cases).
- **copy-deps**: indexed location took 1% less wall time vs noindex; OS overhead 25.16 vs 4.00 CPU-s (401% vs 62% of workload CPU); largest overhead category: indexer (25.69 CPU-s summed over cases).
- **clonefile-deps**: indexed location took 4% more wall time vs noindex; OS overhead 24.82 vs 2.51 CPU-s (775% vs 80% of workload CPU); largest overhead category: indexer (23.86 CPU-s summed over cases).
- Cloning deps with clonefile (`cp -c -R`) was 2.0× faster than a plain copy in the indexed location.
- Cloning deps with clonefile (`cp -c -R`) was 2.1× faster than a plain copy in the noindex location.
- smallfiles: throughput at K=2 is 1.5× K=1 (76% parallel efficiency).
- spawn: throughput at K=2 is 2.4× K=1 (118% parallel efficiency).
- spawn-new-binary: throughput at K=2 is 1.3× K=1 (67% parallel efficiency).
- git: throughput at K=2 is 1.6× K=1 (78% parallel efficiency).
- copy-deps: throughput at K=2 is 1.6× K=1 (79% parallel efficiency).
- clonefile-deps: throughput at K=2 is 1.6× K=1 (78% parallel efficiency).
- First exec of a never-seen binary took 869.92 ms vs 7.14 ms warm (system /usr/bin/true: 3.73 ms); security CPU 0.48 vs 0.00 CPU-s.
- Background processes classified as "other" that used noticeable CPU (classification candidates): 2.1.295 (158.29s), 2.1.296 (11.19s), spindump (3.69s), system_profiler (2.39s), t3-resource-monitor (1.30s), ninjarmm-macagent (1.30s), top (0.90s), AssetMetricsExtension (0.86s).

## Notes on this run

MacBook Pro 14" M2 Pro (10 cores, 16 GB, two fans, which stayed off: 0 rpm
throughout), macOS 27.0, a company-managed machine with the CrowdStrike Falcon
endpoint security extension and the NinjaRMM agent installed. `coolant` built
from source at 47eb826 (v0.2.0-beta.1), run with `coolant bench --preset quick`.

- **The Spotlight gap reproduces.** After copying a 3,000-file tree the indexed
  folder cost 14.1 CPU-s of overhead against 1.6 in `.noindex` (M4: 20.9 / 0.7,
  M1: 16.3 / 1.6); clonefile 12.1 / 0.8; git 10.0 / 1.5 with 1 agent and
  19.2 / 3.1 with 2. Wall time differed by only 1–4%.
- **Everything that starts a process or touches a file was 2–6× slower than on
  the M1 and M4.** `/usr/bin/true` ran 264 times a second (M1: 1,011, M4: 1,507);
  a warm exec took 3.7 ms (M1: 1.0, M4: 0.7). A never-seen binary took
  870–920 ms to start the first time (M1 and M4: 450–500 ms) and 7 ms after
  that (M1: 1.5, M4: 2.0). Copying the deps tree took 2.4 s (M1: 1.1,
  M4: 0.6), and the copy's own CPU was 1.8 CPU-s (M1: 0.9, M4: 0.6), so the
  extra cost is charged to the agent's processes, not to a background daemon.
  The likely cause is Falcon, which checks every exec and file open through
  Endpoint Security before the kernel lets it continue. We could not turn it
  off to confirm this, because the company manages it.
- **Falcon's own CPU is invisible.** `ps` reports 0:00 CPU time for all of
  Falcon's processes, so the "security" column shows only `syspolicyd` and
  friends (0.5–0.9 CPU-s per case). The cost shows up only as slower
  workloads.
- **The machine was noisy.** Other Claude Code sessions ran during the
  benchmark. Their processes are named after the version (`2.1.295`,
  `2.1.296`, from `~/.local/share/claude/versions/`), so they were classified as
  "other" and used 169 CPU-s in total, almost all of it during
  `spawn-new-binary` at K=2 in the indexed folder (172.9 CPU-s "other (bg)").
  Treat both K=2 `spawn-new-binary` cells as outliers, including the
  "12% less wall time" line above. This run had one repetition;
  `--preset full` takes the median of 3.
- **It stayed cool.** CPU temperature stayed between 35 and 38 °C, CPU power
  between 0.6 and 6.2 W, and the fans never started.
