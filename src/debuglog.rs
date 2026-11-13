//! A log of each run, for finding out why it crashed: what the app was
//! doing (a line per notable step, written straight to disk so nothing is
//! lost when the process dies), everything printed to the terminal (stderr
//! goes here too: GPU and kernel messages), and a panic's backtrace.
//!
//! While the app runs, `logs/running-<pid>` names its log. A normal close
//! removes it; one left behind by a process that's gone means that run
//! crashed (or was force-quit), and the next start says where its log is.
//! The last `KEEP` logs are kept, in `$XDG_DATA_HOME/oxide360/logs`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const KEEP: usize = 10;

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);
static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// `$XDG_DATA_HOME/oxide360/logs` (in the Flatpak, inside its own data folder).
pub fn dir() -> Option<PathBuf> {
    crate::recovery::default_dir().and_then(|d| d.parent().map(|p| p.join("logs")))
}

fn marker(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("running-{pid}"))
}

/// Logs of runs that ended without closing properly (their process is
/// gone): the markers are cleared, the logs returned (newest first).
pub fn crashed(dir: &Path, alive: impl Fn(u32) -> bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(pid) = name.strip_prefix("running-").and_then(|p| p.parse::<u32>().ok()) else { continue };
        if alive(pid) && pid != std::process::id() {
            continue;
        }
        if let Ok(log) = std::fs::read_to_string(e.path()) {
            let log = PathBuf::from(log.trim());
            if log.is_file() {
                out.push(log);
            }
        }
        let _ = std::fs::remove_file(e.path());
    }
    out.sort();
    out.reverse();
    out
}

/// Only the newest `keep` logs stay (never one a running app is writing).
pub fn prune(dir: &Path, keep: usize, in_use: &[PathBuf]) {
    let mut logs: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "log")).collect();
    logs.sort();
    let extra = logs.len().saturating_sub(keep);
    for p in logs.into_iter().take(extra) {
        if !in_use.contains(&p) {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// A file-name-safe local time stamp ("2026-11-12_21-04-33").
fn stamp() -> String {
    let out = std::process::Command::new("date").arg("+%Y-%m-%d_%H-%M-%S").stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output();
    out.ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()).to_string()
    })
}

/// Start this run's log; returns crashed runs' logs (for the popup).
pub fn start() -> Vec<PathBuf> {
    let _ = STARTED.set(std::time::Instant::now());
    let Some(dir) = dir() else { return Vec::new() };
    if std::fs::create_dir_all(&dir).is_err() {
        return Vec::new();
    }
    let crashed = crashed(&dir, crate::recovery::process_alive);
    let path = dir.join(format!("oxide360_{}_{}.log", stamp(), std::process::id()));
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else { return crashed };
    let _ = std::fs::write(marker(&dir, std::process::id()), path.display().to_string());
    if let Ok(mut l) = LOG.lock() {
        *l = Some(file);
    }
    prune(&dir, KEEP, &[&[path.clone()], crashed.as_slice()].concat());
    line(&format!("Oxide360 {} starting (pid {})", env!("CARGO_PKG_VERSION"), std::process::id()));
    line(&format!("System: {}", system()));
    tee_stderr();
    install_panic_hook();
    crashed
}

/// What it's running on: the distribution, kernel, desktop and session,
/// and whether it's the Flatpak.
fn system() -> String {
    let os = std::fs::read_to_string("/etc/os-release").ok().and_then(|t| t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string()))).unwrap_or_else(|| "unknown Linux".into());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").map(|s| s.trim().to_string()).unwrap_or_default();
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let flatpak = if std::env::var_os("FLATPAK_ID").is_some() { ", Flatpak" } else { "" };
    format!("{os}, kernel {kernel}, desktop {} ({}){flatpak}", env("XDG_CURRENT_DESKTOP"), env("XDG_SESSION_TYPE"))
}

/// A line in the log, with the seconds since start.
pub fn line(text: &str) {
    let t = STARTED.get().map_or(0.0, |s| s.elapsed().as_secs_f64());
    if let Ok(mut l) = LOG.lock() {
        if let Some(f) = l.as_mut() {
            let _ = writeln!(f, "[{t:9.3}] {text}");
            let _ = f.flush();
        }
    }
}

/// The run closed normally: no crash to report next time.
pub fn closed() {
    line("closed normally");
    if let Some(dir) = dir() {
        let _ = std::fs::remove_file(marker(&dir, std::process::id()));
    }
}

/// A panic: its message, where, and the backtrace, then Rust's usual report.
fn install_panic_hook() {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        line(&format!("PANIC in thread {thread}: {info}\n{bt}"));
        before(info);
    }));
}

/// Everything written to stderr also goes into the log (and still to the
/// terminal): a pipe in its place, read by a thread that writes both.
fn tee_stderr() {
    unsafe extern "C" {
        fn pipe(fds: *mut i32) -> i32;
        fn dup(fd: i32) -> i32;
        fn dup2(old: i32, new: i32) -> i32;
    }
    use std::os::fd::FromRawFd;
    let mut fds = [0i32; 2];
    // SAFETY: plain POSIX calls on descriptors this process owns; each new
    // descriptor is handed to exactly one File.
    unsafe {
        if pipe(fds.as_mut_ptr()) != 0 {
            return;
        }
        let terminal = dup(2);
        if terminal < 0 || dup2(fds[1], 2) < 0 {
            return;
        }
        let mut reader = std::fs::File::from_raw_fd(fds[0]);
        let mut terminal = std::fs::File::from_raw_fd(terminal);
        let _write_end = std::fs::File::from_raw_fd(fds[1]); // (fd 2 keeps the pipe open)
        std::mem::forget(_write_end);
        let _ = std::thread::Builder::new().name("stderr-log".into()).spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 4096];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let _ = terminal.write_all(&buf[..n]);
                if let Ok(mut l) = LOG.lock() {
                    if let Some(f) = l.as_mut() {
                        let _ = f.write_all(&buf[..n]);
                        let _ = f.flush();
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oxide360-logs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_run_that_never_closed_is_found_once() {
        let d = temp("crash");
        let log = d.join("oxide360_a.log");
        std::fs::write(&log, "x").unwrap();
        std::fs::write(marker(&d, 999_991), log.display().to_string()).unwrap();
        // One still running is left alone.
        let other = d.join("oxide360_b.log");
        std::fs::write(&other, "y").unwrap();
        std::fs::write(marker(&d, 999_992), other.display().to_string()).unwrap();
        let alive = |pid: u32| pid == 999_992;
        assert_eq!(crashed(&d, alive), vec![log]);
        assert!(crashed(&d, alive).is_empty(), "told once");
        assert!(marker(&d, 999_992).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn only_the_newest_logs_are_kept() {
        let d = temp("prune");
        for k in 0..14 {
            std::fs::write(d.join(format!("oxide360_2026-11-{k:02}.log")), "").unwrap();
        }
        let in_use = vec![d.join("oxide360_2026-11-00.log")];
        prune(&d, 10, &in_use);
        let mut left: Vec<String> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left.len(), 11, "10 newest, plus the one in use: {left:?}");
        assert_eq!(left[0], "oxide360_2026-11-00.log");
        assert_eq!(left[1], "oxide360_2026-11-04.log");
        let _ = std::fs::remove_dir_all(&d);
    }
}
