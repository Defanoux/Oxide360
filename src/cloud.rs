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
    /// Where it is on this computer (as last found).
    pub path: PathBuf,
    /// Its network address ("smb://192.168.50.5/defanoux/oxide360") when it's
    /// a share KDE or GNOME mounted: their mount folder changes from one login
    /// to the next, so the folder is found again from this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl CloudFolder {
    /// A folder named after its last part (a network mount's address noted).
    pub fn at(path: PathBuf) -> CloudFolder {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let url = kio_url(&path).or_else(|| gvfs_url(&path));
        CloudFolder { name, path, url }
    }

    /// Where the folder is now: a network share is mounted (again) if it can
    /// be. Slow when a share is away — run it off the main thread.
    pub fn resolve(&self) -> Result<PathBuf, String> {
        if self.path.is_dir() {
            return Ok(self.path.clone());
        }
        if let Some(url) = &self.url {
            if let Some(p) = kio_mount(url).filter(|p| p.is_dir()) {
                return Ok(p);
            }
            if let Some(p) = gvfs_path(&self.path).filter(|_| gio_mount(url)).filter(|p| p.is_dir()) {
                return Ok(p);
            }
            return Err(format!("can't reach {url} — is the server on, and this computer on its network?"));
        }
        Err(format!("{} isn't there — is the share mounted?", self.path.display()))
    }
}

/// `/run/user/1000/kio-fuse-AbCd/smb/host/share/dir` (KDE's mount of a
/// network folder): its address, `smb://host/share/dir`.
pub fn kio_url(path: &Path) -> Option<String> {
    let parts: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let k = parts.iter().position(|p| p.starts_with("kio-fuse-"))?;
    if parts.get(1).map(String::as_str) != Some("run") || parts.get(2).map(String::as_str) != Some("user") {
        return None;
    }
    let scheme = parts.get(k + 1)?;
    let rest = parts.get(k + 2..)?;
    (!rest.is_empty()).then(|| format!("{scheme}://{}", rest.join("/")))
}

/// `/run/user/1000/gvfs/smb-share:server=host,share=s/dir` (GNOME's mount):
/// its address, `smb://host/s/dir`.
pub fn gvfs_url(path: &Path) -> Option<String> {
    let parts: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let k = parts.iter().position(|p| p == "gvfs")?;
    let mount = parts.get(k + 1)?;
    let (kind, args) = mount.split_once(':')?;
    let scheme = kind.strip_suffix("-share").unwrap_or(kind);
    let get = |key: &str| args.split(',').find_map(|kv| kv.strip_prefix(key).and_then(|v| v.strip_prefix('=')));
    let host = get("server").or_else(|| get("host"))?;
    let mut url = format!("{scheme}://{host}");
    if let Some(share) = get("share") {
        url.push('/');
        url.push_str(share);
    }
    for p in &parts[k + 2..] {
        url.push('/');
        url.push_str(p);
    }
    Some(url)
}

/// A gvfs path as it would be now (gvfs mounts keep their folder name).
fn gvfs_path(path: &Path) -> Option<PathBuf> {
    gvfs_url(path).map(|_| path.to_path_buf())
}

/// Ask KDE to mount `url` (KIO FUSE); the local folder it's at.
fn kio_mount(url: &str) -> Option<PathBuf> {
    let out = run_for("gdbus", &["call", "--session", "--dest", "org.kde.KIOFuse", "--object-path", "/org/kde/KIOFuse", "--method", "org.kde.KIOFuse.VFS.mountUrl", url], Duration::from_secs(20))?;
    // ('/run/user/1000/kio-fuse-AbCd/smb/host/share',)
    let start = out.find('\'')? + 1;
    let end = out[start..].find('\'')? + start;
    Some(PathBuf::from(&out[start..end]))
}

/// Ask GNOME to mount `url` (gvfs).
fn gio_mount(url: &str) -> bool {
    run_for("gio", &["mount", url], Duration::from_secs(20)).is_some()
}

/// Run `cmd` and give back what it printed, unless it fails or takes longer than `limit`.
fn run_for(cmd: &str, args: &[&str], limit: Duration) -> Option<String> {
    let mut child = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let began = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                use std::io::Read;
                child.stdout.take()?.read_to_string(&mut out).ok()?;
                return status.success().then_some(out);
            }
            Ok(None) if began.elapsed() < limit => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// What a look through the cloud folders (off the main thread) finds.
#[derive(Debug)]
pub enum Found {
    /// Folder `k`: where it is now and its designs, or why it can't be read.
    Folder(usize, Result<(PathBuf, Vec<Design>), String>),
    /// The Recent list: each design (where it was listed, where it is now),
    /// whether it's there, and when it was saved.
    Recent(Vec<(PathBuf, PathBuf, bool, Option<SystemTime>)>),
}

/// Look through `folders` (mounting shares that need it) and check the
/// `recent` designs, telling `send` as each is done. Recent designs in a
/// folder that's moved (a new login's mount) are looked for where it is now.
pub fn look(folders: &[CloudFolder], recent: &[PathBuf], mut send: impl FnMut(Found) -> bool) {
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (k, f) in folders.iter().enumerate() {
        let found = f.resolve().and_then(|at| {
            if at != f.path {
                moved.push((f.path.clone(), at.clone()));
            }
            list(&at).map(|l| (at, l))
        });
        if !send(Found::Folder(k, found)) {
            return;
        }
    }
    let seen = recent
        .iter()
        .map(|p| {
            let now = moved.iter().find_map(|(from, to)| remap(p, from, to)).unwrap_or_else(|| p.clone());
            let at = stamp(&now);
            (p.clone(), now.clone(), at.is_some() || now.exists(), at)
        })
        .collect();
    send(Found::Recent(seen));
}

/// `path` moved from under `from` to under `to` (a folder found somewhere new).
pub fn remap(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    path.strip_prefix(from).ok().map(|rest| to.join(rest))
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
    /// Written and removed off the main thread (a share can be slow or away).
    background: bool,
}

impl LockGuard {
    /// `take`, with the file written (and later removed) in the background:
    /// opening a design never waits on the share for its lock.
    pub fn take_in_background(design: &Path, lock: Lock) -> LockGuard {
        let (d, l) = (design.to_path_buf(), lock.clone());
        std::thread::spawn(move || {
            if let Ok(text) = serde_json::to_string(&l) {
                let _ = std::fs::write(lock_path(&d), text);
            }
        });
        LockGuard { design: design.to_path_buf(), lock, background: true }
    }

    /// Write the lock for `design` now (a failure — a read-only share — just
    /// means no lock: opening the design matters more).
    #[cfg(test)]
    pub fn take(design: &Path, lock: Lock) -> Option<LockGuard> {
        let text = serde_json::to_string(&lock).ok()?;
        std::fs::write(lock_path(design), text).ok()?;
        Some(LockGuard { design: design.to_path_buf(), lock, background: false })
    }

    pub fn design(&self) -> &Path {
        &self.design
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let (design, lock) = (self.design.clone(), self.lock.clone());
        let let_go = move || {
            if read_lock(&design).as_ref() == Some(&lock) {
                let _ = std::fs::remove_file(lock_path(&design));
            }
        };
        if self.background {
            std::thread::spawn(let_go);
        } else {
            let_go();
        }
    }
}

// ---------- copies on this computer, for working offline ----------

/// How many cloud designs keep a copy on this computer (ones still to be
/// uploaded are kept whatever).
pub const KEEP: usize = 10;

/// A cloud design's copy on this computer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// The design in the cloud folder.
    pub cloud: PathBuf,
    /// The copy's file name in the cache folder.
    pub file: String,
    /// When it was last opened or saved here (seconds since 1970).
    pub used: u64,
    /// The cloud file's time when the two were last the same (nanoseconds
    /// since 1970; None: not in the cloud yet).
    pub base: Option<u64>,
    /// Saved here and not uploaded yet.
    pub pending: bool,
    /// Upload even though the cloud file changed (the user said Overwrite).
    #[serde(default)]
    pub force: bool,
    /// The cloud file changed since, and someone has to choose: left alone until then.
    #[serde(default)]
    pub conflict: bool,
    /// "Available offline": kept whatever, and brought up to date when the cloud one changes.
    #[serde(default)]
    pub keep: bool,
}

fn nanos(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
}

/// The cloud file's time, as `Entry::base` keeps it.
pub fn base_of(path: &Path) -> Option<u64> {
    stamp(path).map(nanos)
}

/// The copies on this computer: a folder with the files and an index.
#[derive(Clone, Debug)]
pub struct Cache {
    pub dir: PathBuf,
}

impl Cache {
    /// `~/.local/share/oxide360/cloud-cache`.
    pub fn default_cache() -> Option<Cache> {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share")))?;
        Some(Cache { dir: base.join("oxide360").join("cloud-cache") })
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index.json")
    }

    pub fn entries(&self) -> Vec<Entry> {
        std::fs::read_to_string(self.index_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    fn write_index(&self, list: &[Entry]) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("couldn't make {}: {e}", self.dir.display()))?;
        let tmp = self.dir.join("index.json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(list).unwrap_or_default()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, self.index_path()).map_err(|e| e.to_string())
    }

    pub fn entry(&self, cloud: &Path) -> Option<Entry> {
        self.entries().into_iter().find(|e| e.cloud == cloud)
    }

    /// Where `cloud`'s copy is (if there is one).
    pub fn copy_of(&self, cloud: &Path) -> Option<PathBuf> {
        self.entry(cloud).map(|e| self.dir.join(e.file)).filter(|p| p.is_file())
    }

    /// Keep `bytes` as `cloud`'s copy (`pending`: still to upload; `base`:
    /// the cloud file's time it matches). The oldest copies past `KEEP` go,
    /// unless they're still to upload. Returns the copy's path.
    pub fn keep(&self, cloud: &Path, bytes: &[u8], base: Option<u64>, pending: bool) -> Result<PathBuf, String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("couldn't make {}: {e}", self.dir.display()))?;
        let mut list = self.entries();
        let old = list.iter().position(|e| e.cloud == cloud).map(|k| list.remove(k));
        let file = old.as_ref().map(|e| e.file.clone()).unwrap_or_else(|| {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            cloud.hash(&mut h);
            let name = cloud.file_name().map_or_else(|| "design".into(), |n| n.to_string_lossy().into_owned());
            format!("{:016x}-{name}", h.finish())
        });
        let path = self.dir.join(&file);
        let tmp = self.dir.join(format!("{file}.tmp"));
        std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, &path)).map_err(|e| format!("couldn't keep a copy on this computer: {e}"))?;
        // Saved here while a choice is waiting: still waiting (and still forced if it was).
        let (force, conflict) = old.as_ref().filter(|_| pending).map_or((false, false), |e| (e.force, e.conflict));
        let keep = old.as_ref().is_some_and(|e| e.keep);
        let base = if pending { old.as_ref().map_or(base, |e| e.base.or(base)) } else { base };
        list.insert(0, Entry { cloud: cloud.to_path_buf(), file, used: unix_now(), base, pending, force, conflict, keep });
        // Too many: the least lately used that are in the cloud already (and not kept on purpose) go.
        let spare = |e: &Entry| !e.pending && !e.keep;
        while list.iter().filter(|e| spare(e)).count() > KEEP {
            let Some(k) = list.iter().rposition(spare) else { break };
            let gone = list.remove(k);
            let _ = std::fs::remove_file(self.dir.join(gone.file));
        }
        self.write_index(&list)?;
        Ok(path)
    }

    /// Change `cloud`'s entry.
    pub fn update(&self, cloud: &Path, f: impl FnOnce(&mut Entry)) {
        let mut list = self.entries();
        if let Some(e) = list.iter_mut().find(|e| e.cloud == cloud) {
            f(e);
            let _ = self.write_index(&list);
        }
    }

    /// Forget `cloud`'s copy (unless it has saves not uploaded yet).
    #[allow(dead_code)] // (the file viewer's menu, next)
    pub fn forget(&self, cloud: &Path) {
        let mut list = self.entries();
        if let Some(k) = list.iter().position(|e| e.cloud == cloud && !e.pending) {
            let gone = list.remove(k);
            let _ = std::fs::remove_file(self.dir.join(gone.file));
            let _ = self.write_index(&list);
        }
    }

    /// Bring the copies kept on purpose up to date with their cloud files
    /// (ones with saves not uploaded are left: they're newer). Off the main thread.
    pub fn refresh_kept(&self) {
        for e in self.entries().into_iter().filter(|e| e.keep && !e.pending) {
            let now = base_of(&e.cloud);
            if now.is_some() && now != e.base {
                if let Ok(bytes) = std::fs::read(&e.cloud) {
                    let _ = self.keep(&e.cloud, &bytes, now, false);
                }
            }
        }
    }

    /// The copies still to upload (and not waiting on a choice).
    pub fn to_upload(&self) -> Vec<Entry> {
        self.entries().into_iter().filter(|e| e.pending && (!e.conflict || e.force)).collect()
    }
}

/// "Available offline" on (copied from the cloud now if there's no copy
/// yet — off the main thread) or off (the copy goes like any other).
#[allow(dead_code)] // (the file viewer's menu, next)
pub fn set_kept(cache: &Cache, cloud: &Path, on: bool) -> Result<(), String> {
    if on && cache.entry(cloud).is_none() {
        let bytes = std::fs::read(cloud).map_err(|e| format!("couldn't copy it from the cloud: {e}"))?;
        cache.keep(cloud, &bytes, base_of(cloud), false)?;
    }
    cache.update(cloud, |e| e.keep = on);
    Ok(())
}

/// Where designs are saved on this computer when there's no cloud folder:
/// the Documents folder (as the desktop names it) / Oxide360.
pub fn local_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".config"));
    let docs = std::fs::read_to_string(config.join("user-dirs.dirs")).ok().and_then(|t| documents_from(&t, &home)).unwrap_or_else(|| home.join("Documents"));
    Some(docs.join("Oxide360"))
}

/// `XDG_DOCUMENTS_DIR="$HOME/Dokumente"` out of a `user-dirs.dirs` file.
fn documents_from(text: &str, home: &Path) -> Option<PathBuf> {
    let line = text.lines().find(|l| l.trim_start().starts_with("XDG_DOCUMENTS_DIR="))?;
    let v = line.split_once('=')?.1.trim().trim_matches('"');
    let p = match v.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(v),
    };
    (p.is_absolute() && p != home).then_some(p)
}

/// What uploading a copy did.
#[derive(Clone, Debug, PartialEq)]
pub enum Uploaded {
    /// In the cloud now; the cloud file's new time.
    Done(Option<u64>),
    /// The cloud file was saved from elsewhere since: not replaced.
    Changed,
    /// Saved next to it under another name instead (`conflict_copies`).
    AsCopy(PathBuf),
    /// The cloud folder can't be reached: try again later.
    Away(String),
}

/// The name a copy that couldn't replace `cloud` is uploaded under:
/// "key (conflict, framework 2026-11-05 14.05).ox360".
pub fn conflict_name(cloud: &Path, host: &str, when: SystemTime) -> PathBuf {
    let stem = cloud.file_stem().map_or_else(|| "design".into(), |s| s.to_string_lossy().into_owned());
    let secs = when.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (y, m, d) = civil(secs.div_euclid(86400));
    let (hh, mm) = (secs.rem_euclid(86400) / 3600, secs.rem_euclid(3600) / 60);
    cloud.with_file_name(format!("{stem} (conflict, {host} {y:04}-{m:02}-{d:02} {hh:02}.{mm:02}).{}", crate::doc::EXTENSION))
}

/// Days since 1970 as (year, month, day) (UTC).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Upload `e`'s copy from `cache` to its cloud file. A cloud file saved from
/// elsewhere since the copy was made isn't replaced: `Changed` (when
/// `as_copy` is false: someone will choose), or uploaded beside it under a
/// conflict name. Runs off the main thread.
pub fn upload(cache: &Cache, e: &Entry, as_copy: bool) -> Uploaded {
    let Some(dir) = e.cloud.parent() else { return Uploaded::Away("no folder".into()) };
    if !dir.is_dir() {
        return Uploaded::Away(format!("{} can't be reached", dir.display()));
    }
    let Ok(bytes) = std::fs::read(cache.dir.join(&e.file)) else { return Uploaded::Away("the copy on this computer is gone".into()) };
    let now = base_of(&e.cloud);
    let changed = now.is_some() && now != e.base && !e.force;
    let to = if changed {
        if !as_copy {
            return Uploaded::Changed;
        }
        conflict_name(&e.cloud, &host_name(), SystemTime::now())
    } else {
        e.cloud.clone()
    };
    let tmp = to.with_extension(format!("{}.tmp", crate::doc::EXTENSION));
    if let Err(err) = std::fs::write(&tmp, &bytes).and_then(|()| std::fs::rename(&tmp, &to)) {
        let _ = std::fs::remove_file(&tmp);
        return Uploaded::Away(format!("couldn't write {}: {err}", to.display()));
    }
    if changed { Uploaded::AsCopy(to) } else { Uploaded::Done(base_of(&e.cloud)) }
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
        let stale = LockGuard { design: design.clone(), lock: me.clone(), background: false };
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
    fn network_mounts_remember_their_address() {
        let kde = Path::new("/run/user/1000/kio-fuse-sqeqML/smb/192.168.50.5/defanoux/oxide360");
        assert_eq!(kio_url(kde).as_deref(), Some("smb://192.168.50.5/defanoux/oxide360"));
        assert_eq!(kio_url(Path::new("/run/user/1000/kio-fuse-x/webdavs/cloud.example.org/remote.php/dav/files/sam")).as_deref(), Some("webdavs://cloud.example.org/remote.php/dav/files/sam"));
        let gnome = Path::new("/run/user/1000/gvfs/smb-share:server=192.168.50.5,share=defanoux/oxide360");
        assert_eq!(gvfs_url(gnome).as_deref(), Some("smb://192.168.50.5/defanoux/oxide360"));
        assert_eq!(kio_url(Path::new("/home/g/kio-fuse-notes/a")), None, "only KDE's own mount folder");
        assert_eq!(gvfs_url(Path::new("/mnt/truenas")), None);
        let f = CloudFolder::at(kde.to_path_buf());
        assert_eq!((f.name.as_str(), f.url.as_deref()), ("oxide360", Some("smb://192.168.50.5/defanoux/oxide360")));
        // Found again after a new login: recent designs move with it.
        let now = Path::new("/run/user/1000/kio-fuse-QQQQ/smb/192.168.50.5/defanoux/oxide360");
        assert_eq!(remap(&kde.join("keys/key.ox360"), kde, now), Some(now.join("keys/key.ox360")));
        assert_eq!(remap(Path::new("/home/g/a.ox360"), kde, now), None);
        // A plain folder that's there is just there.
        let d = temp("resolve");
        assert_eq!(CloudFolder::at(d.clone()).resolve(), Ok(d.clone()));
        assert!(CloudFolder::at(d.join("gone")).resolve().unwrap_err().contains("mounted"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_look_lists_each_folder_then_checks_recent() {
        let d = temp("look");
        std::fs::write(d.join("a.ox360"), "{}").unwrap();
        let folders = vec![CloudFolder::at(d.clone()), CloudFolder::at(d.join("away"))];
        let recent = vec![d.join("a.ox360"), d.join("gone.ox360")];
        let mut got = Vec::new();
        look(&folders, &recent, |f| {
            got.push(f);
            true
        });
        assert!(matches!(&got[0], Found::Folder(0, Ok((at, l))) if *at == d && l.len() == 1));
        assert!(matches!(&got[1], Found::Folder(1, Err(_))));
        let Found::Recent(seen) = &got[2] else { panic!("{got:?}") };
        assert_eq!(seen.iter().map(|s| s.2).collect::<Vec<_>>(), vec![true, false]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn copies_are_kept_for_offline_and_the_oldest_go() {
        let d = temp("cache");
        let cache = Cache { dir: d.join("cache") };
        let cloud = d.join("nas");
        std::fs::create_dir_all(&cloud).unwrap();
        for k in 0..12 {
            cache.keep(&cloud.join(format!("d{k}.ox360")), b"x", Some(1), false).unwrap();
        }
        let list = cache.entries();
        assert_eq!(list.len(), KEEP, "ten kept");
        assert_eq!(list[0].cloud, cloud.join("d11.ox360"), "newest first");
        assert!(cache.copy_of(&cloud.join("d0.ox360")).is_none(), "the oldest went");
        assert_eq!(std::fs::read_dir(&cache.dir).unwrap().count(), KEEP + 1, "its file too (+ the index)");
        // One saved offline is kept whatever.
        cache.keep(&cloud.join("offline.ox360"), b"mine", None, true).unwrap();
        for k in 20..32 {
            cache.keep(&cloud.join(format!("d{k}.ox360")), b"x", Some(1), false).unwrap();
        }
        assert!(cache.copy_of(&cloud.join("offline.ox360")).is_some());
        assert_eq!(cache.to_upload().len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_copy_uploads_unless_the_cloud_one_changed_since() {
        let d = temp("upload");
        let cache = Cache { dir: d.join("cache") };
        let nas = d.join("nas");
        std::fs::create_dir_all(&nas).unwrap();
        let design = nas.join("key.ox360");
        std::fs::write(&design, "theirs v1").unwrap();
        // Opened (copy kept, matching the cloud), then saved offline.
        cache.keep(&design, b"theirs v1", base_of(&design), false).unwrap();
        cache.keep(&design, b"mine", None, true).unwrap();
        let e = cache.entry(&design).unwrap();
        assert!(e.pending && e.base == base_of(&design), "still based on what was opened");
        assert!(matches!(upload(&cache, &e, false), Uploaded::Done(Some(_))));
        assert_eq!(std::fs::read_to_string(&design).unwrap(), "mine");
        // Someone saved there since: not replaced…
        cache.keep(&design, b"mine v2", base_of(&design), false).unwrap();
        cache.keep(&design, b"mine v3", None, true).unwrap();
        std::fs::write(&design, "theirs v2").unwrap();
        std::fs::File::options().write(true).open(&design).unwrap().set_modified(SystemTime::now() + Duration::from_secs(9)).unwrap();
        let e = cache.entry(&design).unwrap();
        assert_eq!(upload(&cache, &e, false), Uploaded::Changed);
        assert_eq!(std::fs::read_to_string(&design).unwrap(), "theirs v2");
        // … put beside it when nobody's there to choose …
        let Uploaded::AsCopy(beside) = upload(&cache, &e, true) else { panic!() };
        assert!(beside.file_name().unwrap().to_string_lossy().starts_with("key (conflict, "), "{beside:?}");
        assert_eq!(std::fs::read_to_string(&beside).unwrap(), "mine v3");
        assert_eq!(std::fs::read_to_string(&design).unwrap(), "theirs v2");
        // … or replaced when told to.
        cache.update(&design, |e| e.force = true);
        assert!(matches!(upload(&cache, &cache.entry(&design).unwrap(), false), Uploaded::Done(_)));
        assert_eq!(std::fs::read_to_string(&design).unwrap(), "mine v3");
        // Away: says so, nothing written.
        let gone = Entry { cloud: d.join("unplugged/key.ox360"), ..cache.entry(&design).unwrap() };
        assert!(matches!(upload(&cache, &gone, false), Uploaded::Away(_)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_design_kept_offline_stays_and_follows_the_cloud() {
        let d = temp("kept");
        let cache = Cache { dir: d.join("cache") };
        let nas = d.join("nas");
        std::fs::create_dir_all(&nas).unwrap();
        let key = nas.join("key.ox360");
        std::fs::write(&key, "v1").unwrap();
        set_kept(&cache, &key, true).unwrap();
        assert!(cache.entry(&key).unwrap().keep, "copied and kept");
        for k in 0..15 {
            cache.keep(&nas.join(format!("d{k}.ox360")), b"x", Some(1), false).unwrap();
        }
        assert!(cache.copy_of(&key).is_some(), "not pushed out by newer ones");
        // Changed in the cloud: brought up to date.
        std::fs::write(&key, "v2").unwrap();
        std::fs::File::options().write(true).open(&key).unwrap().set_modified(SystemTime::now() + Duration::from_secs(9)).unwrap();
        cache.refresh_kept();
        assert_eq!(std::fs::read_to_string(cache.copy_of(&key).unwrap()).unwrap(), "v2");
        set_kept(&cache, &key, false).unwrap();
        cache.forget(&key);
        assert!(cache.copy_of(&key).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_local_folder_is_in_the_desktops_documents() {
        let home = Path::new("/home/g");
        assert_eq!(documents_from("# xdg\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOCUMENTS_DIR=\"$HOME/Dokumente\"\n", home), Some(PathBuf::from("/home/g/Dokumente")));
        assert_eq!(documents_from("XDG_DOCUMENTS_DIR=\"/data/docs\"", home), Some(PathBuf::from("/data/docs")));
        assert_eq!(documents_from("XDG_DOCUMENTS_DIR=\"$HOME/\"", home), None, "unset (home itself)");
        assert_eq!(documents_from("", home), None);
    }

    #[test]
    fn conflict_copies_say_where_and_when() {
        let t = UNIX_EPOCH + Duration::from_secs(1_793_889_141); // 2026-11-05 14:32 UTC
        assert_eq!(conflict_name(Path::new("/nas/key.ox360"), "framework", t), PathBuf::from("/nas/key (conflict, framework 2026-11-05 14.32).ox360"));
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19723), (2024, 1, 1));
    }

    #[test]
    fn a_design_is_in_the_cloud_when_its_under_a_cloud_folder() {
        let f = vec![CloudFolder::at(PathBuf::from("/mnt/truenas/cad"))];
        assert_eq!(f[0].name, "cad");
        assert!(in_cloud(&f, Path::new("/mnt/truenas/cad/keys/key.ox360")));
        assert!(!in_cloud(&f, Path::new("/home/g/Documents/key.ox360")));
    }
}
