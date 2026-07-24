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

## In Progress

- Cross-platform release packaging
- Model-routing analytics

## Agent Routing

The following agents are configured in `~/.config/opencode/agent/` and mapped in [`MODEL_ROUTING.md`](../MODEL_ROUTING.md):

| Agent | Model | Use |
| --- | --- | --- |
| `@explorer` | Ollama qwen3-coder (local) | Read-only codebase exploration |
| `@local` | Ollama qwen3-coder (local) | Sensitive/private code |
| `@junior` | nemotron-3-ultra-free (Zen free) | Routine implementation, docs |
| `@heavy` | GLM 5.2 (Ollama Cloud) | Complex implementation |
| `@heavy2` | GPT 5.6 Sol (Zen paid) | Architecture, second opinion |
| `@reviewer` | GPT 5.6 Sol (Zen paid, read-only) | Independent review |

## Privacy Policy

Hosted models may process non-secret code. Sensitive files, credentials, production data, and security-sensitive changes must use local models (`@local` or `@explorer`). When sensitivity is uncertain, route locally.
