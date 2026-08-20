# Phase Status

## Completed

- Foundation and btop-inspired dashboard
- Explicit cost provenance and Cloud classification
- Read-only OpenCode collector
- Automatic refresh and terminal cleanup
- Non-interactive JSON output
- Ollama response journal
- OpenCode Zen catalog cache
- Hosted-model policy for non-secret code
- CLI filtering and arbitrary time windows
- TOML configuration and CSV export
- Journal idempotency and cost-provenance hardening
- Agent routing configuration with dedicated reviewer and explorer agents
- Modularization into 11 source modules
- Bundled Zen pricing table with context-tier support
- Estimated pricing applied to unknown-cost events
- `--refresh-pricing` command that scrapes the Zen docs page into `~/.local/share/ai-usage-tui/zen-pricing.toml`
- HTTP retry/backoff for rate-limited Zen pricing fetches
- Fixture-based HTML parsing tests for the pricing scraper
- Library crate conversion with integration test suite (full pipeline, config precedence, exports, pricing engine)
- Test fixtures for OpenCode DB, Ollama journal, and Zen pricing HTML
- Background collector framework (`Collector` trait, `CollectorHandle`, `std::thread`-based polling)
- Built-in collectors: `OpenCodeCollector`, `JournalCollector`, `ZenPricingCollector`
- `[collectors.<name>]` config with per-collector `enabled` and `interval`
- TUI uses background collectors by default; `--once`/`--json`/`--csv` stay synchronous
- Budgets and alerts: `BudgetEngine`, `AlertDispatcher`, per-provider/model/global scopes
- `[[budgets.entry]]` config with `--check-budgets` (exit 1 on alerts) and `--webhook` CLI
- TUI alert banner and budget panel toggle (`b` key) with calendar-based period cutoffs
- Model-routing analytics: `RoutingEvent`, `routing_event` table, `--record-routing` capture
- `RoutingEngine` aggregation: cost/task, token efficiency, retry/escalation/defect rates
- `--routing-json` and `--routing-csv` export flags
- TUI routing panel toggle (`t` key) with sortable aggregate table
- Cross-platform packaging: `.tar.gz`/`.deb`/`.rpm` (Linux), `.tar.gz`+Homebrew (macOS), `.zip`+Scoop+Chocolatey (Windows)
- `scripts/release.sh` pre-flight checklist
- Tag-triggered GitHub Actions release workflow with multi-OS matrix, SHA256 checksums, auto GitHub Release
- Package manager templates: Homebrew formula, Scoop manifest, Chocolatey nuspec
- Documentation updated for v0.2.0: README, architecture, data-model, provider-support, release-process
- Version bumped to 0.2.0; CHANGELOG restructured to Keep-a-Changelog format
- Pre-flight checks passed (51 tests, clippy clean, release build verified)
- Security audit passed (cost provenance, privacy, SQL injection, thread safety, no secrets in packaging)

## In progress (unreleased)

Dashboard and analytics work on top of the audit fixes released in v0.3.0. Full detail in
[`CHANGELOG.md`](../CHANGELOG.md) under `[Unreleased]`.

- Four new panels: cost per project (`p`), spend over time (`g`), burn rate against budgets (`w`),
  and sessions (`s`)
- Routing analytics reworked to lead with cost per delivered result, plus escalations derived from
  collected sessions
- Pricing coverage surfaced in the header, and quota-billed usage separated from usage that has no
  price — the coverage figure previously reported a deliberate refusal to price as a failure
- `?` key reference; the footer no longer truncates on an 80-column terminal
- `src/ui.rs` split into `src/ui/`, with the project's first TUI rendering tests

### Released in v0.3.0 (audit-driven correctness, distribution, coverage)

- Accounting fixes: stable `event_id` deduplication, reasoning tokens billed, integer pricing
  rates parsed, missing rates yield `UNKNOWN COST` rather than `$0.00`, refreshed pricing applied
  as an overlay, local calendar-day boundaries shared by `TODAY` and daily budgets
- Claude Code JSONL collector, with `session_id` and `project` attribution on `Usage`
- Layered model-ID resolution (dated, provider-namespaced, dash-versioned, `:cloud`-suffixed ids)
- Incremental ingestion for the OpenCode collector
- `claude-opus-5` and `claude-mythos-5` added to the pricing table
- Windows path resolution; `--webhook` actually dispatches; token-based classification
- Release workflow builds and verifies per-architecture artifacts; packaging manifests rendered
  at release time
- CI matrix (Linux/macOS/Windows), MSRV job, `cargo-deny`, Dependabot
- 212 tests (from 51), clippy clean

**Next steps and outstanding audit findings: [`docs/roadmap.md`](roadmap.md).**

## Released

- v0.3.0 — 2026-08-19
- v0.2.0 — 2026-07-24

## Agent Routing

Agent-to-model assignments live in the OpenCode workspace config
(`~/.config/opencode/opencode.json` and `~/.config/opencode/ROUTING.md`), not in this repository.
[`MODEL_ROUTING.md`](../MODEL_ROUTING.md) carries the policy — tier discipline, the privacy
boundary, escalation signals, and the evaluation schema — without duplicating the mapping.

A copy of the agent table used to live here and in `MODEL_ROUTING.md`; both drifted from the live
config. Do not reintroduce one.

## Privacy Policy

Hosted models may process non-secret code. Sensitive files, credentials, production data, and security-sensitive changes must use local models (`@local` or `@explorer`). When sensitivity is uncertain, route locally.
