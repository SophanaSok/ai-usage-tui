# Changelog

## Unreleased

- Added explicit local, free, calculated, estimated, reported, and unavailable cost states.
- Added read-only OpenCode database access with a busy timeout.
- Added automatic dashboard refresh.
- Added `--once`, `--json`, `--db`, `--refresh-interval`, and `--version` options.
- Added production-readiness and contribution documentation.
- Added an opt-in Ollama response journal via `--record-ollama`.
- Added `--journal` and `--refresh-zen` commands.
- Added a distinct Cloud usage category.
- Added arbitrary `--days` ranges and provider/model filters.
- Added journal idempotency and atomic Zen cache updates.
- Added cost-provenance, timestamp, range, and filter tests.
- Added completed-response validation and mutually exclusive action checks.
- Added TOML configuration and CSV export.
- Added cross-source deduplication between OpenCode history and the local journal.
- Added modularization into 11 source modules (cli, config, model, classify, collectors, export, ui, utils, helpers, pricing).
- Added bundled Zen pricing table with context-tier support.
- Added estimated pricing for unknown-cost events using the bundled table.
- Added `--refresh-pricing` command that scrapes the Zen docs page into `~/.local/share/ai-usage-tui/zen-pricing.toml`.
- Added HTTP retry/backoff for rate-limited Zen pricing fetches.
- Added fixture-based HTML parsing tests for the pricing scraper.
- Converted to library crate (`src/lib.rs`) enabling integration testing.
- Added integration test suite covering full pipeline, config precedence, export formats, and pricing engine.
- Added test fixtures for OpenCode DB, Ollama journal, and Zen pricing HTML.
- Added background collector framework with `Collector` trait, `CollectorHandle`, and `std::thread`-based polling.
- Added `OpenCodeCollector`, `JournalCollector`, and `ZenPricingCollector` built-in collectors.
- Added `[collectors.<name>]` TOML config section with `enabled` and `interval` per collector.
- TUI now uses background collectors by default; `--once`/`--json`/`--csv` stay synchronous.
- Added graceful shutdown via `AtomicBool` flag; `Drop` impl triggers shutdown automatically.
- Added `docs/background-collectors.md` architecture doc.
- Added budgets and alerts: `BudgetEngine`, `AlertDispatcher`, per-provider/model/global scopes.
- Added `[[budgets.entry]]` TOML config with `scope`, `period`, `limit`, `warn`, `critical`.
- Added `--check-budgets` (JSON output, exit 1 if alerts active) and `--webhook URL` CLI flags.
- Added TUI alert banner (yellow/critical) and budget panel toggle (`b` key).
- Added calendar-based period cutoffs (daily at 00:00 UTC, monthly on 1st).
- Added in-memory alert dedup (1-hour window) for webhook dispatch.
- Budget spend only counts `ProviderReported`, `Calculated`, and `Estimated` costs.
- Added model-routing analytics: `RoutingEvent` struct, `routing_event` journal table, `--record-routing` capture.
- Added `RoutingEngine` with aggregation (cost/task, token efficiency, retry/escalation/defect rates).
- Added `--routing-json` and `--routing-csv` export flags.
- Added TUI routing panel toggle (`t` key) with AGENT/MODEL/TOKENS/COST/RETRY%/DEFECTS/TASKS table.
- Added `docs/routing-analytics.md` schema doc.
- Privacy: routing events store only metadata — no prompts, completions, API keys, or credentials.

## 0.1.0

- Initial btop-inspired OpenCode usage dashboard.
