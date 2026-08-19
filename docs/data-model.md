# Data Model

The normalized usage event is the contract shared by collectors, aggregation, exports, and the UI.

```text
timestamp
provider
model
category: local | cloud | free | paid | unknown
cost_status: provider_reported | calculated | estimated | free | local | unavailable
request_count
input_tokens
output_tokens
reasoning_tokens
cache_read_tokens
cache_write_tokens
cost
latency (planned)
error_status (planned)
project (populated by the Claude Code collector from `cwd`)
session (populated by the Claude Code and OpenCode collectors)
source (planned)
```

Historical records should retain the pricing snapshot or source used to calculate their cost. This is not yet implemented — costs are currently re-derived from the active pricing table, so a `--refresh-pricing` rewrites historical figures. See [`roadmap.md`](roadmap.md) (finding 1.6). Provider adapters should tolerate missing optional fields and preserve the event with an explicit unknown status.

The local journal currently stores usage metadata in `usage_event`. It intentionally excludes prompt and completion content.

## Budget Configuration

```text
scope: global | provider | model
period: daily | weekly | monthly
limit: float
```

Budget alerts fire when spend exceeds the configured threshold and are displayed in the TUI banner. Webhook dispatch is optional and configured under `[budgets.webhook]`.

## Routing Event

The routing event captures agent-to-model routing decisions:
task, phase, agent, model, provider, category, tokens, cost,
retries, escalations, test_result, review_defects, created

Stored in `routing_event` table. See [`routing-analytics.md`](routing-analytics.md).