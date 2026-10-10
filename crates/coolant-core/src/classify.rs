//! Sorts processes into categories (agent, indexer, security scanner,
//! language server, ...) and attributes each process to the agent session it
//! descends from, if any.
//!
//! Rules are plain data so that adding a platform means adding names, not code.

use coolant_platform::Process;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// Coding agent CLIs.
    Agent,
    /// Editors, terminals and apps that host agents.
    AgentHost,
    LanguageServer,
    /// Compilers, bundlers, test runners.
    Build,
    PackageManager,
    Vcs,
    /// rg, grep, find, cat and other short-lived shell tools.
    Search,
    /// Other node/bun/python processes: dev servers, scripts.
    Runtime,
    /// Spotlight, Windows Search, ...
    Indexer,
    /// Code-signing checks, malware scanners, EDR.
    Security,
    /// File system event and metadata daemons.
    FsDaemon,
    /// Time Machine, iCloud, Dropbox.
    BackupSync,
    ContainerVm,
    Browser,
    /// The desktop itself: window server, Dock, Finder, wallpaper.
    Desktop,
    /// The audio server; busy while an app holds the microphone or plays.
    Audio,
    /// Writing crash reports (ReportCrash, spindump): a program crashing.
    CrashReporting,
    Kernel,
    Other,
}

impl Category {
    /// Every category in display order.
    pub const ALL: [Category; 19] = [
        Self::Agent,
        Self::AgentHost,
        Self::LanguageServer,
        Self::Build,
        Self::PackageManager,
        Self::Vcs,
        Self::Search,
        Self::Runtime,
        Self::Indexer,
        Self::Security,
        Self::FsDaemon,
        Self::BackupSync,
        Self::ContainerVm,
        Self::Browser,
        Self::Desktop,
        Self::Audio,
        Self::CrashReporting,
        Self::Kernel,
        Self::Other,
    ];

    /// Whether this is work the OS does in reaction to agents rather than
    /// work the agents asked for. A tuned workspace can shrink these.
    pub fn is_overhead(self) -> bool {
        matches!(self, Self::Indexer | Self::Security | Self::FsDaemon | Self::BackupSync)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::AgentHost => "agent-host",
            Self::LanguageServer => "language-server",
            Self::Build => "build",
            Self::PackageManager => "package-manager",
            Self::Vcs => "vcs",
            Self::Search => "search",
            Self::Runtime => "runtime",
            Self::Indexer => "indexer",
            Self::Security => "security",
            Self::FsDaemon => "fs-daemon",
            Self::BackupSync => "backup-sync",
            Self::ContainerVm => "container-vm",
            Self::Browser => "browser",
            Self::Desktop => "desktop",
            Self::Audio => "audio",
            Self::CrashReporting => "crash-reporting",
            Self::Kernel => "kernel",
            Self::Other => "other",
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

use Category::*;

/// Command-line rules run first so that e.g. a node process running
/// tsserver is a language server, not a generic runtime.
static CMD_RULES: LazyLock<Vec<(Category, Regex)>> = LazyLock::new(|| {
    [
        (Agent, r"@anthropic-ai/claude-code|/claude(\s|$)|/claude/versions/[0-9]|@openai/codex|/codex(\s|$)|@google/gemini-cli|/opencode(\s|$)|/aider(\s|$)|@sourcegraph/amp|cursor-agent"),
        (LanguageServer, r"tsserver\.js|typescript-language-server|vtsls|eslintServer|eslint-language-server|tailwindcss-language-server|vscode-.*-language-server|pyright|pylance|biome lsp|oxc_language_server|astro-ls|svelte-language-server|vue-language-server"),
        (Build, r"/(tsc|esbuild|vite|webpack|rollup|turbo|nx|jest|vitest|playwright|next|tsx|ts-node|swc)(\.js|\.mjs|\.cjs)?(\s|$)|node_modules/\.bin/(tsc|vite|vitest|jest|next|turbo|eslint|prettier)"),
        (PackageManager, r"/(npm|npx|pnpm|yarn)(-cli)?(\.js|\.cjs)?(\s|$)|corepack"),
    ]
    .into_iter()
    .map(|(c, re)| (c, Regex::new(re).unwrap()))
    .collect()
});

const NAME_RULES: &[(Category, &[&str])] = &[
    (
        Agent,
        &["claude", "codex", "gemini", "opencode", "aider", "amp", "cursor-agent", "goose", "droid", "copilot", "qwen"],
    ),
    (
        AgentHost,
        &[
            "Code Helper",
            "Code Helper (Plugin)",
            "Code Helper (Renderer)",
            "Electron",
            "Cursor",
            "Zed",
            "zed",
            "T3 Code",
            "t3",
            "t3-resource-monitor",
            "Conductor",
            "Windsurf",
            "Claude",
            "Codex",
            "Ghostty",
            "iTerm2",
            "Terminal",
            "WezTerm",
            "wezterm-gui",
            "kitty",
            "Alacritty",
            "tmux",
            "zellij",
        ],
    ),
    (
        LanguageServer,
        &[
            "gopls",
            "rust-analyzer",
            "clangd",
            "sourcekit-lsp",
            "jdtls",
            "pyright-langserver",
            "basedpyright-langserver",
            "ruff",
            "lua-language-server",
            "zls",
            "elixir-ls",
            "taplo",
            "marksman",
        ],
    ),
    (
        Build,
        &[
            "go",
            "compile",
            "link",
            "asm",
            "cgo",
            "cargo",
            "rustc",
            "rustup",
            "rustdoc",
            "swift-frontend",
            "swift-build",
            "swiftc",
            "clang",
            "clang++",
            "cc",
            "ld",
            "ld64",
            "make",
            "ninja",
            "cmake",
            "esbuild",
            "bazel",
            "javac",
            "gradle",
            "xcodebuild",
            "tsgo",
            "codesign",
        ],
    ),
    (PackageManager, &["npm", "pnpm", "yarn", "pip", "pip3", "uv", "poetry", "brew", "bundle", "gem", "composer"]),
    (
        Vcs,
        &[
            "git",
            "git-remote-https",
            "git-remote-http",
            "git-upload-pack",
            "git-receive-pack",
            "jj",
            "gh",
            "git-lfs",
            "git-credential-osxkeychain",
            "ssh",
        ],
    ),
    (
        Search,
        &[
            "rg", "grep", "egrep", "find", "fd", "ag", "ack", "fzf", "ls", "cat", "head", "tail", "wc", "sed", "awk",
            "xargs", "stat", "true", "sort", "cut", "tr", "jq",
        ],
    ),
    (
        Runtime,
        &["node", "bun", "deno", "python", "python3", "ruby", "php", "java", "dotnet", "sh", "bash", "zsh", "fish"],
    ),
    (
        Indexer,
        &[
            "mds",
            "mds_stores",
            "mdworker",
            "mdworker_shared",
            "mdsync",
            "corespotlightd",
            "Spotlight",
            "mdbulkimport",
            "SearchIndexer.exe",
            "SearchProtocolHost.exe",
            "SearchFilterHost.exe",
            "tracker-miner-fs-3",
            "tracker-extract-3",
            "localsearch-3",
            "baloo_file",
            "updatedb",
        ],
    ),
    (
        Security,
        &[
            "syspolicyd",
            "XprotectService",
            "XProtect",
            "XProtectBehaviorService",
            "XProtectRemediator",
            "amfid",
            "trustd",
            "tccd",
            "endpointsecurityd",
            "taskgated",
            "taskgated-helper",
            "MRT",
            "codesigning_monitor",
            "wdavdaemon",
            "wdavdaemon_enterprise",
            "wdavdaemon_unprivileged",
            "com.crowdstrike.falcon.Agent",
            "falcond",
            "SentinelAgent",
            "SophosScanD",
            "Kandji Agent",
            "ninjarmm-macagent",
            "osqueryd",
            "MsMpEng.exe",
            "MpDefenderCoreService.exe",
            "NisSrv.exe",
            "clamd",
            "auditd",
            "mdatp",
            "falcon-sensor",
        ],
    ),
    (
        FsDaemon,
        &[
            "fseventsd",
            "apfsd",
            "diskarbitrationd",
            "revisiond",
            "filecoordinationd",
            "lsd",
            "watchman",
            "fswatch",
            "inotifywait",
            "inotifywatch",
            "fsnotifier",
            "mount_apfs",
            "deleted",
            "fsck_apfs",
        ],
    ),
    (
        BackupSync,
        &[
            "backupd",
            "backupd-helper",
            "cloudd",
            "bird",
            "fileproviderd",
            "FileProvider",
            "Dropbox",
            "Google Drive",
            "OneDrive",
            "nsurlsessiond",
            "photoanalysisd",
            "mediaanalysisd",
        ],
    ),
    (
        ContainerVm,
        &[
            "com.apple.Virtualization.VirtualMachine",
            "OrbStack",
            "Docker",
            "Docker Desktop",
            "com.docker.backend",
            "com.docker.build",
            "com.docker.vpnkit",
            "com.docker.virtualization",
            "qemu-system-aarch64",
            "limactl",
            "vfkit",
            "container-runtime-linux",
            "vmmem",
            "vmmemWSL",
        ],
    ),
    (
        Browser,
        &[
            "Google Chrome",
            "Safari",
            "com.apple.WebKit.WebContent",
            "com.apple.WebKit.GPU",
            "com.apple.WebKit.Networking",
            "Arc",
            "Firefox",
            "firefox",
            "plugin-container",
            "Brave Browser",
            "chrome-headless-shell",
            "Chromium",
        ],
    ),
    (
        Desktop,
        &[
            "WindowServer",
            "Dock",
            "Finder",
            "SystemUIServer",
            "ControlCenter",
            "NotificationCenter",
            "loginwindow",
            "WallpaperAerialsExtension",
            "WallpaperAgent",
            "WallpaperImageExtension",
            "contextstored",
            "corebrightnessd",
            "universalaccessd",
            "TextInputMenuAgent",
            "Spotlight Helper",
            "dwm.exe",
            "explorer.exe",
            "gnome-shell",
            "kwin_wayland",
            "kwin_x11",
            "Xorg",
            "Xwayland",
            "plasmashell",
        ],
    ),
    (
        Audio,
        &[
            "coreaudiod",
            "audiomxd",
            "AudioComponentRegistrar",
            "com.apple.audio.DriverHelper",
            "audiodg.exe",
            "pipewire",
            "pulseaudio",
            "wireplumber",
        ],
    ),
    (
        CrashReporting,
        &[
            "ReportCrash",
            "spindump",
            "osanalyticshelper",
            "SubmitDiagInfo",
            "WerFault.exe",
            "systemd-coredump",
            "apport",
        ],
    ),
    (Kernel, &["kernel_task", "System", "launchd"]),
];

/// Exact names first; a case-insensitive fallback where the first rule
/// wins keeps "claude" (the CLI) and "Claude" (the desktop app) distinct.
type NameMaps = (HashMap<&'static str, Category>, HashMap<String, Category>);

static BY_NAME: LazyLock<NameMaps> = LazyLock::new(|| {
    let (mut exact, mut lower) = (HashMap::new(), HashMap::new());
    for &(cat, names) in NAME_RULES {
        for &n in names {
            exact.insert(n, cat);
            lower.entry(n.to_lowercase()).or_insert(cat);
        }
    }
    (exact, lower)
});

fn lookup(name: &str) -> Option<Category> {
    let (exact, lower) = &*BY_NAME;
    exact.get(name).or_else(|| lower.get(&name.to_lowercase())).copied()
}

/// The category of a single process, ignoring its ancestry.
pub fn of(name: &str, command: &str) -> Category {
    if !command.is_empty()
        && let Some((cat, _)) = CMD_RULES.iter().find(|(_, re)| re.is_match(command))
    {
        return *cat;
    }
    let base = command.split(' ').next().unwrap_or("").rsplit('/').next().unwrap_or("");
    if let Some(c) = lookup(name).or_else(|| lookup(base)) {
        return c;
    }
    // Chromium/Electron helpers carry the app name: "Foo Helper (Renderer)".
    if let Some(c) = name.split_once(" Helper").and_then(|(app, _)| lookup(app)) {
        return c;
    }
    if name.starts_with("com.apple.DriverKit") {
        return Kernel;
    }
    Other
}

/// A process with its category and owning agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Labeled {
    #[serde(flatten)]
    pub process: Process,
    pub category: Category,
    /// Nearest agent ancestor ("claude:1234"), or empty when the process
    /// doesn't descend from an agent.
    pub session: String,
}

/// Classifies a snapshot and attributes each process to the agent session
/// it descends from.
pub fn label(procs: Vec<Process>) -> Vec<Labeled> {
    let mut out: Vec<Labeled> = procs
        .into_iter()
        .map(|p| {
            let category = of(&p.name, &p.command);
            Labeled { process: p, category, session: String::new() }
        })
        .collect();
    let by_pid: HashMap<i32, usize> = out.iter().enumerate().map(|(i, l)| (l.process.pid, i)).collect();

    // Walk up from each process to the nearest agent, memoizing as we go.
    let mut memo: HashMap<i32, String> = HashMap::with_capacity(out.len());
    for i in 0..out.len() {
        let mut chain = Vec::new();
        let mut cur = Some(i);
        let mut found = String::new();
        while let Some(j) = cur {
            let p = &out[j];
            if let Some(s) = memo.get(&p.process.pid) {
                found = s.clone();
                break;
            }
            chain.push(p.process.pid);
            if p.category == Agent && !is_wrapped(&out, j, &by_pid) {
                found = format!("{}:{}", agent_name(&p.process), p.process.pid);
                break;
            }
            let ppid = p.process.ppid;
            cur = if ppid > 1 && ppid != p.process.pid && chain.len() < 64 { by_pid.get(&ppid).copied() } else { None };
        }
        for pid in chain {
            memo.insert(pid, found.clone());
        }
    }
    for l in &mut out {
        l.session = memo.remove(&l.process.pid).unwrap_or_default();
    }
    out
}

/// An agent process whose parent is the same agent (e.g. a node shim
/// exec'ing the real binary) belongs to the parent's session.
fn is_wrapped(out: &[Labeled], i: usize, by_pid: &HashMap<i32, usize>) -> bool {
    by_pid
        .get(&out[i].process.ppid)
        .is_some_and(|&j| out[j].category == Agent && agent_name(&out[j].process) == agent_name(&out[i].process))
}

static AGENT_NAMES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"claude|codex|gemini|opencode|aider|amp|cursor-agent|goose|droid|copilot|qwen").unwrap()
});

fn agent_name(p: &Process) -> String {
    for s in [&p.name, &p.command] {
        if let Some(m) = AGENT_NAMES.find(&s.to_lowercase()) {
            return m.as_str().to_string();
        }
    }
    p.name.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories() {
        let cases = [
            ("claude", "claude --output-format stream-json", Agent),
            // The native installer runs a binary named after its version.
            ("2.1.295", "/Users/u/.local/share/claude/versions/2.1.295 --resume x", Agent),
            ("Claude", "/Applications/Claude.app/Contents/MacOS/Claude", AgentHost),
            ("node", "node /x/node_modules/typescript/lib/tsserver.js --stdio", LanguageServer),
            ("node", "node /usr/local/bin/codex exec", Agent),
            ("node", "node --env-file=.env src/main.ts", Runtime),
            ("mds_stores", "/System/Library/Metadata.framework/Support/mds_stores", Indexer),
            ("syspolicyd", "/usr/libexec/syspolicyd", Security),
            ("coreaudiod", "/usr/sbin/coreaudiod", Audio),
            ("ReportCrash", "/System/Library/CoreServices/ReportCrash agent", CrashReporting),
            ("Docker Desktop Helper (Renderer)", "", ContainerVm),
            ("Google Chrome Helper (Renderer)", "", Browser),
            ("com.apple.DriverKit-AppleBCMWLAN", "", Kernel),
            ("git", "git status", Vcs),
            ("mystery", "", Other),
        ];
        for (name, cmd, want) in cases {
            assert_eq!(of(name, cmd), want, "{name} / {cmd}");
        }
    }

    #[test]
    fn sessions() {
        let p = |pid, ppid, name: &str, cmd: &str| Process {
            pid,
            ppid,
            name: name.into(),
            command: cmd.into(),
            ..Default::default()
        };
        let procs = vec![
            p(10, 1, "T3 Code", ""),
            p(20, 10, "claude", "claude --verbose"),
            p(30, 20, "zsh", "zsh"),
            p(40, 30, "git", "git status"),
            p(50, 1, "mds_stores", ""),
        ];
        let got: HashMap<i32, String> = label(procs).into_iter().map(|l| (l.process.pid, l.session)).collect();
        assert_eq!(got[&10], "");
        assert_eq!(got[&20], "claude:20");
        assert_eq!(got[&40], "claude:20");
        assert_eq!(got[&50], "");
    }
}
