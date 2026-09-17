# Reading ai-usage-tui from an agent

You are reading this because someone asked you about their AI token usage, cost, or model routing,
and `ai-usage-tui` is installed. It reads the local logs of Claude Code, Codex CLI, GitHub Copilot,
Gemini CLI, OpenCode and journaled local models. It sends nothing anywhere. This guide is printed
by `ai-usage-tui --agent-guide`; the key-by-key reference is `ai-usage-tui --schema`.

## 1. Start with the summary

```sh
ai-usage-tui --summary-json            # last 7 days (the default range)
ai-usage-tui --summary-json --month    # --today | --week | --month | --days N | --all
```

One line of JSON, roughly 25-35 KB: `totals`, then the same bucket shape `by_category`,
`by_model`, `by_project`, `by_session` (largest first, `--top N`, default 10, the rest folded into
`other`) and `by_day`; plus `sources`, `pricing`, `burn`, `budgets`, `limits`, `escalations`,
`provenance` and `routing`.

**Do not start with `--json`.** It prints one object per request -- megabytes, often more than
your context window. Use it only after narrowing (step 2).

`pricing` says what every dollar figure rests on: the currency (USD, list price), the dates the
bundled rate tables were cut, and `warnings` when a refreshed cache was refused or the tables are
over 90 days old. If they are old, say the estimates use rates as of those dates.

Check `sources` first. A source with `present: false` was not found; `status` says when data had
to be skipped, when records were missing a token count (they are kept and left unpriced, so
`unpriced_requests` rises and token totals are a minimum) and when records had no timestamp (they
appear only under `--all`); `detail` says how billing was decided. If the summary looks empty or wrong,
`ai-usage-tui --doctor` prints the same diagnosis for a human.

## 2. Drill down instead of pulling rows

Every value you need is in the summary, spelled exactly as the filter wants it:

```sh
ai-usage-tui --summary-json --project /path/from/by_project    # one project: its models, sessions, days
ai-usage-tui --summary-json --session ID_FROM_BY_SESSION       # one session
ai-usage-tui --summary-json --model NAME --month               # one model (also --provider NAME)
ai-usage-tui --summary-json --top 25                           # longer lists; --top 0 lists everything
ai-usage-tui --csv - --session ID                              # rows, only when you need them; CSV is ~4x smaller than --json
```

## 3. Reading rules -- these are not optional

1. **`null` means unknown or not recorded. It never means 0.** A `cost` of `null` is not free. A
   `cache_hit_pct` of `null` means no cache tokens were recorded -- several sources never report
   them -- not that caching failed.
2. **`cost` is dollars actually billed per token.** When `cost_is_floor` is true, or
   `unpriced_requests > 0`, it is a minimum. Say "at least".
3. **`quota_requests` are real cost with no per-request price**: work billed against a subscription
   (Claude Max/Pro, ChatGPT plans, Copilot). They are never in `cost`. On a subscription account
   most buckets have `cost: null` and that is correct. Report it as "billed against the plan",
   never as "$0" or "free": the plan costs money, and nothing here says how much per request.
4. **`api_equivalent_cost` was never charged.** It is what plan-billed requests would have cost at
   API list rates. Use it to *weigh* plan-billed work (which model or project is heaviest). Never
   call it spend, and never call the difference from the plan price "savings". **On a subscription
   there is no per-request money to save**: a change in usage moves quota consumption (`limits`),
   not a bill. Do not write "you would save $X".
5. **A token share is not a cost share.** `tokens` counts cache reads, which dominate agent
   sessions and are priced far below output tokens. To rank by weight use `cost`, or
   `api_equivalent_cost` on a plan -- not `share_of_tokens_pct` alone.
6. **`routing` is empty unless the user feeds it.** It holds test runs journaled by the Claude Code
   hook (`contrib/claude-code/`) or `--record-routing`. `escalations` is different: it is derived
   from ordinary usage and is always there.
7. Percentages are 0..100. Timestamps are unix seconds. Days are local time.

## 4. What to look for

Report what the numbers show, with the numbers. Each of these is a place to look, not a verdict;
the thresholds are yours to judge and to state.

**Token usage**
- *Cache effectiveness.* `metrics.cache_hit_pct` by project and by session. A high-volume project
  well below the user's other projects is re-sending context uncached: long-lived sessions that
  were restarted, prompts whose prefix changes every turn, tools that rewrite large files.
- *Heavy requests.* `metrics.tokens_per_request` by model and by session. Very large values mean
  very long contexts; compare sessions in the same project.
- *Where the volume is.* `by_project` and `by_session` with `other`: whether a few sessions
  carry most of the usage, and which days (`by_day`) spike.
- *Output and reasoning share.* `output_pct` and `reasoning_pct` by model -- output tokens are the
  expensive ones. `reasoning_pct: null` means the source does not report a split.

**Routing**
- *Model mix.* `by_model`: which models take what share of `cost` (or `api_equivalent_cost`).
  `list_input_rate` (dollars per million input tokens, from the pricing table) is what says which
  model is the expensive one -- rank by it, not by what you expect a name to mean.
  Then `--project` to see the mix per project, and `by_session` rows whose `models` include an
  expensive model alongside few `requests` and low `tokens_per_request` -- small jobs on the
  largest model.
- *Escalations.* `escalations.transitions`: sessions that opened on a cheaper model and moved up,
  with `cost_after`. **`to` is always the pricier model**: compare `from_input_rate` and
  `to_input_rate`, and never infer the direction from model names (a newer or differently named
  model can sit above one you expect to be the top). A move to a cheaper model is not an
  escalation and is not listed. The data does not say *why* a session moved. A high `escalation_rate` with large `cost_after` means the cheaper model is being tried
  on work it does not finish.
- *Is the expensive model earning it?* `routing.aggregates`: `cost_per_success`, `success_rate`,
  `retry_rate` per agent and model. `cost_per_success` is a figure only when `cost_basis` is
  `exact` or `free`; otherwise it is `null` and `cost_basis` says why (`quota`: plan-billed, no
  dollar figure; `floor`/`unpriced`: some of the spend has no price). On a subscription compare
  `success_rate` and `retry_rate` across models instead. If `routing.events`
  is 0, say the measurement is not set up rather than guessing, and point at
  `contrib/claude-code/README.md`.

**Limits and budgets**
- `limits[].windows[].percent_used` with `resets_in_secs`, beside `burn.tokens_per_minute`:
  whether a plan window is likely to run out before it resets. `stale: true` means do not rely on it.
- `budgets`: every configured budget with `pct` and `level`, including ones still `OK`.

## 5. What not to claim

- No dollar figure the documents do not contain. Do not price hypotheticals ("on Sonnet this
  would have cost...", "you would save about $100"): the tool deliberately does not, and a list
  input rate alone cannot price a request whose tokens are mostly cache reads and output.
- An escalation is a *fact about what happened*, not a routing rule that misfired. Moving to a
  pricier model can be the right call; whether it was is what `routing.aggregates` measures.
- No comparison across a `null`. "Project A caches better than B" needs both `cache_hit_pct`
  values present.
- No causes the data does not record. A cache-hit percentage says nothing about whether tasks
  succeeded; an escalation does not say the first model failed or hit a limit. Only
  `routing.aggregates` measures outcomes.
- No conclusions from thin data: check `requests` before quoting a percentage, and note that
  `burn.cost_per_hour` is `null` below five requests for this reason.
- State the range you read (`range.label`) and any filters with every finding.

## 6. Privacy

Project paths and session ids are in these documents. When you read them, they go to whichever
model provider you run on. The tool itself never transmits anything; if the user is sensitive
about paths, summarise rather than quoting them.

## 7. Doing more than reading

This guide is about reading. When the user wants something changed or built, there is a guide
for that too, printed by the same flag with a topic:

- `ai-usage-tui --agent-guide setup` -- set it up for them: the config file and budgets, the
  Claude Code hook and status line, timers, and what each command writes or sends.

If the flag rejects a topic, the installed version predates it: say so, and that upgrading adds it.
