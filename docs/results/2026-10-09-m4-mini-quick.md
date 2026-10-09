# coolant benchmark

Machine: Mac16,10, Apple M4, 10 cores, darwin 27.0.1. Median of 1 repetition(s).

Cells are `indexed / noindex`. CPU is in CPU-seconds over the workload plus the settle tail; categories exclude the benchmark's own processes. *Overhead* = indexer + security + fs-daemon + backup-sync; *overhead %* is relative to the workload's own CPU. *bg excess* is CPU used by everything else on the machine above its idle rate just before the case.

## smallfiles

create a node_modules-like tree of 3000 files (0.5–8 KB), stat-walk it, read every file, delete it; ops = file operations.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 0.32 / 0.32 | 37134 / 37341 | 0.29 / 0.28 | 3.92 / 0.40 | 1375.9 / 141.7 | 3.64 / 0.11 | 0.00 / 0.00 | 0.28 / 0.29 | 1.36 / 0.64 | 5.60 / 1.51 | 29 / 7 | 41→45 / 40→44 | 0.9 / 1.0 | 1001 / 1001 |
| 2 | 0.53 / 0.47 | 45202 / 51455 | 0.84 / 0.79 | 3.91 / 0.79 | 466.1 / 100.7 | 3.30 / 0.19 | 0.00 / 0.00 | 0.61 / 0.60 | 1.74 / 0.80 | 6.44 / 1.74 | 95 / 57 | 40→48 / 40→46 | 1.5 / 1.4 | 1001 / 1001 |

## spawn

exec /usr/bin/true 300 times sequentially; ops = execs.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 0.20 / 0.20 | 1507 / 1502 | 0.15 / 0.15 | 0.02 / 0.01 | 14.3 / 7.5 | 0.01 / 0.00 | 0.00 / 0.00 | 0.01 / 0.01 | 0.40 / 0.34 | -0.00 / 0.10 | 365 / 319 | 39→41 / 39→42 | 0.4 / 0.4 | 1001 / 1001 |
| 2 | 0.21 / 0.22 | 2828 / 2697 | 0.34 / 0.36 | 0.02 / 0.04 | 7.1 / 11.0 | 0.01 / 0.00 | 0.00 / 0.00 | 0.01 / 0.02 | 0.21 / 0.31 | 0.21 / 0.09 | 610 / 1051 | 38→41 / 39→42 | 0.4 / 0.4 | 1001 / 1001 |

| K | first exec ms | warm exec ms |
|---|---|---|
| 1 | 1.97 / 1.17 | 0.66 / 0.66 |
| 2 | 1.31 / 1.28 | 0.71 / 0.74 |

## spawn-new-binary

exec 10 freshly built, ad-hoc signed, never-seen binaries 300 times in total; ops = execs.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 5.48 / 5.07 | 55 / 59 | 0.25 / 0.33 | 0.43 / 0.38 | 173.5 / 114.9 | 0.01 / 0.00 | 0.40 / 0.35 | 0.01 / 0.01 | 1.16 / 1.32 | 0.94 / 0.85 | 325 / 717 | 38→41 / 38→41 | 0.3 / 0.3 | 1002 / 1002 |
| 2 | 5.34 / 5.07 | 112 / 118 | 0.74 / 0.80 | 0.70 / 0.62 | 94.8 / 78.4 | 0.01 / 0.00 | 0.64 / 0.61 | 0.03 / 0.01 | 1.83 / 1.28 | 2.53 / 2.26 | 803 / 1359 | 38→41 / 38→42 | 0.5 / 0.5 | 1001 / 1001 |

| K | first exec ms | warm exec ms |
|---|---|---|
| 1 | 489.90 / 448.98 | 2.00 / 1.99 |
| 2 | 471.01 / 449.63 | 1.71 / 1.97 |

## git

git init a 1500-file repo, add+commit, 20× git status (editing a file every 10), add and remove 2 worktrees; ops = git commands.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 1.34 / 1.30 | 20 / 21 | 1.19 / 1.20 | 3.99 / 1.16 | 336.2 / 96.8 | 2.54 / 0.53 | 0.01 / 0.01 | 0.69 / 0.62 | 0.64 / 0.00 | 4.05 / 1.99 | 277 / 279 | 38→45 / 39→46 | 2.2 / 1.9 | 1000 / 1001 |
| 2 | 1.69 / 1.61 | 32 / 34 | 3.23 / 3.15 | 12.90 / 2.15 | 399.8 / 68.4 | 9.05 / 0.91 | 0.01 / 0.02 | 1.31 / 1.22 | 0.07 / 0.00 | -6.35 / 3.35 | 636 / 835 | 40→51 / 39→49 | 3.8 / 3.2 | 1001 / 1001 |

## copy-deps

copy a 3000-file deps tree into a new worktree dir with cp -R; ops = files.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 0.63 / 0.58 | 4734 / 5156 | 0.56 / 0.54 | 20.85 / 0.71 | 3692.2 / 132.2 | 19.38 / 0.34 | 0.00 / 0.00 | 0.42 / 0.37 | 1.19 / 0.63 | 4.71 / 1.08 | 37 / 11 | 40→47 / 39→46 | 2.2 / 1.1 | 1001 / 1001 |
| 2 | 0.88 / 0.75 | 6828 / 8010 | 1.41 / 1.40 | 20.31 / 1.60 | 1445.1 / 114.3 | 19.49 / 0.86 | 0.00 / 0.00 | 0.83 / 0.74 | 1.00 / 0.73 | 0.46 / 1.75 | 236 / 115 | 40→48 / 39→47 | 2.9 / 1.7 | 1001 / 1001 |

## clonefile-deps

copy a 3000-file deps tree into a new worktree dir with cp -c -R (APFS clonefile); ops = files.

| K | wall s | ops/s | workload CPU | overhead CPU | overhead % | indexer | security | fs-daemon | other (bg) | bg excess | spawns | CPU °C start→max | CPU W | fan max |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 0.35 / 0.35 | 8508 / 8686 | 0.32 / 0.31 | 17.40 / 0.58 | 5504.5 / 189.9 | 14.97 / 0.12 | 0.00 / 0.01 | 0.47 / 0.45 | 1.32 / 0.56 | -1.38 / 1.84 | 37 / 349 | 39→43 / 39→46 | 1.7 / 1.9 | 1001 / 1001 |
| 2 | 0.45 / 0.44 | 13479 / 13608 | 0.81 / 0.82 | 19.56 / 1.23 | 2423.7 / 150.7 | 18.56 / 0.31 | 0.00 / 0.00 | 1.00 / 0.92 | 1.00 / 0.50 | -0.49 / 1.37 | 144 / 406 | 39→45 / 39→43 | 2.5 / 1.4 | 1001 / 1001 |

## copy vs clonefile

| location | K | cp -R wall s | cp -c -R wall s | speedup | cp -R workload CPU | clonefile workload CPU | cp -R overhead | clonefile overhead |
|---|---|---|---|---|---|---|---|---|
| indexed | 1 | 0.63 | 0.35 | 1.8× | 0.56 | 0.32 | 20.85 | 17.40 |
| indexed | 2 | 0.88 | 0.45 | 2.0× | 1.41 | 0.81 | 20.31 | 19.56 |
| noindex | 1 | 0.58 | 0.35 | 1.7× | 0.54 | 0.31 | 0.71 | 0.58 |
| noindex | 2 | 0.75 | 0.44 | 1.7× | 1.40 | 0.82 | 1.60 | 1.23 |

## Busiest background processes (all cases, summed)

| process | category | CPU s |
|---|---|---|
| mdworker_shared | indexer | 54.41 |
| com.apple.Virtualization.VirtualMachine | container-vm | 46.27 |
| mds_stores | indexer | 20.75 |
| mds | indexer | 16.28 |
| T3 Code (Nightly) Helper (Renderer) | agent-host | 12.48 |
| fseventsd | fs-daemon | 10.69 |
| WindowServer | other | 10.36 |
| T3 Code (Nightly) Helper | agent-host | 9.51 |
| node | runtime | 7.42 |
| com.docker.backend | container-vm | 6.70 |
| mediaanalysisd | backup-sync | 6.27 |
| contextstored | other | 3.30 |
| Finder | other | 2.02 |
| launchd | kernel | 1.72 |
| Docker Desktop Helper (Renderer) | container-vm | 1.33 |

## What this shows

- **smallfiles**: indexed location took 8% more wall time vs noindex; OS overhead 7.83 vs 1.19 CPU-s (697% vs 112% of workload CPU); largest overhead category: indexer (7.23 CPU-s summed over cases).
- **spawn**: indexed location took 3% less wall time vs noindex; OS overhead 0.05 vs 0.05 CPU-s (9% vs 10% of workload CPU); largest overhead category: fs-daemon (0.05 CPU-s summed over cases).
- **spawn-new-binary**: indexed location took 7% more wall time vs noindex; OS overhead 1.13 vs 1.00 CPU-s (114% vs 89% of workload CPU); largest overhead category: security (2.00 CPU-s summed over cases).
- **git**: indexed location took 4% more wall time vs noindex; OS overhead 16.89 vs 3.32 CPU-s (383% vs 76% of workload CPU); largest overhead category: indexer (13.03 CPU-s summed over cases).
- **copy-deps**: indexed location took 14% more wall time vs noindex; OS overhead 41.17 vs 2.32 CPU-s (2089% vs 119% of workload CPU); largest overhead category: indexer (40.07 CPU-s summed over cases).
- **clonefile-deps**: indexed location took 1% more wall time vs noindex; OS overhead 36.95 vs 1.82 CPU-s (3291% vs 161% of workload CPU); largest overhead category: indexer (33.96 CPU-s summed over cases).
- Cloning deps with clonefile (`cp -c -R`) was 1.9× faster than a plain copy in the indexed location.
- Cloning deps with clonefile (`cp -c -R`) was 1.7× faster than a plain copy in the noindex location.
- smallfiles: throughput at K=2 is 1.4× K=1 (69% parallel efficiency).
- spawn: throughput at K=2 is 1.8× K=1 (90% parallel efficiency).
- spawn-new-binary: throughput at K=2 is 2.0× K=1 (100% parallel efficiency).
- git: throughput at K=2 is 1.6× K=1 (81% parallel efficiency).
- copy-deps: throughput at K=2 is 1.6× K=1 (78% parallel efficiency).
- clonefile-deps: throughput at K=2 is 1.6× K=1 (78% parallel efficiency).
- First exec of a never-seen binary took 448.98 ms vs 1.99 ms warm (system /usr/bin/true: 0.66 ms); security CPU 0.35 vs 0.00 CPU-s.
- Background processes classified as "other" that used noticeable CPU (classification candidates): WindowServer (10.36s), contextstored (3.30s), Finder (2.02s), t3-resource-monitor (0.54s), loginwindow (0.35s), Dock (0.12s).
