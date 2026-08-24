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

No field is required. `agent`, `model`, `task` and `tokens` are the useful minimum — an event
without them is stored, but aggregates under `unknown` with nothing to count. Anything omitted
takes a default:

| Field | Default |
| --- | --- |
| `agent`, `model`, `provider` | `unknown` |
| `task`, `phase` | empty string |
| `requests` | `1` (values below 1 are raised to 1) |
| `tokens`, `retries`, `escalations`, `review_defects` | `0` |
| `category` | `UNKNOWN` |
| `cost_status` | `unavailable` |
| `cost` | `null` — no cost, not `$0.00` |
| `test_result` | `null` — unobserved, not a failure |
| `created` | now |

The full optional set: `provider`, `phase`, `category`, `cost_status`, `requests`, `cost`,
`retries`, `escalations`, `test_result` (`true`/`false`), `review_defects`, `created` (unix
seconds).

Events are deduplicated on the identity `routing:{agent}:{model}:{task}:{created}`; a second
event with the same identity is ignored, not updated. Two events with the same agent, model and
task recorded in the same second therefore collapse into one — pass `created` explicitly when
batching.

## Exporting Analytics

Export the aggregates as JSON — `{source, events: <count>, aggregates: [...]}`. Individual
events are not exported:
```sh
ai-usage-tui --routing-json
```

Export as CSV:
```sh
ai-usage-tui --routing-csv routing.csv
```

One row per aggregate. CSV columns:
`agent,model,provider,tasks,tokens,cost,retries,escalations,test_passes,test_failures,review_defects`

## TUI Routing View

`t` toggles the routing panel. `Esc` (like `q`) quits the app; it does not return to the
dashboard. The panel has two blocks:

- **ROUTING — cost per delivered result**: one row per agent/model/provider, sorted cheapest per
  passing test first. Columns: `AGENT`, `MODEL`, `$/SUCCESS`, `PASS`, `RETRY`, `ESC`, `DEFECT`,
  `TOKENS`, `TASKS`. A row with nothing passing sorts last and shows `—`, never `$0.00`.
- **ESCALATIONS — derived from sessions**: the derived block described above. Drawn only when
  there is at least one session to report.

## Aggregation

Events are grouped by agent, model and provider (`routing::aggregate`, `src/routing.rs`). Each
aggregate carries:

- `tasks`: number of events
- `tokens`: sum of `tokens`
- `cost`: sum of `cost`; an event without one contributes `0`
- `retries`, `escalations`, `review_defects`: sums
- `test_passes`, `test_failures`: events with `test_result` `true` / `false`

The JSON export adds `retry_rate`, `escalation_rate` and `defect_rate` (per task, in percent).
The TUI adds `success_rate` (passes over observed results) and `cost_per_success` (cost over
passes); both are `—` when unobserved, never `0`.

## Schema

`RoutingEvent` (`src/model.rs`), one row per event in the `routing_event` table:

```text
task            string
phase           string
agent           string
model           string
provider        string
category        LOCAL | FREE | PAID | CLOUD | UNKNOWN
requests        integer, at least 1
tokens          integer — one counter, no input/output split
cost            number | null
cost_status     reported | calculated | estimated | free | local | quota | unavailable
retries         integer
escalations     integer
test_result     true | false | null
review_defects  integer
created         unix seconds (the timestamp column)
```

## Data Caveats

- Routing events are opt-in; they are only recorded when explicitly sent via `--record-routing`.
- No prompt or completion content is stored.
- Cost is optional; if omitted, only token counts are aggregated.
- Test result is optional; pass rate is calculated only from events that include it.