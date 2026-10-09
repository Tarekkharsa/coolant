# coolant

**Why is my machine hot while AI coding agents run, and what can I do about it?**

Running several coding agents in parallel (Claude Code, Codex, Gemini CLI, …)
makes laptops and desktops run hot, especially Macs. Much of that heat isn't the
agents themselves. It's the work the OS does *in reaction* to them:

- **Spotlight** re-indexing every file agents write (dependencies, build output, worktrees)
- **Security checks** (`syspolicyd`, XProtect, `amfid`, EDR) on thousands of short-lived processes and fresh binaries
- **File system daemons** fanning out change events to every watcher
- **Duplicated tooling**: one `tsserver` per worktree, dev servers left running after an agent finished
- **APFS metadata cost** for small-file-heavy workloads (`git status`, `npm install`)

coolant measures this. It attributes CPU, disk I/O, process spawns, temperature
and power to **categories** (agent, indexer, security, language server, …) and to
**agent sessions** (an agent plus everything it launched), then turns the numbers
into findings you can act on.

macOS is supported today, without root. Linux and Windows backends are planned;
the architecture keeps all OS-specific code behind one trait.

## Install

```sh
cargo install --git https://github.com/Tarekkharsa/coolant coolant-cli
```

## Usage

```sh
coolant profile -d 120 -o run.json   # record 2 minutes while your agents work
coolant report run.json              # re-render a saved run
coolant sensors                      # temperature / power / fan readings
coolant bench --preset quick         # reproducible agent-like workloads
```

Example (Mac mini M4):

```
Findings
  [medium] 24 orphaned dev processes using 0.06 cores
      Dev servers, watchers or builds whose parent (often an agent or terminal) is gone: node (pid 89492), …

Where the CPU went
  category       cores  share  disk written  spawns
  container-vm    1.85    89%          5 MB       0
  runtime         0.04     2%           0 B       0
  agent           0.02     1%           0 B       0
  indexer *       0.00     0%           0 B       0
  …
```

## How it works

- **Processes**: `/bin/ps` (setuid on macOS, so it sees system daemons) plus
  `proc_pid_rusage` for richer counters on your own processes.
- **Short-lived processes**: agents spawn huge numbers of millisecond-long
  `git`/`rg`/`node` processes that sampling never sees. coolant recovers their
  CPU from each parent's *reaped-children* counter, and counts spawns by watching
  for gaps in PID allocation.
- **Sensors without root**: CPU die temperatures via IOHID, GPU and fan via SMC,
  power via IOReport and kernel coalition energy, thermal pressure via `notify`.

See [docs/architecture.md](docs/architecture.md) for the details.

## Roadmap

1. **Profiler + benchmarks** (now): measure where the heat comes from.
2. **Workspace layer**: agent worktrees on a tuned location (no Spotlight, no
   backups), APFS clonefile for dependencies, shared watchers and language servers.
3. **Filesystem layer**: copy-on-write agent workspaces (FSKit on macOS, FUSE /
   overlayfs on Linux, ProjFS / Dev Drive on Windows).

## License

MIT
