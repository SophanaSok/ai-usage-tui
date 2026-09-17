use serde_json::Value;

pub fn string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).map(String::from))
}
pub fn number(value: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| {
            value.get(*key).and_then(Value::as_u64).or_else(|| {
                value
                    .get(*key)
                    .and_then(Value::as_f64)
                    .map(|value| value as u64)
            })
        })
        .unwrap_or(0)
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
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    let temporary = path.with_file_name(name);
    let written =
        std::fs::write(&temporary, contents).and_then(|()| std::fs::rename(&temporary, path));
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
