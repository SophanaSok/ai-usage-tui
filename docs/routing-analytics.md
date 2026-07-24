# Routing Analytics

Routing analytics track which agents and models were used for each development task, along with token usage, cost, and test outcomes. This enables cost and quality analysis by agent, model, and task type.

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