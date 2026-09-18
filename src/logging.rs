//! A minimal append-only diagnostic log.
//!
//! The dashboard runs under an alternate screen, so stderr is invisible: a collector failure
//! written there is lost the moment the frame redraws. Before this existed, the only trace of
//! a failing collector was a fragment of a concatenated header string, and a *panicking* one
//! left no trace at all.
//!
//! Off unless `AI_USAGE_LOG` is set. A monitoring tool has no business quietly accumulating a
//! log file on a user's disk, and the privacy boundary is easier to reason about when the
//! default is "writes nothing."
//!
//! Bounded: past [`MAX_LOG_BYTES`] the file is renamed to `<name>.old`, replacing the previous
//! one, and a fresh file is started -- so the log and its one backup never hold more than twice
//! the cap. It used to grow for as long as the variable stayed set. Several processes write the
//! same file at once (a dashboard, every hook, every status-line redraw), and the rotation is
//! built for that without a lock file: whoever finds the *path* over the cap renames it, and a
//! process whose open handle is over the cap while the path is not knows it is holding the
//! backup and reopens. One race is accepted: two processes can both see the path over the cap,
//! and the second rename then replaces the big backup with the first one's small new file. What
//! is lost is old diagnostics, within a window of microseconds, and the alternative is a lock
//! this module is deliberately too small to carry.
//!
//! Deliberately not `tracing`. What is needed here is "collector errors survive to a file the
//! user can read," not spans, subscribers, or structured fields — and this project keeps its
//! dependency surface small enough that `cargo deny` output stays reviewable by hand.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// The size past which the log is rotated. Five mebibytes is weeks of an open dashboard now
/// that a poll is logged when its count changes and not every thirty seconds.
pub const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        })
    }
}

struct Sink {
    path: PathBuf,
    file: Mutex<File>,
}

impl Sink {
    fn open(path: PathBuf) -> Option<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok()?;
        }
        let file = open_for_append(&path).ok()?;
        Some(Sink {
            path,
            file: Mutex::new(file),
        })
    }

    /// Append one line, rotating first if the file has outgrown `cap`. Errors are ignored, as
    /// they always were here: a log that cannot be written must not fail what it is logging.
    fn write_line(&self, line: &str, cap: u64) {
        // A poisoned log mutex must not take down the collector that is trying to report a
        // failure, so recover the guard rather than unwrapping it.
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        rotate_if_over(&self.path, &mut file, cap);
        let _ = file.write_all(line.as_bytes());
        let _ = file.flush();
    }
}

fn open_for_append(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// `<name>.old`, appended to the whole file name: `with_extension` would turn `x.log` into
/// `x.old` and a log named `diagnostics` into one that collides with nothing predictable.
pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".old");
    PathBuf::from(name)
}

/// See the module documentation for why this needs no lock.
fn rotate_if_over(path: &Path, file: &mut File, cap: u64) {
    let over = |len: std::io::Result<u64>| len.is_ok_and(|len| len > cap);
    // One `fstat` a line, on the handle: cheap, and true whatever happened to the path.
    if !over(file.metadata().map(|meta| meta.len())) {
        return;
    }
    // The handle is over the cap. If the path is as well, they are the same file and it is ours
    // to move aside; if it is not, another process already did and this handle is the backup.
    if over(fs::metadata(path).map(|meta| meta.len())) {
        let _ = fs::rename(path, backup_path(path));
    }
    // Reopen either way. If that fails the old handle is kept, so the line lands in the backup
    // rather than nowhere.
    if let Ok(fresh) = open_for_append(path) {
        *file = fresh;
    }
}

static SINK: OnceLock<Option<Sink>> = OnceLock::new();

/// Where the log would be written when `AI_USAGE_LOG` is truthy but not a path.
///
/// Public for `--uninstall`, which removes it: the one file this tool writes that no cache path
/// function names.
pub fn default_log_path() -> Option<PathBuf> {
    Some(
        crate::utils::data_root()?
            .join("ai-usage-tui")
            .join("ai-usage-tui.log"),
    )
}

/// The rotated backup of [`default_log_path`], which `--uninstall` removes with it.
pub fn default_log_backup_path() -> Option<PathBuf> {
    default_log_path().map(|path| backup_path(&path))
}

/// Resolve `AI_USAGE_LOG` into a destination.
///
/// Unset, empty, `0`, `off`, `false`, `no` disable logging. `1`, `on`, `true`, `yes` select the
/// default path under the data directory. Anything else is taken as a literal path, so
/// `AI_USAGE_LOG=/tmp/usage.log` works without a second variable.
fn configured_path(value: Option<&str>) -> Option<PathBuf> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    match value.to_ascii_lowercase().as_str() {
        "0" | "off" | "false" | "no" => None,
        "1" | "on" | "true" | "yes" => default_log_path(),
        _ => Some(PathBuf::from(value)),
    }
}

fn sink() -> Option<&'static Sink> {
    SINK.get_or_init(|| {
        let path = configured_path(std::env::var("AI_USAGE_LOG").ok().as_deref())?;
        Sink::open(path)
    })
    .as_ref()
}

/// The active log file, if logging is enabled. Shown in `--help` output and the status line so
/// a user who hits a problem can be told where to look.
pub fn log_path() -> Option<PathBuf> {
    sink().map(|sink| sink.path.clone())
}

pub fn log(level: Level, target: &str, message: &str) {
    let Some(sink) = sink() else {
        return;
    };
    let line = format!(
        "{} {:<5} {}: {}\n",
        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z"),
        level.to_string(),
        target,
        message
    );
    sink.write_line(&line, MAX_LOG_BYTES);
}

pub fn info(target: &str, message: &str) {
    log(Level::Info, target, message);
}

pub fn warn(target: &str, message: &str) {
    log(Level::Warn, target, message);
}

pub fn error(target: &str, message: &str) {
    log(Level::Error, target, message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logging_is_off_unless_explicitly_enabled() {
        assert_eq!(configured_path(None), None);
        assert_eq!(configured_path(Some("")), None);
        assert_eq!(configured_path(Some("0")), None);
        assert_eq!(configured_path(Some("off")), None);
        assert_eq!(configured_path(Some("FALSE")), None);
    }

    #[test]
    fn an_explicit_path_is_used_verbatim() {
        assert_eq!(
            configured_path(Some("/tmp/ai-usage.log")),
            Some(PathBuf::from("/tmp/ai-usage.log"))
        );
    }

    #[test]
    fn truthy_values_select_the_default_path() {
        // Only assert the shape: the default path depends on the host's data directory, which
        // is exactly what this indirection exists to hide.
        if default_log_path().is_some() {
            assert_eq!(configured_path(Some("1")), default_log_path());
            assert_eq!(configured_path(Some("yes")), default_log_path());
        }
    }

    #[test]
    fn levels_render_as_fixed_width_labels() {
        assert_eq!(Level::Info.to_string(), "INFO");
        assert_eq!(Level::Warn.to_string(), "WARN");
        assert_eq!(Level::Error.to_string(), "ERROR");
    }

    fn sink_in(dir: &tempfile::TempDir) -> (Sink, PathBuf) {
        let path = dir.path().join("sub").join("x.log");
        (Sink::open(path.clone()).expect("sink"), path)
    }

    /// Bug: a log that grew for as long as `AI_USAGE_LOG` stayed set.
    #[test]
    fn a_log_over_the_cap_is_moved_aside_and_started_again() {
        let dir = tempfile::TempDir::new().unwrap();
        let (sink, path) = sink_in(&dir);
        let backup = backup_path(&path);
        assert_eq!(
            backup.file_name().unwrap(),
            "x.log.old",
            "appended, not substituted"
        );

        sink.write_line("first line, well past a cap of ten bytes\n", 10);
        assert!(!backup.exists(), "nothing to rotate before the first write");
        sink.write_line("second\n", 10);
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            "first line, well past a cap of ten bytes\n"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "second\n");

        // One backup, replaced: the pair never holds more than twice the cap plus a line.
        sink.write_line("third, also longer than the cap\n", 3);
        sink.write_line("fourth\n", 3);
        assert_eq!(fs::read_to_string(&path).unwrap(), "fourth\n");
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            "third, also longer than the cap\n",
            "the previous backup is replaced, not kept beside"
        );
        // Under the cap, nothing moves.
        sink.write_line("fifth\n", 1_000);
        assert_eq!(fs::read_to_string(&path).unwrap(), "fourth\nfifth\n");
    }

    /// Bug: a dashboard whose log another process rotated goes on writing into the backup for
    /// the rest of its life -- or worse, renames the small new file over the big backup.
    #[test]
    fn a_handle_another_process_rotated_reopens_the_live_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let (dashboard, path) = sink_in(&dir);
        let hook = Sink::open(path.clone()).expect("a second process");
        let backup = backup_path(&path);

        dashboard.write_line("a long line from the dashboard\n", 10);
        hook.write_line("hook\n", 10);
        assert_eq!(fs::read_to_string(&path).unwrap(), "hook\n");
        let moved_aside = fs::read_to_string(&backup).unwrap();

        // The dashboard's handle is now the backup: over the cap, while the path is not.
        dashboard.write_line("dash\n", 10);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "hook\ndash\n",
            "the line went into the backup instead of the live file"
        );
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            moved_aside,
            "the small live file was renamed over the backup"
        );
    }

    /// Bug: a rotation that cannot happen taking the line, or the process, with it.
    #[test]
    fn a_rotation_that_fails_still_writes_the_line() {
        let dir = tempfile::TempDir::new().unwrap();
        let (sink, path) = sink_in(&dir);
        fs::create_dir_all(backup_path(&path).join("in-the-way")).unwrap();
        sink.write_line("a long line, past the cap\n", 5);
        sink.write_line("kept\n", 5);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "a long line, past the cap\nkept\n"
        );
    }
}
