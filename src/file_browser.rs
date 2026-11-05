//! The file viewer's work on files (as Fusion's Data panel): what's in a
//! folder (projects are folders, then designs, newest first), and renaming,
//! moving to another project, making a project, and deleting — keeping the
//! offline copies (`cloud::Cache`) pointing at the right file.

use crate::cloud::Cache;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A project (folder) or design in a folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub path: PathBuf,
    /// A design's name without `.ox360`; a folder's name.
    pub name: String,
    pub folder: bool,
    pub modified: Option<SystemTime>,
}

/// What's in `dir`: folders (by name), then designs (newest first). Hidden
/// files, half-written saves and lock files are left out.
pub fn list(dir: &Path) -> Result<Vec<Item>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("couldn't read {}: {e}", dir.display()))?;
    let (mut folders, mut designs) = (Vec::new(), Vec::new());
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if file.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let modified = entry.metadata().ok().and_then(|m| m.modified().ok());
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            folders.push(Item { path, name: file, folder: true, modified });
        } else if is_design(&path) {
            let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
            designs.push(Item { path, name, folder: false, modified });
        }
    }
    folders.sort_by_key(|f| f.name.to_lowercase());
    designs.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    folders.extend(designs);
    Ok(folders)
}

pub fn is_design(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == crate::doc::EXTENSION || e == crate::doc::OLD_EXTENSION)
}

/// A file or folder moved from `from` to `to`: its offline copy follows, and
/// so does anything in `recent` under it.
fn follow(from: &Path, to: &Path, cache: Option<&Cache>, recent: &mut [PathBuf]) {
    for p in recent.iter_mut() {
        if let Some(now) = crate::cloud::remap(p, from, to) {
            *p = now;
        }
    }
    if let Some(c) = cache {
        for e in c.entries() {
            if let Some(now) = crate::cloud::remap(&e.cloud, from, to) {
                c.update(&e.cloud, |x| x.cloud = now);
            }
        }
    }
}

/// Rename the design (or project) at `path` to `name` in the same folder.
pub fn rename(path: &Path, name: &str, cache: Option<&Cache>, recent: &mut [PathBuf]) -> Result<PathBuf, String> {
    let name = crate::save_dialog::clean_name(name)?;
    let to = if path.is_dir() { path.with_file_name(&name) } else { path.with_file_name(format!("{name}.{}", crate::doc::EXTENSION)) };
    if to == path {
        return Ok(to);
    }
    if to.exists() {
        return Err(format!("there's already a {} called {name} there", if path.is_dir() { "project" } else { "design" }));
    }
    std::fs::rename(path, &to).map_err(|e| format!("couldn't rename it: {e}"))?;
    follow(path, &to, cache, recent);
    Ok(to)
}

/// Move the design (or project) at `path` into the folder `dir`.
pub fn move_to(path: &Path, dir: &Path, cache: Option<&Cache>, recent: &mut [PathBuf]) -> Result<PathBuf, String> {
    let name = path.file_name().ok_or("nothing to move")?;
    let to = dir.join(name);
    if to == path {
        return Ok(to);
    }
    if dir.starts_with(path) {
        return Err("a project can't go inside itself".into());
    }
    if to.exists() {
        return Err(format!("there's already a {} there", name.to_string_lossy()));
    }
    // (A rename where it can be; across disks, a copy then the old one removed.)
    if std::fs::rename(path, &to).is_err() {
        if path.is_dir() {
            return Err("couldn't move the project there".into());
        }
        std::fs::copy(path, &to).map_err(|e| format!("couldn't move it: {e}"))?;
        std::fs::remove_file(path).map_err(|e| format!("copied, but couldn't remove the old one: {e}"))?;
    }
    follow(path, &to, cache, recent);
    Ok(to)
}

/// A new project (folder) called `name` in `dir`.
pub fn new_project(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let name = crate::save_dialog::clean_name(name)?;
    let p = dir.join(&name);
    if p.exists() {
        return Err(format!("there's already a {name} there"));
    }
    std::fs::create_dir_all(&p).map_err(|e| format!("couldn't make the project: {e}"))?;
    Ok(p)
}

/// How `delete` gets rid of something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delete {
    /// To the desktop's trash (`gio trash`); an error where there's none (some network shares).
    Trash,
    /// Gone for good.
    ForGood,
}

/// Delete `path` (`how`); its offline copy goes too.
pub fn delete(path: &Path, how: Delete, cache: Option<&Cache>) -> Result<(), String> {
    match how {
        Delete::Trash => {
            let ok = std::process::Command::new("gio").arg("trash").arg(path).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
            if !ok {
                return Err("there's no trash there".into());
            }
        }
        Delete::ForGood => {
            let r = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
            r.map_err(|e| format!("couldn't delete it: {e}"))?;
        }
    }
    if let Some(c) = cache {
        for e in c.entries().into_iter().filter(|e| e.cloud.starts_with(path)) {
            c.update(&e.cloud, |x| x.pending = false);
            c.forget(&e.cloud);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oxide360-browse-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(p: &Path, age: u64) {
        std::fs::write(p, "{}").unwrap();
        std::fs::File::options().write(true).open(p).unwrap().set_modified(SystemTime::now() - std::time::Duration::from_secs(age)).unwrap();
    }

    #[test]
    fn projects_come_first_then_designs_newest_first() {
        let d = temp("list");
        std::fs::create_dir_all(d.join("Keys")).unwrap();
        std::fs::create_dir_all(d.join("brackets")).unwrap();
        touch(&d.join("old.ox360"), 900);
        touch(&d.join("new.ox360"), 5);
        touch(&d.join(".~new.ox360.lock"), 0);
        touch(&d.join("readme.txt"), 0);
        let names: Vec<(String, bool)> = list(&d).unwrap().into_iter().map(|i| (i.name, i.folder)).collect();
        assert_eq!(names, vec![("brackets".into(), true), ("Keys".into(), true), ("new".into(), false), ("old".into(), false)]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn renaming_and_moving_keep_recent_and_the_offline_copy_on_the_file() {
        let d = temp("ops");
        let cache = Cache { dir: d.join("cache") };
        let key = d.join("key.ox360");
        touch(&key, 0);
        cache.keep(&key, b"{}", None, false).unwrap();
        let mut recent = vec![key.clone(), d.join("other.ox360")];
        // Rename (a typed .ox360 isn't doubled).
        let renamed = rename(&key, "key-noelle.ox360", Some(&cache), &mut recent).unwrap();
        assert_eq!(renamed, d.join("key-noelle.ox360"));
        assert!(renamed.is_file() && !key.exists());
        assert_eq!(recent[0], renamed);
        assert!(recent[0].is_file(), "the very file (no trailing slash): {:?}", recent[0]);
        assert!(cache.entry(&renamed).is_some() && cache.entry(&key).is_none());
        // Taken names are refused.
        touch(&d.join("taken.ox360"), 0);
        assert!(rename(&renamed, "taken", Some(&cache), &mut recent).is_err());
        // Into a new project.
        let project = new_project(&d, "Keys").unwrap();
        assert!(new_project(&d, "Keys").is_err());
        let moved = move_to(&renamed, &project, Some(&cache), &mut recent).unwrap();
        assert_eq!(moved, project.join("key-noelle.ox360"));
        assert_eq!(recent[0], moved);
        assert!(cache.copy_of(&moved).is_some());
        // A project renamed: designs in it follow.
        let again = rename(&project, "Car keys", Some(&cache), &mut recent).unwrap();
        assert_eq!(recent[0], again.join("key-noelle.ox360"));
        assert!(move_to(&again, &again.join("x"), None, &mut recent).is_err(), "not into itself");
        // Deleted for good (tests never use the desktop's trash): gone, copy too.
        delete(&again.join("key-noelle.ox360"), Delete::ForGood, Some(&cache)).unwrap();
        assert!(!again.join("key-noelle.ox360").exists());
        assert!(cache.entries().is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
