//! Guards against documentation drifting from the code.
//!
//! Two releases (v0.4.0, v0.4.1) shipped with the README quick start still pinning v0.3.0,
//! because nothing checked it. These tests are deliberately cheap string checks: they run under
//! the existing `cargo test --all-targets` job on every OS and fail *before* a release, not after.

use std::collections::BTreeSet;

const README: &str = include_str!("../README.md");
/// Every `.rs` file under `src/`, walked at test time.
///
/// This was a hand-maintained list of five `include_str!`s with a comment asking the next person
/// to remember. A new collector reading `env::var("YOURS_HOME")` -- exactly what `codex.rs` does
/// for `CODEX_HOME` -- was invisible to the check that exists to catch it, until someone thought
/// to add the file. Walking the tree cannot be forgotten.
fn env_sources() -> Vec<String> {
    fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
        let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}"));
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(
                    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}")),
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut out,
    );
    assert!(out.len() >= 20, "only {} source files walked", out.len());
    out
}
/// Read by the code but deliberately not rows in the README table: the Windows stand-ins for
/// `HOME` and the XDG directories are described in the sentence under the table, and the
/// agents' own API-key variables are documented where billing detection is.
const ENV_NOT_IN_TABLE: &[&str] = &[
    "HOME",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "LOCALAPPDATA",
    "APPDATA",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_GENAI_USE_VERTEXAI",
];

/// Every `VERSION=vX.Y.Z` and `ai-usage-tui-vX.Y.Z` literal in the README must name the crate
/// version being built. Deleting the examples cannot make this pass: at least one pin must exist.
#[test]
fn readme_pins_the_current_release() {
    let expected = env!("CARGO_PKG_VERSION");
    let mut pins = Vec::new();
    for (index, line) in README.lines().enumerate() {
        for marker in ["VERSION=v", "ai-usage-tui-v"] {
            let mut rest = line;
            while let Some(at) = rest.find(marker) {
                let after = &rest[at + marker.len()..];
                let version: String = after
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                let version = version.trim_end_matches('.').to_string();
                if !version.is_empty() {
                    pins.push((index + 1, line.trim().to_string(), version));
                }
                rest = after;
            }
        }
    }
    assert!(
        !pins.is_empty(),
        "README.md no longer contains any VERSION=v… or ai-usage-tui-v… example; \
         the quick start should pin the release"
    );
    let stale: Vec<String> = pins
        .iter()
        .filter(|(_, _, version)| version != expected)
        .map(|(line, text, version)| format!("README.md:{line} pins {version}: {text}"))
        .collect();
    assert!(
        stale.is_empty(),
        "README.md pins a release other than {expected}:\n{}",
        stale.join("\n")
    );
}

/// The README's CLI reference table and `parse_cli`'s match arms must list the same long flags.
///
/// A flag added to one and not the other is the drift this catches; short forms (`-h`, `-V`)
/// are ignored because the table folds them into the long-flag row.
#[test]
fn readme_cli_table_matches_the_parser() {
    let documented = readme_cli_flags();
    let parsed = parser_long_flags();

    let undocumented: Vec<_> = parsed.difference(&documented).collect();
    let phantom: Vec<_> = documented.difference(&parsed).collect();
    assert!(
        undocumented.is_empty() && phantom.is_empty(),
        "README CLI reference and src/cli.rs disagree.\n\
         parsed by cli.rs but missing from the README table: {undocumented:?}\n\
         in the README table but not parsed by cli.rs: {phantom:?}"
    );
    assert!(
        documented.len() >= 20,
        "only {} flags found in the README table; the section marker may have moved",
        documented.len()
    );
}

/// Backtick-quoted `--flag` tokens inside the "CLI reference" table, up to the environment
/// variables that follow it. Tolerant of column layout: only the token matters.
fn readme_cli_flags() -> BTreeSet<String> {
    let start = README
        .find("## CLI reference")
        .expect("README.md has a '## CLI reference' section");
    let section = &README[start..];
    let end = section
        .find("Environment variables")
        .expect("the CLI reference is followed by the environment-variable table");
    let mut flags = BTreeSet::new();
    for line in section[..end].lines().filter(|l| l.starts_with("| `")) {
        for cell in line.split('`').skip(1).step_by(2) {
            if let Some(flag) = cell.split_whitespace().next() {
                if flag.starts_with("--") {
                    flags.insert(flag.to_string());
                }
            }
        }
    }
    flags
}

/// Every long flag the parser accepts, asked of clap directly.
///
/// This used to scrape `"--flag" =>` match arms out of `src/cli.rs` with a hand-rolled scanner,
/// because there was a hand-rolled parser to scrape. Querying the `Command` is not just tidier —
/// it cannot be fooled by a flag defined in a way the scanner did not anticipate, which a text
/// scan silently reports as "not a flag".
///
/// The companion guard that compared `--help` against the parser is gone, deliberately: clap
/// generates the help from these same definitions, so the two can no longer disagree. That
/// invariant is structural now rather than tested.
fn parser_long_flags() -> BTreeSet<String> {
    // `build()` first: clap adds `--help` and `--version` lazily, so `get_arguments()` on an
    // unbuilt command omits the two flags the README does document.
    let mut command = ai_usage_tui::cli::command();
    command.build();
    command
        .get_arguments()
        .filter_map(|arg| arg.get_long())
        // `--help` and `--version` are included: clap defines them like any other argument and
        // the README documents both. The hand-rolled scanner missed them because they were
        // spelled `"-h" | "--help"` and it only matched the first literal in an arm.
        .map(|long| format!("--{long}"))
        .collect()
}

/// The README's panel table and `ui::keys::BINDINGS` must offer the same panel keys.
///
/// The bindings lived in four places -- the event loop, the `?` overlay, `--help` and this table
/// -- with nothing keeping them in step. The first three read one table now; this is what keeps
/// the fourth honest.
#[test]
fn readme_panel_table_matches_the_key_bindings() {
    let documented = readme_panel_keys();
    let bound: BTreeSet<char> = ai_usage_tui::ui::keys::panel_keys()
        .map(|(key, _)| key)
        .collect();

    let undocumented: Vec<_> = bound.difference(&documented).collect();
    let phantom: Vec<_> = documented.difference(&bound).collect();
    assert!(
        undocumented.is_empty() && phantom.is_empty(),
        "README panel table and src/ui/keys.rs disagree.\n\
         bound in keys.rs but missing from the README table: {undocumented:?}\n\
         in the README table but not bound: {phantom:?}"
    );
    assert!(
        documented.len() >= 5,
        "only {} panel keys found in the README; the table may have moved",
        documented.len()
    );
}

/// Single-character backtick cells in the README's panel table — `| Budgets | `b` | ... |`.
fn readme_panel_keys() -> BTreeSet<char> {
    let mut keys = BTreeSet::new();
    for line in README.lines().filter(|l| l.starts_with("| ")) {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        // A panel row is `| Name | `k` | description |`.
        if cells.len() >= 4 {
            let cell = cells[2];
            if let Some(inner) = cell.strip_prefix('`').and_then(|c| c.strip_suffix('`')) {
                let mut chars = inner.chars();
                if let (Some(c), None) = (chars.next(), chars.next()) {
                    if c.is_ascii_alphabetic() {
                        keys.insert(c);
                    }
                }
            }
        }
    }
    keys
}

/// The README's environment-variable table must name every variable the code reads, apart from
/// the documented exceptions, and nothing the code does not read.
#[test]
fn readme_env_table_matches_the_code() {
    let start = README
        .find("Environment variables")
        .expect("README.md has an environment-variable table");
    let section = &README[start..];
    let end = section.find("\n## ").unwrap_or(section.len());
    let documented: BTreeSet<String> = section[..end]
        .lines()
        .filter(|line| line.starts_with("| `"))
        .filter_map(|line| line.split('`').nth(1))
        .filter(|name| name.chars().all(|c| c.is_ascii_uppercase() || c == '_'))
        .map(str::to_string)
        .collect();

    // Every shape in which this codebase names an environment variable. `utils.rs` reads through
    // an injected `Env` lookup (so its tests need not mutate the process's environment), which is
    // why `non_empty(env, "…")` is here alongside the direct `std::env` calls. This guard caught
    // that refactor: the variables were still read, and the old two-prefix scanner reported every
    // one of them as documented-but-never-read.
    const CALL_SHAPES: &[&str] = &["var_os(\"", "var(\"", "non_empty(env, \""];
    let mut read: BTreeSet<String> = BTreeSet::new();
    for source in &env_sources() {
        for shape in CALL_SHAPES {
            let mut rest = source.as_str();
            while let Some(at) = rest.find(shape) {
                let after = &rest[at + shape.len()..];
                let name: String = after
                    .chars()
                    .take_while(|c| c.is_ascii_uppercase() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    read.insert(name);
                }
                rest = after;
            }
        }
    }

    // The detector names its variables in a list rather than reading them one by one.
    for name in ENV_NOT_IN_TABLE {
        read.remove(*name);
    }

    let undocumented: Vec<_> = read.difference(&documented).collect();
    let phantom: Vec<_> = documented.difference(&read).collect();
    assert!(
        undocumented.is_empty() && phantom.is_empty(),
        "README environment table and the code disagree.\n\
         read by the code but missing from the table: {undocumented:?}\n\
         in the table but never read: {phantom:?}"
    );
    assert!(
        documented.len() >= 8,
        "only {} variables found; marker moved?",
        documented.len()
    );
}
