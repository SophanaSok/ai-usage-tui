# Background Collectors

Collectors poll data sources on a fixed interval and merge what they find into an in-memory
snapshot the dashboard reads. Each runs on its own OS thread — there is no async runtime — and is
supervised: a collector that fails is reported, and one that panics is restarted.

## Architecture

```text
[collector thread] --poll--> [source] --parse--> [Usage] --\
[collector thread] --poll--> [source] --parse--> [Usage] ---> [CollectorState] <-- snapshot() -- [TUI]
[collector thread] --poll--> [source] --parse--> [Usage] --/        |
                                                                    +-- health() -> status line
```

`CollectorState` holds the merged `Vec<Usage>`, a `HashSet<UsageKey>` membership index that keeps
merges linear rather than quadratic, the set of source labels, and per-collector health. Pricing is
loaded once at spawn and applied after each merge, not re-parsed from disk on every poll.

Collectors do **not** write to the journal database. The journal is a *source* — written by
`--record-ollama` and `--record-routing`, read by the journal collector — not a sink. Only the
in-memory state is shared between collectors and the UI.

### Deduplication

Merges key on `UsageKey`, which prefers a stable `event_id` (OpenCode's message id, Claude Code's
`requestId`, the journal's `event_id` column) and falls back to the usage shape *plus* its
timestamp. Token counts alone are not an identity: agent loops routinely produce distinct requests
with byte-identical counts, and keying on shape alone silently collapsed them.

Because dedup is by identity, a collector may safely re-read rows it has already seen — which is
what makes the incremental cursors below safe to make inclusive.

## Health and failure

Every collector carries a liveness state, surfaced in the header and written to the log:

| State | Meaning |
| --- | --- |
| `starting` | Spawned; no poll has completed yet |
| `ok` | Last poll succeeded |
| `failing` | Still polling, but the last attempt returned an error |
| `restarting` | Panicked; waiting out a backoff before the next attempt |
| `dead` | Panicked more than five times; this source will never update again |

A collector that is nominally `ok` but has not completed a poll in three intervals is reported
**stale** — a hung poll returns no error, so an error-only status line cannot see it.

When any collector is degraded, the header status turns red and names the collector and its error.
This is deliberate: a monitoring tool that quietly stops collecting looks exactly like one with
nothing to report.

**Restarts.** A panicking collector used to retire permanently, leaving the UI showing its last
numbers as though they were current. Panics now restart with exponential backoff (2s, doubling,
capped at 60s) and give up only after five attempts.

**Shutdown.** `shutdown()` signals a condvar, so a sleeping collector wakes immediately rather than
after up to a second of poll-check granularity. `Drop` joins every thread, so no collector is still
mid-poll — holding a SQLite handle — after the handle is gone.

## Logging

Set `AI_USAGE_LOG` to capture collector errors, panics and restarts:

```sh
AI_USAGE_LOG=1 ai-usage-tui                       # default path under the data directory
AI_USAGE_LOG=/tmp/ai-usage.log ai-usage-tui       # explicit path
```

Off unless set. The dashboard holds the alternate screen, so anything written to stderr is
invisible; before this existed a panicking collector left no trace anywhere.

The log records timestamps, levels, collector names and error text. It never contains prompts,
completions, or credentials — the same boundary the collectors themselves observe.

## OpenCode collector

Reads assistant messages from the OpenCode SQLite database, opened `SQLITE_OPEN_READ_ONLY`. Polls
every 30 seconds by default. Extracts provider, model, token buckets, reported cost, and the
message id used for deduplication.

Resumes from a `time_created` high-water mark rather than re-reading the whole message table each
poll. The cursor is inclusive by design; `event_id` deduplication absorbs the boundary overlap.

```toml
[collectors.opencode]
enabled = true
interval = 30
```

## Claude Code collector

Reads `~/.claude/projects/**/*.jsonl` — Claude Code's own session logs, and on most machines the
largest source of Anthropic usage. Polls every 30 seconds by default.

Tails each file from a remembered byte offset, so history is parsed once. A file that has shrunk is
treated as rotated and re-read from the start; a trailing partial line is left for the next poll
rather than parsed half-written.

**Only the `usage` block of each line is parsed.** Session transcripts contain source code, command
output, and secrets; no message content is read or retained. A test plants a fake
`AWS_SECRET_ACCESS_KEY` in a transcript and fails if it reaches a usage record.

Claude Code reports no cost, so these rows arrive `Unavailable` and are priced by the pricing
engine, or left explicitly unpriced.

Before each poll the collector decides, on its own thread, whether the account bills per token or
against a subscription (`src/collector/billing.rs`: `billing` override, then Anthropic API-key
environment variables, then `oauthAccount` in `~/.claude.json`, else per-token and "billing
unknown"), and stamps every row it returns. The decision is sticky: once evidence is found it is
kept for the life of the process, so a poll that catches `~/.claude.json` half-written — Claude
Code rewrites it constantly — cannot flip new rows to a different status from the rows already
merged. An unknown decision is re-examined on the next poll. Subscription rows are turned into
`quota` by the pricing engine, with the list-rate figure kept as `api_equivalent_cost`.

```toml
[collectors.claude_code]
enabled = true
interval = 30
billing = "auto"                        # auto | subscription | api
# config_json = "/home/user/.claude.json"
```

Override the root with `--claude-dir PATH`, the `claude_dir` config key, or `CLAUDE_PROJECTS_DIR`.
`config_json` names Claude Code's config document when it is not at `~/.claude.json`; without it
the path follows `CLAUDE_CONFIG_DIR`, or is derived from an overridden root as
`<root>/../../.claude.json`. `billing` and `config_json` are rejected at parse time under any
other collector table.

## Journal collector

Reads the local journal database — Ollama events recorded via `--record-ollama` and routing events
via `--record-routing`. Polls every 60 seconds by default.

```toml
[collectors.journal]
enabled = true
interval = 60
```

## Zen pricing collector

Scrapes the Zen pricing table from the OpenCode docs page and writes a cache under the data
directory. Disabled by default; hourly when enabled.

The cache is applied as an **overlay** on the pricing table compiled into the binary, never as a
replacement, so a partial or malformed refresh cannot delete pricing that shipped with the release.

```toml
[collectors.zen_pricing]
enabled = false
interval = 3600
```

## Configuration

The `[collectors]` table is optional; every collector has a default. A missing section means
defaults, but a *malformed* config file is an error rather than a silent fallback to defaults.

```toml
[collectors.opencode]
enabled = true
interval = 30

[collectors.claude_code]
enabled = true
interval = 30
billing = "auto"                        # auto | subscription | api
# config_json = "/home/user/.claude.json"

[collectors.journal]
enabled = true
interval = 60

[collectors.zen_pricing]
enabled = false
interval = 3600
```

## Journal database

SQLite at `~/.local/share/ai-usage-tui/usage.db` by default. Override with `--journal PATH`, the
`journal` config key, or `AI_USAGE_JOURNAL_PATH`.

```sql
CREATE TABLE usage_event (
    id INTEGER PRIMARY KEY,
    event_id TEXT,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    category TEXT NOT NULL,
    cost_status TEXT NOT NULL,
    requests INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    reasoning_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER NOT NULL,
    cost REAL,
    created INTEGER NOT NULL
);
```

`event_id` carries a `UNIQUE` index and is what makes re-recording an event idempotent. There is a
parallel `routing_event` table with the same identity treatment.

## TUI integration

The dashboard calls `snapshot()` on its refresh interval (default 30s, `--refresh-interval`). That
clones the merged vector under a read lock and returns; it never waits on collector I/O, opens a
database, or reads the clock on the render path.
