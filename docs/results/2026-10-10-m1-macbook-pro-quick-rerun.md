# coolant benchmark

Machine: MacBookPro17,1, Apple M1, 8 cores, darwin 26.5.2. Median of 1 repetition(s).

Cells are `indexed / noindex`. CPU is in CPU-seconds over the workload plus the settle tail; categories exclude the benchmark's own processes. *Overhead* = indexer + security + fs-daemon + backup-sync; *overhead %* is relative to the workload's own CPU. *bg excess* is CPU used by everything else on the machine above its idle rate just before the case.

## smallfiles

create a node_modules-like tree of 3000 files (0.5–8 KB), stat-walk it, read every file, delete it; ops = file operations.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 0.47 / 0.56 | 25431 / 21461 | 0.42 / 0.52 | 2.69 / 0.93 | 635.5 / 180.7 | 2.21 / 0.18 | 0.01 / 0.19 | 0.46 / 0.56 | 1.95 / 3.24 | 5.08 / 1.43 | 216 / 140 | 53→56 / 57→65 | 1.2 / 3.7 | 1207 / 1220 |
| 2 | 0.78 / 0.66 | 30929 / 36154 | 1.27 / 1.20 | 4.32 / 1.29 | 340.0 / 107.5 | 3.26 / 0.29 | 0.02 / 0.00 | 1.04 / 0.99 | 2.94 / 2.33 | 6.56 / 2.43 | 267 / 832 | 51→66 / 49→56 | 2.8 / 1.6 | 1204 / 1211 |

## spawn

exec /usr/bin/true 300 times sequentially; ops = execs.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 0.30 / 0.33 | 1013 / 898 | 0.24 / 0.26 | 0.08 / 0.47 | 32.8 / 183.6 | 0.07 / 0.13 | 0.00 / 0.29 | 0.01 / 0.03 | 1.85 / 4.36 | 0.27 / -1.04 | 627 / 600 | 47→49 / 50→58 | 0.7 / 1.5 | 1212 / 1219 |
| 2 | 0.33 / 0.39 | 1797 / 1525 | 0.59 / 0.58 | 0.18 / 0.67 | 31.4 / 115.2 | 0.09 / 0.55 | 0.09 / 0.02 | 0.01 / 0.09 | 1.95 / 5.69 | -0.62 / -3.14 | 722 / 1030 | 53→61 / 54→60 | 3.6 / 4.5 | 1220 / 1218 |

| K | first exec ms | warm exec ms |
|---|---|---|
| 1 | 2.50 / 1.74 | 0.98 / 1.11 |
| 2 | 4.23 / 2.31 | 1.10 / 1.31 |

## spawn-new-binary

exec 10 freshly built, ad-hoc signed, never-seen binaries 300 times in total; ops = execs.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 4.50 / 5.59 | 67 / 54 | 0.37 / 0.36 | 3.91 / 1.85 | 1054.9 / 511.7 | 0.03 / 0.04 | 3.88 / 1.78 | 0.01 / 0.02 | 5.73 / 5.68 | 3.32 / 2.37 | 439 / 557 | 58→72 / 55→66 | 6.0 / 3.2 | 1218 / 1223 |
| 2 | 8.74 / 8.99 | 69 / 67 | 0.60 / 0.54 | 3.96 / 3.84 | 657.3 / 708.0 | 0.12 / 0.03 | 3.82 / 3.78 | 0.02 / 0.02 | 5.77 / 5.12 | 6.61 / 4.18 | 823 / 3930 | 46→54 / 49→52 | 1.3 / 1.1 | 1224 / 1217 |

| K | first exec ms | warm exec ms |
|---|---|---|
| 1 | 406.78 / 517.34 | 1.50 / 1.42 |
| 2 | 807.95 / 834.22 | 1.55 / 1.48 |

## git

git init a 1500-file repo, add+commit, 20× git status (editing a file every 10), add and remove 2 worktrees; ops = git commands.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 1.90 / 1.84 | 14 / 15 | 1.79 / 1.74 | 4.37 / 1.67 | 243.5 / 96.0 | 3.27 / 0.65 | 0.01 / 0.01 | 1.08 / 1.01 | 0.84 / 0.76 | 1.51 / 1.61 | 386 / 386 | 46→52 / 43→47 | 1.9 / 1.7 | 1219 / 1217 |
| 2 | 2.41 / 2.31 | 22 / 23 | 4.50 / 4.44 | 22.19 / 2.95 | 492.7 / 66.4 | 20.26 / 1.03 | 0.02 / 0.03 | 1.89 / 1.87 | 1.60 / 3.18 | 0.02 / 7.77 | 553 / 1013 | 46→65 / 48→64 | 3.8 / 3.4 | 1224 / 1222 |

## copy-deps

copy a 3000-file deps tree into a new worktree dir with cp -R; ops = files.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 1.03 / 0.91 | 2905 / 3293 | 0.91 / 0.84 | 15.18 / 1.36 | 1675.6 / 161.5 | 14.53 / 0.74 | 0.00 / 0.00 | 0.66 / 0.61 | 2.77 / 1.72 | 0.10 / 0.36 | 44 / 124 | 43→47 / 43→46 | 1.7 / 1.2 | 1219 / 1220 |
| 2 | 1.49 / 1.17 | 4030 / 5125 | 2.37 / 2.16 | 12.84 / 2.95 | 542.3 / 136.1 | 11.37 / 1.70 | 0.03 / 0.00 | 1.38 / 1.24 | 4.52 / 1.78 | 1.96 / 6.57 | 167 / 292 | 47→61 / 43→46 | 4.5 / 2.3 | 1216 / 1220 |

## clonefile-deps

copy a 3000-file deps tree into a new worktree dir with cp -c -R (APFS clonefile); ops = files.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 0.57 / 0.57 | 5252 / 5244 | 0.54 / 0.53 | 13.61 / 1.23 | 2518.6 / 229.9 | 12.86 / 0.51 | 0.00 / 0.03 | 0.75 / 0.69 | 2.98 / 2.64 | -0.14 / 1.76 | 39 / 14 | 45→47 / 42→46 | 1.7 / 1.2 | 1219 / 1220 |
| 2 | 0.80 / 0.78 | 7500 / 7670 | 1.41 / 1.45 | 10.76 / 3.47 | 764.0 / 238.8 | 9.33 / 1.96 | 0.00 / 0.00 | 1.43 / 1.51 | 3.32 / 3.04 | -2.75 / 7.95 | 51 / 150 | 44→61 / 44→47 | 2.7 / 2.7 | 1216 / 1216 |

## copy vs clonefile

| location | K | cp -R wall s | cp -c -R wall s | speedup | cp -R workload CPU | clonefile workload CPU | cp -R overhead | clonefile overhead |
|---|---|---|---|---|---|---|---|---|
| indexed | 1 | 1.03 | 0.57 | 1.8× | 0.91 | 0.54 | 15.18 | 13.61 |
| indexed | 2 | 1.49 | 0.80 | 1.9× | 2.37 | 1.41 | 12.84 | 10.76 |
| noindex | 1 | 0.91 | 0.57 | 1.6× | 0.84 | 0.53 | 1.36 | 1.23 |
| noindex | 2 | 1.17 | 0.78 | 1.5× | 2.16 | 1.45 | 2.95 | 3.47 |

## Busiest background processes (all cases, summed)

| process | category | CPU s |
|---|---|---|
| mdworker_shared | indexer | 54.28 |
| coreaudiod | other | 33.10 |
| ReportCrash | other | 18.57 |
| WindowServer | desktop | 17.92 |
| fseventsd | fs-daemon | 16.95 |
| mds | indexer | 13.38 |
| Google Chrome Helper (Renderer) | browser | 12.47 |
| mds_stores | indexer | 10.75 |
| XprotectService | security | 9.91 |
| T3 Code (Nightly) Helper (Renderer) | agent-host | 9.09 |
| T3 Code (Nightly) Helper | agent-host | 3.90 |
| Google Chrome Helper | browser | 3.87 |
| Finder | desktop | 3.58 |
| Hex | other | 3.17 |
| Google Chrome | browser | 3.08 |

## What this shows

- **smallfiles**: indexed location took 2% more wall time vs noindex; OS overhead 7.01 vs 2.22 CPU-s (414% vs 130% of workload CPU); largest overhead category: indexer (5.95 CPU-s summed over cases).
- **spawn**: indexed location took 13% less wall time vs noindex; OS overhead 0.26 vs 1.14 CPU-s (32% vs 136% of workload CPU); largest overhead category: indexer (0.83 CPU-s summed over cases).
- **spawn-new-binary**: indexed location took 9% less wall time vs noindex; OS overhead 7.88 vs 5.69 CPU-s (809% vs 629% of workload CPU); largest overhead category: security (13.26 CPU-s summed over cases).
- **git**: indexed location took 4% more wall time vs noindex; OS overhead 26.55 vs 4.62 CPU-s (422% vs 75% of workload CPU); largest overhead category: indexer (25.21 CPU-s summed over cases).
- **copy-deps**: indexed location took 21% more wall time vs noindex; OS overhead 28.02 vs 4.30 CPU-s (856% vs 143% of workload CPU); largest overhead category: indexer (28.33 CPU-s summed over cases).
- **clonefile-deps**: indexed location took 1% more wall time vs noindex; OS overhead 24.37 vs 4.70 CPU-s (1251% vs 236% of workload CPU); largest overhead category: indexer (24.66 CPU-s summed over cases).
- Cloning deps with clonefile (`cp -c -R`) was 1.8× faster than a plain copy in the indexed location.
- Cloning deps with clonefile (`cp -c -R`) was 1.5× faster than a plain copy in the noindex location.
- smallfiles: throughput at K=2 is 1.7× K=1 (84% parallel efficiency).
- spawn: throughput at K=2 is 1.7× K=1 (85% parallel efficiency).
- spawn-new-binary: throughput at K=2 is 1.2× K=1 (62% parallel efficiency).
- git: throughput at K=2 is 1.6× K=1 (80% parallel efficiency).
- copy-deps: throughput at K=2 is 1.6× K=1 (78% parallel efficiency).
- clonefile-deps: throughput at K=2 is 1.5× K=1 (73% parallel efficiency).
- First exec of a never-seen binary took 517.34 ms vs 1.42 ms warm (system /usr/bin/true: 1.11 ms); security CPU 1.78 vs 0.29 CPU-s.
- Background processes classified as "other" that used noticeable CPU (classification candidates): coreaudiod (33.10s), ReportCrash (18.57s), Hex (3.17s), sandboxd (1.71s), duetexpertd (1.48s), iconservicesagent (0.97s), PerfPowerServices (0.94s), corespeechd (0.93s).

## Notes on this run

Second Quick run on the same MacBook Pro 13" M1 (8 cores, 8 GB, macOS 26.5.2,
on battery), about 40 minutes after the
[first](2026-10-10-m1-macbook-pro-quick.md). `coolant` built from source at
3981651, run with `coolant bench --preset quick`.

- **It reproduces the first run.** Indexed vs `.noindex` overhead: 15.2 vs 1.4
  CPU-s for `cp -R` (first run 16.3 / 1.6), 13.6 vs 1.2 for clonefile
  (11.2 / 1.1), and 22.2 vs 3.0 for git with 2 agents (15.3 / 2.8). Clonefile
  was again 1.5–1.9× faster than `cp -R`.
- **Security checks again cost about 1.8–3.9 CPU-s per 10 new binaries.** A
  first exec took 407–517 ms with 1 agent and 808–834 ms with 2 (first run:
  461–498 and 825–832 ms). New-binary throughput again barely scaled from 1 to
  2 agents (62% parallel efficiency; first run 57%).
- **The fan was running this time**, at about 1,210 rpm (its minimum) in every
  case. In the first run it read 0 rpm throughout.
- **Background noise was similar.** `coreaudiod` used 33 CPU-s and
  `ReportCrash` 19 CPU-s again; Chrome was open (19 CPU-s across its
  processes). The idle run in
  [2026-10-10-m1-macbook-pro-idle.md](2026-10-10-m1-macbook-pro-idle.md)
  explains most of this noise. Cells whose direction flipped against the first
  run, and should be read as noise: `spawn` (indexed now 13% faster),
  `spawn-new-binary` K=1 (indexed now 9% faster), and the K=1 `spawn` overhead
  (0.08 vs 0.47).
