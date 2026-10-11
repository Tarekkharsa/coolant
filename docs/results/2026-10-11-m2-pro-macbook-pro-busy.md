# What a MacBook Pro M2 Pro was doing with an agent at work

MacBook Pro 14" M2 Pro (10 cores, 16 GB, two fans), macOS 27.0, company-managed
with CrowdStrike Falcon, charging (43%), 1.6 days since boot.
`coolant profile -d 120` while a Claude Code session (with its subagents) worked on a
TypeScript monorepo, the day after the [second Quick run](2026-10-10-m2-pro-macbook-pro-quick-rerun.md).
`coolant` built from source at 0362915 (0.2.0-beta.8).

**6.77 cores were busy on average**, measured from the host CPU counters. The
category and session tables below overcount badly (see finding 1), so read
them for their shape, not their totals.

```
coolant profile: Mac14,9 Apple M2 Pro (16.0 GB, 10 cores), darwin 27.0
120s recorded, 6.77 cores busy on average

Thermals: CPU 60°C → max 61°C (avg 58°C), CPU power avg 14.8 W / max 22.5 W, SoC avg 14.9 W, fan max 4915 rpm

Findings
  [high] 155 processes launched per second
      Every launch costs fork/exec and dynamic linking, plus code-signing checks on macOS. Short-lived git/rg/node processes add up.

Where the CPU went
  category         cores  share  disk written  spawns
  agent             9.26   137%          2 MB       0
  runtime           4.10    61%         61 MB    1164
  other             1.27    19%          2 MB   16686
  vcs               0.81    12%          1 MB     681
  build             0.55     8%         39 MB       3
  security *        0.13     2%           0 B       3
  desktop           0.08     1%         20 KB       0
  agent-host        0.06     1%          2 MB       9
  browser           0.04     1%          3 MB       1
  indexer *         0.04     1%           0 B      10
  kernel            0.03     0%           0 B       0
  fs-daemon *       0.02     0%           0 B       0
  package-manager   0.00     0%           0 B      33
  audio             0.00     0%           0 B       0
  search            0.00     0%           0 B      70
  backup-sync *     0.00     0%           0 B       0
  unattributed      0.00     0%
  * OS overhead triggered by file and process activity

Agent sessions (agent + everything it launched)
  session       cores  disk written  spawns                  top categories
  claude:64882  15.05        102 MB    1214  agent 61%, runtime 27%, vcs 5%
  claude:90320   0.09         16 KB      78            other 69%, agent 31%
  claude:13598   0.00           0 B       0                      agent 100%

Top processes
  pid                           name  category  cores  memory       session
  17828                         node   runtime   0.27  2.3 GB  claude:64882
  28913                         node   runtime   0.22  2.3 GB  claude:64882
  95099     XProtectRemediatorPirrit     other   0.19  101 MB
  25215                         node     build   0.18  808 MB  claude:64882
  25593                          tsc     build   0.17  2.8 GB  claude:64882
  23372                          tsc     build   0.15  3.1 GB  claude:64882
  28444                     tsgolint     other   0.15  2.7 GB  claude:64882
  27252                     tsgolint     other   0.14  2.7 GB  claude:64882
  21922  XProtectRemediatorSheepSwap     other   0.12  204 MB
  26701   XProtectRemediatorBundlore     other   0.11   31 MB
  16425                     tsgolint     other   0.11  2.5 GB  claude:64882
  429                         trustd  security   0.10   11 MB
  25043  XProtectRemediatorSnowDrift     other   0.08   73 MB
  28464     XProtectRemediatorAdload     other   0.08   50 MB
  19579                         node   runtime   0.07  1.2 GB  claude:64882
```

## 1. The profile counted 1,108 CPU-s in one second

The `agent` row says 9.26 cores (137%) and the session `claude:64882` 15.05
cores, on a 10-core Mac. The categories add up to 1,972 CPU-s against 813
CPU-s the machine actually used. Almost all of the difference is one sample:

| t (s) | busy cores (host) | `agent` | `runtime` |
|---|---|---|---|
| 71.1 | 6.49 | 0.00 | 1.67 |
| 72.1 | 5.07 | **1,108.11** | 0.25 |
| 73.1 | 4.85 | 0.00 | 0.70 |

At 72 s the long-running `node` processes the session had started (a
`vite build`, about 3 cores of `runtime` until then) ended, and the Claude
process's reaped-children CPU jumped by their whole lifetime, including the
CPU they used before the profile started.

The cause is in the short-lived-children step of `Profiler::sample`
(`crates/coolant-core/src/profile.rs`, around line 309). A parent's growth in
reaped-children CPU is credited only with the processes that vanished whose
*direct* parent it is (`gone[ppid]`). When a child and its own children exit
in the same interval (a shell or `pnpm` and the `node` processes under it), the
grandchildren's CPU reaches the Claude process through the child, but the
credit for the grandchildren goes to `gone[child]`, which nothing reads,
because the child is gone too. Everything the grandchildren used is counted
again as "hidden", and with no named spawns in that second it lands in the
parent's own category, `agent`.

Eight more samples overcounted by 2–40 CPU-s each (179 CPU-s in total), in
`runtime` and `vcs`, the categories of what the session spawns (`node`,
`git`). These are probably the same thing on a smaller scale.

**Fix suggestions:** credit each vanished process to its nearest ancestor that
is still alive, walking up through other vanished processes; and cap the
hidden CPU a sample can add at what the host counters say was busy minus what
was already attributed. Neither needs elevated privileges.

## 2. One session, not several

The machine had three `claude` processes, but only one was working:

- `claude:64882` (started from a terminal 39 minutes before, working in
  `gha-tanstack`) ran everything: `vite build`, two `tsc` (2.8 and 3.1 GB),
  three `tsgolint` (2.5–2.7 GB each), and `git`. Its subagents run inside the same
  process, so the profile can't split them.
- `claude:90320` was the session running this profile.
- `claude:13598` is `claude --chrome-native-host`, started by Google Chrome
  for the Claude in Chrome extension. It is not an agent and did nothing.

## 3. The Mac got warm and the fans ran

CPU 58–61 °C, CPU power 14.8 W on average (22.5 W at most), fans up to
4,915 rpm. In both benchmark runs the day before, the CPU stayed at
35–38 °C under 7 W and the fans never started. Thermal pressure stayed
nominal.

## 4. 155 process launches per second

18,660 launches in 120 s. 16,686 of them (89%) are attributed to "other",
because the spawn tracker had no name for them, so the profile can't say what
they were. On a Mac with Falcon each launch costs about 4 ms (see the
[benchmark](2026-10-10-m2-pro-macbook-pro-quick-rerun.md)), so 155 a
second add up to about 0.6 s of start-up time every second, spread over the
session's tool calls.

## 5. Apple's malware scanners ran during the profile

Six `XProtectRemediator*` processes (Pirrit, SheepSwap, Bundlore, SnowDrift,
Adload, SnowBeagle) used about 0.6 cores together, classified as "other". These
are XProtect's periodic scans, unrelated to the agent.

## Other observations

- **Memory.** The 40 busiest processes reported 33.8 GB resident on a 16 GB
  Mac, most of it in `tsc`, `tsgolint` and `node`. Resident size counts shared
  pages once per process, so this doesn't prove the Mac was short of memory;
  after the run swap was 0 and 65% of memory was free. Coolant doesn't record
  memory pressure or swap, so the profile can't say whether it was.
- **Spotlight and security were small here** (under 0.2 cores together, plus
  0.6 cores of XProtect scans), because the session worked in folders it had
  already built.

## Suggestions for coolant

From this run:

- **profile: fix the reaped-children double count** (finding 1). Until then,
  every busy profile with build tools in it can show impossible numbers.
- **classify:** `XProtectRemediator*` as `security`; `tsgolint` (oxlint's
  type-aware linter, 2.5 GB each) as `build`.
- **sessions:** `claude --chrome-native-host` is a browser bridge, not an
  agent session; leave it out of "Agent sessions", or show it as a host.
- **profile: record memory pressure and swap** alongside the thermals, so a
  run with several 3 GB type checkers can say whether the Mac was swapping.
- **spawns:** 89% of launches had no name. If the name can't be read at exec
  time, the parent's category or command would still say more than "other".
