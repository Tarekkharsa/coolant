---
name: coolant
description: Keep your own work from overheating the Mac or running it out of memory, using the coolant CLI. Use before starting several builds, type checkers, test runs, browser tests or subagents at once; when you finish a task (to stop servers and watchers you started); right after creating a git worktree; and when the user says the Mac is hot, slow or loud.
---

# Coolant

coolant measures where a Mac's CPU, memory and heat go while coding agents
work (builds, type checkers, Spotlight, security checks, leftover dev servers)
and fixes what it can, safely. Its output is plain text meant for people; read
it and pass on what matters in a sentence or two.

Find it with `command -v coolant`, or use
`/Applications/Coolant.app/Contents/MacOS/coolant`. If neither exists, tell
the user how to install it and stop; don't install it yourself:

```sh
curl -fsSL https://raw.githubusercontent.com/Tarekkharsa/coolant/main/scripts/install.sh | sh
```

## Check your own footprint

`coolant agent` (about 3 seconds) shows what *you* are doing to the Mac, not
just the Mac as a whole:

```
Mac: 6.1 of 10 cores busy · 58°C · thermal nominal · memory normal (41% free) · swap 0 B
This agent (claude:70867): 4.4 cores, 9.8 GB in 23 processes
  heaviest: tsc 2.6 GB (pid 18932), tsgolint 2.5 GB (pid 15960), node 2.0 GB (pid 18531)
Other agent sessions: 1, 0.2 cores
Servers and watchers in ~/code/app:
  pid 18531   node vite --port 5173          running 42 min, started by this agent
Verdict: busy. Don't add parallel work: run builds, type checks and tests one at a time.
```

`--json` gives the same as JSON. Run it:

- **Before starting several builds, type checkers, test suites, browser
  tests or subagents at once.** Follow the verdict:
  - `ok`: go ahead.
  - `busy`: don't add parallel work; run things one at a time.
  - `pause`: the Mac is hot or short of memory. Let what's running finish
    and stop what you no longer need before starting anything heavy. Tell
    the user in one line.
- **When you finish a task.** Stop the servers and watchers *started by this
  agent* that you no longer need (`kill <pid>`), and any test or build runs of
  yours still going. Leave the ones started by another agent or by no agent
  (their parent exited) alone; list them for the user and ask.

Each `tsc`, `tsgo`, `tsgolint`, `vite build`, `next build` or headless browser
can take gigabytes and several cores. Prefer one at a time, prefer
incremental modes (`tsc -b`, `--incremental`, `cargo check` over
`cargo build`), and give test runners fewer workers when the verdict isn't
`ok` (`vitest --maxWorkers=2`, `playwright test --workers=2`).

Check before a batch and after a task, not in a loop.

`coolant agent` needs 0.2.0-beta.13 or later. With an older version use
`coolant sensors -n 1` (temperature, thermal state, and from beta.10, memory
and swap), and suggest updating (the app's *Check for Updates*).

## When the user says the Mac is hot or slow

1. `coolant doctor --json` lists checks as `{status, title, detail, fix}`.
   Report the `warn` items with their `fix`; skip `ok`, and mention `info`
   only if it explains the problem.
2. `coolant profile -d 60 -q` records 60 seconds and prints a report: busy
   cores by category (OS overhead is marked `*`), agent sessions, top
   processes, memory and swap, and findings with fixes. Run it in the
   background if you can, and tell the user it takes a minute.
3. Summarize the two or three biggest causes and their fixes. If the cause is
   the agents' own work (builds, type checkers, tests), say so plainly and
   offer to do less at once; coolant can't fix that part.

## After creating a git worktree

A new worktree has no build output, so its first build redoes everything.
Clone the main checkout's build folders (`target/`, `node_modules`, …) into
it with copy-on-write. It takes about a second, uses no extra disk until
files change, and skips folders that already exist:

```sh
coolant prewarm path/to/worktree
```

## Ask the user first

Never do these without the user's explicit OK, and say what each will change:

- `coolant sweep --yes` or `--remove-merged` (deletes build folders or worktrees);
  plain `coolant sweep` is a dry run and is fine.
- `coolant tune` (moves folders), anything with `--force`, `coolant tune --undo`.
- `coolant projects on|off|all` and `coolant protect --remove`.
- `coolant prewarm --install-hook` (adds a git hook).
- Stopping a process you didn't start (`coolant agent` says who started each
  server). List them and ask.
- `coolant bench` (several minutes of heavy load).

Never move or symlink agent worktree folders yourself; Claude Code then
refuses to create new worktrees.

## Sharing a run

`coolant profile -d 120 -o run.json` saves the raw run, and
`coolant report run.json` prints it again. Runs contain process names,
command lines and folder paths; check with the user before sharing one
publicly.
