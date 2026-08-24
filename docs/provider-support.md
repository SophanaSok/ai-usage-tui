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

## Omarchy (subscription limits)

Not a provider: Omarchy 4's Agents panel writes one JSON record per agent under
`${XDG_STATE_HOME:-~/.local/state}/omarchy/agents/usage/`, and the `l` panel reads
them (`src/omarchy/mod.rs`). `claude.json` and `codex.json` carry rate-limit windows
(`limits`: label, percent 0..1, `resetsAt`) and a plan label (`tierLabel`); `fireworks.json`
carries a balance, not limits, and is skipped. Six fields per record are read; credentials,
Omarchy's probe cache, `authHelpText` and token tallies are not, and no request is made.
The `tierLabel` is the last billing signal for the Claude Code and Codex collectors, after
the explicit setting, the API-key environment variables, and `~/.claude.json`. Override the
directory with `--omarchy-dir` or `[omarchy] dir`; `[omarchy] limits = false` disables it.

The other direction is `--omarchy-record` (`src/omarchy/record.rs`), which writes this tool's
usage as `<id>.json` into the same directory so the panel gains a tab per id in `[omarchy]
records` (default `["opencode"]`). Which sources feed which id:

| Record id | Rows | Why |
| --- | --- | --- |
| `opencode` | every OpenCode row, all providers, priced | Omarchy has no OpenCode collector |
| `ollama` | journal rows with provider `ollama` | Omarchy has no local-model tab |
| `claude`, `codex`, `fireworks` | refused | Omarchy's own files; a record so named would overwrite them |

Claude Code and Codex rows are never written: Omarchy's `claude` and `codex` tabs already cover
those logs. Those collectors also fold OpenCode's anthropic/openai rows into their tabs, so such a
row can show in both the `opencode` tab and Omarchy's — display overlap, not double counting,
since the panel never sums tabs. Every configured budget becomes a `limits[]` meter with the
spend `--check-budgets` reports (all sources), and `[omarchy] balance = true` draws one
(`balance_budget`, default `global/monthly`) as the panel's prepaid ledger. Nothing is written
unless the flag is given.

## Billing detection

Claude Code and Codex both write identical transcripts whether they run on an API key or on a
subscription, and nothing on a usage line says which. Priced at list rates, a plan's traffic reads
as real spend and trips budgets on money that was never charged — so the collector decides how the
account pays *before* pricing runs.

### Claude Code

Claude Code writes identical transcripts whether it runs on an API
key or a Pro/Max plan, and nothing on a usage line says which. Priced at list
rates, a subscription's traffic reads as real spend and trips budgets on money
that was never charged, so the collector decides how the account pays before
pricing runs. In order: an explicit `billing` setting; then, if any of
`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_USE_BEDROCK`, or
`CLAUDE_CODE_USE_VERTEX` is set in the environment, per-token; then, if Claude
Code's own `~/.claude.json` has an `oauthAccount` block, subscription, with the
plan named from its rate-limit tier (`default_claude_max_20x` → "Max 20x");
then the plan label in Omarchy's record for the agent, if Omarchy's agents
panel is present (see [Subscription limits](omarchy.md#subscription-limits)),
subscription; otherwise per-token, with a visible "billing unknown" hint. Per-token rows are
priced `estimated` as before. Subscription rows carry `cost_status = quota`,
`cost = null`, and the list-rate figure as `api_equivalent_cost`.

### Codex CLI

As with Claude Code, a rollout looks the same on an API key and on
a ChatGPT plan. In order: an explicit `billing` setting; then, if
`OPENAI_API_KEY` or `CODEX_API_KEY` is set in the environment, per-token;
then the plan label in Omarchy's record for the agent, if Omarchy's agents
panel is present, subscription; otherwise per-token, with a visible "billing
unknown" hint. No Codex config
document is read — `~/.codex/auth.json` is a credential file and is never
opened — so `config_json` is rejected under `[collectors.codex]`. Force the
answer with `[collectors.codex] billing = "subscription"` or `"api"` (default
`"auto"`), or `--codex-billing MODE`. The decision is printed on the source
line: `Codex: ~/.codex (N sessions) · api billing` or `· billing unknown — set
[collectors.codex] billing`. The same line appends `· N token events disagree
with running totals` when the CLI's cumulative counter did not advance by a
call's own figure, so a change in what the CLI emits is visible rather than a
silent under-count.

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