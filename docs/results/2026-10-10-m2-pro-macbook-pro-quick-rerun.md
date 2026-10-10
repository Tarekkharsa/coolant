# coolant benchmark

Machine: Mac14,9, Apple M2 Pro, 10 cores, darwin 27.0. Median of 1 repetition(s).

Cells are `indexed / noindex`. CPU is in CPU-seconds over the workload plus the settle tail; categories exclude the benchmark's own processes. *Overhead* = indexer + security + fs-daemon + backup-sync; *overhead %* is relative to the workload's own CPU. *bg excess* is CPU used by everything else on the machine above its idle rate just before the case.

## smallfiles

create a node_modules-like tree of 3000 files (0.5–8 KB), stat-walk it, read every file, delete it; ops = file operations.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 0.96 / 0.92 | 12513 / 13073 | 0.74 / 0.71 | 2.68 / 0.57 | 361.7 / 79.6 | 2.32 / 0.18 | 0.00 / 0.00 | 0.36 / 0.39 | 0.37 / 0.20 | 3.84 / 2.51 | 44 / 15 | 35→36 / 35→36 | 1.6 / 1.3 | 0 / 0 |
| 2 | 1.35 / 1.21 | 17828 / 19887 | 2.19 / 2.00 | 4.22 / 1.16 | 192.4 / 58.0 | 3.25 / 0.35 | 0.06 / 0.00 | 0.90 / 0.81 | 0.43 / 0.21 | 8.55 / 4.86 | 1131 / 18 | 36→36 / 35→36 | 3.3 / 2.3 | 0 / 0 |

## spawn

exec /usr/bin/true 300 times sequentially; ops = execs.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 1.23 / 1.20 | 244 / 250 | 0.45 / 0.45 | 0.09 / 0.12 | 19.6 / 27.2 | 0.01 / 0.12 | 0.00 / 0.00 | 0.01 / 0.00 | 0.00 / 0.00 | 1.89 / -1.89 | 320 / 317 | 35→36 / 35→36 | 0.5 / 0.6 | 0 / 0 |
| 2 | 0.95 / 0.96 | 631 / 625 | 0.81 / 0.81 | 0.01 / 0.07 | 1.3 / 8.9 | 0.00 / 0.06 | 0.00 / 0.00 | 0.01 / 0.01 | 0.00 / 0.00 | 1.37 / 1.21 | 619 / 619 | 35→36 / 36→36 | 1.3 / 1.3 | 0 / 0 |

| K | first exec ms | warm exec ms |
|---|---|---|
| 1 | 8.65 / 10.17 | 4.08 / 3.98 |
| 2 | 8.38 / 10.26 | 3.15 / 3.17 |

## spawn-new-binary

exec 10 freshly built, ad-hoc signed, never-seen binaries 300 times in total; ops = execs.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 11.50 / 10.07 | 26 / 30 | 0.54 / 0.56 | 0.70 / 0.62 | 130.2 / 110.7 | 0.03 / 0.11 | 0.61 / 0.47 | 0.06 / 0.04 | 2.12 / 1.01 | 0.14 / 5.96 | 380 / 1767 | 35→36 / 35→36 | 0.8 / 0.8 | 0 / 0 |
| 2 | 9.80 / 10.27 | 61 / 58 | 0.99 / 1.00 | 0.67 / 0.65 | 67.8 / 65.1 | 0.02 / 0.02 | 0.65 / 0.62 | 0.00 / 0.01 | 0.74 / 0.75 | -0.58 / 4.76 | 635 / 640 | 35→36 / 35→36 | 0.9 / 0.8 | 0 / 0 |

| K | first exec ms | warm exec ms |
|---|---|---|
| 1 | 930.86 / 782.16 | 7.54 / 7.74 |
| 2 | 793.96 / 847.01 | 6.42 / 6.21 |

## git

git init a 1500-file repo, add+commit, 20× git status (editing a file every 10), add and remove 2 worktrees; ops = git commands.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 3.34 / 3.20 | 8 / 8 | 2.74 / 2.58 | 9.76 / 1.52 | 355.7 / 58.9 | 5.40 / 0.86 | 0.07 / 0.01 | 0.72 / 0.65 | 0.00 / 0.00 | -3.99 / 5.22 | 1302 / 411 | 35→36 / 35→36 | 3.8 / 2.8 | 0 / 0 |
| 2 | 4.10 / 4.04 | 13 / 13 | 6.90 / 6.73 | 18.95 / 3.06 | 274.7 / 45.5 | 17.37 / 1.61 | 0.07 / 0.03 | 1.51 / 1.42 | 0.00 / 0.00 | 10.03 / 13.71 | 861 / 802 | 36→37 / 35→36 | 6.7 / 5.0 | 0 / 0 |

## copy-deps

copy a 3000-file deps tree into a new worktree dir with cp -R; ops = files.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 2.65 / 2.53 | 1132 / 1185 | 2.05 / 1.90 | 15.08 / 1.55 | 734.8 / 81.6 | 14.50 / 1.01 | 0.00 / 0.00 | 0.58 / 0.54 | 0.57 / 1.66 | 1.02 / 3.07 | 49 / 18 | 36→37 / 36→37 | 2.9 / 2.3 | 0 / 0 |
| 2 | 3.30 / 2.82 | 1818 / 2127 | 5.18 / 4.42 | 11.98 / 2.04 | 231.2 / 46.2 | 10.47 / 0.96 | 0.37 / 0.00 | 1.14 / 1.08 | 3.21 / 0.27 | 7.69 / 9.27 | 632 / 21 | 37→37 / 36→37 | 6.0 / 4.1 | 0 / 0 |

## clonefile-deps

copy a 3000-file deps tree into a new worktree dir with cp -c -R (APFS clonefile); ops = files.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 1.15 / 1.14 | 2611 / 2627 | 0.90 / 0.88 | 8.36 / 0.78 | 934.1 / 88.7 | 7.77 / 0.26 | 0.00 / 0.00 | 0.59 / 0.52 | 0.39 / 0.22 | 1.10 / -0.13 | 23 / 21 | 37→38 / 37→37 | 1.9 / 1.5 | 0 / 0 |
| 2 | 1.45 / 1.46 | 4138 / 4122 | 2.32 / 2.28 | 8.82 / 1.44 | 380.8 / 63.3 | 7.60 / 0.29 | 0.03 / 0.00 | 1.19 / 1.15 | 0.72 / 0.30 | 0.17 / 7.96 | 529 / 17 | 37→37 / 37→38 | 3.8 / 2.9 | 0 / 0 |

## copy vs clonefile

| location | K | cp -R wall s | cp -c -R wall s | speedup | cp -R workload CPU | clonefile workload CPU | cp -R overhead | clonefile overhead |
|---|---|---|---|---|---|---|---|---|
| indexed | 1 | 2.65 | 1.15 | 2.3× | 2.05 | 0.90 | 15.08 | 8.36 |
| indexed | 2 | 3.30 | 1.45 | 2.3× | 5.18 | 2.32 | 11.98 | 8.82 |
| noindex | 1 | 2.53 | 1.14 | 2.2× | 1.90 | 0.88 | 1.55 | 0.78 |
| noindex | 2 | 2.82 | 1.46 | 1.9× | 4.42 | 2.28 | 2.04 | 1.44 |

## Busiest background processes (all cases, summed)

| process | category | CPU s |
|---|---|---|
| mdworker_shared | indexer | 44.58 |
| mds | indexer | 13.69 |
| fseventsd | fs-daemon | 13.39 |
| mds_stores | indexer | 11.81 |
| Finder | desktop | 4.42 |
| mediaanalysisd | backup-sync | 3.56 |
| launchd | kernel | 2.11 |
| contextstored | desktop | 1.89 |
| WindowServer | desktop | 1.82 |
| syspolicyd | security | 1.51 |
| com.apple.DriverKit-AppleBCMWLAN | kernel | 1.36 |
| t3 | agent-host | 1.36 |
| t3-resource-monitor | other | 1.29 |
| zed | agent-host | 1.17 |
| ninjarmm-macagent | other | 1.16 |

## What this shows

- **smallfiles**: indexed location took 8% more wall time vs noindex; OS overhead 6.90 vs 1.73 CPU-s (235% vs 64% of workload CPU); largest overhead category: indexer (6.10 CPU-s summed over cases).
- **spawn**: indexed location took 1% more wall time vs noindex; OS overhead 0.10 vs 0.19 CPU-s (8% vs 15% of workload CPU); largest overhead category: indexer (0.19 CPU-s summed over cases).
- **spawn-new-binary**: indexed location took 5% more wall time vs noindex; OS overhead 1.38 vs 1.28 CPU-s (90% vs 82% of workload CPU); largest overhead category: security (2.35 CPU-s summed over cases).
- **git**: indexed location took 3% more wall time vs noindex; OS overhead 28.71 vs 4.58 CPU-s (298% vs 49% of workload CPU); largest overhead category: indexer (25.24 CPU-s summed over cases).
- **copy-deps**: indexed location took 11% more wall time vs noindex; OS overhead 27.06 vs 3.59 CPU-s (374% vs 57% of workload CPU); largest overhead category: indexer (26.94 CPU-s summed over cases).
- **clonefile-deps**: indexed location took the same wall time vs noindex; OS overhead 17.18 vs 2.23 CPU-s (535% vs 70% of workload CPU); largest overhead category: indexer (15.92 CPU-s summed over cases).
- Cloning deps with clonefile (`cp -c -R`) was 2.3× faster than a plain copy in the indexed location.
- Cloning deps with clonefile (`cp -c -R`) was 2.1× faster than a plain copy in the noindex location.
- smallfiles: throughput at K=2 is 1.5× K=1 (76% parallel efficiency).
- spawn: throughput at K=2 is 2.5× K=1 (125% parallel efficiency).
- spawn-new-binary: throughput at K=2 is 2.0× K=1 (98% parallel efficiency).
- git: throughput at K=2 is 1.6× K=1 (79% parallel efficiency).
- copy-deps: throughput at K=2 is 1.8× K=1 (90% parallel efficiency).
- clonefile-deps: throughput at K=2 is 1.6× K=1 (78% parallel efficiency).
- First exec of a never-seen binary took 782.16 ms vs 7.74 ms warm (system /usr/bin/true: 3.98 ms); security CPU 0.47 vs 0.00 CPU-s.
- Background processes classified as "other" that used noticeable CPU (classification candidates): t3-resource-monitor (1.29s), ninjarmm-macagent (1.16s), system_profiler (1.02s), cloudflared (0.68s), handy (0.65s), top (0.56s), opendirectoryd (0.53s), runningboardd (0.42s).

## Notes on this run

Second Quick run on the same MacBook Pro 14" M2 Pro (company-managed,
CrowdStrike Falcon, macOS 27.0), a few hours after
[the first](2026-10-10-m2-pro-macbook-pro-quick.md), with `coolant` built from
source at 3981651. No other agent session was busy this time, so background
noise stayed low: "other (bg)" was at most 3.2 CPU-s per case (first run: up to
172.9).

- **The results reproduce.** Spotlight overhead after copying the deps tree:
  15.1 vs 1.6 CPU-s (first run: 14.1 vs 1.6); clonefile 8.4 vs 0.8 (12.1 vs
  0.8); git with 2 agents 19.0 vs 3.1 (19.2 vs 3.1). Wall time differed by
  0–11%.
- **Program starts are still slow.** `/usr/bin/true` ran 244–250 times a
  second (first run: 264–267) at 4.0 ms per warm exec. A never-seen binary took
  780–930 ms to start the first time and 6–8 ms after that. `coolant doctor`
  now reports this ("Starting a program takes 2.8 ms on this Mac") and names
  Falcon Sensor as the cause.
- **New binaries scale with 2 agents after all.** Throughput went from 26–30 to
  58–61 ops/s (98% parallel efficiency; first run: 67%). The first run's
  low number came from the busy Claude Code sessions, as suspected.
- **Claude Code classification.** Commit 3981651 classifies processes run
  from `~/.local/share/claude/versions/<ver>` as agents (its unit test passes).
  No such process was busy during this run, so the benchmark could not show
  the change; the only Claude Code CPU was 0.7 CPU-s, classified as agent.
- **Still classified as "other":** `t3-resource-monitor` (1.3 CPU-s, part of
  T3 Code), `ninjarmm-macagent` (1.2, NinjaRMM remote management),
  `cloudflared` (0.7), `handy` (0.7).
- **It stayed cool.** CPU temperature stayed between 35 and 38 °C, CPU power
  between 0.5 and 6.7 W, and the fans never started.
