//! Guards against documentation drifting from the code.
//!
//! Two releases (v0.4.0, v0.4.1) shipped with the README quick start still pinning v0.3.0,
//! because nothing checked it. These tests are deliberately cheap string checks: they run under
//! the existing `cargo test --all-targets` job on every OS and fail *before* a release, not after.

use std::collections::BTreeSet;

const README: &str = include_str!("../README.md");
const CLI_SOURCE: &str = include_str!("../src/cli.rs");
/// Every file that reads the environment. A new `env::var` elsewhere must be listed here.
const ENV_SOURCES: &[&str] = &[
    include_str!("../src/utils.rs"),
    include_str!("../src/logging.rs"),
    include_str!("../src/collector/claude_code.rs"),
    include_str!("../src/collector/codex.rs"),
    include_str!("../src/collector/billing.rs"),
];
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

/// `--help` must offer the same long flags the parser accepts and the README documents.
///
/// `readme_cli_table_matches_the_parser` compared two of the three lists and left the help text
/// out, which is how a stray `(default: ~/.claude/projects)` line sat orphaned under
/// `--omarchy-record` -- inherited from a `--claude-dir` entry three flags above it -- through
/// several releases. Three lists, one comparison.
#[test]
fn help_text_lists_every_flag_the_parser_accepts() {
    let helped = help_long_flags();
    let parsed = parser_long_flags();

    let unhelped: Vec<_> = parsed.difference(&helped).collect();
    let phantom: Vec<_> = helped.difference(&parsed).collect();
    assert!(
        unhelped.is_empty() && phantom.is_empty(),
        "`--help` and src/cli.rs `parse_cli` disagree.\n\
         parsed by cli.rs but missing from OPTIONS in print_help: {unhelped:?}\n\
         listed in print_help but not parsed by cli.rs: {phantom:?}"
    );
    assert!(
        helped.len() >= 20,
        "only {} flags found in the help text; the OPTIONS marker may have moved",
        helped.len()
    );
}

/// Long flags declared in `print_help`'s `OPTIONS:` block.
///
/// A flag is only counted where it is *declared* -- the first token of its line -- so a flag
/// named inside another flag's description is not mistaken for an entry of its own, and the
/// wrapped continuation lines are skipped.
fn help_long_flags() -> BTreeSet<String> {
    let start = CLI_SOURCE
        .find("OPTIONS:")
        .expect("print_help has an OPTIONS: block");
    let section = &CLI_SOURCE[start..];
    let end = section
        .find("ENVIRONMENT:")
        .expect("OPTIONS is followed by ENVIRONMENT");
    let mut flags = BTreeSet::new();
    for line in section[..end].lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('-') {
            continue;
        }
        for token in trimmed.split([' ', ',']) {
            if let Some(flag) = token.strip_prefix("--") {
                if !flag.is_empty() {
                    flags.insert(format!("--{flag}"));
                }
                break;
            }
        }
    }
    flags
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

/// String literals that begin a `match` arm in `parse_cli`: `"--flag" =>` or `"--flag" |`.
/// Literals used elsewhere in the file — help text, tests such as `"--not-a-real-option"` —
/// are followed by other characters and are not counted.
fn parser_long_flags() -> BTreeSet<String> {
    let mut flags = BTreeSet::new();
    let mut rest = CLI_SOURCE;
    while let Some(open) = rest.find("\"--") {
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('"') else {
            break;
        };
        let literal = &after_open[..close];
        let tail = after_open[close + 1..].trim_start_matches(' ');
        if tail.starts_with("=>") || tail.starts_with('|') {
            flags.insert(literal.to_string());
        }
        rest = &after_open[close + 1..];
    }
    flags
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

    let mut read: BTreeSet<String> = BTreeSet::new();
    for source in ENV_SOURCES {
        let mut rest = *source;
        while let Some(at) = rest.find("var") {
            let after = &rest[at..];
            let literal = after
                .strip_prefix("var_os(\"")
                .or_else(|| after.strip_prefix("var(\""));
            if let Some(literal) = literal {
                let name: String = literal
                    .chars()
                    .take_while(|c| c.is_ascii_uppercase() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    read.insert(name);
                }
            }
            rest = &rest[at + 3..];
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
