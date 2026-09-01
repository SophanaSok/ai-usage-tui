# Provider Support

## OpenCode

Reads assistant usage metadata from the local OpenCode SQLite database. The database is opened
read-only and only assistant-message usage metadata is read. The default path follows
`XDG_DATA_HOME` when set, otherwise `~/.local/share/opencode/opencode.db`; select another with
`--db PATH`, the `db` config setting, or `OPENCODE_DB_PATH`.

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
setting, or `CODEX_HOME`); `sessions/` and `archived_sessions/` are scanned recursively beneath
it. Enabled by default; disable via `[collectors.codex] enabled = false`. Files the CLI has
compressed to `.jsonl.zst` are not read.

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
the bundled table — the `gpt-5`, `gpt-5.1`, `gpt-5.2`, `gpt-5.3-codex`, `gpt-5.4`, `gpt-5.5`, and
`gpt-5.6` families, including their `-codex`, `-mini`, `-nano`, and `-pro` variants where
published — and a model absent from it stays `unavailable`; no rate is invented.

**Privacy:** rollouts hold prompts, tool-call arguments and outputs, and reasoning summaries; none
of it is parsed or retained. `~/.codex/auth.json` is a credential file and is never opened.

## GitHub Copilot

| | |
| --- | --- |
| Source | The CLI's SQLite store; the legacy session log as a fallback |
| Default path | `~/.copilot/` (override: `--copilot-dir`, `copilot_dir`, or `COPILOT_HOME`) |
| Store | `session-store.db`, else `session.db`, else `data.db` — **selected by schema, not name** |
| Table | `assistant_usage_events`, one row per model request |
| Fallback | `session-state/<session-id>/events.jsonl`, `session.shutdown` records only |
| Identity | `copilot:<session_id>:<turn_index>`; legacy rows key on the cumulative total |
| Billing signal | None available — a seat is the only way to use Copilot, so `auto` resolves to subscription |
| Project attribution | `cwd`, falling back to `repository` — read from the `sessions` table, which is where the shipping build keeps them |

Copilot has moved its usage between three shapes over its life, and the store's filename has
moved with them. The collector tries each candidate and picks whichever one actually has an
`assistant_usage_events` table, so a build that renames the file again reads as absent rather
than as zero. Where no such table exists, the legacy log's shutdown aggregates are read instead.

**Token buckets.** `input_tokens` is inclusive of `cache_read_tokens` and `cache_write_tokens`,
and `output_tokens` is inclusive of `reasoning_tokens`. Both are subtracted back out so the five
buckets stay disjoint — this is the Codex convention, and a unit test asserts `total_tokens()` is
unchanged by the split. Leaving them folded in would count every cached token twice, and price it
twice.

**Identity is the row, not the turn.** `assistant_usage_events.id` is the only column that is
one-per-request. A turn fans out: a tool-using prompt writes a `user` row and one `agent` row per
follow-up call, all sharing a `turn_index` — which the shipping build writes as `0` on every row
anyway. Keying on the turn reported one request per turn and threw the rest of the turn's tokens
away. On a build with no `id` the key falls back to the session, the turn, the timestamp and the
call's own counts, which is still content-derived — a store copied between machines dedups
against the original rather than doubling it.

**Resuming across a text timestamp.** `created_at` is declared `TEXT` and written as RFC 3339.
SQLite orders every integer before every string, so a `created_at >= <integer>` bound is always
true against that column and filters nothing. The cursor keeps the store's own spelling of its
high-water mark and compares text with text; fixed-width RFC 3339 sorts lexicographically in
chronological order, and the comparison is `>=`, so a mismatched spelling can only re-read rows
that dedup then drops.

**Legacy aggregates are cumulative.** A `session.shutdown` reports the session's running totals,
not the turn's, and a resumed session writes several. The collector remembers the last snapshot
per session and model and emits only the difference; emitting each snapshot whole would report
every earlier turn again on each resume.

**Billing.** A Copilot seat bills premium requests against a plan, not tokens — confirmed
against a real account, where a `session.shutdown` reporting `"totalPremiumRequests": 1` carries
`modelMetrics.<model>.requests = {"count": 1, "cost": 1}`, so that `cost` is a request count and
not a dollar figure. The money-shaped column is `total_nano_aiu`, in nano AI Units, which is a
Copilot billing unit rather than an amount charged. Neither is reported as `cost`. There is no
API-key mode and therefore no environment variable whose presence would mean per-token billing,
so `api_env_vars("copilot")` is deliberately an explicit empty list and an unevidenced decision
resolves to `subscription` rather than falling through to per-token. Rows carry
`cost_status = quota`, `cost = null`, and the list rate as `api_equivalent_cost`. An explicit
`[collectors.copilot] billing = "api"` is still honoured.

**What is deliberately not read.** The store's `turns` table holds `user_message` and
`assistant_response`; the legacy log's `user.message` and `assistant.message` records hold the
same plus tool-call arguments. None of it is parsed, under the same planted-credential test the
Claude Code and Codex collectors carry.

**What is deliberately not reconstructed.** Two Copilot shapes carry no usable measurement, and
neither is estimated:

- Per-`assistant.message` records in the legacy log expose an output count but record input as
  `0`. A row asserting zero input tokens is the same false statement as one asserting zero cost.
- VS Code's Copilot transcripts carry no token counts at all.

The common fallback for both — in every other tool that reports Cursor or Copilot spend — is to
divide a character count by four and price the result. A token count inferred from message length
is not a measurement, and once priced it is indistinguishable in a total from a figure a provider
actually reported. Copilot usage this tool cannot measure is reported as absent. See also
[Why there is no Cursor collector](../README.md#why-there-is-no-cursor-collector).

**Validated against a real account.** Copilot CLI 1.0.82, driven non-interactively, produced the
store this collector is now pinned to: `tests/fixtures/copilot_home/session-store.db` is that
capture with identifiers and paths redacted. The schema probe held — nothing produced a wrong
number — but three assumptions did not, and the roadmap records them: the turn-keyed identity
above, `cwd`/`repository` living on `sessions`, and the integer-bound cursor. Still unchecked, and
wanting a different account rather than another run: a store carried through several CLI upgrades,
and whether a paid plan ever writes a `request_multiplier` other than `1.0`.

## Gemini CLI

| | |
| --- | --- |
| Source | OpenTelemetry log file, **opt-in** |
| Default path | `~/.gemini/telemetry.json` (override: `--gemini-dir`, or Gemini's own `GEMINI_TELEMETRY_OUTFILE`) |
| Enabled by | `{"telemetry":{"enabled":true,"target":"local","outfile":"..."}}` in `~/.gemini/settings.json` |
| Format | Concatenated **pretty-printed** JSON objects — not JSONL |
| Record | `attributes["event.name"] == "gemini_cli.api_response"` |
| Identity | `prompt_id` + `event.timestamp` + `total_token_count` |
| Billing signal | `GEMINI_API_KEY` / `GOOGLE_API_KEY` / `GOOGLE_GENAI_USE_VERTEXAI`, else Omarchy's record |
| Project attribution | None — Gemini's telemetry records no working directory |

Gemini CLI persists no usage without that setting: session totals live in UI state and are lost
on exit, and saved chats under `<project temp>/chats` hold conversation history and an auth type
with no token counts. `--doctor` reports the source as absent and prints the setting to add; this
tool never edits Gemini's settings itself.

**Token buckets.** Google reports `cachedContentTokenCount` as a *subset* of `promptTokenCount`,
unlike Anthropic which reports cache reads alongside input. The collector subtracts it so the
buckets stay disjoint and a cached token is not billed as fresh input as well. `toolUsePromptTokenCount`
is likewise already inside the prompt count and is deliberately not added again.
`thoughtsTokenCount` *is* separate from `candidatesTokenCount` and maps to the reasoning bucket.

**Identity.** One `prompt_id` covers a whole tool-use loop, so several `api_response` records
share it. Keying on it alone would deduplicate real requests away and under-report spend, so the
timestamp and total are part of the key.

**Validated against real output.** `tests/fixtures/gemini_telemetry.json` is a redacted capture
written by Gemini CLI itself. No account is needed to reproduce one: point
`GOOGLE_GEMINI_BASE_URL` at a local HTTP server that returns a `generateContent` body with a
`usageMetadata` block, set `security.auth.selectedType` to `gemini-api-key` in
`~/.gemini/settings.json`, and run `gemini --skip-trust -p ...`. The real CLI, SDK and exporter
produce the file; nothing leaves the machine and nothing is billed.

**Records that are not usage.** The file also contains `api_request`, `user_prompt`,
`model_routing`, `gen_ai.client.inference.operation.details` and OTLP **metric** records. The
metric records have no `attributes` key at all, so anything indexing `["attributes"]` breaks on
them; the reader skips every record whose `event.name` is not `gemini_cli.api_response`.

**What is deliberately not read.** The `resource` block carries the host name, home directory
paths and the full command line — including the prompt when it was passed with `-p`. Only
`attributes` is read, and only the usage fields within it.

**Format caveat.** The exporter is `JSON.stringify(record, 2) + "\n"`, so records are
pretty-printed and concatenated. The file cannot be split on newlines, and a poll can land
mid-record while the CLI is writing — the reader consumes only complete top-level objects and
advances its offset to the end of the last one.

## Ollama

Ollama response metrics expose prompt and output token counts, but Ollama does not provide a complete historical usage database. `--record-ollama` provides an opt-in local journal for requests made after tracking is enabled: pipe a completed response into it and the token counts are stored.

```sh
curl -s http://localhost:11434/api/generate \
  -d '{"model":"qwen3-coder:30b","prompt":"hello","stream":false}' \
  | ai-usage-tui --record-ollama
```

Newline-delimited streaming responses (`"stream":true`) also work: only the final event with
`done: true` is recorded, since that is the one carrying the counts. Replaying the same completed
event does not duplicate it. The journal defaults to `~/.local/share/ai-usage-tui/usage.db`;
override it with `--journal PATH`, the `journal` config setting, or `AI_USAGE_JOURNAL_PATH`.

The journal collector polls every 60 seconds (configurable via the `interval` key under `[collectors.journal]`) and aggregates Ollama events into the same normalized shape.

## Ollama Cloud

Token counts can be observed when returned by the client response. Account quota and GPU-based Cloud billing are not currently exposed through the supported API, so the tool must not invent a dollar cost. Cloud-routed models are displayed as `CLOUD`, never as local usage.

These rows carry `cost_status = quota`, not `unavailable`. The distinction matters: `unavailable` means "this should carry a price and does not", which is a gap worth reporting, while `quota` means "this is billed, but not per token at any rate we can know". They were the same value until 2026-08-19, and every panel reporting pricing coverage consequently read this deliberate refusal as a failure — the header showed 71% priced on a dataset where 100% of priceable work was priced.

## Pricing tables

When a provider reports no cost, it is estimated from two tables bundled in the binary, with no
network needed: `pricing/litellm.tsv` (~3,450 keys across 88 providers, generated from
[LiteLLM's community table](https://github.com/BerriAI/litellm) by
`scripts/refresh-litellm-pricing.py`) and `pricing/zen.toml` (~60 curated models: OpenCode Zen
ids, stealth models, and anything the community table gets wrong). Together they price 1,491
distinct model names. The curated table is applied on top of the community one, and a refreshed
cache (`--refresh-pricing`, below) on top of that, so a hand-checked rate always wins.

Keys can be provider-qualified. Where providers genuinely charge differently for the same model
name — Bedrock's variants, the aggregators — the rate follows the provider on the usage row. For
the ~180 names where providers disagree, no bare key is published at all: a model whose provider
is not recognised stays `UNKNOWN COST` rather than borrowing someone else's rate. Historical
events are priced at the rates in effect when they happened; see
[`data-model.md`](data-model.md) for the effective-dated periods that make that hold.

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

### GitHub Copilot

There is nothing to detect. Copilot is sold as a seat — Pro, Business or Enterprise — and bills
premium requests against it; there is no API-key mode and so no environment signal. `detect`'s
unevidenced fallthrough is per-token, which here would let pricing put list-rate dollars into
budgets for money that was never charged, so `SourceRoots::copilot_decision` resolves the
unevidenced case to subscription instead. In order: an explicit `billing` setting; then the plan
label in Omarchy's record for the agent, if present; otherwise subscription. Force per-token with
`[collectors.copilot] billing = "api"` or `--copilot-billing api`.

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
- Copilot collector: polls the CLI store every 30s (configurable), resuming from a `created_at`
  high-water mark kept in the store's own type; the legacy fallback tails by byte offset with a
  per-session cumulative total
- Journal collector: polls journal DB every 60s (configurable)
- Zen Pricing collector: refreshes hourly when enabled (opt-in)

Collectors never write to the journal database. Each merges into the shared in-memory `CollectorState`, and the dashboard calls `snapshot()` on its own refresh interval (default 30s), keeping the UI responsive. The journal is a source — written by `--record-ollama` / `--record-routing`, read by the journal collector — not a sink.

## Budgets

Budget limits can be configured per provider, model, or globally with daily or monthly periods. Alerts appear in the TUI banner. Actionable alerts are POSTed to `--webhook URL` (or `budgets.webhook`) when one is configured; the dispatch runs on a background thread so it never blocks rendering, and repeat alerts at the same level are suppressed for an hour. That suppression is in-memory (per process), so a cron-driven `--check-budgets` re-POSTs on every run. Periods use local calendar boundaries: daily matches the dashboard's `TODAY` range; monthly is the calendar month, not the dashboard's trailing 30-day range. Check budgets from CLI with `ai-usage-tui --check-budgets` (exits 1 if thresholds exceeded).