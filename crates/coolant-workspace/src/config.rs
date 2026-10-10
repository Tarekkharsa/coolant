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
    /// The projects (main checkouts) Coolant may change: their worktrees'
    /// files and the dev servers running in them. `None` means every
    /// project, as before projects could be chosen; the app starts with an
    /// empty list, so nothing is touched until a project is switched on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projects: Option<Vec<String>>,
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
        // ~/code and ~/Code are one folder on a case-insensitive volume.
        let mut seen = std::collections::BTreeSet::new();
        roots.retain(|p| seen.insert(canonical(p)));
        // Repositories cloned straight into the home folder.
        roots.push(h);
        roots
    }

    /// The protected path containing `p`, if any.
    pub fn protected_root(&self, p: &Path) -> Option<PathBuf> {
        let p = canonical(p);
        self.protect.iter().map(|q| canonical(&expand(q))).find(|q| p.starts_with(q))
    }

    /// Whether `p` is inside (or is) a protected path.
    pub fn is_protected(&self, p: &Path) -> bool {
        let p = canonical(p);
        self.protect.iter().any(|q| p.starts_with(canonical(&expand(q))))
    }

    /// Whether the project whose main checkout is `repo` is switched on.
    pub fn manages_repo(&self, repo: &Path) -> bool {
        let Some(list) = &self.projects else { return true };
        let repo = canonical(repo);
        list.iter().any(|p| canonical(&expand(p)) == repo)
    }

    /// Whether Coolant may change `p`, or stop a process running in it: it
    /// isn't protected, and it belongs to a project that is switched on.
    pub fn may_touch(&self, p: &Path) -> bool {
        !self.is_protected(p) && (self.projects.is_none() || project_of(p).is_some_and(|r| self.manages_repo(&r)))
    }

    /// [`Config::may_touch`] for a process whose folder may be unknown: an
    /// unknown folder is only fair game when every project is.
    pub fn may_touch_cwd(&self, cwd: Option<&Path>) -> bool {
        match cwd {
            Some(c) => self.may_touch(c),
            None => self.projects.is_none(),
        }
    }

    /// Switches the project at `repo` on or off (starting a list if there
    /// was none).
    pub fn set_managed(&mut self, repo: &Path, on: bool) {
        let repo = canonical(repo);
        let list = self.projects.get_or_insert_with(Vec::new);
        list.retain(|p| canonical(&expand(p)) != repo);
        if on {
            list.push(tilde(&repo));
        }
    }
}

/// The main checkout of the git project containing `p`. A linked worktree
/// (or submodule) leads back through its `.git` file to its repository.
pub fn project_of(p: &Path) -> Option<PathBuf> {
    let p = canonical(p);
    let dir = p.ancestors().find(|d| d.join(".git").exists())?;
    let git = dir.join(".git");
    if git.is_dir() {
        return Some(dir.to_path_buf());
    }
    // "gitdir: <repo>/.git/worktrees/<name>"
    let text = std::fs::read_to_string(&git).ok()?;
    let gitdir = PathBuf::from(text.strip_prefix("gitdir:")?.trim());
    let gitdir = if gitdir.is_absolute() { gitdir } else { dir.join(gitdir) };
    let common = gitdir.ancestors().find(|a| a.file_name().is_some_and(|n| n == ".git"))?;
    Some(canonical(common.parent()?))
}

/// Canonicalizes when possible, so symlinked and real paths compare equal.
pub fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_projects_switched_on_may_be_touched() {
        let base = canonical(&std::env::temp_dir()).join(format!("coolant-projects-{}", std::process::id()));
        let (app, other) = (base.join("app"), base.join("other"));
        for repo in [&app, &other] {
            std::fs::create_dir_all(repo.join(".git")).unwrap();
        }
        // A linked worktree outside its repository, as agents make them.
        let wt = base.join("wt");
        std::fs::create_dir_all(wt.join("src")).unwrap();
        std::fs::create_dir_all(app.join(".git/worktrees/wt")).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}/.git/worktrees/wt\n", app.display())).unwrap();
        assert_eq!(project_of(&wt.join("src")), Some(app.clone()));

        let mut cfg = Config::default();
        assert!(cfg.may_touch(&other) && cfg.may_touch_cwd(None), "no list: every project");
        cfg.projects = Some(Vec::new());
        assert!(!cfg.may_touch(&app) && !cfg.may_touch(&wt) && !cfg.may_touch_cwd(None));
        cfg.set_managed(&app, true);
        assert!(cfg.may_touch(&app.join("x")) && cfg.may_touch(&wt.join("src")));
        assert!(!cfg.may_touch(&other) && !cfg.may_touch(&base));
        cfg.protect.push(wt.display().to_string());
        assert!(!cfg.may_touch(&wt), "protection still wins");
        cfg.set_managed(&app, false);
        assert!(!cfg.may_touch(&app) && cfg.projects == Some(Vec::new()));
        let _ = std::fs::remove_dir_all(&base);
    }
}
