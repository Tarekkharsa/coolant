//! User configuration in `~/.config/coolant/config.json`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Paths coolant must never move, delete or stop processes in, e.g. a
    /// demo that is exposed to other people. `~` is expanded.
    #[serde(default)]
    pub protect: Vec<String>,
    /// Where to look for git repositories. Defaults to common code folders.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<String>,
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Expands a leading `~`.
pub fn expand(s: &str) -> PathBuf {
    match s.strip_prefix("~") {
        Some(rest) => home().join(rest.trim_start_matches('/')),
        None => PathBuf::from(s),
    }
}

/// Shortens a path under the home folder to `~/...` for display.
pub fn tilde(p: &Path) -> String {
    match p.strip_prefix(home()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

impl Config {
    pub fn path() -> PathBuf {
        home().join(".config/coolant/config.json")
    }

    pub fn load() -> Result<Config> {
        let path = Self::path();
        match std::fs::read(&path) {
            Ok(data) => serde_json::from_slice(&data).with_context(|| format!("parsing {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    /// Folders searched for repositories.
    pub fn roots(&self) -> Vec<PathBuf> {
        if !self.roots.is_empty() {
            return self.roots.iter().map(|r| expand(r)).collect();
        }
        let h = home();
        let mut roots: Vec<PathBuf> =
            ["Documents/GitHub", "Developer", "code", "Code", "src", "projects", "Projects", "dev", "work"]
                .iter()
                .map(|d| h.join(d))
                .filter(|p| p.is_dir())
                .collect();
        // Repositories cloned straight into the home folder.
        roots.push(h);
        roots
    }

    /// Whether `p` is inside (or is) a protected path.
    pub fn is_protected(&self, p: &Path) -> bool {
        let p = canonical(p);
        self.protect.iter().any(|q| p.starts_with(canonical(&expand(q))))
    }
}

/// Canonicalizes when possible, so symlinked and real paths compare equal.
pub fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}
