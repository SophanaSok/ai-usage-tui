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
project (populated by the Claude Code, Codex and Copilot collectors from `cwd`; Copilot falls back to `repository`; journaled by `--record-event` when the adapter sends it)
session (populated by the Claude Code, Codex — the thread id — OpenCode and Copilot collectors; journaled by `--record-event` when the adapter sends it)
```

Not part of the contract, and not collected: latency, error status, and a per-row source id.

Historical events are priced at the rates that were in effect when they happened, not at whatever the table says now. `pricing/zen.toml` carries effective-dated `[[model."x".period]]` blocks with a `through` date, and `estimate_cost` selects the period covering the event's date before falling back to current rates. A `--refresh-pricing` therefore no longer rewrites historical figures, provided the rate change is recorded as a new period rather than an overwrite.

Provider adapters should tolerate missing optional fields and preserve the event with an explicit unknown status.

The local journal currently stores usage metadata in `usage_event`. It intentionally excludes prompt and completion content.

`--record-event` writes this contract directly: one JSON object per line on stdin, spelled with
the keys above (`input_tokens`, `created`, `session_id`, …). It is the way in for a tool with no
collector, and it is strict where a collector is tolerant, because an adapter's author learns
only from the exit status: `provider`, `model`, `input_tokens`, `output_tokens` and one of
`event_id` or `created` are required, an unknown key is refused by name, and one unreadable event
refuses the batch before the journal is opened. A supplied `event_id` is stored as
`event:<provider>:<id>`, since identities share one namespace across sources. Of the money fields
an adapter may state two things and no others: a `cost` the tool itself recorded (`reported`), or
`"billing": "subscription"` (stored as `quota`, read back as a subscription row so it gains
`api_equivalent_cost` on the same path a native collector's does). A journaled session takes part
in escalation detection like any other.

Three nullable columns carry this -- `session_id`, `project`, `billing` -- added at the end of
`usage_event`. An older build's writer and reader name their columns, so they are unaffected and
the schema version below did not move.

The journal's schema version lives in SQLite's `PRAGMA user_version` (`JOURNAL_SCHEMA_VERSION` in
`src/collector/journal.rs`, currently `1`). Writers migrate their table under `BEGIN IMMEDIATE`, so
concurrent hooks cannot race a migration, and refuse a journal stamped with a newer version than
they know rather than writing into a shape they may not understand. Readers open it read-only and
adapt to the columns they find.

### Pruning

Nothing deletes from the journal but `--prune-journal DAYS` (`journal::prune`): it is the only
copy of what was recorded into it. Under the same write lock the writers take, and refusing a
newer journal as they do, it deletes rows whose `created` is older than `DAYS` days -- never fewer
than 31, and never later than the first of the local month, which is how far back a monthly
budget reads -- and then runs `VACUUM`. It creates nothing: no file, no table, no version stamp.
Kept on purpose, and reported:

- **The `usage_event` row with the highest `id`.** `id` is `INTEGER PRIMARY KEY` without
  `AUTOINCREMENT`, so a new row takes `MAX(id) + 1`; delete the highest and its id is reused
  below the journal collector's cursor, for a row an open dashboard would never read. `created`
  is the emitter's, so a replayed old log really can own the highest ids.
- **`routing_event` rows of a Claude Code session that has newer rows.** `--claude-code-hook`
  sums a session's rows (`attributed_requests`) to know which requests it has already
  attributed; without the old ones it would attribute them again. The session is the `event_id`
  through its second colon (`claude-code:{session}:`). Rows under any other id prune by
  `created` alone.
- **Undated rows** (`created <= 0`): their age is not known.

One table prunes without being reported: `withheld_test_run`, the hook's tally of test runs it saw
and could not record (UTC day, agent, reason, count -- see
[`routing-analytics.md`](routing-analytics.md)). A day goes once the whole of it is older than the
cutoff. It is a count nothing reads back row by row, so it has no line in the report.

Two consequences. An identity is forgotten with its row, so replaying an old log after a prune
records it again. And `--routing-json` still means all history -- all that was kept. `VACUUM`
preserves the ids of both tables and rewrites the file in place, so neither the cursor nor an
open reader is disturbed; it needs free disk space about the size of the journal, and if it fails
the rows stay deleted, the command exits `2`, and running it again retries the `VACUUM` alone.
No index on `created` exists or is added: a full scan is fine for a rare command, and an index is
a schema change every older writer would have to tolerate.

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