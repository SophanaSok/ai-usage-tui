# Contributing

## Development

Install Rust through [rustup](https://rustup.rs/), then run:

```sh
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build --release
```

## Changes

- Keep provider-specific logic in a collector, not in the UI.
- Do not collect prompts, completions, API keys, or credentials.
- Add tests for parsing, classification, and aggregation changes.
- Preserve explicit cost provenance; never convert unknown cost to zero.
- Update the README and changelog for user-visible behavior.

## Pull Requests

Describe the user-visible behavior, data sources, privacy impact, and verification commands. Keep pull requests focused and avoid unrelated formatting changes.
