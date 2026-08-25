# Local checks, matching .github/workflows/ci.yml.
#
#   cargo install just     # once
#   just                   # list recipes
#   just check             # everything CI runs, in CI's order
#
# The point is that `just check` and CI cannot drift: if you change one, change the other. The
# raw cargo commands are in CONTRIBUTING.md for anyone who would rather not install just.

_default:
    @just --list

# Everything CI runs, in CI's order. This is the pre-push check.
#
# It really is everything now. It used to stop after `test-doc`, omitting the msrv, docs and
# audit jobs -- three of the seven -- while the comments above and here both claimed otherwise,
# so a green `just check` still left three ways to arrive red.
#
# `msrv` needs the 1.88 toolchain (`rustup toolchain install 1.88`) and `deny` needs cargo-deny
# (`cargo install cargo-deny`); both say so if they are missing.
check: fmt-check lint test test-doc msrv docs deny

# Rustfmt, as CI checks it.
fmt-check:
    cargo fmt --all -- --check

# Apply formatting.
fmt:
    cargo fmt --all

# Clippy with warnings denied, as CI runs it.
lint:
    cargo clippy --all-targets --all-features --locked -- -D warnings

# The test suite. `--locked` is not decoration; see CONTRIBUTING.md.
test:
    cargo test --all-targets --locked

test-doc:
    cargo test --doc --locked

# The MSRV job: the toolchain named by `rust-version` in Cargo.toml must compile the tree.
msrv:
    #!/usr/bin/env bash
    set -euo pipefail
    version="$(grep -m1 '^rust-version' Cargo.toml | cut -d'"' -f2)"
    rustup toolchain list | grep -q "^${version}" \
        || { echo "MSRV toolchain ${version} is not installed: rustup toolchain install ${version}" >&2; exit 1; }
    cargo "+${version}" check --all-targets --locked

# The docs job: rustdoc with warnings denied, and every relative Markdown link resolving.
docs:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked
    python3 scripts/check-markdown-links.py .

# Dependency advisories and licence policy, as CI runs it.
deny:
    #!/usr/bin/env bash
    set -euo pipefail
    command -v cargo-deny >/dev/null \
        || { echo "cargo-deny is not installed: cargo install cargo-deny" >&2; exit 1; }
    cargo deny check

# Run the dashboard against the committed fixture, hermetically.
#
# Without these overrides the dashboard reads your real ~/.claude/projects, ~/.codex, Omarchy
# records and usage journal. The fixture's timestamps are from 2023, so --all is the range that shows anything.
run *ARGS:
    cargo run --locked -- --db tests/fixtures/opencode_test.db --all \
        --claude-dir /nonexistent --codex-dir /nonexistent --omarchy-dir /nonexistent \
        --journal /nonexistent/journal.db {{ARGS}}

# What every data source resolved to on this machine. Reads your real paths, on purpose.
doctor:
    cargo run --locked -- --doctor

# Generate the man page and shell completions into dist/. Needed before `cargo deb`/`cargo
# generate-rpm`, which list them as assets; the release workflow runs this itself.
assets:
    cargo build --release --locked
    mkdir -p dist/completions
    ./target/release/ai-usage-tui --man > dist/ai-usage-tui.1
    ./target/release/ai-usage-tui --completions bash > dist/completions/ai-usage-tui.bash
    ./target/release/ai-usage-tui --completions zsh  > dist/completions/_ai-usage-tui
    ./target/release/ai-usage-tui --completions fish > dist/completions/ai-usage-tui.fish
    @echo "wrote dist/"

# Regenerate pricing/litellm.tsv from LiteLLM's community table. Needs network.
pricing:
    scripts/refresh-litellm-pricing.py

# Fail if the committed pricing snapshot has drifted from upstream. Not in CI: upstream changes
# constantly and a red build on someone else's commit is noise, not signal.
pricing-check:
    scripts/refresh-litellm-pricing.py --check

# Release pre-flight. Takes the version without the leading v: `just release 0.6.0`.
release VERSION:
    scripts/release.sh {{VERSION}}

# What GitHub's About box says, against what Cargo.toml says. Needs `gh auth login`.
#
# Deliberately not part of `just check`: it needs the network and a GitHub login, while `check`
# must stay exactly equal to ci.yml. This is the same comparison .github/workflows/identity.yml
# makes, and it is read-only -- `scripts/identity.sh --apply` is the one that writes.
identity:
    scripts/identity.sh --check
