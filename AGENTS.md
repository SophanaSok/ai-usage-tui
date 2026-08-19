# AGENTS.md

## Cursor Cloud specific instructions

`ai-usage-tui` is a single Rust (edition 2021) CLI/TUI product — a btop-style dashboard for AI token usage. There is no server, no web frontend, and no external service to run; it is a client-side binary that reads local SQLite data sources. Standard dev commands live in `CONTRIBUTING.md` and `.github/workflows/ci.yml`; lint/test/build/run are plain `cargo` (`cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets`, `cargo build`).

Non-obvious caveats:

- Toolchain: **Rust 1.88+** is required — that is the `rust-version` in `Cargo.toml` and what the MSRV CI job pins. It is set by the dependency graph rather than by this crate's own code: `darling` 0.23 and `instability` 0.3.12, both reached through `ratatui`, require it. An older default toolchain fails with an edition or MSRV error; `rustup default stable` fixes it.
- Always build/test with `--locked` to respect the committed `Cargo.lock`; a bare `cargo build` may try to resolve newer incompatible versions.
- The TUI (default `cargo run`) needs a real TTY. For non-interactive checks use `--once`, `--json`, or `--csv`. TUI keys: `1`/`2`/`3`/`4` = today/7d/30d/all range, `r` refresh, `b`/`t`/`p` toggle the budgets/routing/per-project panels, `j`/`k` move selection, `q` quits.
- Fixture data in `tests/fixtures/opencode_test.db` uses old (2023-era) timestamps, so `--today`/`--week` show empty results. Use `--all` to see data, e.g. `cargo run --locked -- --db tests/fixtures/opencode_test.db --all`.
- Data sources are file paths, not ports: OpenCode DB via `--db`/`OPENCODE_DB_PATH`, journal via `--journal`/`AI_USAGE_JOURNAL_PATH`, Claude Code session logs via `--claude-dir`/`CLAUDE_PROJECTS_DIR`. Default paths resolve from `HOME`, falling back to `USERPROFILE`/`%LOCALAPPDATA%`/`%APPDATA%` on Windows, so no single variable is mandatory. Ollama (`:11434`) and Zen pricing HTTP refresh are optional enrichments, not required to run or test.
- Tests are hermetic and must stay that way: anything exercising `load_usage` or `print_once` needs an explicit `--claude-dir`, or it reads the developer's real `~/.claude/projects`.
- `AI_USAGE_LOG=1` (or a path) writes collector diagnostics to a file. The dashboard holds the alternate screen, so stderr is invisible while it runs.
