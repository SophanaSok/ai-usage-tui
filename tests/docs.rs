//! Guards against documentation drifting from the code.
//!
//! Two releases (v0.4.0, v0.4.1) shipped with the README quick start still pinning v0.3.0,
//! because nothing checked it. These tests are deliberately cheap string checks: they run under
//! the existing `cargo test --all-targets` job on every OS and fail *before* a release, not after.

use std::collections::BTreeSet;

const README: &str = include_str!("../README.md");
const CLI_SOURCE: &str = include_str!("../src/cli.rs");

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
