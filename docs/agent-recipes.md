# Building on ai-usage-tui from an agent

You are reading this because someone asked you to build something on their AI usage data: a
status-bar module, an alert, a digest, a report, a spreadsheet. It is printed by
`ai-usage-tui --agent-guide recipes`. The reading rules in `ai-usage-tui --agent-guide` apply to
everything here -- a script that prints `$0` for an unknown cost is wrong in the same way a
sentence is.

## 1. What to build on

Build on these; they are under the tool's stability contract (semantic versioning from 1.0):

- `--summary-json`, `--json`, `--routing-json`, `--check-budgets`: one JSON object each, carrying
  `"schema_version": 1`. `ai-usage-tui --schema` defines every key and enum value. Within a
  schema version keys are only added.
- `--csv -` and `--routing-csv`: columns are only appended, so reading by position keeps working.
- Exit codes: `0` on success; `--check-budgets` exits non-zero when a budget has reached its
  `warn` level. A failure of the tool is *also* non-zero today, so parse the output before you
  treat a non-zero exit as a breach.
- Flag names, config keys and environment variables.

Never parse these; they change in any release: the dashboard, `--once`, `--doctor`,
`--statusline`, error messages, and the prose of these guides.

Rules for the code you write:

- **Read by key and ignore keys you do not know.** Check `schema_version` once and stop with a
  clear message if it is not the one you wrote against.
- **`null` is unknown, never zero.** In `jq`, `.cost // 0` is a bug: `//` replaces `null`, and the
  total it feeds is then wrong in a way nobody can see. Print a word (`// "unknown"`), or skip the
  row and say how many you skipped. In CSV an unknown cost is an empty field, which SQLite and
  spreadsheets sum as `0` -- use `nullif(cost, '')`.
- **Say "at least" when `cost_is_floor` is true**, and never add `api_equivalent_cost` to `cost`:
  it was never charged.
- **Use the summary.** `--json` is one object per request, often tens of megabytes. Filter with
  `--project`, `--session`, `--model`, `--provider` and a range flag before asking for rows.
- **Thresholds are the user's.** Every number below that decides something is a placeholder: ask.
- One run takes a fraction of a second but reads every log; poll once a minute, not once a second.

Each block below opens with what it needs. Those that need only `jq` are run by the project's
test suite against fixture data, as written.

## 2. A status-bar module

Waybar's `custom` module format (`return-type: json`); polybar and tmux want only `.text`.
Shows the most-used fresh plan window, falling back to today's request count.

```sh
# needs: jq
ai-usage-tui --summary-json --today | jq -c '
  [.limits[] | select(.stale | not) | .name as $agent | .windows[] | {agent: $agent, label, percent_used}] as $windows
  | ($windows | max_by(.percent_used)) as $top
  | {
      text: (if $top then "\($top.percent_used | round)%" else "\(.totals.requests) req" end),
      tooltip: ([$windows[] | "\(.agent) \(.label): \(.percent_used | round)%"] | join("\n")),
      class: (if $top == null then "idle"
              elif $top.percent_used >= 90 then "critical"
              elif $top.percent_used >= 75 then "warning"
              else "ok" end)
    }'
```

```jsonc
// needs: waybar -- ~/.config/waybar/config
"custom/ai-usage": {
  "exec": "/path/to/the/script/above",
  "return-type": "json",
  "interval": 60
}
```

## 3. Stop before a plan window runs out

Exits `1` when any fresh window is at or above the limit, so it can gate a batch job:
`guard.sh && run-the-agents`. A stale reading is ignored rather than trusted.

```sh
# needs: jq
# exit: 1 means a window is at or over the limit
LIMIT=90
ai-usage-tui --summary-json --today | jq -e --argjson limit "$LIMIT" '
  [.limits[] | select(.stale | not) | .windows[] | select(.percent_used >= $limit)] | length == 0
' >/dev/null
```

## 4. A budget alert on a schedule

Budgets come from the config file (`ai-usage-tui --agent-guide setup`). On a subscription plan
a budget counts nothing; use the guard above instead. Run this no more often than the user wants
to be told -- every run while a budget is over repeats the alert.

```sh
# needs: jq, notify-send
out=$(ai-usage-tui --check-budgets) && exit 0
echo "$out" | jq -e '.alerts | length > 0' >/dev/null || { echo "ai-usage-tui failed: $out" >&2; exit 2; }
echo "$out" | jq -r '.alerts[] | "\(.scope) \(.period): \(.level), \(.pct | round)% of $\(.limit)\(if .unpriced_requests > 0 then " (at least)" else "" end)"' \
  | while read -r line; do notify-send "AI budget" "$line"; done
```

Or let the tool post it: `webhook = "https://…"` under `[budgets]` sends the same `alerts` array
as JSON on every check.

## 5. A weekly digest in Markdown

```sh
# needs: jq
ai-usage-tui --summary-json --week --top 5 | jq -r '
  def money: if . == null then "unknown" else "$\(. * 100 | round / 100)" end;
  def pct: if . == null then "not recorded" else "\(.)%" end;
  "# AI usage: \(.range.label)",
  "",
  "- \(.totals.requests) requests, \(.totals.tokens) tokens",
  "- billed per token: \(if .totals.cost_is_floor then "at least " else "" end)\(.totals.cost | money)",
  "- billed against a plan: \(.totals.quota_requests) requests (no per-request price; \(.totals.api_equivalent_cost | money) at list rates, never charged)",
  "- no price known: \(.totals.unpriced_requests) requests",
  "",
  "## Models",
  "",
  (.by_model.rows[] | "- \(.model): \(.requests) requests, \(.metrics.share_of_tokens_pct | pct) of tokens, cache hit \(.metrics.cache_hit_pct | pct)"),
  "",
  "## Projects",
  "",
  (.by_project.rows[] | "- \(.project // "(no project recorded)"): \(.requests) requests, \(.sessions) sessions")
'
```

## 6. A per-project table

`--top 0` lists every project instead of folding the tail into `other`.

```sh
# needs: jq
ai-usage-tui --summary-json --month --top 0 | jq -r '
  ["project", "requests", "tokens", "cost", "plan_requests", "unpriced_requests"],
  (.by_project.rows[] | [.project // "(none)", .requests, .tokens, (.cost // "unknown"), .quota_requests, .unpriced_requests])
  | @tsv'
```

## 7. Rows into SQLite

Only when the summary cannot answer the question. The columns are `provider, model, category,
cost_status, requests, input_tokens, output_tokens, reasoning_tokens, cache_read_tokens,
cache_write_tokens, cost, created, project, session_id, api_equivalent_cost`.

```sh
# needs: sqlite3
ai-usage-tui --csv - --month > usage.csv
sqlite3 usage.db <<'SQL'
.mode csv
.import usage.csv usage
select model,
       count(*)                              as requests,
       sum(nullif(cost, ''))                 as cost,        -- NULL when nothing was priced
       sum(cost = '' and cost_status = 'quota') as plan_requests
from usage group by model order by requests desc;
SQL
```

## 8. Check what you built

Run it against a range with data and one without (`--today` on a quiet day): an empty `rows`
array, a `null` cost and a missing `limits` entry are the normal cases, not the edge cases. Then
tell the user which command it runs, how often, and which numbers in it are theirs to change.
