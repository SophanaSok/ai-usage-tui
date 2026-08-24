# Provider Support

## OpenCode

Reads assistant usage metadata from the local OpenCode SQLite database.

The OpenCode collector runs as a background task polling every 30 seconds (configurable via the `interval` key under `[collectors.opencode]`). It merges what it finds into the in-memory snapshot the dashboard reads; it never writes to the journal.

## Claude Code

Reads Anthropic usage from Claude Code's own session logs at
`~/.claude/projects/<project>/<session-id>.jsonl` (override with `--claude-dir`, the
`claude_dir` config setting, or `CLAUDE_PROJECTS_DIR`). Enabled by default; disable via
`[collectors.claude_code] enabled = false`.

Each log is tailed by byte offset, so a poll reads only newly appended lines, and a partially
written line is left unconsumed until it is complete. Events dedupe on `requestId`, falling back
to the message `uuid`. Usage carries a session id and a project name derived from the working
directory.

Claude Code reports no dollar cost, and its transcripts are identical on an API key and on a
Pro/Max subscription. The collector decides once per source which it is (`src/collector/billing.rs`:
an explicit `[collectors.claude_code] billing` or `--claude-billing`; else an Anthropic API-key
variable in the environment means per-token; else an `oauthAccount` block in `~/.claude.json` means
subscription; else per-token with a "billing unknown" hint on the source line) and stamps every row.
Per-token rows are priced from the pricing table and labelled `estimated` — never `reported`.
Subscription rows carry `cost_status = quota` and `cost = null` for the same reason Ollama Cloud
does (below): the work is billed, but not per token at any rate the tool can know. The list-rate
figure survives as `api_equivalent_cost`, a counterfactual that is never summed into cost or
budgets. Model ids arrive dated and dash-versioned (`claude-sonnet-4-5-20250929`); resolution strips
the date and converts the version to the table's dotted form.

**Privacy:** transcripts contain full prompts, completions, file contents, and anything a tool
printed, including secrets. Only the `usage` block and a few identifiers are parsed; no message
content is read or retained. Of `~/.claude.json`, only the presence of `oauthAccount` and its two
rate-limit-tier keys are read; the email, name, organisation and prompt history in the same file
are dropped with the parsed document. `.credentials.json` and `settings.json` are never read.

## Codex CLI

Reads OpenAI usage from Codex CLI's session logs ("rollouts") at
`~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<thread-id>.jsonl` and under
`~/.codex/archived_sessions/` (override the home with `--codex-dir`, the `codex_dir` config
setting, or `CODEX_HOME`). Enabled by default; disable via `[collectors.codex] enabled = false`.
Files the CLI has compressed to `.jsonl.zst` are not read.

Each rollout is tailed by byte offset, with a partial trailing line left for the next poll. Only
three line kinds are read: `session_meta` (thread id, `cwd`), `turn_context` (the model in force
from there on), and `event_msg` / `token_count`, whose `info.last_token_usage` is one model API
call. Following the CLI's own arithmetic, `cached_input_tokens` is split out of `input_tokens` as
cache-read and `reasoning_output_tokens` out of `output_tokens` as reasoning; cache writes stay
inside input because OpenAI bills them at the input rate. Re-emissions whose running total did not
advance (rate-limit refreshes, resumes) and post-compaction estimates are skipped. A forked thread
copies its ancestor's history into a new file, so identity is content-based
(`codex:<timestamp>:<call tokens>:<running total>`) and the copy dedupes against the original.
Usage carries the thread id as session and the working directory as project.

Codex reports no dollar cost, and rollouts are identical on an API key and on a ChatGPT plan. The
decision is made once per source by the same `src/collector/billing.rs`: an explicit
`[collectors.codex] billing` or `--codex-billing`; else `OPENAI_API_KEY` or `CODEX_API_KEY` in the
environment means per-token; else per-token with a "billing unknown" hint on the source line. No
Codex config document is read, so `config_json` is rejected under `[collectors.codex]`. The source
line also appends `· N token events disagree with running totals` when the CLI's cumulative
counter did not move by a call's own figure. Rows are `openai` / `PAID`, priced `estimated` from
the bundled table — `gpt-5`, `gpt-5.1`, `gpt-5.2`, `gpt-5.3-codex`, `gpt-5.4`, `gpt-5.5`, and
`gpt-5.6` families — and a model absent from it stays `unavailable`.

**Privacy:** rollouts hold prompts, tool-call arguments and outputs, and reasoning summaries; none
of it is parsed or retained. `~/.codex/auth.json` is a credential file and is never opened.

## Ollama

Ollama response metrics expose prompt and output token counts, but Ollama does not provide a complete historical usage database. `--record-ollama` provides an opt-in local journal for requests made after tracking is enabled.

The journal collector polls every 60 seconds (configurable via the `interval` key under `[collectors.journal]`) and aggregates Ollama events into the same normalized shape.

## Ollama Cloud

Token counts can be observed when returned by the client response. Account quota and GPU-based Cloud billing are not currently exposed through the supported API, so the tool must not invent a dollar cost. Cloud-routed models are displayed as `CLOUD`, never as local usage.

These rows carry `cost_status = quota`, not `unavailable`. The distinction matters: `unavailable` means "this should carry a price and does not", which is a gap worth reporting, while `quota` means "this is billed, but not per token at any rate we can know". They were the same value until 2026-08-19, and every panel reporting pricing coverage consequently read this deliberate refusal as a failure — the header showed 71% priced on a dataset where 100% of priceable work was priced.

## OpenCode Zen

Zen usage can be read from OpenCode history. `--refresh-pricing` scrapes the live Zen pricing table from the docs page with retry/backoff and caches it at `~/.local/share/ai-usage-tui/zen-pricing.toml`. The background `ZenPricingCollector` refreshes hourly when enabled via the `enabled` key under `[collectors.zen_pricing]`. Pricing snapshots are applied to historical events for cost calculation.

## Background Collectors

- OpenCode collector: polls OpenCode DB every 30s (configurable), resuming from a
  `time_created` high-water mark rather than re-reading the whole table
- Claude Code collector: polls session logs every 30s (configurable), tailing by byte offset
- Codex collector: polls rollouts every 30s (configurable), tailing by byte offset with a
  per-file cursor
- Journal collector: polls journal DB every 60s (configurable)
- Zen Pricing collector: refreshes hourly when enabled (opt-in)

Collectors never write to the journal database. Each merges into the shared in-memory `CollectorState`, and the dashboard calls `snapshot()` on its own refresh interval (default 30s), keeping the UI responsive. The journal is a source — written by `--record-ollama` / `--record-routing`, read by the journal collector — not a sink.

## Budgets

Budget limits can be configured per provider, model, or globally with daily or monthly periods. Alerts appear in the TUI banner. Actionable alerts are POSTed to `--webhook URL` (or `budgets.webhook`) when one is configured; the dispatch runs on a background thread so it never blocks rendering, and repeat alerts at the same level are suppressed for an hour. That suppression is in-memory (per process), so a cron-driven `--check-budgets` re-POSTs on every run. Periods use local calendar boundaries: daily matches the dashboard's `TODAY` range; monthly is the calendar month, not the dashboard's trailing 30-day range. Check budgets from CLI with `ai-usage-tui --check-budgets` (exits 1 if thresholds exceeded).