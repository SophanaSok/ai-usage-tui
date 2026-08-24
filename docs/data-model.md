# Data Model

The normalized usage event is the contract shared by collectors, aggregation, exports, and the UI.

```text
event_id (stable per-event identity from the source, used for dedup; null falls back to shape + timestamp)
timestamp
provider
model
category: local | cloud | free | paid | unknown
cost_status: reported | calculated | estimated | free | local | quota | unavailable
request_count
input_tokens
output_tokens
reasoning_tokens
cache_read_tokens
cache_write_tokens
cost
billing: per_token | subscription (set by the collector; subscription rows become `quota`)
api_equivalent_cost: float | null (list-rate figure for subscription rows only; never summed into cost; the last, 15th, CSV column)
latency (planned)
error_status (planned)
project (populated by the Claude Code and Codex collectors from `cwd`)
session (populated by the Claude Code, Codex — the thread id — and OpenCode collectors)
source (planned)
```

Historical events are priced at the rates that were in effect when they happened, not at whatever the table says now. `pricing/zen.toml` carries effective-dated `[[model."x".period]]` blocks with a `through` date, and `estimate_cost` selects the period covering the event's date before falling back to current rates. A `--refresh-pricing` therefore no longer rewrites historical figures, provided the rate change is recorded as a new period rather than an overwrite.

Provider adapters should tolerate missing optional fields and preserve the event with an explicit unknown status.

The local journal currently stores usage metadata in `usage_event`. It intentionally excludes prompt and completion content.

## Budget Configuration

```text
scope: global | provider | model
period: daily | monthly
limit: float
```

Budget alerts fire when spend exceeds the configured threshold and are displayed in the TUI banner. Webhook dispatch is optional and configured via the `webhook` key of the `[budgets]` table (or `--webhook URL`, which overrides it).

## Routing Event

The routing event captures agent-to-model routing decisions:
task, phase, agent, model, provider, category, tokens, cost,
retries, escalations, test_result, review_defects, created

Stored in `routing_event` table. See [`routing-analytics.md`](routing-analytics.md).