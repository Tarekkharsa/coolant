# The M2 Pro with an agent at work, after the overcount fix

Same MacBook Pro 14" M2 Pro (10 cores, 16 GB, macOS 27.0, CrowdStrike Falcon)
as the [first busy profile](2026-10-11-m2-pro-macbook-pro-busy.md), 2.4 days
since boot. `coolant profile -d 120` while one Claude Code session (with its
subagents) ran type checks, linters, a build and a headless browser in a
TypeScript monorepo. `coolant` built from source at 9db5a19 (0.2.0-beta.9);
the Coolant app 0.2.0-beta.9 was running.

**8.75 cores were busy on average.** The categories no longer add up to more
than the machine used, but one sample is still too high (finding 1).

```
coolant profile: Mac14,9 Apple M2 Pro (16.0 GB, 10 cores), darwin 27.0
120s recorded, 8.75 cores busy on average

Thermals: CPU 55°C → max 62°C (avg 58°C), CPU power avg 16.5 W / max 23.4 W, SoC avg 16.7 W, fan max 3045 rpm
Memory: at least 38% available, swap 0 B

Findings
  [high] File system daemons are using 0.75 cores
      fseventsd and friends fan out change events to every watcher; many agents plus editors watching the same trees multiply this.
  [high] 68 processes launched per second
      Every launch costs fork/exec and dynamic linking, plus code-signing checks on macOS. Short-lived git/rg/node processes add up. 82% exited too quickly to be named; they're counted under "other".
  [medium] Spotlight indexing is using 0.38 cores
      mds_stores/mdworker re-index files as agents write them: dependencies, build output, worktrees.
      fix: Keep agent worktrees in a folder ending in .noindex, or add them under System Settings > Spotlight > Search Privacy.
  [medium] Memory pressure was warning for 24s
      macOS was short of memory: compressing, swapping and asking apps to free memory. As little as 38% was available; swap went from 0 B to 0 B.
      fix: Fewer parallel builds and type checkers (tsc, tsgolint, vite each take gigabytes), or fewer agent sessions at once.

Where the CPU went
  category         cores  share  disk written  spawns
  runtime           1.81    21%        403 MB     406
  other             1.43    16%          2 MB    7051
  browser           1.33    15%          2 MB      65
  build             1.10    13%         16 MB       7
  fs-daemon *       0.75     9%           0 B       0
  indexer *         0.38     4%        568 KB      17
  desktop           0.33     4%           0 B       0
  vcs               0.20     2%          3 MB     531
  agent-host        0.09     1%          4 MB       4
  agent             0.08     1%          1 MB       0
  security *        0.04     0%           0 B       4
  crash-reporting   0.03     0%           0 B       0
  kernel            0.02     0%           0 B       0
  search            0.01     0%           0 B      74
  package-manager   0.01     0%           0 B      29
  audio             0.00     0%           0 B       0
  backup-sync *     0.00     0%           0 B       0
  unattributed      1.15    13%
  * OS overhead triggered by file and process activity

Agent sessions (agent + everything it launched)
  session       cores  disk written  spawns                       top categories
  claude:70867   4.40        423 MB     550  runtime 41%, browser 29%, build 25%
  claude:8765    0.10         16 KB      79                 other 68%, agent 32%

Top processes
  pid                     name   category  cores  memory       session
  77601            coolant-app      other   0.77   97 MB
  353                fseventsd  fs-daemon   0.75   10 MB
  18600                   node    runtime   0.30  1.5 GB  claude:70867
  18531                   node    runtime   0.30  2.0 GB  claude:70867
  18600                   node    runtime   0.29  2.0 GB  claude:70867
  495               mds_stores    indexer   0.22   54 MB
  18932                    tsc      build   0.19  2.3 GB  claude:70867
  420             WindowServer    desktop   0.18   69 MB
  15960               tsgolint      build   0.17  2.5 GB  claude:70867
  16481                    tsc      build   0.17  2.5 GB  claude:70867
  21150  chrome-headless-shell    browser   0.17  130 MB  claude:70867
  15355                    tsc      build   0.16  2.6 GB  claude:70867
  17791               tsgolint      build   0.16  2.5 GB  claude:70867
  54471        Raycast Backend      other   0.14  205 MB
  378                      mds    indexer   0.14   17 MB
```

## 1. The overcount fix works, with one leftover

In the first busy profile the categories summed to 1,972 CPU-s against 813
measured. Now they sum to 912.7 CPU-s against 1,050.8 measured.
("Unattributed", 138.1 CPU-s, is the difference between the two, so the table
always adds up.)

A single sample can still be too high. At 78 s the `runtime` row got
36.5 cores in one second on a 10-core Mac:

| t (s) | busy cores (host) | `runtime` |
|---|---|---|
| 77.1 | 8.51 | 3.3 |
| 78.1 | 8.62 | **36.5** |
| 79.1 | 8.51 | 1.8 |

The 36.5 is exactly the lifetime CPU of one `node` process, PID 18600, which
appears twice in the top-processes table: once with its full command line
(35.0 CPU-s, 2.0 GB) and once as plain `node` (36.5 CPU-s, 1.5 GB). A process
is identified by `(pid, start_ms)` (`crates/coolant-core/src/profile.rs`,
`type Key`), and `start_ms` is 0 when `proc_pid_rusage` fails. For one sample
the details of PID 18600 could not be read, probably because it was exiting,
so it got a second key, `(18600, 0)`, and its whole lifetime CPU was counted
again as a new process. The per-sample cap from the fix covers only the CPU
of children that came and went, not newly seen processes, so this got through.
The `runtime` row is about 31 CPU-s too high and "unattributed" about 31 CPU-s
too low.

**Fix suggestion:** when a process has no `start_ms`, match it to the
previous sample's entry with the same PID instead of treating it as new; and
apply the same cap per sample to the CPU of processes seen for the first
time.

## 2. The Coolant app was cleaning up a worktree

The busiest process was `coolant-app` at 0.77 cores (92 CPU-s in 2 minutes),
classified as "other". It was not idle: `sample` showed its time in
`remove_dir_all` (`__unlinkat`, `__getdirentries64`), and `lsof` showed it
inside the `node_modules/.pnpm` tree of an idle worktree. It had been running
for more than 6 minutes and was still at 70–74% of a core after the profile.

The sweep deletes in place at normal priority (`sweep.rs`, line 97). Deleting
a large `node_modules` makes `fseventsd` send an event for every file and
Spotlight update its index, which likely accounts for most of the 0.75 cores
of `fseventsd` and part of the 0.38 cores of indexing. On this Mac, Falcon
also checks each file operation (see finding 6 in the findings).

## 3. Memory pressure, without swap

macOS reported memory pressure "warning" for 24 s. At the lowest point 38% of
memory was available, and swap stayed at 0 B. The session ran three `tsc`
(2.3–2.6 GB each), two `tsgolint` (2.5 GB each) and `node` processes of
1.5–2.0 GB at the same time.

## 4. Other observations

- **The Mac stayed warm.** CPU 55–62 °C, 16.5 W on average (23.4 W at most),
  fans up to 3,045 rpm (first busy profile: 4,915 rpm).
- **68 process launches per second**, 82% of them too short-lived to be named.
- **A headless browser.** `chrome-headless-shell`, started by the session for
  browser tests, put 1.33 cores under `browser`.
- **Only one session was working.** The other `claude` process was the session
  running this profile. The Claude in Chrome bridge was not running this time.

## Suggestions for coolant

- **profile: key processes without a start time by PID** (finding 1), and cap
  the CPU of newly seen processes per sample.
- **sweep: delete at background priority.** Rename the folder out of the
  worktree first (instant), then delete it on a thread with background QoS
  (`setpriority(PRIO_DARWIN_THREAD, …, PRIO_DARWIN_BG)` or a
  `QOS_CLASS_BACKGROUND` queue), so a large clean-up doesn't compete with the
  agents and heat the Mac.
- **classify:** put `coolant-app` and `coolant` in a category of their own
  (or leave them out), so the tool doesn't show up as "other".
- **report:** the memory finding says "swap went from 0 B to 0 B" and
  "compressing, swapping" when swap never grew; say "no swap was used" in
  that case.
