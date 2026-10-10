# What an idle MacBook Pro M1 was doing

MacBook Pro 13" M1 (8 cores, 8 GB, one fan), macOS 26.5.2, on battery (56%),
12.8 days since boot. `coolant profile -d 120` with no agent working and no
benchmark running, between the [second Quick run](2026-10-10-m1-macbook-pro-quick-rerun.md)
and a Full run. `coolant` built from source at 3981651.

**1.63 cores were busy on average with nothing to do.** Almost none of it
was Spotlight or security (2% together). It was mostly three things, none of
them agent work.

```
coolant profile: MacBookPro17,1 Apple M1 (8.0 GB, 8 cores), darwin 26.5.2
120s recorded, 1.63 cores busy on average

Thermals: CPU 42°C → max 58°C (avg 44°C), CPU power avg 1.8 W / max 6.7 W, SoC avg 3.2 W, fan max 1218 rpm

Findings
  [medium] 19 processes launched per second
      Every launch costs fork/exec and dynamic linking, plus code-signing checks on macOS. Short-lived git/rg/node processes add up.

Where the CPU went
  category       cores  share  disk written  spawns
  other           0.69    42%          3 MB    2238
  desktop         0.21    13%           0 B       0
  vcs             0.19    12%           0 B      23
  agent-host      0.19    12%         16 MB       0
  browser         0.09     6%          5 MB       5
  agent           0.02     1%        236 KB       0
  indexer *       0.02     1%          3 MB       3
  kernel          0.01     1%           0 B       1
  security *      0.01     1%           0 B       0
  fs-daemon *     0.00     0%           0 B       1
  runtime         0.00     0%         24 KB      35
  backup-sync *   0.00     0%           0 B       0
  search          0.00     0%           0 B       7
  unattributed    0.19    12%
  * OS overhead triggered by file and process activity

Agent sessions (agent + everything it launched)
  session       cores  disk written  spawns        top categories
  claude:31989   0.05         68 KB      16  other 72%, agent 28%
  claude:1799    0.01        168 KB       0            agent 100%

Top processes
  pid                                name    category  cores  memory  session
  444                          coreaudiod       other   0.24   29 MB
  434                        WindowServer     desktop   0.19   56 MB
  2726                        ReportCrash       other   0.11  119 MB
  48289  T3 Code (Nightly) Helper (Rende…  agent-host   0.11  442 MB
  49979                               Hex       other   0.09  101 MB
  1039                                git         vcs   0.07  202 MB
  2232                                git         vcs   0.06  185 MB
  2728                                git         vcs   0.06  115 MB
  9137           T3 Code (Nightly) Helper  agent-host   0.04   56 MB
  58858                          sandboxd       other   0.03    5 MB
  621                         ReportCrash       other   0.03   11 MB
  20355                       duetexpertd       other   0.02   42 MB
  683                       Google Chrome     browser   0.02  258 MB
  9500                  T3 Code (Nightly)  agent-host   0.02  616 MB
  15396   Google Chrome Helper (Renderer)     browser   0.02   73 MB
```

## 1. The home folder is a git repository, and the agent host scans all of it

`~` contains a `.git` with one commit (`first commit`, 2025-06-26), so every
folder under it, including every project not in a repository of its own, is
"inside" a 1-million-file working tree. T3 Code polls the git state of the
project a thread works in, and for a folder under `~` that means running

```
git ls-files --others --exclude-standard -z --      # cwd: /Users/tarek
```

at the root of the home folder. Measured by hand, one run took **20.2 s wall
and 8.2 CPU-s** (1.7 user, 6.6 sys), listed 1,045,980 untracked files, and hit
TCC "Operation not permitted" on 141 protected folders on the way. A
poll over 90 s saw 3 separate runs, so this alone keeps roughly a quarter of a
core busy, and each run holds about 200 MB of memory on an 8 GB Mac. The same command in this repository
takes 0.01 s. In the profile it shows as the three `git` processes under the agent
host: 23 CPU-s and 854 MB read from disk in 120 s (category `vcs`).

The fix is to remove or move the stray repository (`mv ~/.git ~/home-git-backup`
after checking nothing in it is needed). A `git status` from any tool in any
folder under `~` that isn't its own repository has the same cost.

## 2. `coreaudiod` uses a quarter of a core

`coreaudiod` was the busiest process: 0.24 cores at idle (28.6 CPU-s in
120 s), 33–35 CPU-s in each Quick benchmark, and 696 CPU-minutes since boot.
Nothing was playing. The default input is the built-in microphone, and two
HAL plug-ins load into the audio server: Apple's `ParrotAudioPlugin` and
`KrispAudio.driver` 1.44.1 from 2022, whose app is no longer installed. Their
helper processes use almost no CPU, so the cost is in `coreaudiod` itself.

The likely client is Hex, a dictation app that has been running for 2 days
(0.09 cores of its own): 2 days at 22% of a core is about 634 CPU-minutes, close
to the 696 since boot, which suggests `coreaudiod` was nearly idle before Hex
started. An app that keeps the microphone open keeps the audio engine running.
This is circumstantial; we did not quit Hex to confirm it.

## 3. Crash loops

`ReportCrash` used 0.14 cores (13.1 + 3.4 CPU-s in two processes) at idle and
19–20 CPU-s in each Quick run. `coolant doctor` lists the cause:

```
[ warn ] php-fpm crashed 49 times in the last 24 hours
[ warn ] mysqld crashed 24 times in the last 24 hours
[ warn ] watchman crashed 24 times in the last 24 hours
```

The `.ips` file names show crashes every few minutes for days (php-fpm 99
reports, mysqld 76, watchman 126 in `~/Library/Logs/DiagnosticReports`).

## Other observations

- **19 process launches per second at idle**, in bursts of 20–400 every 10–20 s
  rather than a steady stream, almost all attributed to "other" (2,238 of
  2,313 spawns), so the profile can't say what launched them.
- **The fan ran at its minimum (about 1,200 rpm) throughout,** at a CPU of
  42–58 °C. In the first Quick run an hour earlier it read 0 rpm.
- **Disk:** 13 GB free of 245 GB (94% used). Fine for the benchmarks, but a
  cold debug build of the GUI app plus a prewarmed copy would come close.
- `coolant sweep` found nothing to clean: no linked worktrees on this machine.

## Suggestions for coolant

From this machine:

- **doctor: flag a git repository at `$HOME`** (or at `/`, `~/Documents`, …
  with a very large untracked set). Every git-aware tool, agent host and editor
  pays for it on every poll, and it was the largest single fixable cost here.
- **doctor: crash recency comes from the file's modification time,** but
  macOS rewrites old reports: files named for Oct 6, 8 and 9 had a modification
  time of today, so doctor said "last crash 0 min ago" when the newest crash was
  hours old, and counted 49 php-fpm crashes where the file names show about 24 in
  the last 24 hours (the count includes `/Library/Logs/DiagnosticReports`,
  which has 60 more php-fpm reports). The first line of each `.ips` file has a
  `"timestamp"` field with the real crash time.
- **classify:** `coreaudiod` and `ReportCrash` were the two biggest "other"
  processes in every run. An `audio` category, and `ReportCrash`/`spindump`
  under crash reporting, would make the idle report self-explanatory.
- **sensors:** fan speed prints as a raw float (`fans [1196.311279296875]`).
