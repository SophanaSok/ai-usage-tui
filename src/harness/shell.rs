//! What a shell command line says about a test run, and whether its exit status can be trusted
//! to say it.
//!
//! A hook that fires after a Bash call sees the command line and the exit status of the whole
//! line — not of the test runner inside it. `cargo test 2>&1 | tail -20` exits with `tail`'s
//! status, so a red run reads as green; `cargo build && cargo test` fails when the build does,
//! before a test ran. Recording either as a test result would be inventing one. So a command
//! line is only an observation when its status *is* the runner's, and this module is the
//! whole of that judgement, in one place, so the rule can be read and tested rather than
//! re-derived in each harness.

/// Whether the command line's exit status speaks for the test runner it contains.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observable {
    /// A zero exit means the runner passed.
    pub on_success: bool,
    /// A non-zero exit means the runner failed.
    pub on_failure: bool,
}

/// Test runners this tool recognises, as the leading tokens of a simple command. A trailing `*`
/// matches any token with that prefix, so `npm run test:unit` is a test run and `npm run build`
/// is not. Deliberately a short, reviewable list rather than a heuristic on the word "test":
/// `grep test`, `echo "cargo test"` and `cat test.log` are not test runs.
const RUNNERS: &[&[&str]] = &[
    &["cargo", "test"],
    &["cargo", "nextest", "run"],
    &["pytest"],
    &["py.test"],
    &["python", "-m", "pytest"],
    &["python3", "-m", "pytest"],
    &["python", "-m", "unittest"],
    &["python3", "-m", "unittest"],
    &["tox"],
    &["npm", "test"],
    &["npm", "t"],
    &["npm", "run", "test*"],
    &["pnpm", "test"],
    &["pnpm", "t"],
    &["pnpm", "run", "test*"],
    &["yarn", "test"],
    &["yarn", "run", "test*"],
    &["bun", "test"],
    &["bun", "run", "test*"],
    &["deno", "test"],
    &["vitest"],
    &["jest"],
    &["mocha"],
    &["go", "test"],
    &["gotestsum"],
    &["just", "test*"],
    &["make", "test*"],
    &["make", "check"],
    &["ctest"],
    &["meson", "test"],
    &["mix", "test"],
    &["rspec"],
    &["rake", "test"],
    &["rails", "test"],
    &["dotnet", "test"],
    &["swift", "test"],
    &["gradle", "test"],
    &["./gradlew", "test"],
    &["gradlew", "test"],
    &["mvn", "test"],
    &["./mvnw", "test"],
    &["sbt", "test"],
    &["phpunit"],
    &["vendor/bin/phpunit"],
    &["php", "artisan", "test"],
    &["composer", "test"],
    &["zig", "build", "test"],
    &["dart", "test"],
    &["flutter", "test"],
];

/// Wrappers that run whatever follows them, so `npx vitest` and `timeout 60 cargo test` are
/// recognised by what they wrap. `*` consumes one token, whatever it is.
const WRAPPERS: &[&[&str]] = &[
    &["env"],
    &["time"],
    &["nice"],
    &["timeout", "*"],
    &["npx"],
    &["pnpm", "exec"],
    &["pnpm", "dlx"],
    &["bunx"],
    &["uv", "run"],
    &["poetry", "run"],
    &["pipenv", "run"],
    &["hatch", "run"],
    &["pdm", "run"],
    &["bundle", "exec"],
];

/// The operator before a simple command, which decides whether it ran at all and whether the
/// line's status is its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Start,
    /// `;` or a newline: the previous status is discarded, this command always runs.
    Seq,
    And,
    Or,
    Pipe,
    /// A single `&`: the line moves on with status 0 before the command has finished.
    Background,
}

/// What the exit status of `command` would say about the test runner in it, or `None` when it
/// contains no recognised runner or its status would not be the runner's.
///
/// Command substitution and heredocs make the line unreadable by this splitter — a runner inside
/// `$(…)` never sets the status, and a runner on a heredoc line is text being written to a file
/// — so a line containing either is not an observation at all.
pub fn test_runner(command: &str) -> Option<Observable> {
    if command.contains("$(") || command.contains('`') || command.contains("<<") {
        return None;
    }
    let segments = split(command);
    let mut observable: Option<Observable> = None;
    for (index, (_, text)) in segments.iter().enumerate() {
        if !is_test_runner(text) {
            continue;
        }
        // A zero status always speaks for the runner once the shapes that discard its status are
        // excluded below: nothing that ran after it in an `&&` chain can turn its failure into a
        // success. The question is only ever whether a non-zero status is the runner's.
        let on_success = true;
        let mut on_failure = true;
        // Backwards to the start of the runner's `&&`/`||` chain — `;` and a newline begin a
        // new one. A `||` directly before the runner means it ran only if the command before
        // failed, so a zero status may be that command's alone and nothing can be said. Any
        // `&&` or `||` earlier in the chain means a non-zero status may be an earlier command's
        // failure, before a test ran.
        for j in (0..=index).rev() {
            match segments[j].0 {
                Op::Start | Op::Seq => break,
                Op::Or if j == index => return None,
                Op::And | Op::Or => on_failure = false,
                Op::Pipe | Op::Background => {}
            }
        }
        // Forwards to the end of the line. Every `&&` after the runner keeps a zero status
        // honest — the runner passed, whatever else did — and makes a non-zero one ambiguous.
        // Anything else replaces the runner's status with a later command's: `| tail`, `; echo`,
        // `|| true`, or `&`, which moves on before the runner has finished.
        for (op, _) in &segments[index + 1..] {
            match op {
                Op::And => on_failure = false,
                Op::Seq | Op::Or | Op::Pipe | Op::Background | Op::Start => return None,
            }
        }
        observable = Some(match observable {
            Some(previous) => Observable {
                on_success: previous.on_success && on_success,
                on_failure: previous.on_failure && on_failure,
            },
            None => Observable {
                on_success,
                on_failure,
            },
        });
    }
    observable.filter(|o| o.on_success || o.on_failure)
}

/// The line as simple commands, each with the operator that precedes it. Quotes are not
/// tracked: an operator inside a string mis-splits the line, and the fragments then have no
/// runner at their head, which is the harmless direction.
fn split(command: &str) -> Vec<(Op, String)> {
    let chars: Vec<char> = command.chars().collect();
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut op = Op::Start;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let prev = i.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(i + 1).copied();
        let found = match c {
            '&' if next == Some('&') => {
                i += 1;
                Some(Op::And)
            }
            '|' if next == Some('|') => {
                i += 1;
                Some(Op::Or)
            }
            '|' => Some(Op::Pipe),
            ';' | '\n' => Some(Op::Seq),
            // `2>&1`, `&>` and `|&` are redirections, not a background job.
            '&' if !matches!(prev, Some('>') | Some('<') | Some('|')) && next != Some('>') => {
                Some(Op::Background)
            }
            _ => None,
        };
        match found {
            Some(next_op) => {
                segments.push((op, std::mem::take(&mut current)));
                op = next_op;
            }
            None => current.push(c),
        }
        i += 1;
    }
    segments.push((op, current));
    segments
}

/// Whether one simple command is a recognised test runner, looking through leading variable
/// assignments and wrappers.
fn is_test_runner(segment: &str) -> bool {
    let cleaned = segment
        .trim()
        .trim_start_matches(['(', '{'])
        .trim_end_matches([')', '}']);
    let mut tokens: Vec<&str> = cleaned.split_whitespace().collect();
    loop {
        while tokens.first().is_some_and(|t| is_assignment(t)) {
            tokens.remove(0);
        }
        if tokens.is_empty() {
            return false;
        }
        if RUNNERS.iter().any(|runner| matches_prefix(&tokens, runner)) {
            return true;
        }
        let Some(wrapper) = WRAPPERS
            .iter()
            .find(|wrapper| matches_prefix(&tokens, wrapper))
        else {
            return false;
        };
        tokens.drain(..wrapper.len());
    }
}

fn matches_prefix(tokens: &[&str], pattern: &[&str]) -> bool {
    pattern.len() <= tokens.len()
        && pattern
            .iter()
            .zip(tokens)
            .all(|(p, t)| match p.strip_suffix('*') {
                Some(prefix) => t.starts_with(prefix),
                None => p == t,
            })
}

/// `NAME=value`, as a shell would read it at the head of a command.
fn is_assignment(token: &str) -> bool {
    token.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: Option<Observable> = Some(Observable {
        on_success: true,
        on_failure: true,
    });
    const PASS_ONLY: Option<Observable> = Some(Observable {
        on_success: true,
        on_failure: false,
    });

    #[test]
    fn a_plain_runner_is_observed_both_ways() {
        for command in [
            "cargo test",
            "cargo test --all-targets --locked",
            "pytest tests/",
            "python3 -m pytest -x",
            "npm test",
            "npm run test:unit",
            "go test ./...",
            "just test",
            "make check",
            "RUST_BACKTRACE=1 cargo test",
            "env FOO=1 cargo test",
            "timeout 120 cargo test",
            "npx vitest run",
            "uv run pytest",
            "bundle exec rspec",
            "cargo test 2>&1",
            "cargo test >out.log 2>&1",
            "cargo build; cargo test",
            "cargo build\ncargo test",
        ] {
            assert_eq!(test_runner(command), BOTH, "{command:?}");
        }
    }

    #[test]
    fn a_line_that_only_mentions_a_runner_is_not_a_test_run() {
        for command in [
            "grep -rn \"cargo test\" docs/",
            "echo cargo test",
            "cat test.log",
            "ls tests/",
            "git log --grep test",
            "npm run build",
            "cargo build",
            "cargo clippy --all-targets -- -D warnings",
            "make",
            "bash -c \"cargo test\"",
            "! cargo test",
            "",
        ] {
            assert_eq!(test_runner(command), None, "{command:?}");
        }
    }

    /// The exit status the hook sees is the line's, not the runner's. Where the two can differ,
    /// the observation is withheld in that direction rather than recorded as a guess.
    #[test]
    fn a_status_the_shell_discards_is_not_an_observation() {
        for command in [
            "cargo test 2>&1 | tail -20",
            "cargo test | grep FAILED",
            "cargo test; echo done",
            "cargo test || true",
            "cargo test || echo failed",
            "cargo test &",
            "cargo test && echo ok || echo failed", // always exits 0
            "cargo test | tail; cargo test",        // one run's status is gone
            "cargo test && cargo clippy | tail",
            "echo $(cargo test)",
            "out=`cargo test`",
            "cat > run.sh <<'EOF'\ncargo test && echo ok\nEOF",
        ] {
            assert_eq!(test_runner(command), None, "{command:?}");
        }
    }

    /// `A && cargo test`: a zero status means the runner passed, but a non-zero one may be A's.
    /// `cargo test && B`: a zero status means the runner passed, a non-zero one may be B's.
    #[test]
    fn a_runner_chained_with_and_is_observed_only_on_success() {
        for command in [
            "cargo build && cargo test",
            "cargo test && cargo clippy",
            "cargo fmt --check && cargo test && cargo doc",
            "cargo test && cargo test --doc",
            // `cd` into a directory that is not there fails before a test runs.
            "cd crate && cargo test",
            "(cd sub && cargo test)",
        ] {
            assert_eq!(test_runner(command), PASS_ONLY, "{command:?}");
        }
    }

    /// `A || cargo test` runs the tests only when A failed, so a zero status may be A's alone.
    #[test]
    fn a_runner_after_or_is_never_observed() {
        assert_eq!(test_runner("cargo check || cargo test"), None);
    }

    #[test]
    fn redirections_are_not_background_jobs() {
        assert_eq!(test_runner("cargo test 2>&1"), BOTH);
        assert_eq!(test_runner("cargo test &>log"), BOTH);
        assert_eq!(test_runner("cargo test |& tee log"), None);
    }

    #[test]
    fn assignments_are_recognised_as_a_shell_would() {
        assert!(is_assignment("FOO=bar"));
        assert!(is_assignment("_x=1"));
        assert!(is_assignment("EMPTY="));
        assert!(!is_assignment("1x=2"));
        assert!(!is_assignment("--flag=value"));
        assert!(!is_assignment("a.b=c"));
        assert!(!is_assignment("=x"));
    }
}
