//! A test runner's own word for how a run went, read from the output a hook payload carries.
//!
//! `shell` decides whether a command line's exit status speaks for the runner on it, and for
//! most real lines it does not: `cargo test 2>&1 | grep -E "^test result" | head -40` exits with
//! `head`'s status. Captured from Claude Code 2.1.x, that line with a **failing** test fires
//! `PostToolUse` — the success hook — so the status was never going to be the way in. The
//! runner's summary line is still in the payload's `stdout`, and it is the runner saying what
//! happened rather than the shell.
//!
//! Every marker here is a line a runner really printed, kept under `tests/fixtures/hook/`. A
//! runner with no fixture has no marker: jest, vitest, nextest and the rest are recognised by
//! `shell` and say nothing here, which leaves their piped runs withheld and counted, as before.
//!
//! The two directions are not equally safe, and are not treated equally:
//!
//! - **A failure marker is always believed.** A filter can hide one; it cannot make one.
//! - **A pass needs the end of the output to be there**, because the end is where every one of
//!   these runners puts a failure. `tail` keeps the end. `head` cuts it, so a pass is withheld
//!   when a `head` on the line filled its limit, or its limit cannot be read. A line that selects
//!   passing summaries by name (`grep "result: ok"`) cannot show a failure at all, and never
//!   yields a pass.

use crate::harness::shell::{self, Family};

/// What the runner's output says: `Some(true)` passed, `Some(false)` failed, `None` when the
/// output does not say — no summary line survived, the runner is one this module has no
/// captured output for, or a pass could not be told from a cut-off failure.
pub fn from_output(command: &str, output: &str) -> Option<bool> {
    let families = shell::families(command);
    if families.is_empty() {
        return None;
    }
    let mut passed = false;
    for line in output.lines() {
        let line = strip_ansi(line);
        let line = line.trim_end();
        for family in &families {
            match family.read(line) {
                Some(false) => return Some(false),
                Some(true) => passed = true,
                None => {}
            }
        }
    }
    (passed && !selects_passes_only(command) && !end_may_be_cut(command, output)).then_some(true)
}

impl Family {
    /// What one line of output says, if it is one of this runner's summary lines.
    fn read(self, line: &str) -> Option<bool> {
        match self {
            Family::Libtest => libtest(line),
            Family::Pytest => pytest(line),
            Family::Go => go(line),
            Family::Deno => deno(line),
        }
    }
}

/// `test result: ok. 2 passed; 0 failed; …` and `test result: FAILED. 1 passed; 1 failed; …`,
/// one per test binary; `test tests::name ... FAILED`; and cargo's own last line, which is all a
/// short `tail` leaves: `error: test failed, to rerun pass `--lib`` or, under `--no-fail-fast`,
/// `error: 1 target failed:` — printed *after* a later binary's `test result: ok.`.
fn libtest(line: &str) -> Option<bool> {
    if line.starts_with("test result: FAILED.")
        || line.starts_with("error: test failed")
        || (line.starts_with("test ") && line.ends_with(" ... FAILED"))
        || targets_failed(line)
    {
        return Some(false);
    }
    line.starts_with("test result: ok.").then_some(true)
}

/// `error: 1 target failed:` / `error: 3 targets failed:`.
fn targets_failed(line: &str) -> bool {
    line.strip_prefix("error: ")
        .and_then(|rest| rest.split_once(' '))
        .is_some_and(|(count, rest)| {
            !count.is_empty()
                && count.chars().all(|c| c.is_ascii_digit())
                && (rest.starts_with("target failed") || rest.starts_with("targets failed"))
        })
}

/// `===== 1 failed, 1 passed in 0.02s =====`, or the same without the rule under `-q`; and the
/// short summary's `FAILED test_bad.py::test_bad - …`.
fn pytest(line: &str) -> Option<bool> {
    if line.starts_with("FAILED ") || line.starts_with("ERROR ") {
        return Some(false);
    }
    let body = line.trim_matches(|c: char| c == '=' || c == ' ');
    let (counts, time) = body.rsplit_once(" in ")?;
    let seconds = time.split_whitespace().next()?.strip_suffix('s')?;
    if seconds.is_empty() || !seconds.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let mut passed = false;
    let mut failed = false;
    for part in counts.split(", ") {
        let (count, what) = part.split_once(' ')?;
        if count.is_empty() || !count.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        match what {
            "failed" | "error" | "errors" => failed = true,
            "passed" => passed = true,
            _ => {}
        }
    }
    if failed {
        Some(false)
    } else {
        passed.then_some(true)
    }
}

/// `ok  \tcap/okpkg\t0.001s`; `--- FAIL: TestBad (0.00s)`, `FAIL\tcap/badpkg\t0.001s` and the
/// bare `FAIL` that ends a failing run.
fn go(line: &str) -> Option<bool> {
    if line == "FAIL" || line.starts_with("FAIL\t") || line.starts_with("--- FAIL: ") {
        return Some(false);
    }
    (line.starts_with("ok  \t") || line.starts_with("ok\t")).then_some(true)
}

/// `ok | 1 passed | 0 failed (1ms)`, `FAILED | 1 passed | 1 failed (2ms)` and the
/// `error: Test failed` that ends a failing run.
fn deno(line: &str) -> Option<bool> {
    if line.starts_with("FAILED | ") || line == "error: Test failed" {
        return Some(false);
    }
    (line.starts_with("ok | ") && line.contains(" passed | ")).then_some(true)
}

/// A line that picks out passing summaries by name can only ever show passes.
fn selects_passes_only(command: &str) -> bool {
    command.contains("result: ok") || command.contains("ok |") || command.contains(" passed in")
}

/// Blank lines Claude Code may have trimmed off the end of what `head` let through. Captured:
/// `cargo test 2>&1 | head -3` arrives as two lines, because the third was blank
/// (`libtest_pass_head_cut.json`). These runners print at most two blank lines in a row.
const TRIMMED_BLANKS: usize = 2;

/// Whether a `head` on the line may have cut the end of the output off: one whose limit the
/// output reached — give or take the blank lines trimmed since — or whose limit cannot be read.
/// Every failure marker above is at or near the end of its runner's output, so an output that
/// stops early can look like a pass.
fn end_may_be_cut(command: &str, output: &str) -> bool {
    let lines = output.trim_end().lines().count() + TRIMMED_BLANKS;
    // Every operator ends a simple command, not only `|`: the machine this was measured on
    // writes `… | head -5; git status`, and `head;` must still be read as `head`.
    command.split(['|', ';', '&', '\n']).any(|stage| {
        let mut tokens = stage.split_whitespace();
        if tokens.next() != Some("head") {
            return false;
        }
        let limit = match (tokens.next(), tokens.next()) {
            // `head` alone prints ten lines.
            (None, _) => Some(10),
            (Some("-n"), Some(count)) | (Some("--lines"), Some(count)) => count.parse().ok(),
            (Some(flag), _) => flag
                .strip_prefix("--lines=")
                .or_else(|| flag.strip_prefix("-n"))
                .or_else(|| flag.strip_prefix('-'))
                .and_then(|count| count.parse::<usize>().ok()),
        };
        limit.is_none_or(|limit| lines >= limit)
    })
}

/// `line` without ANSI colour sequences: deno colours its summary even into a pipe.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.next() == Some('[') {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = format!("{}/tests/fixtures/hook/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"))
    }

    /// The text a payload carries: `tool_response.stdout` for `PostToolUse`, `error` for
    /// `PostToolUseFailure`.
    fn payload_output(name: &str) -> (String, String) {
        let payload: serde_json::Value = serde_json::from_str(&fixture(name)).unwrap();
        let command = payload["tool_input"]["command"]
            .as_str()
            .unwrap()
            .to_string();
        let output = payload["tool_response"]["stdout"]
            .as_str()
            .or_else(|| payload["error"].as_str())
            .unwrap()
            .to_string();
        (command, output)
    }

    #[test]
    fn captured_payloads_say_what_the_runner_said() {
        for (name, expected) in [
            ("libtest_pass_grep.json", Some(true)),
            ("libtest_pass_tail.json", Some(true)),
            ("libtest_pass_bare.json", Some(true)),
            // `head -3` stopped before any summary line.
            ("libtest_pass_head_cut.json", None),
            // `PostToolUse` fired for each of these three: the shell saw `grep` and `tail` succeed.
            ("libtest_fail_grep.json", Some(false)),
            ("libtest_fail_tail.json", Some(false)),
            ("libtest_fail_no_fail_fast_grep.json", Some(false)),
            ("libtest_fail_bare.json", Some(false)),
        ] {
            let (command, output) = payload_output(name);
            assert_eq!(
                from_output(&command, &output),
                expected,
                "{name}: {command}"
            );
        }
    }

    #[test]
    fn the_success_hook_fired_for_a_failing_piped_run() {
        let payload: serde_json::Value =
            serde_json::from_str(&fixture("libtest_fail_grep.json")).unwrap();
        assert_eq!(payload["hook_event_name"], "PostToolUse");
    }

    #[test]
    fn captured_runner_output_says_what_the_runner_said() {
        for (command, name, expected) in [
            (
                "pytest test_ok.py 2>&1 | tail -3",
                "pytest_pass.txt",
                Some(true),
            ),
            (
                "pytest test_bad.py 2>&1 | tail -4",
                "pytest_fail.txt",
                Some(false),
            ),
            (
                "pytest -q test_ok.py 2>&1 | tail -2",
                "pytest_quiet_pass.txt",
                Some(true),
            ),
            (
                "pytest -q test_bad.py 2>&1 | tail -3",
                "pytest_quiet_fail.txt",
                Some(false),
            ),
            ("go test ./okpkg 2>&1 | tail -3", "go_pass.txt", Some(true)),
            (
                "go test ./badpkg 2>&1 | tail -5",
                "go_fail.txt",
                Some(false),
            ),
            ("go test ./... 2>&1 | tail -6", "go_mixed.txt", Some(false)),
            (
                "deno test ok_test.ts 2>&1 | tail -3",
                "deno_pass.txt",
                Some(true),
            ),
            (
                "deno test ok_test.ts 2>&1 | tail -2",
                "deno_pass_colour.txt",
                Some(true),
            ),
            (
                "deno test bad_test.ts 2>&1 | tail -4",
                "deno_fail.txt",
                Some(false),
            ),
            // `--no-fail-fast | tail -6`: the last summary is `ok`, and the failure is cargo's
            // `error: 1 target failed:` two lines below it.
            (
                "cargo test --no-fail-fast 2>&1 | tail -6",
                "libtest_no_fail_fast_tail.txt",
                Some(false),
            ),
            (
                "cargo test 2>&1 | tail -2",
                "libtest_fail_tail_2.txt",
                Some(false),
            ),
        ] {
            assert_eq!(from_output(command, &fixture(name)), expected, "{name}");
        }
    }

    #[test]
    fn one_runners_summary_is_not_read_as_anothers() {
        // pytest's output under a cargo command line, and the reverse: no verdict either way.
        assert_eq!(
            from_output("cargo test | tail -3", &fixture("pytest_pass.txt")),
            None
        );
        let (_, libtest) = payload_output("libtest_pass_grep.json");
        assert_eq!(from_output("pytest | tail -3", &libtest), None);
        // A recipe may run anything, so it is read for every family.
        assert_eq!(
            from_output("make test | tail -3", &fixture("pytest_pass.txt")),
            Some(true)
        );
        assert_eq!(
            from_output("just check 2>&1 | tail -3", &libtest),
            Some(true)
        );
        // A runner with no captured output says nothing, whatever its output looks like.
        assert_eq!(from_output("npx vitest run | tail -3", &libtest), None);
        // And a line with no runner heading a command is not asked at all.
        assert_eq!(
            from_output("grep -rn 'test result: ok.' src/", &libtest),
            None
        );
    }

    #[test]
    fn a_pass_needs_the_end_of_the_output() {
        let two =
            "test result: ok. 2 passed; 0 failed; 0 ignored\ntest result: ok. 0 passed; 0 failed";
        // `head` with room to spare cut nothing.
        assert_eq!(
            from_output("cargo test | grep '^test result' | head -40", two),
            Some(true)
        );
        assert_eq!(
            from_output("cargo test | grep '^test result' | head -n 5", two),
            Some(true)
        );
        // A `head` that filled up may have stopped before the failure — and so may one that
        // looks two lines short of full, since trailing blank lines do not arrive.
        assert_eq!(
            from_output("cargo test | grep '^test result' | head -2", two),
            None
        );
        assert_eq!(
            from_output("cargo test | grep '^test result' | head -4", two),
            None
        );
        assert_eq!(
            from_output("cargo test | grep '^test result' | head -5", two),
            Some(true)
        );
        let (command, cut) = payload_output("libtest_pass_head_cut.json");
        assert_eq!(
            (command.as_str(), cut.lines().count()),
            ("cargo test 2>&1 | head -3", 2)
        );
        assert_eq!(
            from_output("cargo test | grep '^test result' | head -n 2", two),
            None
        );
        assert_eq!(
            from_output("cargo test | grep '^test result' | head -n2", two),
            None
        );
        assert_eq!(
            from_output("cargo test | grep '^test result' | head --lines=2", two),
            None
        );
        assert_eq!(
            from_output("cargo test | grep '^test result' | head -c 50", two),
            None
        );
        // The `head` is found whatever follows it on the line, and a bare one prints ten.
        assert_eq!(
            from_output(
                "cargo test | grep '^test result' | head -2; git status",
                two
            ),
            None
        );
        assert_eq!(
            from_output("cargo test | grep '^test result' | head -5 && echo ok", two),
            Some(true)
        );
        let nine = ["test result: ok. 1 passed; 0 failed"; 9].join("\n");
        assert_eq!(
            from_output("cargo test | grep '^test result' | head; echo done", &nine),
            None
        );
        // A failure is believed however the output was cut.
        let failed = "test result: FAILED. 1 passed; 1 failed";
        assert_eq!(
            from_output("cargo test | grep '^test result' | head -1", failed),
            Some(false)
        );
        // A filter that can only show passes proves nothing.
        assert_eq!(
            from_output("cargo test | grep 'test result: ok'", two),
            None
        );
    }

    #[test]
    fn lines_that_only_resemble_a_summary_are_not_one() {
        for line in [
            "  test result: ok.", // indented: quoted, not printed by the harness
            "// test result: FAILED.",
            "test result: okay",
            "the build finished in 3s", // pytest's shape needs counts
            "12 warnings in 0.2s",      // …and one of them to be `passed`
            "ok",
            "okay\tpkg",
            "FAILURE",
        ] {
            for family in [Family::Libtest, Family::Pytest, Family::Go, Family::Deno] {
                assert_eq!(family.read(line), None, "{family:?} {line:?}");
            }
        }
    }

    #[test]
    fn colour_is_taken_off_before_a_line_is_read() {
        assert_eq!(
            strip_ansi("\u{1b}[0m\u{1b}[32mok\u{1b}[0m | 1 passed"),
            "ok | 1 passed"
        );
        assert_eq!(strip_ansi("plain"), "plain");
    }
}
