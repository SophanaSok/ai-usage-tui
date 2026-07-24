# Background Collectors

Background collectors run in dedicated Tokio tasks and poll data sources on a configurable interval. They write normalized usage events to the local journal database. The TUI reads from the journal for display, keeping the UI responsive regardless of collector latency.

## Collector Architecture

Each collector runs in its own Tokio task with its own interval timer. Collectors share the journal database connection pool. They write normalized `UsageEvent` records using the same schema as the TUI reads.

```text
[Collector Task] --poll interval--> [Data Source] --parse--> [UsageEvent] --> [Journal DB] <-- [TUI]
```

## OpenCode Collector

Reads assistant message records from the OpenCode SQLite database. Polls every 30 seconds by default (configurable via `[collectors.opencode.interval]`). Extracts model, tokens, cost, timestamps, and project/session metadata.

Configuration:
```toml
[collectors.opencode]
enabled = true
interval = 30
```

## Journal Collector (Ollama)

Polls the local journal database for new Ollama events recorded via `--record-ollama`. Polls every 60 seconds by default (configurable via `[collectors.journal.interval]`). Aggregates events into the normalized schema.

Configuration:
```toml
[collectors.journal]
enabled = true
interval = 60
```

## Zen Pricing Collector

Scrapes the live Zen pricing table from the OpenCode docs with retry/backoff. Caches pricing at `~/.local/share/ai-usage-tui/zen-pricing.json`. Refreshes hourly when enabled. Applies pricing snapshots to historical events for cost calculation.

Configuration:
```toml
[collectors.zen_pricing]
enabled = true
interval = 3600
```

## Configuration

All collectors are configured in the TOML config file under their respective sections. The `[collectors]` table is optional; defaults are used when omitted.

```toml
[collectors.opencode]
enabled = true
interval = 30

[collectors.journal]
enabled = true
interval = 60

[collectors.zen_pricing]
enabled = true
interval = 3600
```

## Journal Database

The journal is a SQLite database at `~/.local/share/ai-usage-tui/usage.db` by default. Override with `--journal PATH` or the `journal` config key / `AI_USAGE_JOURNAL_PATH` environment variable.

Schema:
```sql
CREATE TABLE usage_event (
    id INTEGER PRIMARY KEY,
    timestamp INTEGER NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    category TEXT NOT NULL,
    cost_status TEXT NOT NULL,
    request_count INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    reasoning_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER NOT NULL,
    cost REAL,
    latency INTEGER,
    error_status TEXT,
    project TEXT,
    session TEXT,
    source TEXT NOT NULL
);
```

## TUI Integration

The TUI reads from the journal database on each refresh cycle (default 30s, configurable via `refresh_interval` / `--refresh-interval`). Background collectors write asynchronously; the TUI never blocks on collector I/O.