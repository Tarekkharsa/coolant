//! Updates from GitHub Releases: the same steps as `scripts/install.sh`
//! (download the newest .dmg, check its SHA-256, replace the app), then a
//! relaunch. Uses curl, hdiutil and ditto, so files carry no quarantine
//! flag and the app opens without a Gatekeeper prompt.

use std::path::{Path, PathBuf};
use std::process::Command;

const REPO: &str = "Tarekkharsa/coolant";

/// This build's version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub dmg_url: String,
}

/// The newest release that has a Coolant .dmg (betas included).
pub fn latest() -> Result<Option<Release>, String> {
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "20", &format!("https://api.github.com/repos/{REPO}/releases")])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("couldn't reach GitHub".into());
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let releases = json.as_array().ok_or("unexpected reply from GitHub")?;
    let found = releases.iter().find_map(|r| {
        let dmg = r.get("assets")?.as_array()?.iter().find_map(|a| {
            let name = a.get("name")?.as_str()?;
            (name.starts_with("Coolant-") && name.ends_with(".dmg"))
                .then(|| a.get("browser_download_url")?.as_str().map(str::to_string))?
        })?;
        let tag = r.get("tag_name")?.as_str()?;
        Some(Release { version: tag.trim_start_matches('v').to_string(), dmg_url: dmg })
    });
    Ok(found)
}

/// Whether version `a` is newer than `b`. Versions look like `0.2.0` or
/// `0.2.0-beta.4`; a release is newer than any of its betas.
pub fn newer(a: &str, b: &str) -> bool {
    key(a) > key(b)
}

fn key(v: &str) -> (Vec<u64>, u64, u64) {
    let (main, pre) = v.split_once('-').unwrap_or((v, ""));
    let nums = main.split('.').map(|n| n.parse().unwrap_or(0)).collect();
    // No pre-release sorts after every pre-release of the same version.
    let (is_release, pre_n) =
        if pre.is_empty() { (1, 0) } else { (0, pre.rsplit('.').next().and_then(|n| n.parse().ok()).unwrap_or(0)) };
    (nums, is_release, pre_n)
}

/// The running app's bundle, when it runs from one.
pub fn app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)
}

/// Downloads `release`, checks it, and replaces the app bundle at `app`.
/// Blocking; run it off the UI thread.
pub fn install(release: &Release, app: &Path) -> Result<(), String> {
    let tmp = std::env::temp_dir().join(format!("coolant-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let result = install_in(release, app, &tmp);
    let _ = Command::new("hdiutil").args(["detach", "-quiet"]).arg(tmp.join("mnt")).output();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

fn install_in(release: &Release, app: &Path, tmp: &Path) -> Result<(), String> {
    let dmg = tmp.join("Coolant.dmg");
    run(Command::new("curl").args(["-fsSL", "--max-time", "300", "-o"]).arg(&dmg).arg(&release.dmg_url), "download")?;
    let sums = Command::new("curl")
        .args(["-fsSL", "--max-time", "20", &format!("{}.sha256", release.dmg_url)])
        .output()
        .map_err(|e| e.to_string())?;
    let expected = String::from_utf8_lossy(&sums.stdout).split_whitespace().next().unwrap_or("").to_string();
    let actual = Command::new("shasum").args(["-a", "256"]).arg(&dmg).output().map_err(|e| e.to_string())?;
    let actual = String::from_utf8_lossy(&actual.stdout).split_whitespace().next().unwrap_or("").to_string();
    if expected.is_empty() || expected != actual {
        return Err("the download didn't match its checksum; nothing was changed".into());
    }
    let mnt = tmp.join("mnt");
    std::fs::create_dir_all(&mnt).map_err(|e| e.to_string())?;
    run(
        Command::new("hdiutil").args(["attach", "-quiet", "-nobrowse", "-readonly", "-mountpoint"]).arg(&mnt).arg(&dmg),
        "open the download",
    )?;
    let new_app = mnt.join("Coolant.app");
    if !new_app.join("Contents/MacOS/coolant-app").exists() {
        return Err("the download doesn't contain Coolant.app".into());
    }
    // Copy next to the old app first, then swap, so a failed copy leaves
    // the installed app untouched.
    let staged = app.with_extension("app.new");
    let _ = std::fs::remove_dir_all(&staged);
    run(Command::new("ditto").arg(&new_app).arg(&staged), "copy the new version")?;
    let old = app.with_extension("app.old");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(app, &old).map_err(|e| format!("replacing the app: {e}"))?;
    if let Err(e) = std::fs::rename(&staged, app) {
        let _ = std::fs::rename(&old, app);
        return Err(format!("replacing the app: {e}"));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let out = cmd.output().map_err(|e| format!("couldn't {what}: {e}"))?;
    if !out.status.success() {
        return Err(format!("couldn't {what}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

/// Starts the app at `app` again a moment after this process exits.
pub fn relaunch(app: &Path) {
    let _ = Command::new("/bin/sh").arg("-c").arg("sleep 1; open \"$0\"").arg(app).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Downloads the newest release and installs it over a stand-in app in
    /// a temp folder (network; run with --ignored).
    #[test]
    #[ignore]
    fn installs_the_newest_release_over_an_app() {
        let base = std::env::temp_dir().join(format!("coolant-update-test-{}", std::process::id()));
        let app = base.join("Coolant.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("Contents/MacOS/old"), "").unwrap();
        let release = latest().unwrap().expect("a release with a .dmg");
        install(&release, &app).unwrap();
        assert!(app.join("Contents/MacOS/coolant-app").exists());
        assert!(!app.join("Contents/MacOS/old").exists());
        assert!(!base.join("Coolant.app.old").exists() && !base.join("Coolant.app.new").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn versions_compare_with_betas() {
        assert!(newer("0.2.0-beta.5", "0.2.0-beta.4"));
        assert!(newer("0.2.0-beta.10", "0.2.0-beta.9"));
        assert!(newer("0.2.0", "0.2.0-beta.9"));
        assert!(newer("0.3.0-beta.1", "0.2.0"));
        assert!(!newer("0.2.0-beta.4", "0.2.0-beta.4"));
        assert!(!newer("0.2.0-beta.4", "0.2.0"));
        assert!(!newer("0.1.0", "0.2.0-beta.1"));
    }
}
