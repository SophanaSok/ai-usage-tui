# AGENTS.md

## Cursor Cloud specific instructions

`ai-usage-tui` is a single Rust (edition 2021) CLI/TUI product — a btop-style dashboard for AI token usage. There is no server, no web frontend, and no external service to run; it is a client-side binary that reads local SQLite data sources. Standard dev commands live in `CONTRIBUTING.md` and `.github/workflows/ci.yml`; lint/test/build/run are plain `cargo` (`cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets`, `cargo build`).

Non-obvious caveats:

- Toolchain: the dependency graph (via the committed `Cargo.lock`) pulls crates that require `edition2024`, so Rust **1.85+** is required. The base VM's default rustup toolchain may be older (e.g. 1.83), which fails with `feature 'edition2024' is required`. The update script sets `rustup default stable` to fix this; if you hit that error, run `rustup default stable`.
- Always build/test with `--locked` to respect the committed `Cargo.lock`; a bare `cargo build` may try to resolve newer incompatible versions.
- The TUI (default `cargo run`) needs a real TTY. For non-interactive checks use `--once`, `--json`, or `--csv`. TUI keys: `1`/`2`/`3`/`4` = today/7d/30d/all range, `j`/`k` move selection, `q` quits.
- Fixture data in `tests/fixtures/opencode_test.db` uses old (2023-era) timestamps, so `--today`/`--week` show empty results. Use `--all` to see data, e.g. `cargo run --locked -- --db tests/fixtures/opencode_test.db --all`.
- Data sources are file paths, not ports: OpenCode DB via `--db`/`OPENCODE_DB_PATH`, journal via `--journal`/`AI_USAGE_JOURNAL_PATH`. Default paths resolve under `$HOME`, so `HOME` must be set. Ollama (`:11434`) and Zen pricing HTTP refresh are optional enrichments, not required to run or test.
