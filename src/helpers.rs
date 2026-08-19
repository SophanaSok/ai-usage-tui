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

/// Whether an error is a downstream reader closing the pipe — a normal way for a command in a
/// pipeline to end, not a failure to report.
pub fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe)
}

#[cfg(test)]
mod tests {
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
