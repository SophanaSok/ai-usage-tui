use serde_json::Value;

pub fn string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).map(String::from))
}
/// The largest count believed. No request is near it, and above it a float no longer holds an
/// integer exactly -- so a figure past this is a format change or a damaged file, not a reading.
pub const MAX_COUNT: u64 = 1 << 53;

/// A JSON value as a token count, or `None` when it is not one.
///
/// A whole number from zero to [`MAX_COUNT`], written as an integer or as a float (`1200.0` is
/// what some writers emit). Everything else is refused: a negative, a fraction, a string, a
/// number too large to be one. Each of those used to become a figure -- `-1` read as `0`, `1.5`
/// as `1`, and `1e308` or anything past `u64::MAX` as 18,446,744,073,709,551,615 tokens, which
/// then overflowed the first total it was added to. Found by `collector::mutation`.
fn as_count(field: &Value) -> Option<u64> {
    let whole = match field.as_u64() {
        Some(whole) => whole,
        None => {
            let float = field.as_f64()?;
            if !float.is_finite() || float < 0.0 || float.fract() != 0.0 {
                return None;
            }
            // Saturates above `u64::MAX`, which the bound below then refuses.
            float as u64
        }
    };
    (whole <= MAX_COUNT).then_some(whole)
}

/// A count under any of `keys`, or `None` when the record carries none of them -- or carries
/// one that is not a count (see `as_count`).
///
/// `number` answers `0` for an absent key, which is right for a field a source only sometimes
/// reports (cache, reasoning) and wrong for one it always does: there, absent means the format
/// changed, and `0` is an invented reading of it.
pub fn count(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| as_count(value.get(*key)?))
}

/// `count`, noting in `missing` when the field was not there. For the fields a source always
/// reports: the caller marks the row `incomplete` rather than trusting the `0`.
pub fn required(value: &Value, keys: &[&str], missing: &mut bool) -> u64 {
    count(value, keys).unwrap_or_else(|| {
        *missing = true;
        0
    })
}

/// A count a source only sometimes reports: `0` when it is absent, or is not a count.
pub fn number(value: &Value, keys: &[&str]) -> u64 {
    count(value, keys).unwrap_or(0)
}

/// Write a line to stdout, returning the I/O error instead of panicking on it.
///
/// `println!` panics when the write fails, and a closed pipe is a write failure: `ai-usage-tui
/// --json | head` aborted with "failed printing to stdout: Broken pipe" rather than exiting
/// cleanly, as did `| grep -q` and quitting out of `| less`. The usual fix is to restore the
/// default `SIGPIPE` disposition, which needs `libc` and an `unsafe` block; this crate has
/// neither and is not going to acquire them for a print.
pub fn print_line(line: &str) -> std::io::Result<()> {
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    out.write_all(line.as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()
}

/// Replace `path` with `contents` so a reader sees the old file or the new one, never half of one.
///
/// Temporary-then-rename, with the temporary named per process. Every cache this tool writes has
/// more than one writer: two dashboards each run the `zen_pricing` collector, a scheduled
/// `--check-update` can land beside an opted-in `--doctor`, and each open Claude Code session feeds
/// `--statusline`. Writers that share a temporary race -- the first rename moves the second's
/// half-written file into place, and the second rename finds nothing to move. That rule was
/// written down in `statusline` and `omarchy::record` and not followed by the update and pricing
/// caches; it lives here now so there is one spelling of it. A temporary that could not be renamed
/// is removed, so a failed write leaves nothing behind.
pub fn write_atomic(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    write_atomic_with_mode(path, contents, None)
}

/// [`write_atomic`], with the file's permission bits chosen by the caller.
///
/// `None` leaves them to the umask, as every cache does. `Some(mode)` is for a file that is not
/// this tool's: `install` rewrites Claude Code's `settings.json`, whose `env` block may hold
/// keys, and a rename would otherwise replace a `0600` file the user set with a `0644` one. The
/// temporary is created owner-only and widened to `mode` before it holds a byte, so no wider
/// half-written file ever exists. On other platforms the mode is ignored.
pub fn write_atomic_with_mode(
    path: &std::path::Path,
    contents: &[u8],
    mode: Option<u32>,
) -> std::io::Result<()> {
    use std::io::Write as _;

    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    let temporary = path.with_file_name(name);
    let written = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if mode.is_some() {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt as _;
            // `OpenOptions::mode` is masked by the umask; this is not.
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        file.write_all(contents)?;
        file.flush()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// Whether an error is a downstream reader closing the pipe — a normal way for a command in a
/// pipeline to end, not a failure to report.
pub fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe)
}

#[cfg(test)]
mod tests {
    /// Two processes writing one cache must not share a temporary: the rename of one would move
    /// the other's half-written file into place. The update and pricing caches used a fixed
    /// `json.tmp` / `toml.tmp`, on the belief that only the dashboard ever wrote them.
    #[test]
    fn write_atomic_replaces_the_file_and_leaves_no_temporary_behind() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("cache.json");
        super::write_atomic(&path, b"one").unwrap();
        super::write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");

        // A directory where the file should go makes the rename fail after the temporary exists.
        let blocked = dir.path().join("blocked.json");
        std::fs::create_dir(&blocked).unwrap();
        assert!(super::write_atomic(&blocked, b"x").is_err());

        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.iter().all(|name| !name.ends_with(".tmp")),
            "temporaries left behind: {names:?}"
        );
    }

    use super::*;

    #[test]
    fn a_closed_pipe_is_recognised_regardless_of_context() {
        let error = anyhow::Error::from(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "closed",
        ))
        .context("writing usage JSON");
        assert!(is_broken_pipe(&error));
    }

    #[test]
    fn other_io_errors_are_not_treated_as_a_closed_pipe() {
        let error = anyhow::Error::from(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "nope",
        ));
        assert!(!is_broken_pipe(&error));
        assert!(!is_broken_pipe(&anyhow::anyhow!("unrelated")));
    }
}

#[cfg(test)]
mod count_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_count_is_a_whole_number_in_range_and_nothing_else() {
        let record = json!({
            "int": 1200, "float": 1200.0, "zero": 0, "max": MAX_COUNT,
            "negative": -1, "fraction": 1.5, "huge_float": 1e308, "past_max": MAX_COUNT + 1,
            "i64_max": i64::MAX, "text": "1200", "null": null, "object": {},
        });
        for (key, expected) in [
            ("int", 1200),
            ("float", 1200),
            ("zero", 0),
            ("max", MAX_COUNT),
        ] {
            assert_eq!(count(&record, &[key]), Some(expected), "{key}");
        }
        for key in [
            "negative",
            "fraction",
            "huge_float",
            "past_max",
            "i64_max",
            "text",
            "null",
            "object",
            "absent",
        ] {
            assert_eq!(count(&record, &[key]), None, "{key}");
            assert_eq!(number(&record, &[key]), 0, "{key}");
            let mut missing = false;
            assert_eq!(required(&record, &[key], &mut missing), 0);
            assert!(
                missing,
                "{key}: a required count that is not one flags the row"
            );
        }
        // A key that is there and unreadable does not stop a later spelling being read.
        assert_eq!(count(&record, &["text", "int"]), Some(1200));
    }
}
