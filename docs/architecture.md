# Architecture

The application is designed around a provider-neutral usage pipeline, with one registry of
sources (`src/collector/registry.rs`) and two ways of draining it:

```text
                         /-> background collectors -> shared state (Arc<RwLock>) -> TUI
collector::registry::SOURCES
                         \-> one-shot read (load_usage) -> JSON / CSV / budgets / --doctor
```

Both arms iterate the same `SOURCES` list, so a source is either in both or in neither. This
used to be two hand-maintained wirings: `main::build_collectors` for the dashboard and a
five-call `load_usage` for everything else. A provider added to one and not the other appeared
in the dashboard and was silently missing from every export.

Collectors read assistant message records from the OpenCode SQLite database, Claude Code session logs (`~/.claude/projects/**/*.jsonl`), Codex CLI rollouts (`~/.codex/{sessions,archived_sessions}/**/*.jsonl`), GitHub Copilot's CLI store (`~/.copilot/`), and the local Ollama/routing journal. Future collectors follow the same normalized shape.

## Privacy Boundary

Collectors may read usage metadata, model identifiers, timestamps, and calculated costs. They must not persist or transmit prompts, completions, API keys, or credentials.

This constraint is load-bearing for the Claude Code collector in particular: session transcripts contain full prompts, completions, file contents, and anything a tool printed — including secrets read from a `.env`. The collector parses only the `usage` block and a handful of identifiers. A test plants a fake credential in a transcript line and fails if it appears anywhere in the resulting record.

The Omarchy reader (`src/omarchy`) sits inside the same boundary. Omarchy's agents panel fetches each subscription's rate-limit windows from the vendor with the agent's saved sign-in; this tool reads only the finished display records it writes (`${XDG_STATE_HOME:-~/.local/state}/omarchy/agents/usage/*.json`), six fields each — never the OAuth token, never over HTTP, never Omarchy's probe cache, and it writes nothing there.

The one write into Omarchy's directory is `--omarchy-record` (`src/omarchy/record.rs`, driven by `write_omarchy_records` in `src/main.rs`): explicit and opt-in, it writes `<id>.json` — token counts, model ids, request and session counts, budget figures; never content, never a path — atomically (temporary then rename, mode 0600) and exits. Ids are limited to `opencode` and `ollama` so Omarchy's own `claude`/`codex`/`fireworks` files cannot be overwritten. The TUI, `--json` and `--csv` never write there; a test asserts it.

## Cost Provenance

Every cost value must be labeled as provider-reported, calculated, estimated, free, local, or unavailable. A missing cost must never be rendered as a paid zero.

This extends to the pricing table: a rate that is absent is distinct from a rate published as `0.0`. Token buckets with usage but no published rate yield `unavailable`, not a cheaper non-zero total. The refreshed pricing cache is applied as an *overlay* on the bundled table, so a lossy or unparseable refresh can never delete pricing that shipped in the binary.

## Background Collectors

Background collectors run in dedicated `std::thread`-based polling loops and merge normalized usage events into the shared in-memory `CollectorState`. They never write to the journal database — the journal is a source (written by `--record-ollama` / `--record-routing`, read by the journal collector), not a sink. The TUI calls `snapshot()` on its own refresh interval (default 30s), keeping the UI responsive regardless of collector latency. Configuration lives in the `[collectors.opencode]`, `[collectors.claude_code]`, `[collectors.codex]`, `[collectors.copilot]`, `[collectors.gemini]`, `[collectors.journal]`, and `[collectors.zen_pricing]` sections of the config file.

Omarchy's subscription limits are not a collector: `App::refresh()` reads the records directly beside the routing table, and `--json` reads them once for its `limits` array. `[omarchy]` (`dir`, `limits`, `records`, `balance`, `balance_budget`) and `--omarchy-dir` configure it; the record's `tierLabel` is also the last signal in the billing decision.

Ingestion is incremental: the OpenCode and Copilot collectors resume from a high-water mark (`time_created` and `created_at` respectively) and the Claude Code and Codex collectors tail each session log by byte offset, so none re-reads history on every poll.

See [`background-collectors.md`](background-collectors.md) for the `Collector` trait, `CollectorHandle`, and thread model.

## Budget Engine

`BudgetEngine` checks spend against configured limits on each aggregation cycle. `AlertDispatcher` posts webhook alerts when `--webhook` or `budgets.webhook` is configured, from both `--check-budgets` and the TUI. Dispatch runs on a background thread so the blocking HTTP call never touches the render path, and repeat alerts at the same level are suppressed for an hour (in-memory, per process).

Budget periods use local calendar boundaries. The daily period shares the dashboard's `TODAY` boundary so the two can never disagree; the monthly period is the calendar month, deliberately not the dashboard's trailing 30-day range.

See [`data-model.md`](data-model.md#budget-configuration) for budget schema.

## Routing Engine

`routing::aggregate` (`src/routing.rs`) aggregates routing events by agent, model, and provider. Aggregations power the TUI routing view (`t` key) and `--routing-json` / `--routing-csv` exports.

`harness::claude_code` (`src/harness/`) is the one shipped emitter: `--claude-code-hook` turns a Claude Code `PostToolUse`/`PostToolUseFailure` payload into a routing event when it observed a test run, with `harness::shell` deciding whether the command line's exit status was the runner's. It reads the transcript the payload names with the Claude Code collector's own `parse_line`, prices the attempt with the same billing decision and `PricingEngine`, and writes through `journal::record_routing_event` — the same path `--record-routing` takes.

See [`routing-analytics.md`](routing-analytics.md) for event schema and aggregation logic.