# Architecture

coolant is a Cargo workspace. Everything OS-specific lives behind one trait.

```
crates/
  coolant-platform/   Backend trait + shared types (Process, Sensors, Spawn, …)
    src/darwin/       macOS: ps + libproc, IOHID/SMC/IOReport/coalition sensors, PID-gap spawn counter
    src/linux/        Linux: /proc, hwmon/thermal/RAPL sensors, /proc/loadavg PID-cursor spawn counter
    src/windows.rs    stub (planned: ETW, WMI thermal zones, Defender counters)
  coolant-core/       classify (process → category + agent session),
                      profile (Session/Recorder: sampling and attribution), report (text + findings)
  coolant-bench/      reproducible agent-like workloads run under the profiler
  coolant-workspace/  worktree discovery, doctor, tune, prewarm, sweep, protect, Tailscale exposure,
                      actions (one-step fixes with undo, for the app)
  coolant-cli/        the `coolant` binary
  coolant-app/        GPUI menu bar app: dashboard, Fixes panel, leftovers
```

## Data flow

```
Backend::processes / cpu / sensors / spawn_counter
        │  every interval
        ▼
classify::label ──► profile::Recorder ──► profile::Run (JSON) ──► report::text / findings
                                              ▲
                     coolant-bench wraps each workload in a Recorder
```

## How CPU gets attributed

1. **Live processes**: CPU time delta between two snapshots. A process is
   identified by (PID, start time), which survives exec and guards against PID reuse.
2. **Short-lived processes** (git, rg, node one-offs that live for milliseconds):
   a parent's *reaped-children* CPU grows by each child's lifetime CPU.
   Subtracting what we already saw those children use gives CPU spent by
   processes we never sampled. It is split across categories by the names the
   spawn counter caught for that parent's children.
3. **Unattributed** = machine busy time − all of the above (kernel work,
   interrupts, other users' short-lived processes).

Each process belongs to the **session** of its nearest agent ancestor
(`claude:1234`), so a `git` run by an agent counts against that agent.

## Workspace layer

`coolant-workspace` acts on what the profiler finds. Every action goes through
the same guards:

1. **Protected paths** from `~/.config/coolant/config.json` (`coolant protect`).
2. **In use**: a process has its working directory inside, or names the
   folder on its command line (`--env-file=/that/worktree/.env`).
3. **Exposed**: a process listens on a port that `tailscale serve` forwards to.
4. **Scope**: only linked worktrees and known artifact folders, never main
   checkouts, home or code roots (unless `--force`).

## Adding a platform

Implement `coolant_platform::Backend` in `crates/coolant-platform/src/<os>/`,
select it in `current()`, and add the platform's daemon names to the rule tables
in `coolant-core/src/classify.rs` (e.g. `MsMpEng.exe` → security,
`SearchIndexer.exe` → indexer). Nothing in `profile`, `report` or `bench`
should need to change.
