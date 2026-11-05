//! Cloud folders: designs kept on a share the user hosts themselves — a
//! TrueNAS / Samba share mounted on this computer, or a Nextcloud folder
//! kept in sync by the Nextcloud client. To the app each is just a folder;
//! this module lists the designs in one, keeps a small lock file beside a
//! design while it's open (so another computer opening it is told who has
//! it), and notices when a design was saved from elsewhere since it was
//! opened (so a save doesn't silently overwrite that).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A folder the start page lists designs from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudFolder {
    /// What it's called on the start page ("TrueNAS", "Nextcloud").
    pub name: String,
    pub path: PathBuf,
}

impl CloudFolder {
    /// A folder named after its last part.
    pub fn at(path: PathBuf) -> CloudFolder {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        CloudFolder { name, path }
    }
}

/// Is `path` inside one of `folders`?
pub fn in_cloud(folders: &[CloudFolder], path: &Path) -> bool {
    folders.iter().any(|f| path.starts_with(&f.path))
}

/// A design found in a folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Design {
    pub path: PathBuf,
    /// The file name without `.ox360`.
    pub name: String,
    /// The subfolder it's in, relative to the cloud folder ("" at the top).
    pub folder: String,
    pub modified: Option<SystemTime>,
}

/// How deep into subfolders `list` looks (the folder itself is 0).
const DEPTH: usize = 3;

/// The designs in `dir` and its subfolders, newest first. Hidden files,
/// half-written saves and lock files are left out. An error says why the
/// folder can't be read (a share that isn't mounted, say).
pub fn list(dir: &Path) -> Result<Vec<Design>, String> {
    if !dir.is_dir() {
        return Err(format!("{} isn't there — is the share mounted?", dir.display()));
    }
    let mut out = Vec::new();
    walk(dir, dir, 0, &mut out).map_err(|e| format!("couldn't read {}: {e}", dir.display()))?;
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<Design>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let file = entry.file_name().to_string_lossy().into_owned();
        if file.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            if depth < DEPTH {
                // A subfolder that can't be read is skipped, not the whole list.
                let _ = walk(root, &path, depth + 1, out);
            }
            continue;
        }
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
        if !matches!(ext.as_deref(), Some(crate::doc::EXTENSION | crate::doc::OLD_EXTENSION)) {
            continue;
        }
        let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let folder = path.parent().and_then(|p| p.strip_prefix(root).ok()).map_or_else(String::new, |p| p.display().to_string());
        let modified = entry.metadata().ok().and_then(|m| m.modified().ok());
        out.push(Design { path, name, folder, modified });
    }
    Ok(())
}

/// When the file at `path` was last written (None: it isn't there).
pub fn stamp(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Was `path` written since `seen` (when it was opened or saved here)?
pub fn changed_since(path: &Path, seen: Option<SystemTime>) -> bool {
    match (stamp(path), seen) {
        (Some(now), Some(seen)) => now != seen,
        _ => false,
    }
}

/// Who has a design open: kept in a small file beside it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lock {
    pub host: String,
    pub user: String,
    pub pid: u32,
    /// Seconds since 1970.
    pub since: u64,
}

impl Lock {
    /// This program on this computer, now.
    pub fn mine() -> Lock {
        Lock { host: host_name(), user: std::env::var("USER").unwrap_or_default(), pid: std::process::id(), since: unix_now() }
    }

    /// "gavin on framework, since 14:05 (2 h ago)" style, for a warning.
    pub fn describe(&self, now: SystemTime) -> String {
        let at = UNIX_EPOCH + Duration::from_secs(self.since);
        let who = if self.user.is_empty() { "someone".to_string() } else { self.user.clone() };
        format!("{who} on {}, opened {}", if self.host.is_empty() { "another computer" } else { &self.host }, ago(now, at))
    }
}

/// The lock file for `design`: `.~name.ox360.lock` beside it (hidden).
pub fn lock_path(design: &Path) -> PathBuf {
    let name = design.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    design.with_file_name(format!(".~{name}.lock"))
}

pub fn read_lock(design: &Path) -> Option<Lock> {
    serde_json::from_str(&std::fs::read_to_string(lock_path(design)).ok()?).ok()
}

/// Someone else holding `design` open: their lock, unless it's this
/// program's own or was left by a program on this computer that's gone.
pub fn held_elsewhere(design: &Path, me: &Lock, alive: impl Fn(u32) -> bool) -> Option<Lock> {
    let lock = read_lock(design)?;
    let same_computer = lock.host == me.host;
    if same_computer && (lock.pid == me.pid || !alive(lock.pid)) {
        return None;
    }
    Some(lock)
}

/// Holds a design's lock file while it's open; dropping it removes the file
/// (if it's still ours).
#[derive(Debug)]
pub struct LockGuard {
    design: PathBuf,
    lock: Lock,
}

impl LockGuard {
    /// Write the lock for `design` (a failure — a read-only share — just
    /// means no lock: opening the design matters more).
    pub fn take(design: &Path, lock: Lock) -> Option<LockGuard> {
        let text = serde_json::to_string(&lock).ok()?;
        std::fs::write(lock_path(design), text).ok()?;
        Some(LockGuard { design: design.to_path_buf(), lock })
    }

    pub fn design(&self) -> &Path {
        &self.design
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if read_lock(&self.design).as_ref() == Some(&self.lock) {
            let _ = std::fs::remove_file(lock_path(&self.design));
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// This computer's name.
pub fn host_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_default()
}

/// How long ago `then` was, in words ("just now", "5 min ago", "yesterday").
pub fn ago(now: SystemTime, then: SystemTime) -> String {
    let s = now.duration_since(then).map_or(0, |d| d.as_secs());
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86400 => format!("{} h ago", s / 3600),
        86400..172800 => "yesterday".into(),
        _ if s < 30 * 86400 => format!("{} days ago", s / 86400),
        _ if s < 365 * 86400 => format!("{} months ago", (s / (30 * 86400)).max(1)),
        _ => format!("{} years ago", s / (365 * 86400)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oxide360-cloud-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn lists_designs_in_the_folder_and_its_subfolders_newest_first() {
        let d = temp("list");
        std::fs::create_dir_all(d.join("keys/old")).unwrap();
        std::fs::create_dir_all(d.join(".hidden")).unwrap();
        let write = |p: &str, age: u64| {
            let path = d.join(p);
            std::fs::write(&path, "{}").unwrap();
            let t = SystemTime::now() - Duration::from_secs(age);
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(t).unwrap();
        };
        write("bracket.ox360", 300);
        write("keys/key.ox360", 10);
        write("keys/old/first.ocad", 9000);
        write("notes.txt", 0);
        write("bracket.ox360.tmp", 0);
        write(".hidden/secret.ox360", 0);
        write(".~bracket.ox360.lock", 0);
        let found = list(&d).unwrap();
        let names: Vec<(&str, &str)> = found.iter().map(|x| (x.name.as_str(), x.folder.as_str())).collect();
        assert_eq!(names, vec![("key", "keys"), ("bracket", ""), ("first", "keys/old")]);
        assert!(list(&d.join("not-mounted")).unwrap_err().contains("mounted"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_lock_says_who_has_it_and_goes_when_let_go() {
        let d = temp("lock");
        let design = d.join("key.ox360");
        std::fs::write(&design, "{}").unwrap();
        let me = Lock { host: "framework".into(), user: "gavin".into(), pid: 100, since: unix_now() };
        assert_eq!(held_elsewhere(&design, &me, |_| true), None, "nobody has it");
        // Another computer has it open.
        let theirs = Lock { host: "desk".into(), user: "sam".into(), pid: 7, since: unix_now() - 7200 };
        let guard = LockGuard::take(&design, theirs.clone()).unwrap();
        assert!(lock_path(&design).file_name().unwrap().to_string_lossy().starts_with(".~"), "hidden beside it");
        assert_eq!(held_elsewhere(&design, &me, |_| true), Some(theirs.clone()));
        assert_eq!(theirs.describe(SystemTime::now()), "sam on desk, opened 2 h ago");
        drop(guard);
        assert_eq!(read_lock(&design), None, "removed when closed");
        // Left by this computer: ours, or a program that's gone, doesn't count.
        let mine = LockGuard::take(&design, me.clone()).unwrap();
        assert_eq!(held_elsewhere(&design, &me, |_| true), None);
        let crashed = Lock { pid: 55, ..me.clone() };
        std::mem::forget(mine);
        let _left = LockGuard::take(&design, crashed.clone()).unwrap();
        assert_eq!(held_elsewhere(&design, &me, |_| false), None, "its program is gone");
        assert_eq!(held_elsewhere(&design, &me, |_| true), Some(crashed), "another copy of the app here");
        // Someone else's lock isn't removed by ours going.
        let other = LockGuard::take(&design, theirs.clone()).unwrap();
        let stale = LockGuard { design: design.clone(), lock: me.clone() };
        drop(stale);
        assert_eq!(read_lock(&design), Some(theirs));
        drop(other);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_save_from_elsewhere_is_noticed() {
        let d = temp("stamp");
        let f = d.join("a.ox360");
        std::fs::write(&f, "1").unwrap();
        let seen = stamp(&f);
        assert!(!changed_since(&f, seen));
        std::fs::File::options().write(true).open(&f).unwrap().set_modified(SystemTime::now() + Duration::from_secs(5)).unwrap();
        assert!(changed_since(&f, seen));
        assert!(!changed_since(&d.join("gone.ox360"), seen), "a missing file isn't a conflict");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn times_read_as_words() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let at = |s: u64| ago(now, now - Duration::from_secs(s));
        assert_eq!(at(5), "just now");
        assert_eq!(at(300), "5 min ago");
        assert_eq!(at(7200), "2 h ago");
        assert_eq!(at(90000), "yesterday");
        assert_eq!(at(5 * 86400), "5 days ago");
        assert_eq!(at(90 * 86400), "3 months ago");
        assert_eq!(at(800 * 86400), "2 years ago");
        assert_eq!(ago(now, now + Duration::from_secs(60)), "just now", "a clock a little ahead");
    }

    #[test]
    fn a_design_is_in_the_cloud_when_its_under_a_cloud_folder() {
        let f = vec![CloudFolder::at(PathBuf::from("/mnt/truenas/cad"))];
        assert_eq!(f[0].name, "cad");
        assert!(in_cloud(&f, Path::new("/mnt/truenas/cad/keys/key.ox360")));
        assert!(!in_cloud(&f, Path::new("/home/g/Documents/key.ox360")));
    }
}
