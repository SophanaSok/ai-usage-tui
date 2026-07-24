# Changelog

## 0.2.0 - 2026-07-24

### Added

- Background collector framework with `Collector` trait, `CollectorHandle`, and `std::thread`-based polling.
- Built-in collectors: `OpenCodeCollector` (30s), `JournalCollector` (60s), `ZenPricingCollector` (3600s, opt-in).
- `[collectors.<name>]` TOML config section with `enabled` and `interval` per collector.
- Budgets and alerts: `BudgetEngine`, `AlertDispatcher`, per-provider/model/global scopes.
- `[[budgets.entry]]` TOML config with `scope`, `period`, `limit`, `warn`, `critical`.
- `--check-budgets` (JSON output, exit 1 if alerts active) and `--webhook URL` CLI flags.
- TUI alert banner (yellow/critical) and budget panel toggle (`b` key).
- Calendar-based period cutoffs (daily at 00:00 UTC, monthly on 1st).
- In-memory alert dedup (1-hour window) for webhook dispatch.
- Model-routing analytics: `RoutingEvent` struct, `routing_event` journal table, `--record-routing` capture.
- `RoutingEngine` with aggregation (cost/task, token efficiency, retry/escalation/defect rates).
- `--routing-json` and `--routing-csv` export flags.
- TUI routing panel toggle (`t` key) with AGENT/MODEL/TOKENS/COST/RETRY%/DEFECTS/TASKS table.
- `--refresh-pricing` command that scrapes the Zen docs page into `~/.local/share/ai-usage-tui/zen-pricing.toml`.
- HTTP retry/backoff for rate-limited Zen pricing fetches.
- Fixture-based HTML parsing tests for the pricing scraper.
- Library crate conversion (`src/lib.rs`) enabling integration testing.
- Integration test suite covering full pipeline, config precedence, export formats, and pricing engine.
- Test fixtures for OpenCode DB, Ollama journal, and Zen pricing HTML.
- Cross-platform packaging: `.tar.gz`, `.deb`, `.rpm` (Linux), `.tar.gz` + Homebrew (macOS), `.zip` + Scoop + Chocolatey (Windows).
- `scripts/release.sh` pre-flight checklist (branch check, tests, clippy, build, version verification).
- Tag-triggered GitHub Actions release workflow with multi-OS matrix build, SHA256 checksums, and auto-generated GitHub Release from CHANGELOG.
- `[package.metadata.deb]` and `[package.metadata.generate-rpm]` Cargo.toml sections.
- Package manager templates: Homebrew formula, Scoop manifest, Chocolatey nuspec + install script.
- `docs/background-collectors.md` and `docs/routing-analytics.md` architecture docs.

### Changed

- TUI now uses background collectors by default; `--once`/`--json`/`--csv` stay synchronous.
- Converted to library crate (`src/lib.rs`) enabling integration testing.
- Graceful shutdown via `AtomicBool` flag; `Drop` impl triggers shutdown automatically.
- Budget spend only counts `ProviderReported`, `Calculated`, and `Estimated` costs.
- Privacy: routing events store only metadata — no prompts, completions, API keys, or credentials.

## 0.1.0

- Initial btop-inspired OpenCode usage dashboard.