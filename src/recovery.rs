//! Autosave and crash recovery (as Fusion's recovery files): designs with
//! unsaved changes are written now and then to a recovery folder; a normal
//! close or a save removes them; whatever is left at the next start (from a
//! run that didn't close properly) is offered back.
//!
//! Files are named `<pid>-<tab>.opencad` with a `.json` beside each saying
//! which file it came from, so a second opencad running at the same time
//! never takes another one's autosaves for a crash's.

use crate::doc::Document;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How often a changed design is autosaved: every 2 minutes
/// (`OPENCAD_AUTOSAVE_SECONDS` changes it, for testing).
pub fn every_seconds() -> f64 {
    std::env::var("OPENCAD_AUTOSAVE_SECONDS").ok().and_then(|s| s.parse::<f64>().ok()).filter(|s| *s > 0.0).unwrap_or(120.0)
}

/// `$XDG_DATA_HOME/opencad/recovery`, or `~/.local/share/opencad/recovery`.
pub fn default_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share")))?;
    Some(base.join("opencad").join("recovery"))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Meta {
    /// The design's own file, if it has one.
    original: Option<PathBuf>,
    /// Its title when it was autosaved ("bracket v3", "Untitled").
    title: String,
}

/// An autosave left behind by a run that didn't close properly.
#[derive(Clone, Debug, PartialEq)]
pub struct Recovered {
    pub file: PathBuf,
    pub original: Option<PathBuf>,
    pub title: String,
}

fn stem(pid: u32, tab: u64) -> String {
    format!("{pid}-{tab}")
}

/// Autosave tab `tab` of this process.
pub fn save(dir: &Path, pid: u32, tab: u64, doc: &Document, original: Option<&Path>, title: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("couldn't make {}: {e}", dir.display()))?;
    let s = stem(pid, tab);
    crate::doc::save(&dir.join(format!("{s}.opencad")), doc)?;
    let meta = Meta { original: original.map(Path::to_path_buf), title: title.to_string() };
    std::fs::write(dir.join(format!("{s}.json")), serde_json::to_string(&meta).unwrap_or_default()).map_err(|e| e.to_string())
}

/// Remove tab `tab`'s autosave (saved, closed, or recovered).
pub fn remove(dir: &Path, pid: u32, tab: u64) {
    let s = stem(pid, tab);
    let _ = std::fs::remove_file(dir.join(format!("{s}.opencad")));
    let _ = std::fs::remove_file(dir.join(format!("{s}.json")));
}

/// Remove a recovered file (and its note) once it has been opened or discarded.
pub fn forget(r: &Recovered) {
    let _ = std::fs::remove_file(&r.file);
    let _ = std::fs::remove_file(r.file.with_extension("json"));
}

/// Autosaves of processes that aren't running any more (`alive` says
/// whether a process id is), oldest first.
pub fn left_behind(dir: &Path, alive: impl Fn(u32) -> bool) -> Vec<Recovered> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(std::time::SystemTime, Recovered)> = Vec::new();
    for e in entries.flatten() {
        let file = e.path();
        if file.extension().is_none_or(|x| x != "opencad") {
            continue;
        }
        let Some(pid) = file.file_stem().and_then(|s| s.to_str()).and_then(|s| s.split('-').next()).and_then(|p| p.parse::<u32>().ok()) else { continue };
        if alive(pid) {
            continue; // another opencad, still open
        }
        let meta: Meta = std::fs::read_to_string(file.with_extension("json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Meta { original: None, title: "Untitled".into() });
        let when = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        out.push((when, Recovered { file, original: meta.original, title: meta.title }));
    }
    out.sort_by_key(|(t, _)| *t);
    out.into_iter().map(|(_, r)| r).collect()
}

/// Is process `pid` running? (Linux: it has a /proc entry.)
pub fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("opencad-recovery-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn autosaves_of_dead_runs_come_back_and_live_ones_are_left_alone() {
        let dir = temp_dir("roundtrip");
        let doc = Document::default();
        save(&dir, 111, 0, &doc, Some(Path::new("/home/x/bracket.opencad")), "bracket v3").unwrap();
        save(&dir, 222, 1, &doc, None, "Untitled").unwrap();
        // 222 is still running: only 111's is offered.
        let found = left_behind(&dir, |pid| pid == 222);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].original.as_deref(), Some(Path::new("/home/x/bracket.opencad")));
        assert_eq!(found[0].title, "bracket v3");
        assert!(crate::doc::load(&found[0].file).is_ok(), "it opens");
        forget(&found[0]);
        assert!(left_behind(&dir, |_| false).iter().all(|r| r.title == "Untitled"), "forgotten once recovered");
        remove(&dir, 222, 1);
        assert!(left_behind(&dir, |_| false).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_folder_means_nothing_to_recover() {
        assert!(left_behind(&temp_dir("missing"), |_| false).is_empty());
        assert!(process_alive(std::process::id()));
    }
}
