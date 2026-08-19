# Routing Analytics

Routing analytics answer a question a usage total cannot: **is the expensive model earning its
cost?** A model at twice the token price that lands the change first time can be cheaper per
delivered result than a cheap one that needs three attempts.

The `t` panel has two blocks, and the split between them is deliberate.

| Block | Where it comes from | What it can say |
| --- | --- | --- |
| **ESCALATIONS** | Derived from usage already collected. No setup. | Which sessions reached for a pricier model than they opened with, and what that cost |
| **ROUTING** | Recorded by your harness via `--record-routing` | Cost per passing test, retry / escalation / defect rates |

They are never merged. A measured pass rate and an inferred transition would be
indistinguishable in one table, which is the same failure `CostStatus` exists to prevent one
level up: nothing this tool inferred should look like something it observed.

## Derived escalations

For each session with more than one request, the model it opened with is compared against every
model it used afterwards. If any of them costs more per input token, the session escalated. The
block reports how many sessions did, the most common opening-to-priciest pairs, and the spend on
models above the opening rate.

What it does **not** claim:

- **An escalation is not a failure of the cheaper model.** Tasks get harder. The number supports
  "this happens N times in M sessions and costs this much", not a verdict.
- **It is not a routing event.** Derived transitions are never written to the journal or folded
  into recorded aggregates.
- **It does not guess at unknown prices.** Ordering two models needs a price for both. Without
  one, the change is reported as unranked rather than assumed flat — so a low escalation count
  is distinguishable from a count taken with one eye shut.

Counting is per session, not per model switch. Sessions interleave models rather than stepping
up once; a real session switched models 20 times, 10 of them upward. Counting each switch and
attributing the spend that followed reported **$233 of escalated spend for a $29 session**. Each
session is characterised once and each request counted once, so the reported figure cannot
exceed what the sessions actually cost.

Sessions without a session id are skipped rather than pooled — pooling would invent adjacency
between unrelated requests.

## Recording Routing Events

Record a routing event from stdin (JSON):

```sh
echo '{"agent":"@heavy","model":"glm-5.2:cloud","task":"refactor","tokens":15000,"cost":0.02,"test_result":true}' | ai-usage-tui --record-routing
```

Required fields: `agent`, `model`, `task`, `tokens`. Optional: `cost`, `test_result` (boolean).

## Exporting Analytics

Export all routing events as JSON:
```sh
ai-usage-tui --routing-json
```

Export as CSV:
```sh
ai-usage-tui --routing-csv routing.csv
```

CSV columns: `timestamp,agent,model,task,tokens,cost,test_result`

## TUI Routing View

Press `t` in the TUI to open the routing analytics view. It shows aggregated tables:

- **By Agent**: total tokens, total cost, task count, pass rate
- **By Model**: total tokens, total cost, task count, pass rate
- **By Task**: total tokens, total cost, task count, pass rate

Navigate with `j/k`. Press `Esc` to return to the main dashboard.

## Aggregation

Events are grouped by agent, model, and task. Aggregations include:

- `total_tokens`: sum of tokens across events
- `total_cost`: sum of cost across events
- `task_count`: number of events
- `pass_rate`: percentage of events with `test_result: true`

## Schema

```text
timestamp
agent
model
task
tokens
cost
test_result: pass | fail
```

## Data Caveats

- Routing events are opt-in; they are only recorded when explicitly sent via `--record-routing`.
- No prompt or completion content is stored.
- Cost is optional; if omitted, only token counts are aggregated.
- Test result is optional; pass rate is calculated only from events that include it.