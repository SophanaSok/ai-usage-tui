# Contributing

## Development

Install Rust through [rustup](https://rustup.rs/), then run:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --locked
cargo test --doc --locked
cargo build --release --locked
```

`--locked` is not decoration. The lockfile is committed and the project ships release binaries,
so a build that silently resolved a different dependency set would test one thing and publish
another. CI passes `--locked` on every one of these, which also makes a dependency PR that edits
`Cargo.toml` without regenerating `Cargo.lock` fail rather than merge. `cargo fmt` takes no
`--locked` because it never resolves dependencies.

## Changes

- Keep provider-specific logic in a collector, not in the UI.
- Do not collect prompts, completions, API keys, or credentials.
- Add tests for parsing, classification, and aggregation changes.
- Preserve explicit cost provenance; never convert unknown cost to zero.
- Update the README and changelog for user-visible behavior.

## Pull Requests

Describe the user-visible behavior, data sources, privacy impact, and verification commands. Keep pull requests focused and avoid unrelated formatting changes.
