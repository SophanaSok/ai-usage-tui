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
//! Deliberately not `tracing`. What is needed here is "collector errors survive to a file the
//! user can read," not spans, subscribers, or structured fields — and this project keeps its
//! dependency surface small enough that `cargo deny` output stays reviewable by hand.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

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

static SINK: OnceLock<Option<Sink>> = OnceLock::new();

/// Where the log would be written when `AI_USAGE_LOG` is truthy but not a path.
fn default_log_path() -> Option<PathBuf> {
    Some(
        crate::utils::data_root()?
            .join("ai-usage-tui")
            .join("ai-usage-tui.log"),
    )
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
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok()?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()?;
        Some(Sink {
            path,
            file: Mutex::new(file),
        })
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
    // A poisoned log mutex must not take down the collector that is trying to report a
    // failure, so recover the guard rather than unwrapping it.
    let mut file = sink
        .file
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _ = file.write_all(line.as_bytes());
    let _ = file.flush();
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
}
