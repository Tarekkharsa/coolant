# The Coolant app on a hot M2 Pro: what it got wrong

MacBook Pro 14" M2 Pro (10 cores, 16 GB, macOS 27.0, CrowdStrike Falcon),
the same morning as the [second busy profile](2026-10-12-m2-pro-macbook-pro-busy-rerun.md).
The Coolant app 0.2.0-beta.9 was running; code references are to 6faa220
(0.2.0-beta.10). One Claude Code session was running browser tests.

The app's panel at 08:26 said:

```
Running warm
56°C · fan 3704 rpm · 5.6 busy cores · memory low

Freed 49.0 GB in 31 worktree(s).
Couldn't apply: those processes have already exited or changed
• Prewarm 31 new worktree(s)          [Prewarm]
  so their first build isn't from scratch
• Starting a program takes 7.5 ms on this Mac (typical: about 1 ms)
• Stop 1 duplicate dev server(s)
```

## 1. Sweep and prewarm undo each other

The app had just swept 31 worktrees (49 GB) and then offered to prewarm 31
"new" worktrees. `coolant doctor` confirmed: "31 active worktree(s) lack
build artifacts the main checkout has". Clicking Prewarm would put the build
folders straight back, and the next sweep would offer to delete them again.

The two rules overlap:

- **Sweep** (`sweep.rs`, line 52) takes any worktree whose branch is merged,
  however recently it was used, plus any worktree idle for 7 days or more.
- **Prewarm** (`doctor.rs`, `fn prewarm`) takes any worktree idle for less
  than 7 days that lacks the main checkout's artifacts, merged or not.

So a merged worktree used in the last 7 days is first swept, then reported as
"cold". The label calls them "new", but they are old worktrees whose
artifacts Coolant itself deleted.

**Fix suggestion:** prewarm only worktrees whose branch is not merged, and
not ones Coolant swept (or that would be swept now). Say "worktrees without
build folders" rather than "new".

## 2. "Stop duplicate dev server" points at short-lived test workers

The duplicate check flagged `node --experimental-import-meta-resolve
--require suppress-warnings.cjs …` ("2 copies … are running", `kill 2398`).
Seconds later no such process existed: polling every 5 s for 30 s found none.
Clicking Stop therefore fails with "those processes have already exited or
changed", which is the red message in the panel.

`host::duplicate_servers` keeps a process when its command line *contains*
any entry of `SERVERS`. The entry `"vite"` also matches `vitest`, whose worker
pool starts several identical `node` processes in the same folder for each
test run. That is the likely match here; we didn't capture the full command
line before the workers exited.

**Fix suggestions:**
- Match server names as whole words or path components (`/vite/`, `vite `,
  `vite.js`), not substrings, so `vitest` doesn't match `vite`.
- Only flag processes that have been running for some minimum time (for
  example 2 minutes), or that listen on a port. Dev servers and watchers are
  long-lived; test workers are not.
- Re-check the group before showing the button, and drop the error from the
  panel once the check no longer applies. The message also doesn't say which
  action failed.

## 3. Program start time depends on load

The panel said starting a program takes **7.5 ms**. `coolant doctor` measured
2.8 ms on the same Mac two days earlier, with nothing else running, and the
Quick benchmarks measured 3.7–4.0 ms. At 7.5 busy cores the measurement mostly
shows the load, not the endpoint security agent.

**Fix suggestion:** measure when the Mac is quiet (for example under 2 busy
cores), keep the lowest recent value, and show "typical: about 1 ms" only next
to a quiet measurement.

## 4. What was heating the Mac

A 60-second profile at 08:24, while the temperature rose:

```
60s recorded, 7.51 cores busy on average
Thermals: CPU 61°C → max 62°C (avg 60°C), CPU power avg 14.7 W / max 17.4 W, SoC avg 15.1 W, fan max 4415 rpm
Memory: at least 43% available, swap 0 B

  category       cores  share
  browser         2.88    38%
  runtime         2.18    29%
  other           0.90    12%
  desktop         0.36     5%

  session       cores  top categories
  claude:70867   4.91  browser 48%, runtime 44%, build 3%
```

- **Browser tests.** The session ran Playwright: about seven
  `chrome-headless-shell` processes (2.4 cores) plus the `node` servers they
  test against (2.2 cores). New browser processes started every few seconds.
- **DisplayLink.** `DisplayLinkUserAgent`, the driver for an external display
  on a DisplayLink dock, used 0.44 cores and had been at about that rate since
  boot (2 days). WindowServer used 0.33 cores. Coolant classifies the driver as
  "other".
- **The sweep had finished.** `coolant-app` was back at 0.3% of a core.
- The CPU went from 55 °C (08:06) to 61–62 °C (08:24) and stayed there; thermal
  pressure stayed nominal. The GPU sensor read 74 °C at 0.25 W of GPU power,
  probably heat from the CPU next to it rather than GPU work (not confirmed).

## 5. Smaller things

- The header says "memory low" and "The Mac is short of memory", while 43%
  of memory was available and no swap was used. The pressure state was
  "warning" for only part of the time.
- The panel's temperature (56 °C) and `coolant sensors` (61 °C) differed by
  5 °C at the same time. The panel may average or use a different sensor; it
  should say which.

## Suggestions for coolant, in order

1. **Sweep/prewarm overlap** (finding 1): the app suggests undoing its own fix.
2. **Duplicate check matches `vitest` as `vite`** and flags short-lived
   workers (finding 2): a Stop button that can't work.
3. **Measure program start time only when the Mac is quiet** (finding 3).
4. **classify:** `DisplayLinkUserAgent` (display driver) as `desktop`;
   `coolant-app` and `coolant` in their own category.
5. **Panel wording:** "memory low" only when pressure is high or swap grows;
   say which action an error belongs to.
