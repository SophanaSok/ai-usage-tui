# Data Model

The normalized usage event is the contract shared by collectors, aggregation, exports, and the UI.
This is the model; the exported spelling of every key and every enum value, with its meaning, is
[`json-glossary.json`](json-glossary.json), printed by `ai-usage-tui --schema` and checked against
real output by a test.

```text
event_id (stable per-event identity from the source, used for dedup; null falls back to shape + timestamp)
timestamp
provider
model
category: LOCAL | CLOUD | FREE | PAID | UNKNOWN (exported in upper case)
cost_status: reported | calculated | estimated | free | local | quota | unavailable
request_count
input_tokens
output_tokens
reasoning_tokens
cache_read_tokens
cache_write_tokens
cost
billing: per_token | subscription (set by the collector; subscription rows become `quota`; exported in each `--json` row)
api_equivalent_cost: float | null (list-rate figure for subscription rows only; never summed into cost; the last, 15th, CSV column)
project (populated by the Claude Code, Codex and Copilot collectors from `cwd`; Copilot falls back to `repository`)
session (populated by the Claude Code, Codex — the thread id — OpenCode and Copilot collectors)
```

Not part of the contract, and not collected: latency, error status, and a per-row source id.

Historical events are priced at the rates that were in effect when they happened, not at whatever the table says now. `pricing/zen.toml` carries effective-dated `[[model."x".period]]` blocks with a `through` date, and `estimate_cost` selects the period covering the event's date before falling back to current rates. A `--refresh-pricing` therefore no longer rewrites historical figures, provided the rate change is recorded as a new period rather than an overwrite.

Provider adapters should tolerate missing optional fields and preserve the event with an explicit unknown status.

The local journal currently stores usage metadata in `usage_event`. It intentionally excludes prompt and completion content.

The journal's schema version lives in SQLite's `PRAGMA user_version` (`JOURNAL_SCHEMA_VERSION` in
`src/collector/journal.rs`, currently `1`). Writers migrate their table under `BEGIN IMMEDIATE`, so
concurrent hooks cannot race a migration, and refuse a journal stamped with a newer version than
they know rather than writing into a shape they may not understand. Readers open it read-only and
adapt to the columns they find.

## Budget Configuration

```text
scope: global | provider | model
period: daily | monthly
limit: float
```

Budget alerts fire when spend exceeds the configured threshold and are displayed in the TUI banner. Webhook dispatch is optional and configured via the `webhook` key of the `[budgets]` table (or `--webhook URL`, which overrides it).

## Routing Event

The routing event captures agent-to-model routing decisions:
event_id, task, phase, agent, model, provider, category, cost_status, requests, tokens, cost,
retries, escalations, test_result, review_defects, created

`retries`, `escalations` and `review_defects` are nullable: null is "not reported", which is not
`0`. The schema block in [`routing-analytics.md`](routing-analytics.md) is authoritative.

Stored in `routing_event` table. See [`routing-analytics.md`](routing-analytics.md).