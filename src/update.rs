//! Is there a newer Oxide360? Once per start (unless turned off in
//! Settings) the latest GitHub release is asked for in the background, and
//! when it's newer than this one, how to update is told the way that fits
//! how this copy was installed: the Flatpak, the Fedora COPR, built from
//! source, or a downloaded copy.
//!
//! The question goes out through `curl` (in Fedora and in the Flatpak
//! runtime both), to GitHub only, and carries nothing about the user.
//! `OXIDE360_UPDATE_URL` points it somewhere else (a `file://` JSON, for
//! trying the notice out).

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

pub const RELEASES: &str = "https://github.com/Defanoux/Oxide360/releases";
const LATEST: &str = "https://api.github.com/repos/Defanoux/Oxide360/releases/latest";
pub const INSTALL_PAGE: &str = "https://defanoux.github.io/Oxide360/";

/// How this copy got here (so what updates it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Install {
    /// The Flatpak (from Oxide360's Flatpak repository or a .flatpak file).
    Flatpak,
    /// The RPM from the Fedora COPR (installed under /usr).
    Copr,
    /// Built from the source with cargo (runs from a `target/` folder).
    Source,
    /// Anything else: the plain tarball, a copy put somewhere by hand.
    Download,
}

impl Install {
    /// This run's.
    pub fn now() -> Install {
        let exe = std::env::current_exe().unwrap_or_default();
        Install::of(std::env::var_os("FLATPAK_ID").is_some(), &exe)
    }

    pub fn of(flatpak: bool, exe: &Path) -> Install {
        if flatpak {
            Install::Flatpak
        } else if exe.starts_with("/usr") {
            Install::Copr
        } else if exe.components().any(|c| c.as_os_str() == "target") {
            Install::Source
        } else {
            Install::Download
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Install::Flatpak => "the Flatpak",
            Install::Copr => "Fedora's COPR",
            Install::Source => "a build from source",
            Install::Download => "a downloaded copy",
        }
    }

    /// How to update: what to do, the terminal command (if there's one),
    /// and a note under it.
    pub fn steps(self) -> (&'static str, Option<&'static str>, &'static str) {
        match self {
            Install::Flatpak => (
                "Open Discover, go to Updates and update Oxide360. Or in a terminal:",
                Some("flatpak update io.github.Defanoux.Oxide360"),
                "If that finds nothing, this copy came from a downloaded .flatpak file: install it again from the install page, and from then on updates come by themselves.",
            ),
            Install::Copr => ("Open Discover, go to Updates and update Oxide360. Or in a terminal:", Some("sudo dnf upgrade oxide360"), "Your designs and settings stay as they are."),
            Install::Source => ("In the folder with the source:", Some("git pull && cargo build --release"), "Then run target/release/oxide360 again."),
            Install::Download => ("Download the new version from the releases page, or install it so it updates by itself (Fedora COPR or Flatpak) from the install page.", None, ""),
        }
    }
}

/// "v0.3.10" / "0.3.10" as numbers to compare ([0, 3, 10]); None for
/// anything that isn't dotted numbers.
pub fn parse_version(s: &str) -> Option<Vec<u32>> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let s = s.split(['-', '+']).next()?;
    let v: Option<Vec<u32>> = s.split('.').map(|p| p.parse().ok()).collect();
    v.filter(|v| !v.is_empty())
}

/// `latest` is a newer version than `current` (0.3 is 0.3.0).
pub fn newer(latest: &str, current: &str) -> bool {
    let (Some(mut a), Some(mut b)) = (parse_version(latest), parse_version(current)) else { return false };
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// "0.3.3" (no "v").
    pub version: String,
    /// Its page on GitHub (what's new).
    pub url: String,
}

/// The release in GitHub's answer for releases/latest.
pub fn parse_release(json: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let tag = v["tag_name"].as_str()?;
    parse_version(tag)?;
    let version = tag.trim().trim_start_matches(['v', 'V']).to_string();
    let url = v["html_url"].as_str().filter(|u| u.starts_with("https://github.com/")).map_or_else(|| format!("{RELEASES}/tag/{tag}"), str::to_string);
    Some(Release { version, url })
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Not asked yet.
    Idle,
    Checking,
    UpToDate(String),
    Available(Release),
    /// Couldn't find out (offline, GitHub not answering…): why.
    Failed(String),
}

static STATUS: Mutex<Status> = Mutex::new(Status::Idle);
/// The Check for Updates window is open.
pub static WINDOW: AtomicBool = AtomicBool::new(false);
/// The start page's notice was closed for this run.
pub static HIDDEN: AtomicBool = AtomicBool::new(false);

pub fn status() -> Status {
    STATUS.lock().map_or(Status::Idle, |s| s.clone())
}

fn set(s: Status) {
    if let Ok(mut now) = STATUS.lock() {
        *now = s;
    }
}

/// Ask GitHub, in the background (nothing if a check is already under way).
/// `repaint` is called when the answer is in.
pub fn check(repaint: impl FnOnce() + Send + 'static) {
    {
        let Ok(mut now) = STATUS.lock() else { return };
        if *now == Status::Checking {
            return;
        }
        *now = Status::Checking;
    }
    let _ = std::thread::Builder::new().name("update-check".into()).spawn(move || {
        let result = ask();
        crate::debuglog::line(&format!("update check: {result:?}"));
        set(result);
        repaint();
    });
}

fn ask() -> Status {
    let url = std::env::var("OXIDE360_UPDATE_URL").unwrap_or_else(|_| LATEST.to_string());
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "10", "-H", "Accept: application/vnd.github+json", "-A"])
        .arg(concat!("Oxide360/", env!("CARGO_PKG_VERSION")))
        .arg(&url)
        .stdin(std::process::Stdio::null())
        .output();
    let out = match out {
        Ok(o) => o,
        Err(_) => return Status::Failed("curl isn't installed".into()),
    };
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).trim().trim_start_matches("curl: ").to_string();
        return Status::Failed(if why.is_empty() { "GitHub didn't answer".into() } else { format!("GitHub didn't answer ({why})") });
    }
    match parse_release(&String::from_utf8_lossy(&out.stdout)) {
        Some(r) if newer(&r.version, env!("CARGO_PKG_VERSION")) => Status::Available(r),
        Some(r) => Status::UpToDate(r.version),
        None => Status::Failed("GitHub's answer didn't name a release".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert_eq!(parse_version("v0.3.10"), Some(vec![0, 3, 10]));
        assert_eq!(parse_version("1.2.0-beta1"), Some(vec![1, 2, 0]));
        assert_eq!(parse_version("latest"), None);
        assert!(newer("v0.3.10", "0.3.9"));
        assert!(newer("0.4.0", "0.3.12"));
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("0.3", "0.3.0"));
        assert!(!newer("0.3.2", "0.3.2"));
        assert!(!newer("0.3.1", "0.3.2"));
        assert!(!newer("nonsense", "0.3.2"));
    }

    #[test]
    fn github_answer_is_read() {
        let json = r#"{"tag_name":"v0.4.0","html_url":"https://github.com/Defanoux/Oxide360/releases/tag/v0.4.0","draft":false,"prerelease":false,"name":"Oxide360 0.4.0"}"#;
        assert_eq!(parse_release(json), Some(Release { version: "0.4.0".into(), url: "https://github.com/Defanoux/Oxide360/releases/tag/v0.4.0".into() }));
        // A link somewhere else isn't followed: the releases page instead.
        let odd = r#"{"tag_name":"v0.4.0","html_url":"https://example.com/x"}"#;
        assert_eq!(parse_release(odd).unwrap().url, "https://github.com/Defanoux/Oxide360/releases/tag/v0.4.0");
        assert_eq!(parse_release(r#"{"tag_name":"v0.5.0","prerelease":true}"#), None);
        assert_eq!(parse_release(r#"{"message":"API rate limit exceeded"}"#), None);
        assert_eq!(parse_release("<html>"), None);
    }

    #[test]
    fn how_it_was_installed() {
        assert_eq!(Install::of(true, Path::new("/app/bin/oxide360")), Install::Flatpak);
        assert_eq!(Install::of(false, Path::new("/usr/bin/oxide360")), Install::Copr);
        assert_eq!(Install::of(false, Path::new("/home/a/oxide360/target/release/oxide360")), Install::Source);
        assert_eq!(Install::of(false, Path::new("/home/a/Apps/oxide360/oxide360")), Install::Download);
        for i in [Install::Flatpak, Install::Copr, Install::Source] {
            assert!(i.steps().1.is_some(), "{i:?} has a command");
        }
    }
}
