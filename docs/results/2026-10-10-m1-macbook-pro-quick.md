# coolant benchmark

Machine: MacBookPro17,1, Apple M1, 8 cores, darwin 26.5.2. Median of 1 repetition(s).

Cells are `indexed / noindex`. CPU is in CPU-seconds over the workload plus the settle tail; categories exclude the benchmark's own processes. *Overhead* = indexer + security + fs-daemon + backup-sync; *overhead %* is relative to the workload's own CPU. *bg excess* is CPU used by everything else on the machine above its idle rate just before the case.

## smallfiles

create a node_modules-like tree of 3000 files (0.5–8 KB), stat-walk it, read every file, delete it; ops = file operations.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 0.55 / 0.64 | 21884 / 18771 | 0.52 / 0.56 | 2.96 / 0.75 | 569.7 / 134.2 | 2.46 / 0.17 | 0.02 / 0.02 | 0.46 / 0.55 | 3.32 / 5.64 | 6.45 / 3.47 | 238 / 317 | 51→54 / 58→73 | 1.6 / 4.6 | 0 / 0 |
| 2 | 0.76 / 0.85 | 31762 / 28378 | 1.26 / 1.39 | 4.10 / 1.37 | 325.0 / 98.5 | 3.50 / 0.24 | 0.04 / 0.01 | 0.56 / 1.12 | 5.43 / 5.56 | 8.11 / 4.27 | 1507 / 0 | 48→62 / 58→67 | 1.9 / 2.7 | 0 / 0 |

## spawn

exec /usr/bin/true 300 times sequentially; ops = execs.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 0.30 / 0.29 | 1011 / 1019 | 0.24 / 0.24 | 0.00 / 0.03 | 1.5 / 11.1 | 0.00 / 0.01 | 0.00 / 0.00 | 0.00 / 0.01 | 1.94 / 1.85 | -0.03 / 0.98 | 438 / 869 | 48→52 / 49→56 | 0.6 / 0.9 | 0 / 0 |
| 2 | 0.36 / 1.27 | 1687 / 471 | 0.60 / 0.67 | 0.12 / 0.20 | 20.5 / 29.8 | 0.01 / 0.09 | 0.10 / 0.09 | 0.01 / 0.02 | 3.26 / 3.68 | 1.24 / 2.29 | 630 / 984 | 55→64 / 50→65 | 2.5 / 3.2 | 0 / 0 |

| K | first exec ms | warm exec ms |
|---|---|---|
| 1 | 5.32 / 1.49 | 0.97 / 0.98 |
| 2 | 2.32 / 11.96 | 1.18 / 4.22 |

## spawn-new-binary

exec 10 freshly built, ad-hoc signed, never-seen binaries 300 times in total; ops = execs.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 5.44 / 5.06 | 55 / 59 | 0.27 / 0.23 | 1.98 / 1.88 | 724.9 / 830.8 | 0.05 / 0.01 | 1.91 / 1.84 | 0.01 / 0.02 | 4.89 / 4.42 | 0.80 / 0.19 | 439 / 749 | 50→62 / 45→47 | 0.9 / 0.8 | 0 / 0 |
| 2 | 8.97 / 8.91 | 67 / 67 | 0.50 / 0.52 | 3.92 / 3.85 | 788.5 / 739.4 | 0.03 / 0.01 | 3.86 / 3.81 | 0.02 / 0.02 | 7.07 / 6.50 | 5.51 / 7.60 | 2555 / 1673 | 42→51 / 43→58 | 1.1 / 1.1 | 0 / 0 |

| K | first exec ms | warm exec ms |
|---|---|---|
| 1 | 497.82 / 461.18 | 1.59 / 1.54 |
| 2 | 832.00 / 824.54 | 1.48 / 1.54 |

## git

git init a 1500-file repo, add+commit, 20× git status (editing a file every 10), add and remove 2 worktrees; ops = git commands.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 2.01 / 2.12 | 13 / 13 | 1.83 / 1.79 | 4.74 / 1.84 | 259.3 / 103.0 | 3.56 / 0.68 | 0.02 / 0.02 | 1.15 / 1.14 | 1.44 / 2.33 | 6.85 / 3.22 | 395 / 387 | 42→45 / 42→46 | 1.8 / 1.8 | 0 / 0 |
| 2 | 2.70 / 2.51 | 20 / 21 | 4.88 / 4.57 | 15.31 / 2.82 | 313.9 / 61.8 | 12.52 / 0.94 | 0.68 / 0.02 | 2.11 / 1.86 | 5.85 / 1.02 | 7.18 / 2.97 | 907 / 1038 | 49→67 / 48→64 | 6.3 / 3.2 | 0 / 0 |

## copy-deps

copy a 3000-file deps tree into a new worktree dir with cp -R; ops = files.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 1.11 / 1.03 | 2692 / 2908 | 0.93 / 0.86 | 16.33 / 1.62 | 1758.6 / 188.3 | 15.62 / 0.78 | 0.00 / 0.24 | 0.70 / 0.60 | 2.78 / 7.13 | -2.83 / 7.14 | 40 / 314 | 55→67 / 46→50 | 1.8 / 1.4 | 0 / 0 |
| 2 | 1.52 / 1.47 | 3941 / 4071 | 2.47 / 2.28 | 16.16 / 2.79 | 653.3 / 122.3 | 14.58 / 1.48 | 0.23 / 0.05 | 1.34 / 1.26 | 4.51 / 9.25 | 1.26 / 5.24 | 334 / 306 | 54→61 / 43→53 | 4.7 / 3.3 | 0 / 0 |

## clonefile-deps

copy a 3000-file deps tree into a new worktree dir with cp -c -R (APFS clonefile); ops = files.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 0.63 / 0.62 | 4729 / 4877 | 0.60 / 0.55 | 11.23 / 1.10 | 1871.2 / 198.9 | 10.19 / 0.29 | 0.06 / 0.06 | 0.72 / 0.70 | 5.72 / 6.51 | -0.53 / 0.11 | 49 / 9 | 53→61 / 44→47 | 1.5 / 1.0 | 0 / 0 |
| 2 | 0.79 / 0.76 | 7601 / 7851 | 1.42 / 1.39 | 11.93 / 2.09 | 839.7 / 150.1 | 10.37 / 0.52 | 0.10 / 0.03 | 1.43 / 1.53 | 5.30 / 1.96 | 0.49 / 2.48 | 158 / 185 | 43→53 / 44→50 | 2.4 / 1.8 | 0 / 0 |

## copy vs clonefile

| location | K | cp -R wall s | cp -c -R wall s | speedup | cp -R workload CPU | clonefile workload CPU | cp -R overhead | clonefile overhead |
|---|---|---|---|---|---|---|---|---|
| indexed | 1 | 1.11 | 0.63 | 1.8× | 0.93 | 0.60 | 16.33 | 11.23 |
| indexed | 2 | 1.52 | 0.79 | 1.9× | 2.47 | 1.42 | 16.16 | 11.93 |
| noindex | 1 | 1.03 | 0.62 | 1.7× | 0.86 | 0.55 | 1.62 | 1.10 |
| noindex | 2 | 1.47 | 0.76 | 1.9× | 2.28 | 1.39 | 2.79 | 2.09 |

## Busiest background processes (all cases, summed)

| process | category | CPU s |
|---|---|---|
| mdworker_shared | indexer | 47.76 |
| coreaudiod | other | 34.80 |
| ReportCrash | other | 20.18 |
| fseventsd | fs-daemon | 17.01 |
| mds | indexer | 13.35 |
| XprotectService | security | 10.17 |
| mobileassetd | other | 9.47 |
| mds_stores | indexer | 9.33 |
| WindowServer | desktop | 9.12 |
| coolant-app | other | 5.14 |
| T3 Code (Nightly) | agent-host | 4.50 |
| Finder | desktop | 4.27 |
| T3 Code (Nightly) Helper (Renderer) | agent-host | 3.57 |
| spindump | other | 3.01 |
| BackgroundShortcutRunner | other | 2.95 |

## What this shows

- **smallfiles**: indexed location took 12% less wall time vs noindex; OS overhead 7.06 vs 2.13 CPU-s (396% vs 109% of workload CPU); largest overhead category: indexer (6.37 CPU-s summed over cases).
- **spawn**: indexed location took 58% less wall time vs noindex; OS overhead 0.13 vs 0.23 CPU-s (15% vs 25% of workload CPU); largest overhead category: security (0.19 CPU-s summed over cases).
- **spawn-new-binary**: indexed location took 3% more wall time vs noindex; OS overhead 5.89 vs 5.72 CPU-s (766% vs 767% of workload CPU); largest overhead category: security (11.42 CPU-s summed over cases).
- **git**: indexed location took 1% more wall time vs noindex; OS overhead 20.06 vs 4.67 CPU-s (299% vs 73% of workload CPU); largest overhead category: indexer (17.71 CPU-s summed over cases).
- **copy-deps**: indexed location took 5% more wall time vs noindex; OS overhead 32.49 vs 4.42 CPU-s (955% vs 140% of workload CPU); largest overhead category: indexer (32.46 CPU-s summed over cases).
- **clonefile-deps**: indexed location took 3% more wall time vs noindex; OS overhead 23.16 vs 3.19 CPU-s (1146% vs 164% of workload CPU); largest overhead category: indexer (21.38 CPU-s summed over cases).
- Cloning deps with clonefile (`cp -c -R`) was 1.9× faster than a plain copy in the indexed location.
- Cloning deps with clonefile (`cp -c -R`) was 1.8× faster than a plain copy in the noindex location.
- smallfiles: throughput at K=2 is 1.5× K=1 (76% parallel efficiency).
- spawn: throughput at K=2 is 0.5× K=1 (23% parallel efficiency).
- spawn-new-binary: throughput at K=2 is 1.1× K=1 (57% parallel efficiency).
- git: throughput at K=2 is 1.7× K=1 (85% parallel efficiency).
- copy-deps: throughput at K=2 is 1.4× K=1 (70% parallel efficiency).
- clonefile-deps: throughput at K=2 is 1.6× K=1 (80% parallel efficiency).
- First exec of a never-seen binary took 461.18 ms vs 1.54 ms warm (system /usr/bin/true: 0.98 ms); security CPU 1.84 vs 0.00 CPU-s.
- Background processes classified as "other" that used noticeable CPU (classification candidates): coreaudiod (34.80s), ReportCrash (20.18s), mobileassetd (9.47s), coolant-app (5.14s), spindump (3.01s), BackgroundShortcutRunner (2.95s), siriactionsd (1.59s), corespeechd (1.32s).

## Notes on this run

MacBook Pro 13" M1 (8 cores, 8 GB, one fan, which stayed off: 0 rpm throughout), macOS 26.5.2, app v0.2.0-beta.1
installed with `scripts/install.sh`, run with `coolant bench --preset quick`.

- **The Spotlight gap reproduces.** After copying a 3,000-file tree the indexed
  folder cost 16.3 CPU-s of overhead against 1.6 in `.noindex` (M4: 20.9 / 0.7);
  clonefile 11.2 / 1.1; git with 2 agents 15.3 / 2.8. Wall time differed by
  only 1–5%.
- **Security checks cost more CPU on M1.** Each set of 10 never-seen binaries
  cost about 1.9 CPU-s in security daemons (M4: 0.35–0.40). With 2 agents the
  first exec took about 830 ms (M4: about 470 ms), and new-binary throughput
  barely scaled (55 → 67 ops/s, 57% parallel efficiency; M4: 100%). The M4
  ran macOS 27, so the OS version may account for part of this.
- **The machine was noisy.** `coreaudiod` used 35 CPU-s and kept using about a
  quarter of a core at idle. `ReportCrash` used 20 CPU-s writing reports for
  crash-looping Homebrew services (`php-fpm`, `mysqld`, `watchman`). "other (bg)"
  ran at 2–9 CPU-s per case vs about 1 on the M4. Treat these cells as outliers:
  `spawn` at K=2 in `.noindex` (1.27 s, which produces the "58% less wall time"
  line above) and the indexed `smallfiles` run beating `.noindex`. This run had
  one repetition; `--preset full` takes the median of 3.
