# Provider Support

## OpenCode

Reads assistant usage metadata from the local OpenCode SQLite database. This is the current supported collector.

The OpenCode collector runs as a background task polling every 30 seconds (configurable via `[collectors.opencode.interval]`). It writes normalized usage events to the local journal database.

## Claude Code

Reads Anthropic usage from Claude Code's own session logs at
`~/.claude/projects/<project>/<session-id>.jsonl` (override with `--claude-dir`, the
`claude_dir` config setting, or `CLAUDE_PROJECTS_DIR`). Enabled by default; disable via
`[collectors.claude_code] enabled = false`.

Each log is tailed by byte offset, so a poll reads only newly appended lines, and a partially
written line is left unconsumed until it is complete. Events dedupe on `requestId`, falling back
to the message `uuid`. Usage carries a session id and a project name derived from the working
directory.

Claude Code reports no dollar cost, so cost is estimated from the pricing table and labelled
`estimated` — never `reported`. Model ids arrive dated and dash-versioned
(`claude-sonnet-4-5-20250929`); resolution strips the date and converts the version to the
table's dotted form.

**Privacy:** transcripts contain full prompts, completions, file contents, and anything a tool
printed, including secrets. Only the `usage` block and a few identifiers are parsed; no message
content is read or retained.

## Ollama

Ollama response metrics expose prompt and output token counts, but Ollama does not provide a complete historical usage database. `--record-ollama` provides an opt-in local journal for requests made after tracking is enabled.

The journal collector polls every 60 seconds (configurable via `[collectors.journal.interval]`) and aggregates Ollama events into the same normalized shape.

## Ollama Cloud

Token counts can be observed when returned by the client response. Account quota and GPU-based Cloud billing are not currently exposed through the supported API, so the tool must not invent a dollar cost. Cloud-routed models are displayed as `CLOUD`, never as local usage.

## OpenCode Zen

Zen usage can be read from OpenCode history. `--refresh-pricing` scrapes the live Zen pricing table from the docs page with retry/backoff and caches it at `~/.local/share/ai-usage-tui/zen-pricing.toml`. The background `ZenPricingCollector` refreshes hourly when enabled via `[collectors.zen_pricing.enabled]`. Pricing snapshots are applied to historical events for cost calculation.

## Background Collectors

- OpenCode collector: polls OpenCode DB every 30s (configurable), resuming from a
  `time_created` high-water mark rather than re-reading the whole table
- Claude Code collector: polls session logs every 30s (configurable), tailing by byte offset
- Journal collector: polls journal DB every 60s (configurable)
- Zen Pricing collector: refreshes hourly when enabled (opt-in)

All collectors write to the journal database; the TUI reads from the journal on its own refresh interval, keeping the UI responsive.

## Budgets

Budget limits can be configured per provider, model, or globally with daily or monthly periods. Alerts appear in the TUI banner. Actionable alerts are POSTed to `--webhook URL` (or `budgets.webhook`) when one is configured; the dispatch runs on a background thread so it never blocks rendering, and repeat alerts at the same level are suppressed for an hour. Periods use local calendar boundaries, matching the dashboard's `TODAY` range. Check budgets from CLI with `ai-usage-tui --check-budgets` (exits 1 if thresholds exceeded).