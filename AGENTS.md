# AGENTS.md

## Working in this repository

`ai-usage-tui` is a single Rust (edition 2021) CLI/TUI product — a btop-style dashboard for AI token usage. There is no server, no web frontend, and no external service to run; it is a client-side binary that reads local SQLite data sources. Standard dev commands live in the `justfile` (`just check` runs exactly what CI runs, in CI's order); the raw equivalents are in `CONTRIBUTING.md` and `.github/workflows/ci.yml` — plain `cargo` (`cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features --locked -- -D warnings`, `cargo test --all-targets --locked`, `cargo build --locked`).

Non-obvious caveats:

- Toolchain: **Rust 1.88+** is required — that is the `rust-version` in `Cargo.toml` and what the MSRV CI job pins. It is set by the dependency graph rather than by this crate's own code: `darling` 0.23 and `instability` 0.3.12, both reached through `ratatui`, require it. An older default toolchain fails with an edition or MSRV error; `rustup default stable` fixes it.
- Always build/test with `--locked` to respect the committed `Cargo.lock`; a bare `cargo build` may resolve a different dependency set than the one that was tested. CI passes `--locked` on every cargo command, so a change that needs a lockfile update fails there rather than drifting silently. (`cargo fmt` is the exception — it resolves nothing.)
- The TUI (default `cargo run`) needs a real TTY. For non-interactive checks use `--once`, `--json`, or `--csv`. TUI key bindings are defined once in `src/ui/keys.rs`; `ai-usage-tui --help` prints them, and the `?` overlay and the footer render the same table. Do not restate them here — this prose was the fifth copy and it drifted.
- Fixture data in `tests/fixtures/opencode_test.db` uses old (2023-era) timestamps, so `--today`/`--week` show empty results. Use `--all` to see data, e.g. `cargo run --locked -- --db tests/fixtures/opencode_test.db --all`.
- Data sources are file paths, not ports: OpenCode DB via `--db`/`OPENCODE_DB_PATH`, journal via `--journal`/`AI_USAGE_JOURNAL_PATH`, Claude Code session logs via `--claude-dir`/`CLAUDE_PROJECTS_DIR`, Codex CLI rollouts via `--codex-dir`/`CODEX_HOME`, Copilot's store via `--copilot-dir`/`COPILOT_HOME`, Omarchy's agents-panel records via `--omarchy-dir`/`XDG_STATE_HOME`. Default paths resolve from `HOME`, falling back to `USERPROFILE`/`%LOCALAPPDATA%`/`%APPDATA%` on Windows, so no single variable is mandatory. Ollama (`:11434`) and Zen pricing HTTP refresh are optional enrichments, not required to run or test.
- Tests are hermetic and must stay that way: anything exercising `load_usage` or `print_once` needs an explicit `--claude-dir`, `--codex-dir`, `--copilot-dir` and `--gemini-dir`, or it reads the developer's real `~/.claude/projects` and `~/.codex` — and, for the billing decision, their real `~/.claude.json`. The config document is derived from `claude_dir` (`<claude_dir>/../../.claude.json`), so a fixture root resolves to a file that does not exist; pass `claude_json` when a test needs a planted one. Codex reads no config document, so `--codex-dir /nonexistent` is sufficient. The billing decision also reads Omarchy's records, so pass `--omarchy-dir /nonexistent` (lib tests set `omarchy_dir`) as well.
- `ai-usage-tui --doctor` prints what every data source resolved to — path searched, rows found, billing decision — without starting the dashboard. Fastest way to check a collector change, and the first thing to ask a bug reporter for.
- Data sources are registered once in `src/collector/registry.rs`; both the one-shot read and the background collectors iterate it, and a test fails the build if a source is reachable from only one.
- `AI_USAGE_LOG=1` (or a path) writes collector diagnostics to a file. The dashboard holds the alternate screen, so stderr is invisible while it runs.

## Extending it

Someone asking you to "make it support X" or "make it show Y" wants one of three things, cheapest first. Pick the cheapest that does the job.

- **No code here at all.** A tool that logs its own token counts goes through `ai-usage-tui --record-event` and a few lines of `jq`; a status bar, a digest or an alert is a script over `--summary-json`. `ai-usage-tui --agent-guide` is the entry point for both and matches the installed version.
- **A new data source, panel or price.** `CONTRIBUTING.md` ("Common contributions") has the file-by-file list, and `.claude/skills/add-data-source/` the order to work in. Most of the list is enforced: make the change, run the tests, and let what fails tell you what is missing.
- **A change to a stable surface** — a flag, a JSON key, a config key, a journal column, an exit code. Read `docs/stability.md` first; additive is a minor release, anything else is breaking.

Rules that outrank the request:

- **Never invent a number.** No token count derived from message length, no absent count read as `0`, no unknown cost rendered as `$0.00`. If the tool does not measure it, there is no row — the README's "Why there is no Cursor collector" is the worked example, and refusing is a complete answer.
- **Work from bytes the tool really wrote.** A parser written from documentation has been wrong every time here. Capture real output, redact it, commit it under `tests/fixtures/`, and measure your rule against it before relying on it. Never commit an unredacted capture: transcripts hold source code and secrets, and only the usage block may be parsed.
- **A test must fail against the bug it is for.** Restore the bug in a scratch copy and watch it fail before you trust it. A guard that iterates the registry, the bindings table or the parser beats a list written into the test.
- **A failure must be visible.** `unwrap_or_default()` on a read, `Err(_) => continue`, a skipped row nobody counts: each renders "broken" as "nothing to report". Count what was skipped and surface it (`collector::skipped`).
- **New work gets an entry under `## [Unreleased]` in `CHANGELOG.md`**, saying what was wrong or missing and why it matters, not only what changed.
