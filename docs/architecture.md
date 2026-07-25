# Architecture

The application is designed around a provider-neutral usage pipeline:

```text
background collectors -> shared state (Arc<RwLock>) -> TUI / export
```

The current collectors read assistant message records from the OpenCode SQLite database and journal Ollama responses. Future collectors will follow the same normalized shape.

## Privacy Boundary

Collectors may read usage metadata, model identifiers, timestamps, and calculated costs. They must not persist or transmit prompts, completions, API keys, or credentials.

## Cost Provenance

Every cost value must be labeled as provider-reported, calculated, estimated, free, local, or unavailable. A missing cost must never be rendered as a paid zero.

## Background Collectors

Background collectors run in dedicated `std::thread`-based polling loops and write normalized usage events to the journal database. The TUI reads from the journal for display, keeping the UI responsive regardless of collector latency. Configuration lives in `[collectors.opencode]` and `[collectors.journal]` sections of the config file.

See [`background-collectors.md`](background-collectors.md) for the `Collector` trait, `CollectorHandle`, and thread model.

## Budget Engine

`BudgetEngine` checks spend against configured limits on each aggregation cycle. `AlertDispatcher` can evaluate thresholds and post webhook alerts, but the current TUI and CLI do not wire webhook dispatch yet.

See [`data-model.md`](data-model.md#budget-configuration) for budget schema.

## Routing Engine

`RoutingEngine` aggregates routing events by agent, model, and provider. Aggregations power the TUI routing view (`t` key) and `--routing-json` / `--routing-csv` exports.

See [`routing-analytics.md`](routing-analytics.md) for event schema and aggregation logic.