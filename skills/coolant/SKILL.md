---
name: coolant
description: Check and reduce the heat, CPU and memory load on a Mac while coding agents work, using the coolant CLI. Use when the user says the Mac is hot, slow or loud, or the fans are running; before starting several builds, type checkers, test runs or subagents at once; and right after creating a git worktree.
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

## Before heavy parallel work

Before starting several builds, type checkers, test suites or subagents at
once, check the machine (about 2 seconds):

```sh
coolant sensors -n 1
```

It prints one line, for example
`cpu 60.2°C … fans 1000 rpm  thermal nominal  memory normal (51% free)  swap 4.2 GB`.
Versions before 0.2.0-beta.10 print `pressure` instead of `thermal` and no
memory; suggest updating (the app's *Check for Updates*).

- `thermal` other than `nominal`, or `memory` `warning`/`critical`: run the
  work one at a time instead of in parallel, don't start more subagents, and
  tell the user why in one line.
- Low free memory (under about 20%) or swap that keeps growing between checks:
  same.
- Otherwise go ahead.

Each `tsc`, `tsgo`, `tsgolint`, `vite build` or `next build` can take 2–3 GB
and several cores. Prefer one at a time, prefer incremental modes
(`tsc -b`, `--incremental`, `cargo check` over `cargo build`), and stop the
watchers and dev servers you started when you no longer need them.

Check once before a batch, not in a loop.

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
- Stopping any process, including leftover dev servers. List them and ask.
- `coolant bench` (several minutes of heavy load).

Never move or symlink agent worktree folders yourself; Claude Code then
refuses to create new worktrees.

## Sharing a run

`coolant profile -d 120 -o run.json` saves the raw run, and
`coolant report run.json` prints it again. Runs contain process names,
command lines and folder paths; check with the user before sharing one
publicly.
