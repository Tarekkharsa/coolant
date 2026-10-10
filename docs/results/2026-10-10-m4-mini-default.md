# coolant benchmark

Machine: Mac16,10, Apple M4, 10 cores, darwin 27.0.1. Median of 1 repetition(s).

Cells are `indexed / noindex`. CPU is in CPU-seconds over the workload plus the settle tail; categories exclude the benchmark's own processes. *Overhead* = indexer + security + fs-daemon + backup-sync; *overhead %* is relative to the workload's own CPU. *bg excess* is CPU used by everything else on the machine above its idle rate just before the case.

## smallfiles

create a node_modules-like tree of 20000 files (0.5–8 KB), stat-walk it, read every file, delete it; ops = file operations.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 2.29 / 2.14 | 34996 / 37431 | 2.13 / 2.04 | 12.37 / 2.75 | 581.0 / 134.6 | 9.94 / 0.68 | 0.00 / 0.07 | 1.98 / 1.99 | 0.58 / 0.33 | 19.33 / 5.71 | 126 / 152 | 36→43 / 35→42 | 2.0 / 1.6 | 1001 / 1001 |
| 2 | 2.96 / 3.00 | 54016 / 53246 | 5.53 / 5.69 | 17.89 / 5.17 | 323.7 / 91.0 | 13.39 / 1.27 | 0.00 / 0.01 | 3.96 / 3.88 | 0.57 / 0.24 | 29.17 / 6.32 | 135 / 20 | 37→46 / 36→46 | 3.1 / 2.7 | 1001 / 1001 |
| 4 | 4.90 / 4.63 | 65346 / 69111 | 17.01 / 16.29 | 28.08 / 10.30 | 165.1 / 63.2 | 20.11 / 2.48 | 0.00 / 0.01 | 7.97 / 7.78 | 0.50 / 0.78 | 32.48 / 7.91 | 179 / 107 | 37→50 / 38→50 | 5.2 / 4.7 | 1001 / 1001 |

## spawn

exec /usr/bin/true 2000 times sequentially; ops = execs.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 1.30 / 1.29 | 1544 / 1549 | 1.07 / 1.06 | 0.07 / 0.06 | 6.2 / 5.9 | 0.01 / 0.02 | 0.01 / 0.00 | 0.04 / 0.04 | 0.00 / 0.00 | 0.68 / 0.50 | 2093 / 2134 | 37→43 / 37→43 | 0.3 / 0.4 | 1000 / 1001 |
| 2 | 1.38 / 1.38 | 2901 / 2890 | 2.32 / 2.32 | 0.03 / 0.03 | 1.2 / 1.2 | 0.00 / 0.01 | 0.01 / 0.00 | 0.01 / 0.01 | 0.00 / 0.00 | 0.33 / 1.84 | 4029 / 4029 | 37→46 / 37→45 | 0.4 / 0.4 | 1001 / 1001 |
| 4 | 1.49 / 1.49 | 5366 / 5386 | 5.10 / 5.09 | 0.03 / 0.04 | 0.5 / 0.7 | 0.01 / 0.02 | 0.00 / 0.00 | 0.01 / 0.01 | 0.00 / 0.00 | 0.93 / 1.66 | 8088 / 8094 | 37→46 / 37→47 | 0.5 / 0.6 | 1001 / 1001 |

| K | first exec ms | warm exec ms |
|---|---|---|
| 1 | 1.19 / 2.88 | 0.65 / 0.64 |
| 2 | 1.24 / 1.13 | 0.69 / 0.69 |
| 4 | 1.22 / 1.38 | 0.74 / 0.74 |

## spawn-new-binary

exec 20 freshly built, ad-hoc signed, never-seen binaries 2000 times in total; ops = execs.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 8.99 / 8.95 | 223 / 224 | 1.73 / 1.69 | 0.74 / 0.69 | 42.9 / 40.7 | 0.02 / 0.02 | 0.69 / 0.63 | 0.02 / 0.01 | 0.44 / 0.42 | 0.51 / -6.26 | 2031 / 2093 | 35→39 / 35→39 | 0.3 / 0.3 | 1001 / 1001 |
| 2 | 8.82 / 8.80 | 453 / 455 | 3.87 / 3.86 | 0.99 / 1.08 | 25.5 / 27.9 | 0.00 / 0.08 | 0.97 / 0.94 | 0.01 / 0.04 | 0.37 / 0.64 | 0.19 / 2.66 | 4140 / 4107 | 35→39 / 35→40 | 0.3 / 0.4 | 1001 / 1002 |
| 4 | 9.06 / 8.85 | 883 / 904 | 8.22 / 8.37 | 1.57 / 1.58 | 19.1 / 18.9 | 0.02 / 0.01 | 1.52 / 1.55 | 0.02 / 0.01 | 0.23 / 0.31 | 1.24 / 5.69 | 8102 / 8190 | 35→42 / 35→41 | 0.5 / 0.5 | 1001 / 1001 |

| K | first exec ms | warm exec ms |
|---|---|---|
| 1 | 324.09 / 322.01 | 1.27 / 1.27 |
| 2 | 313.86 / 311.86 | 1.28 / 1.29 |
| 4 | 323.00 / 310.79 | 1.31 / 1.33 |

## git

git init a 10000-file repo, add+commit, 50× git status (editing a file every 10), add and remove 3 worktrees; ops = git commands.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 10.17 / 9.56 | 6 / 6 | 12.07 / 11.97 | 20.43 / 8.71 | 169.3 / 72.8 | 14.72 / 3.46 | 0.02 / 0.04 | 5.21 / 5.16 | 0.00 / 0.00 | 28.16 / 7.93 | 540 / 520 | 37→49 / 36→47 | 4.0 / 3.9 | 1002 / 1001 |
| 2 | 14.59 / 14.44 | 8 / 8 | 29.84 / 30.49 | 69.87 / 16.15 | 234.2 / 53.0 | 60.25 / 6.41 | 0.04 / 0.03 | 9.57 / 9.69 | 0.00 / 0.00 | -9.41 / 28.73 | 1062 / 1009 | 44→62 / 47→64 | 10.5 / 9.6 | 1001 / 1001 |
| 4 | 24.27 / 24.43 | 10 / 10 | 85.10 / 86.48 | 95.43 / 29.12 | 112.1 / 33.7 | 76.13 / 10.72 | 0.07 / 0.06 | 19.21 / 18.30 | 0.00 / 0.00 | -23.34 / 62.15 | 2126 / 1877 | 52→73 / 48→69 | 12.5 / 11.5 | 1001 / 1001 |

## copy-deps

copy a 20000-file deps tree into a new worktree dir with cp -R; ops = files.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 5.42 / 5.45 | 3691 / 3672 | 4.39 / 4.31 | 50.10 / 6.03 | 1141.3 / 139.8 | 47.18 / 3.39 | 0.18 / 0.00 | 2.68 / 2.63 | 2.88 / 0.37 | 5.71 / 6.36 | 707 / 90 | 53→67 / 51→63 | 8.2 / 6.5 | 1001 / 1001 |
| 2 | 5.73 / 6.52 | 6981 / 6133 | 9.07 / 10.40 | 59.76 / 12.60 | 659.0 / 121.1 | 54.59 / 7.59 | 0.00 / 0.01 | 5.16 / 4.98 | 2.03 / 0.30 | -0.17 / 12.04 | 252 / 27 | 48→61 / 51→66 | 4.2 / 7.9 | 1001 / 1001 |
| 4 | 7.30 / 6.15 | 10958 / 13009 | 21.24 / 20.88 | 71.49 / 17.62 | 336.6 / 84.4 | 61.30 / 7.94 | 0.02 / 0.00 | 10.14 / 9.67 | 1.87 / 0.56 | -1.62 / 16.45 | 306 / 36 | 46→62 / 44→60 | 6.8 / 6.4 | 1001 / 1001 |

## clonefile-deps

copy a 20000-file deps tree into a new worktree dir with cp -c -R (APFS clonefile); ops = files.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 2.46 / 2.30 | 8115 / 8692 | 2.33 / 2.17 | 46.34 / 3.22 | 1987.4 / 148.3 | 43.30 / 0.62 | 0.01 / 0.04 | 3.01 / 2.55 | 2.40 / 0.46 | -2.99 / 8.24 | 122 / 369 | 45→54 / 42→48 | 2.7 / 1.8 | 1001 / 1001 |
| 2 | 3.05 / 2.96 | 13112 / 13527 | 5.78 / 5.62 | 52.99 / 7.73 | 916.3 / 137.5 | 46.77 / 2.05 | 0.02 / 0.00 | 6.19 / 5.67 | 1.04 / 0.33 | 1.32 / 9.53 | 111 / 17 | 44→56 / 42→53 | 3.7 / 3.1 | 1001 / 1000 |
| 4 | 4.05 / 4.07 | 19738 / 19667 | 14.87 / 15.01 | 50.47 / 13.64 | 339.5 / 90.9 | 39.50 / 2.38 | 0.01 / 0.01 | 10.93 / 11.25 | 5.09 / 0.29 | -2.55 / 17.69 | 136 / 93 | 43→59 / 42→56 | 5.6 / 4.8 | 1001 / 1001 |

## copy vs clonefile

| location | K | cp -R wall s | cp -c -R wall s | speedup | cp -R workload CPU | clonefile workload CPU | cp -R overhead | clonefile overhead |
|---|---|---|---|---|---|---|---|---|
| indexed | 1 | 5.42 | 2.46 | 2.2× | 4.39 | 2.33 | 50.10 | 46.34 |
| indexed | 2 | 5.73 | 3.05 | 1.9× | 9.07 | 5.78 | 59.76 | 52.99 |
| indexed | 4 | 7.30 | 4.05 | 1.8× | 21.24 | 14.87 | 71.49 | 50.47 |
| noindex | 1 | 5.45 | 2.30 | 2.4× | 4.31 | 2.17 | 6.03 | 3.22 |
| noindex | 2 | 6.52 | 2.96 | 2.2× | 10.40 | 5.62 | 12.60 | 7.73 |
| noindex | 4 | 6.15 | 4.07 | 1.5× | 20.88 | 15.01 | 17.62 | 13.64 |

## Busiest background processes (all cases, summed)

| process | category | CPU s |
|---|---|---|
| com.apple.Virtualization.VirtualMachine | container-vm | 427.58 |
| fseventsd | fs-daemon | 168.40 |
| mdworker_shared | indexer | 153.85 |
| mds | indexer | 132.58 |
| mds_stores | indexer | 110.84 |
| Finder | desktop | 37.56 |
| contextstored | desktop | 25.32 |
| com.docker.backend | container-vm | 18.22 |
| node | runtime | 17.32 |
| WindowServer | desktop | 16.12 |
| T3 Code (Nightly) Helper | agent-host | 9.93 |
| launchd | kernel | 6.70 |
| claude | agent | 6.03 |
| Docker Desktop Helper (Renderer) | container-vm | 5.75 |
| syspolicyd | security | 5.01 |

## What this shows

- **smallfiles**: indexed location took 4% more wall time vs noindex; OS overhead 58.34 vs 18.23 CPU-s (237% vs 76% of workload CPU); largest overhead category: indexer (47.87 CPU-s summed over cases).
- **spawn**: indexed location took the same wall time vs noindex; OS overhead 0.12 vs 0.13 CPU-s (1% vs 1% of workload CPU); largest overhead category: fs-daemon (0.12 CPU-s summed over cases).
- **spawn-new-binary**: indexed location took 1% more wall time vs noindex; OS overhead 3.30 vs 3.34 CPU-s (24% vs 24% of workload CPU); largest overhead category: security (6.30 CPU-s summed over cases).
- **git**: indexed location took 1% more wall time vs noindex; OS overhead 185.74 vs 53.98 CPU-s (146% vs 42% of workload CPU); largest overhead category: indexer (171.69 CPU-s summed over cases).
- **copy-deps**: indexed location took 2% more wall time vs noindex; OS overhead 181.35 vs 36.24 CPU-s (523% vs 102% of workload CPU); largest overhead category: indexer (182.00 CPU-s summed over cases).
- **clonefile-deps**: indexed location took 3% more wall time vs noindex; OS overhead 149.80 vs 24.59 CPU-s (652% vs 108% of workload CPU); largest overhead category: indexer (134.62 CPU-s summed over cases).
- Cloning deps with clonefile (`cp -c -R`) was 1.9× faster than a plain copy in the indexed location.
- Cloning deps with clonefile (`cp -c -R`) was 1.9× faster than a plain copy in the noindex location.
- smallfiles: throughput at K=4 is 1.8× K=1 (46% parallel efficiency).
- spawn: throughput at K=4 is 3.5× K=1 (87% parallel efficiency).
- spawn-new-binary: throughput at K=4 is 4.0× K=1 (101% parallel efficiency).
- git: throughput at K=4 is 1.6× K=1 (39% parallel efficiency).
- copy-deps: throughput at K=4 is 3.5× K=1 (89% parallel efficiency).
- clonefile-deps: throughput at K=4 is 2.3× K=1 (57% parallel efficiency).
- First exec of a never-seen binary took 322.01 ms vs 1.27 ms warm (system /usr/bin/true: 0.64 ms); security CPU 0.63 vs 0.00 CPU-s.
- Background processes classified as "other" that used noticeable CPU (classification candidates): coreservicesd (3.54s), t3-resource-monitor (1.23s), opendirectoryd (0.92s), mobileassetd (0.75s), logd (0.35s), eligibilityd (0.26s), sysmond (0.18s), contactsd (0.17s).
