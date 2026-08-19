# Architecture

The application is designed around a provider-neutral usage pipeline:

```text
background collectors -> shared state (Arc<RwLock>) -> TUI / export
```

Collectors read assistant message records from the OpenCode SQLite database, Claude Code session logs (`~/.claude/projects/**/*.jsonl`), and the local Ollama/routing journal. Future collectors follow the same normalized shape.

## Privacy Boundary

Collectors may read usage metadata, model identifiers, timestamps, and calculated costs. They must not persist or transmit prompts, completions, API keys, or credentials.

This constraint is load-bearing for the Claude Code collector in particular: session transcripts contain full prompts, completions, file contents, and anything a tool printed — including secrets read from a `.env`. The collector parses only the `usage` block and a handful of identifiers. A test plants a fake credential in a transcript line and fails if it appears anywhere in the resulting record.

## Cost Provenance

Every cost value must be labeled as provider-reported, calculated, estimated, free, local, or unavailable. A missing cost must never be rendered as a paid zero.

This extends to the pricing table: a rate that is absent is distinct from a rate published as `0.0`. Token buckets with usage but no published rate yield `unavailable`, not a cheaper non-zero total. The refreshed pricing cache is applied as an *overlay* on the bundled table, so a lossy or unparseable refresh can never delete pricing that shipped in the binary.

## Background Collectors

Background collectors run in dedicated `std::thread`-based polling loops and write normalized usage events to the journal database. The TUI reads from the journal for display, keeping the UI responsive regardless of collector latency. Configuration lives in the `[collectors.opencode]`, `[collectors.claude_code]`, and `[collectors.journal]` sections of the config file.

Ingestion is incremental: the OpenCode collector resumes from a `time_created` high-water mark and the Claude Code collector tails each session log by byte offset, so neither re-reads history on every poll.

See [`background-collectors.md`](background-collectors.md) for the `Collector` trait, `CollectorHandle`, and thread model.

## Budget Engine

`BudgetEngine` checks spend against configured limits on each aggregation cycle. `AlertDispatcher` posts webhook alerts when `--webhook` or `budgets.webhook` is configured, from both `--check-budgets` and the TUI. Dispatch runs on a background thread so the blocking HTTP call never touches the render path, and repeat alerts at the same level are suppressed for an hour.

Budget periods use local calendar boundaries, shared with the dashboard's `TODAY` range so the two can never disagree.

See [`data-model.md`](data-model.md#budget-configuration) for budget schema.

## Routing Engine

`RoutingEngine` aggregates routing events by agent, model, and provider. Aggregations power the TUI routing view (`t` key) and `--routing-json` / `--routing-csv` exports.

See [`routing-analytics.md`](routing-analytics.md) for event schema and aggregation logic.