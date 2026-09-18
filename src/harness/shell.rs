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

/// What a command line says about a test run.
///
/// The middle case is the one that used to be thrown away. Measured on the author's machine
/// over eighteen days, 845 command lines ran a test runner and four of them had a status that
/// was the runner's own: every other one was trimmed through `grep`, `tail` or `head`, and the
/// hook said nothing about any of them. A harness needs to know a runner ran even when the
/// status cannot speak for it — to look for the runner's own word elsewhere, and to count what
/// it could not record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// No recognised runner heads a command on the line: an ordinary Bash call.
    NoRunner,
    /// A runner ran, and the line's exit status is not its own.
    Withheld(Reason),
    /// A runner ran, and the line's exit status speaks for it in the directions given.
    Observable(Observable),
}

/// Why a line's exit status does not speak for the runner on it. The labels are what the
/// journal's tally and `--summary-json` carry, so they are part of the stable surface: add one,
/// never rename one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Reason {
    /// `cargo test | tail`: the status is the last command's in the pipeline.
    Pipe,
    /// `cargo test; echo done`: the status is the later command's.
    Sequence,
    /// `cargo test || true`: a failure is replaced by what ran because of it.
    OrAfter,
    /// `cargo check || cargo test`: the tests ran only because something else failed.
    AfterOr,
    /// `cargo test &`: the line moved on before the runner finished.
    Background,
    /// `$(…)`, backticks or a heredoc: the line is not readable by this splitter.
    Substitution,
    /// `cargo build && cargo test` that failed: the failure may be an earlier command's. Decided
    /// by the harness, which knows which way the line went; [`verdict`] never returns it.
    AndChain,
}

impl Reason {
    pub const ALL: [Reason; 7] = [
        Reason::Pipe,
        Reason::Sequence,
        Reason::OrAfter,
        Reason::AfterOr,
        Reason::Background,
        Reason::Substitution,
        Reason::AndChain,
    ];

    /// The label stored and exported.
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Pipe => "pipe",
            Reason::Sequence => "sequence",
            Reason::OrAfter => "or_after",
            Reason::AfterOr => "after_or",
            Reason::Background => "background",
            Reason::Substitution => "substitution",
            Reason::AndChain => "and_chain",
        }
    }

    pub fn parse(label: &str) -> Option<Reason> {
        Reason::ALL
            .into_iter()
            .find(|reason| reason.as_str() == label)
    }

    /// The reason in words, to follow "a test run" or a count of them.
    pub fn explain(self) -> &'static str {
        match self {
            Reason::Pipe => "piped into another command, whose status the line took",
            Reason::Sequence => "followed by `;` or a newline, so the status was a later command's",
            Reason::OrAfter => "followed by `||`, which replaces a failure",
            Reason::AfterOr => "run after `||`, so only because something else failed",
            Reason::Background => "put in the background with `&`",
            Reason::Substitution => "on a line with `$(…)`, backticks or a heredoc",
            Reason::AndChain => {
                "in an `&&` chain that failed, where the failure may be another command's"
            }
        }
    }
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
    // As `make check`: the recipe that runs the tests, by the name this project's own
    // `justfile` gives it.
    &["just", "check"],
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

/// What the exit status of `command` would say about the test runner in it.
///
/// Command substitution and heredocs make the line unreadable by this splitter — a runner inside
/// `$(…)` never sets the status, and a runner on a heredoc line is text being written to a file
/// — so a line containing either is never [`Verdict::Observable`]. Whether a runner ran on it is
/// still worth knowing, and is asked of the line with its heredoc bodies taken out: counted with
/// them in, seven in eight of the lines this rule withheld were scripts that only *mentioned* a
/// runner.
pub fn verdict(command: &str) -> Verdict {
    if command.contains("$(") || command.contains('`') || command.contains("<<") {
        let ran = split(&without_heredoc_bodies(command))
            .iter()
            .any(|(_, text)| is_test_runner(text));
        return if ran {
            Verdict::Withheld(Reason::Substitution)
        } else {
            Verdict::NoRunner
        };
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
                Op::Or if j == index => return Verdict::Withheld(Reason::AfterOr),
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
                Op::Pipe => return Verdict::Withheld(Reason::Pipe),
                Op::Or => return Verdict::Withheld(Reason::OrAfter),
                Op::Background => return Verdict::Withheld(Reason::Background),
                Op::Seq | Op::Start => return Verdict::Withheld(Reason::Sequence),
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
    // A zero status always speaks (`on_success` is never cleared), so a runner that got this
    // far is observable in at least one direction.
    observable.map_or(Verdict::NoRunner, Verdict::Observable)
}

/// `command` with the lines of every heredoc body removed, the opening line kept. A body is text
/// on its way to a file or a program's stdin, not commands the shell ran.
fn without_heredoc_bodies(command: &str) -> String {
    let mut kept = Vec::new();
    let mut terminator: Option<String> = None;
    for line in command.lines() {
        match &terminator {
            Some(word) => {
                if line.trim() == word {
                    terminator = None;
                }
            }
            None => {
                kept.push(line);
                terminator = heredoc_terminator(line);
            }
        }
    }
    kept.join("\n")
}

/// The word that ends the heredoc `line` opens, if it opens one: `<<EOF`, `<<-EOF`, `<<'EOF'`,
/// `<< "EOF"`. A here-string (`<<<`) opens nothing.
fn heredoc_terminator(line: &str) -> Option<String> {
    let after = &line[line.find("<<")? + 2..];
    if after.starts_with('<') {
        return None;
    }
    let word: String = after
        .trim_start_matches('-')
        .trim_start()
        .trim_start_matches(['\'', '"'])
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (!word.is_empty()).then_some(word)
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

/// Whose summary lines a runner's output is written in. Only runners whose real output is kept
/// under `tests/fixtures/hook/` have one; see `harness::summary`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Libtest,
    Pytest,
    Go,
    Deno,
}

impl Family {
    const ALL: [Family; 4] = [Family::Libtest, Family::Pytest, Family::Go, Family::Deno];

    /// The families a recognised runner's output may be in: its own for a runner that prints
    /// its own summary, every one for a recipe or script that may run anything (`make test`,
    /// `npm test`), and none for a runner no output was captured from.
    fn of(runner: &[&str]) -> &'static [Family] {
        match runner {
            ["cargo", "test"] => &[Family::Libtest],
            ["pytest"] | ["py.test"] | [_, "-m", "pytest"] => &[Family::Pytest],
            ["go", "test"] => &[Family::Go],
            ["deno", "test"] => &[Family::Deno],
            ["make", ..] | ["just", ..] | ["npm", ..] | ["pnpm", ..] | ["yarn", ..] => &Family::ALL,
            ["bun", "run", ..] | ["tox"] | ["composer", ..] | ["rake", ..] => &Family::ALL,
            _ => &[],
        }
    }
}

/// The summary families to read the output of `command` in: those of every runner that heads a
/// command on it, heredoc bodies aside. Empty when no runner does, which is what keeps a line
/// that only *prints* a summary — `grep -rn "test result: ok" src/` — from being read as a run.
pub fn families(command: &str) -> Vec<Family> {
    let mut found = Vec::new();
    for (_, text) in split(&without_heredoc_bodies(command)) {
        for family in runner_of(&text).map_or(&[][..], Family::of) {
            if !found.contains(family) {
                found.push(*family);
            }
        }
    }
    found
}

/// Whether one simple command is a recognised test runner, looking through leading variable
/// assignments and wrappers.
fn is_test_runner(segment: &str) -> bool {
    runner_of(segment).is_some()
}

/// The entry of [`RUNNERS`] that one simple command is, if it is one.
fn runner_of(segment: &str) -> Option<&'static [&'static str]> {
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
            return None;
        }
        if let Some(runner) = RUNNERS
            .iter()
            .find(|runner| matches_prefix(&tokens, runner))
        {
            return Some(runner);
        }
        let wrapper = WRAPPERS
            .iter()
            .find(|wrapper| matches_prefix(&tokens, wrapper))?;
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

    const BOTH: Verdict = Verdict::Observable(Observable {
        on_success: true,
        on_failure: true,
    });
    const PASS_ONLY: Verdict = Verdict::Observable(Observable {
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
            "just check",
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
            assert_eq!(verdict(command), BOTH, "{command:?}");
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
            // A runner that is only text: inside a substitution whose line is headed by
            // something else, and in the body of a heredoc.
            "echo $(cargo test)",
            "out=`cargo test`",
            "cat > run.sh <<'EOF'\ncargo test && echo ok\nEOF",
            "python3 - <<PY\nimport os\nos.system('x')\ncargo test\nPY\necho done",
        ] {
            assert_eq!(verdict(command), Verdict::NoRunner, "{command:?}");
        }
    }

    /// The exit status the hook sees is the line's, not the runner's. Where the two can differ,
    /// the observation is withheld in that direction rather than recorded as a guess.
    #[test]
    fn a_status_the_shell_discards_is_not_an_observation() {
        for (command, reason) in [
            ("cargo test 2>&1 | tail -20", Reason::Pipe),
            ("cargo test | grep FAILED", Reason::Pipe),
            (
                "cargo test --all-targets --locked 2>&1 | grep -E \"^test result\" | head -40",
                Reason::Pipe,
            ),
            ("cargo test; echo done", Reason::Sequence),
            ("cargo test || true", Reason::OrAfter),
            ("cargo test || echo failed", Reason::OrAfter),
            ("cargo test &", Reason::Background),
            ("cargo test && echo ok || echo failed", Reason::OrAfter), // always exits 0
            ("cargo test | tail; cargo test", Reason::Pipe),           // one run's status is gone
            ("cargo test && cargo clippy | tail", Reason::Pipe),
            // A runner really ran here; it is the line that cannot be read.
            (
                "cargo test --lib 2>&1 | tail -3; echo $(date)",
                Reason::Substitution,
            ),
            (
                "python3 - <<'EOF'\nprint(1)\nEOF\ncargo test",
                Reason::Substitution,
            ),
        ] {
            assert_eq!(verdict(command), Verdict::Withheld(reason), "{command:?}");
        }
    }

    #[test]
    fn a_heredoc_body_is_taken_out_and_its_opening_line_kept() {
        assert_eq!(
            without_heredoc_bodies("cat > f <<'EOF'\ncargo test\nEOF\ncargo test"),
            "cat > f <<'EOF'\ncargo test"
        );
        assert_eq!(
            without_heredoc_bodies("a <<-END\n\tbody\n\tEND\nb"),
            "a <<-END\nb"
        );
        // A here-string opens nothing, so nothing after it is swallowed.
        assert_eq!(
            without_heredoc_bodies("wc <<<x\ncargo test"),
            "wc <<<x\ncargo test"
        );
        assert_eq!(
            heredoc_terminator("cat << \"EOF\""),
            Some("EOF".to_string())
        );
        assert_eq!(heredoc_terminator("x <<< y"), None);
        assert_eq!(heredoc_terminator("no heredoc"), None);
    }

    #[test]
    fn a_runners_family_is_its_own_a_recipes_is_any_and_an_uncaptured_runners_is_none() {
        assert_eq!(
            families("cargo test --locked 2>&1 | tail -3"),
            [Family::Libtest]
        );
        assert_eq!(families("uv run pytest -x | tail -3"), [Family::Pytest]);
        assert_eq!(families("python3 -m pytest | tail"), [Family::Pytest]);
        assert_eq!(families("cd api && go test ./... | tail"), [Family::Go]);
        assert_eq!(families("deno test | tail"), [Family::Deno]);
        assert_eq!(
            families("cargo test | tail; pytest | tail"),
            [Family::Libtest, Family::Pytest]
        );
        assert_eq!(families("make test | tail"), Family::ALL);
        assert_eq!(families("pnpm test >/dev/null"), Family::ALL);
        assert_eq!(families("npx vitest run | tail"), []);
        assert_eq!(families("cargo nextest run | tail"), []);
        assert_eq!(families("grep -rn \"test result: ok\" src/"), []);
        assert_eq!(families("cat > x <<'EOF'\ncargo test\nEOF"), []);
    }

    /// Which runners have a summary family, spelled out: a runner added to `RUNNERS` shows up
    /// here as having none until someone captures its output, and a pattern in `Family::of`
    /// that stops matching its entry shows up as a runner that lost one.
    #[test]
    fn the_runners_with_a_summary_family_are_these() {
        let with_family: Vec<String> = RUNNERS
            .iter()
            .filter(|runner| !Family::of(runner).is_empty())
            .map(|runner| runner.join(" "))
            .collect();
        assert_eq!(
            with_family,
            [
                "cargo test",
                "pytest",
                "py.test",
                "python -m pytest",
                "python3 -m pytest",
                "tox",
                "npm test",
                "npm t",
                "npm run test*",
                "pnpm test",
                "pnpm t",
                "pnpm run test*",
                "yarn test",
                "yarn run test*",
                "bun run test*",
                "deno test",
                "go test",
                "just test*",
                "just check",
                "make test*",
                "make check",
                "rake test",
                "composer test",
            ]
        );
    }

    #[test]
    fn every_reason_has_a_label_that_reads_back() {
        for reason in Reason::ALL {
            assert_eq!(Reason::parse(reason.as_str()), Some(reason));
        }
        assert_eq!(Reason::parse("nonsense"), None);
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
            assert_eq!(verdict(command), PASS_ONLY, "{command:?}");
        }
    }

    /// `A || cargo test` runs the tests only when A failed, so a zero status may be A's alone.
    #[test]
    fn a_runner_after_or_is_never_observed() {
        assert_eq!(
            verdict("cargo check || cargo test"),
            Verdict::Withheld(Reason::AfterOr)
        );
    }

    #[test]
    fn redirections_are_not_background_jobs() {
        assert_eq!(verdict("cargo test 2>&1"), BOTH);
        assert_eq!(verdict("cargo test &>log"), BOTH);
        assert_eq!(
            verdict("cargo test |& tee log"),
            Verdict::Withheld(Reason::Pipe)
        );
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
