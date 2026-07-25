# Provider Support

## OpenCode

Reads assistant usage metadata from the local OpenCode SQLite database. This is the current supported collector.

The OpenCode collector runs as a background task polling every 30 seconds (configurable via `[collectors.opencode.interval]`). It writes normalized usage events to the local journal database.

## Ollama

Ollama response metrics expose prompt and output token counts, but Ollama does not provide a complete historical usage database. `--record-ollama` provides an opt-in local journal for requests made after tracking is enabled.

The journal collector polls every 60 seconds (configurable via `[collectors.journal.interval]`) and aggregates Ollama events into the same normalized shape.

## Ollama Cloud

Token counts can be observed when returned by the client response. Account quota and GPU-based Cloud billing are not currently exposed through the supported API, so the tool must not invent a dollar cost. Cloud-routed models are displayed as `CLOUD`, never as local usage.

## OpenCode Zen

Zen usage can be read from OpenCode history. `--refresh-pricing` scrapes the live Zen pricing table from the docs page with retry/backoff and caches it at `~/.local/share/ai-usage-tui/zen-pricing.toml`. The background `ZenPricingCollector` refreshes hourly when enabled via `[collectors.zen_pricing.enabled]`. Pricing snapshots are applied to historical events for cost calculation.

## Background Collectors

- OpenCode collector: polls OpenCode DB every 30s (configurable)
- Journal collector: polls journal DB every 60s (configurable)
- Zen Pricing collector: refreshes hourly when enabled (opt-in)

All collectors write to the journal database; the TUI reads from the journal on its own refresh interval, keeping the UI responsive.

## Budgets

Budget limits can be configured per provider, model, or globally with daily or monthly periods. Alerts appear in the TUI banner. The CLI accepts `--webhook URL` and config accepts `budgets.webhook`, but webhook dispatch is not currently wired. Check budgets from CLI with `ai-usage-tui --check-budgets` (exits 1 if thresholds exceeded).