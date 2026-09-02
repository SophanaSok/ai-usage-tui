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

// ---------------------------------------------------------------------------------------------
// One identity.
//
// The project's description of itself was hand-copied into seven files plus GitHub's About box,
// in five different wordings, and the README told readers for three releases that crates.io and
// the Homebrew tap did not exist yet. Gemini CLI shipped in v0.7.0 and never reached the README's
// first paragraph.
//
// `Cargo.toml` is the single source of truth: crates.io reads it verbatim at publish, so that one
// consumer is correct structurally. Everything else is either derived from it (clap's `about`,
// the packaging templates' `__DESCRIPTION__`) or checked against it here. GitHub has no manifest,
// so `.github/workflows/identity.yml` does that half -- it needs the network, which these tests
// must never touch.
// ---------------------------------------------------------------------------------------------

const CARGO_TOML: &str = include_str!("../Cargo.toml");
const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");

/// The parsed manifest.
///
/// `toml` is a regular dependency and Cargo puts it in scope for integration tests, so this needs
/// no manifest change. Everything read here -- `Cargo.toml`, `README.md`, `packaging/**` -- is
/// inside the published tarball, so `cargo test` in an unpacked crate still passes. **Nothing in
/// this file may read `.github/`**: `exclude` drops it, and a test that read it would fail for
/// anyone who installed from crates.io.
fn manifest() -> toml::Value {
    toml::from_str(CARGO_TOML).expect("Cargo.toml parses")
}

/// The sources that contribute usage rows, by the name the product calls them.
///
/// `contributes_rows` is the discriminator rather than a second hand-maintained list: it is
/// already `true` for exactly the five sources a one-line description should name, and `false`
/// only for `zen_pricing`, which produces no rows.
fn row_labels() -> Vec<&'static str> {
    ai_usage_tui::collector::registry::SOURCES
        .iter()
        .filter(|spec| spec.contributes_rows)
        .map(|spec| spec.label)
        .collect()
}

fn topics() -> Vec<String> {
    manifest()["package"]["metadata"]["identity"]["topics"]
        .as_array()
        .expect("[package.metadata.identity] topics is an array")
        .iter()
        .map(|value| value.as_str().expect("a topic is a string").to_string())
        .collect()
}

/// A source label as a GitHub topic: `Claude Code` -> `claude-code`.
fn slug(label: &str) -> String {
    label.to_lowercase().replace(' ', "-")
}

/// The README's tagline is the crate description, character for character.
///
/// They were two different sentences, and the one a reader saw depended on whether they arrived
/// from GitHub or from crates.io.
#[test]
fn readme_tagline_is_the_crate_description() {
    let tagline = README
        .lines()
        .find_map(|line| line.strip_prefix("> "))
        .expect("README.md opens with a `> ` tagline under the title");
    assert_eq!(
        tagline.trim(),
        DESCRIPTION,
        "README.md's tagline and Cargo.toml's description are different sentences"
    );
}

/// The crate description names every source that contributes usage rows.
///
/// This is what a stranger reads on crates.io and in GitHub's About box. It described the tool as
/// reading "local and hosted AI usage" and named no source at all, while the GitHub description
/// named two of five.
#[test]
fn crate_description_names_every_usage_source() {
    let missing: Vec<&str> = row_labels()
        .into_iter()
        .filter(|label| !DESCRIPTION.contains(label))
        .collect();
    assert!(
        missing.is_empty(),
        "the crate description does not name {missing:?}.\ndescription: {DESCRIPTION}"
    );
}

/// So does the README's headline -- the title, tagline, opening paragraph and "What it shows".
///
/// Gemini CLI shipped in v0.7.0 with its own README section at "### Gemini CLI", and stayed
/// missing from all three places above it for two releases, because nothing looked.
#[test]
fn readme_headline_names_every_usage_source() {
    let end = README
        .find("## Prerequisites")
        .expect("README.md has a '## Prerequisites' section");
    let headline = &README[..end];
    let missing: Vec<&str> = row_labels()
        .into_iter()
        .filter(|label| !headline.contains(label))
        .collect();
    assert!(
        missing.is_empty(),
        "README.md's opening (title through 'What it shows') does not name {missing:?}"
    );
}

/// Every registered source has a "Data sources" section, and every section is a registered
/// source.
#[test]
fn readme_data_source_sections_match_the_registry() {
    let start = README
        .find("## Data sources")
        .expect("README.md has a '## Data sources' section");
    let rest = &README[start + "## Data sources".len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    let documented: BTreeSet<String> = rest[..end]
        .lines()
        .filter_map(|line| line.strip_prefix("### "))
        .map(|heading| heading.trim().to_string())
        .collect();
    let registered: BTreeSet<String> = ai_usage_tui::collector::registry::SOURCES
        .iter()
        .map(|spec| spec.label.to_string())
        .collect();
    assert_eq!(
        documented, registered,
        "the README's 'Data sources' subsections and registry::SOURCES disagree"
    );
}

/// GitHub's topics cover every keyword and every source.
///
/// The source-name rule is the forcing function: a collector cannot be added without the project
/// admitting in public that it exists. `every_source_is_reachable_from_both_paths` does this for
/// the code paths; this does it for the identity.
#[test]
fn github_topics_cover_the_keywords_and_the_sources() {
    let topics = topics();
    assert!(
        topics.len() <= 20,
        "GitHub allows 20 topics; Cargo.toml lists {}",
        topics.len()
    );
    for topic in &topics {
        assert!(
            !topic.is_empty()
                && topic.len() <= 50
                && topic
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                && !topic.starts_with('-'),
            "{topic:?} is not a valid GitHub topic (lowercase alphanumerics and hyphens, <= 50)"
        );
    }

    let keywords: Vec<String> = manifest()["package"]["keywords"]
        .as_array()
        .expect("keywords is an array")
        .iter()
        .map(|value| value.as_str().expect("a keyword is a string").to_string())
        .collect();
    assert!(
        keywords.len() <= 5,
        "crates.io allows five keywords; Cargo.toml lists {}",
        keywords.len()
    );
    let missing: Vec<&String> = keywords.iter().filter(|k| !topics.contains(k)).collect();
    assert!(
        missing.is_empty(),
        "keywords {missing:?} are not also GitHub topics; a term crates.io advertises and GitHub \
         does not is the drift this pairing exists to prevent"
    );

    let unlisted: Vec<String> = row_labels()
        .into_iter()
        .map(slug)
        .filter(|s| !topics.contains(s))
        .collect();
    assert!(
        unlisted.is_empty(),
        "registered sources {unlisted:?} have no GitHub topic; add them to \
         [package.metadata.identity] topics"
    );
}

/// The description fits every registry it is published to, and survives the release job's `sed`.
///
/// `release.yml` substitutes it into the packaging templates with
/// `sed -e "s|__DESCRIPTION__|${DESCRIPTION}|g"`, where an unescaped `&` means "the whole match"
/// and a `|` ends the expression. Either would render a corrupted manifest rather than failing.
#[test]
fn crate_description_fits_every_registry() {
    assert!(!DESCRIPTION.is_empty(), "the crate has no description");
    assert!(
        DESCRIPTION.chars().count() <= 350,
        "GitHub caps a repository description at 350 characters; this is {}",
        DESCRIPTION.chars().count()
    );
    for bad in ['|', '&', '\\', '\n'] {
        assert!(
            !DESCRIPTION.contains(bad),
            "{bad:?} in the description would corrupt release.yml's sed substitution"
        );
    }
    // A PKGBUILD is bash, and `packaging/aur/PKGBUILD` renders the description into a
    // single-quoted `pkgdesc`. Double quotes are not an option there: the description contains
    // the literal `$0.00`, and bash expanded that to the script's own path, so the rendered
    // pkgdesc read "instead of rendering as /usr/bin/makepkg.00". Nothing escapes a single quote
    // inside single quotes -- the string simply ends -- so the character has to be absent.
    assert!(
        !DESCRIPTION.contains('\''),
        "a single quote in the description would end packaging/aur/PKGBUILD's pkgdesc string; \
         bash has no escape for one inside single quotes, and double quotes expand the $0.00"
    );
}

/// The packaging templates carry the placeholder, not a copy of the description.
#[test]
fn packaging_templates_render_the_description() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for relative in [
        "packaging/homebrew/ai-usage-tui.rb",
        "packaging/scoop/ai-usage-tui.json",
        "packaging/chocolatey/ai-usage-tui.nuspec",
    ] {
        let path = root.join(relative);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(
            text.contains("__DESCRIPTION__"),
            "{relative} has no __DESCRIPTION__ placeholder for release.yml to render"
        );
        assert!(
            !text.contains(DESCRIPTION),
            "{relative} carries a literal copy of the description; use __DESCRIPTION__"
        );
    }
}

/// The AUR `pkgdesc` is one line, derived from the crate description rather than written twice.
///
/// `man PKGBUILD` on pkgdesc: "This should be a brief description of the package and its
/// functionality. Try to keep the description to one line of text and to not use the package's
/// name." The crate description is 300-odd characters because crates.io, GitHub's About box and
/// Homebrew's `desc` all take it whole -- rendering that into `pkgdesc` would be three lines in
/// `pacman -Si` and in every AUR search result.
///
/// The rule is "the clause before the em dash", and `release.yml` spells it as one parameter
/// expansion. This pins the properties that rule has to keep producing, so a description edit
/// that breaks them fails here rather than in an AUR review: if the em dash goes, the split
/// yields the whole string and the length assertion catches it.
#[test]
fn aur_pkgdesc_is_one_line_and_derived() {
    let short = DESCRIPTION
        .split(" \u{2014} ")
        .next()
        .expect("splitting a non-empty string yields at least one part");
    assert!(
        !short.is_empty(),
        "the description starts with the em dash separator, so the derived pkgdesc is empty"
    );
    assert!(
        short.chars().count() <= 100,
        "the pkgdesc derived from the description is {} characters; `man PKGBUILD` asks for one \
         line of text.\nderived: {short}",
        short.chars().count()
    );
    assert!(
        !short.contains(env!("CARGO_PKG_NAME")),
        "`man PKGBUILD` asks that pkgdesc not use the package's name, and the derived one does.\n\
         derived: {short}"
    );

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let pkgbuild = std::fs::read_to_string(root.join("packaging/aur/PKGBUILD"))
        .expect("read packaging/aur/PKGBUILD");

    // Single-quoted and taking the short token: both are load-bearing and both were wrong once.
    assert!(
        pkgbuild.contains("pkgdesc='__DESCRIPTION_SHORT__'"),
        "packaging/aur/PKGBUILD must render the short description into a single-quoted pkgdesc; \
         double quotes let bash expand the $0.00 in the description to the script's own path"
    );

    // The Maintainer comment is the one line in the file with a required position:
    // /usr/share/pacman/PKGBUILD.proto puts it above `pkgname`, and it sat below the explanatory
    // block here once.
    let first = pkgbuild.lines().next().unwrap_or_default();
    assert!(
        first.starts_with("# Maintainer: "),
        "packaging/aur/PKGBUILD's first line must be the Maintainer comment, as \
         /usr/share/pacman/PKGBUILD.proto has it.\ngot: {first}"
    );

    // `ldd` on the shipped binary reports libgcc_s, libc and libm. namcap reports an undeclared
    // dependency as an error, and an empty `depends` is what it looks for.
    assert!(
        pkgbuild.contains("depends=('gcc-libs' 'glibc')"),
        "packaging/aur/PKGBUILD must declare the libraries the binary links: gcc-libs and glibc"
    );

    // namcap warns "Reference to x86_64 should be changed to $CARCH" against the source arrays,
    // and taking that advice breaks the aarch64 package. $CARCH is the *build host's* arch, so
    // inside `source_aarch64` it expands to whatever machine ran makepkg -- and .SRCINFO is
    // generated once on one machine and pushed, so every ARM user would fetch the x86_64 tarball
    // and fail its checksum. Verified by substituting it and reading `makepkg --printsrcinfo`.
    // This is here so the warning cannot be silenced by "fixing" it.
    for (array, literal) in [
        ("source_x86_64", "-x86_64-linux.tar.gz"),
        ("source_aarch64", "-aarch64-linux.tar.gz"),
    ] {
        let line = pkgbuild
            .lines()
            .find(|l| l.starts_with(array))
            .unwrap_or_else(|| panic!("packaging/aur/PKGBUILD has no {array} line"));
        assert!(
            line.contains(literal),
            "{array} must name its architecture literally ({literal}); namcap suggests $CARCH \
             there and that resolves to the builder's arch, breaking the other one.\ngot: {line}"
        );
        assert!(
            !line.contains("$CARCH"),
            "{array} uses $CARCH, which expands to the build host's architecture and would make \
             .SRCINFO point both arches at one tarball.\ngot: {line}"
        );
    }
}

/// The manifest states the description once.
///
/// `[package.metadata.generate-rpm]` carried its own `summary`, a second copy in the same file.
/// cargo-generate-rpm falls back to `package.description`, so the key is gone rather than tested.
#[test]
fn the_manifest_carries_one_description() {
    let manifest = manifest();
    let rpm = &manifest["package"]["metadata"]["generate-rpm"];
    assert!(
        rpm.get("summary").is_none(),
        "[package.metadata.generate-rpm] has its own `summary`; delete it and let \
         cargo-generate-rpm fall back to package.description"
    );
}

/// `--help` and the man page describe the product, not the parser.
///
/// `struct Args` in `src/cli.rs` carried a `///` doc comment explaining why it is separate from
/// `Cli`. Clap promotes a doc comment on the parser struct to `long_about`, so that paragraph was
/// the DESCRIPTION section of `ai-usage-tui --man` -- shipped in the .deb and the .rpm and
/// installed to /usr/share/man/man1/. `man ai-usage-tui` explained the clap migration.
#[test]
fn help_and_man_describe_the_product() {
    let command = ai_usage_tui::cli::command();
    assert_eq!(
        command.get_about().map(|s| s.to_string()).as_deref(),
        Some(DESCRIPTION),
        "clap's `about` is not the crate description"
    );
    assert!(
        command.get_long_about().is_none(),
        "clap has a `long_about`, which becomes the man page's DESCRIPTION section. It comes \
         from a `///` doc comment on `struct Args`; make it `//`.\ngot: {:?}",
        command.get_long_about().map(|s| s.to_string())
    );
}

/// No document claims a release channel that already exists.
///
/// Deliberately a literal phrase ban, and deliberately weak: someone writing "coming soon to
/// crates.io" defeats it entirely. It is worth its four lines only because it failed in four
/// places the day it was written. The claims that can be checked properly are checked properly --
/// `readme_names_every_packaging_template` binds prose to an artifact in the tree, and
/// `identity.yml` checks that each channel the README names actually exists.
#[test]
fn no_stale_publication_notes() {
    const BANNED: &[&str] = &[
        "Not published yet",
        "is unclaimed",
        "not on crates.io",
        "until the maintainer",
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut docs = vec![("README.md".to_string(), README.to_string())];
    for entry in std::fs::read_dir(root.join("docs"))
        .expect("docs/ exists")
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "md") {
            let name = format!("docs/{}", path.file_name().unwrap().to_string_lossy());
            docs.push((name, std::fs::read_to_string(&path).expect("read a doc")));
        }
    }
    let mut stale = Vec::new();
    for (name, text) in &docs {
        for (index, line) in text.lines().enumerate() {
            for phrase in BANNED {
                if line.contains(phrase) {
                    stale.push(format!("{name}:{}: {}", index + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        stale.is_empty(),
        "these claim a release channel that already exists:\n{}",
        stale.join("\n")
    );
}

/// Every packaging template is named in the README, and the README names no other.
///
/// This is the strong, hermetic half of the prose guard: prose naming a distribution mechanism is
/// bound to that mechanism's artifact in the tree. Deleting `packaging/scoop/` while the README
/// still says `scoop install` fails the build. Chocolatey was rendered and attached to every
/// release and the README never mentioned it.
#[test]
fn readme_names_every_packaging_template() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let templates: BTreeSet<String> = std::fs::read_dir(root.join("packaging"))
        .expect("packaging/ exists")
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().to_lowercase())
        .collect();
    assert!(
        !templates.is_empty(),
        "packaging/ has no template directories; the section marker may have moved"
    );
    let lowered = README.to_lowercase();
    let unmentioned: Vec<&String> = templates.iter().filter(|t| !lowered.contains(*t)).collect();
    assert!(
        unmentioned.is_empty(),
        "packaging/{unmentioned:?} exists and is rendered into every release, and the README \
         never mentions it"
    );
}

/// The README links the page `cargo install` installs from.
///
/// It documented `cargo install ai-usage-tui` and linked crates.io nowhere -- not in the badges,
/// not in the text.
#[test]
fn readme_links_the_crates_io_page() {
    let url = format!("https://crates.io/crates/{}", env!("CARGO_PKG_NAME"));
    assert!(
        README.contains(&url),
        "README.md does not link {url}, though it documents `cargo install`"
    );
}
