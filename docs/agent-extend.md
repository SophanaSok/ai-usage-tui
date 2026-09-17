# Extending ai-usage-tui from an agent

You are reading this because someone wants the tool to cover something it does not: a coding
agent, gateway or script whose usage it does not read, or a view it does not have. It is printed
by `ai-usage-tui --agent-guide extend`. Take the cheapest route that does the job.

| They want | Route | Needs |
| --- | --- | --- |
| usage from a tool this one does not read | an adapter into `--record-event` (section 2) | a shell |
| a number, report, alert or bar module it does not show | a script over `--summary-json`: `ai-usage-tui --agent-guide recipes` | a shell |
| a setting changed | `ai-usage-tui --agent-guide setup` | a shell |
| a new panel, a native collector, a corrected price | a change to the source (section 3) | the repository, Rust |

## 1. The rule that outranks the request

**Never invent a number.** This tool's whole claim is that every figure is either measured or
marked unknown, and a row you fabricate is indistinguishable, in a total, from one a provider
reported.

- If the tool does not log token counts -- or logs zeros, as Cursor does -- there is nothing to
  record. Do not estimate them from message length, a character count or a price. Tell the user
  the tool does not measure them; that is a complete answer.
- A `cost` may only be a figure the tool itself recorded. Do not compute one from a rate you know.
- Do not record a tool that `ai-usage-tui --summary-json` already lists under `sources` with
  `present: true`: it would be counted twice.

## 2. An adapter into `--record-event`

Find the tool's own log of requests and read a few real records before writing anything. Then
turn each into one JSON object per line and pipe them in:

```sh
# needs: jq
# A stand-in for the tool's log; in real use this is `jq -c '…' ~/.mytool/usage.jsonl`.
printf '%s\n' '{"id":"r1","ts":1758000000,"model":"claude-sonnet-5","in":1200,"out":300,"cached":9000,"cwd":"/work/app","sid":"s1"}' \
  | jq -c '{provider: "mytool", model, event_id: .id, created: .ts,
            input_tokens: .in, output_tokens: .out, cache_read_tokens: .cached,
            project: .cwd, session_id: .sid}' \
  | ai-usage-tui --record-event
```

| Key | |
| --- | --- |
| `provider`, `model` | required. `provider` is the tool's name, or the API provider if the tool reports it; it and the model name are what pricing looks up |
| `input_tokens`, `output_tokens` | required, measured. `input_tokens` excludes cached tokens when the tool reports those separately; if its input count *includes* them, subtract, or they are billed twice |
| `event_id` | the tool's own id for the request. Preferred: it is what makes a re-run harmless |
| `created` | unix seconds. One of `event_id` and `created` is required |
| `reasoning_tokens`, `cache_read_tokens`, `cache_write_tokens` | optional; leave out what the tool does not report |
| `project` | the working directory, as an absolute path |
| `session_id` | the tool's session or conversation id |
| `cost` | optional: dollars the tool itself recorded for this request. Kept as `reported` |
| `billing` | optional: `"subscription"` when the work is billed against a plan -- it then has no dollar cost. Cannot be combined with `cost` |

It is strict on purpose, and its error is your documentation: an unknown key, a count that is not
a whole number or a line that is not JSON refuses the **whole batch**, names the event and the
key, exits non-zero and writes nothing. Fix the adapter; do not drop the offending field to get
past it.

Recording is idempotent on `event_id`, so the simple design is the right one: re-send the whole
log on a schedule (cron, a systemd timer, the tool's own post-request hook) and let the journal
ignore what it has. The reply says how many were new: `Recorded 3 of 120 usage event(s) … (117
already journaled)`.

Check your work with `ai-usage-tui --summary-json --all --provider mytool`: the request count
should match the log's, and one request's tokens should match by hand. If the model has no list
price, `cost` stays `null` and `unpriced_requests` counts it -- correct, not a fault to fix.
`ai-usage-tui --doctor` shows where the journal is. It stores counts, model names, the project
path and the session id -- never prompt or response text, so do not send any.

## 3. When it needs a change to the source

A new dashboard panel, a collector that tails a store incrementally, a wrong or missing price.
The repository is https://github.com/SophanaSok/ai-usage-tui . Clone it and start with
`AGENTS.md` ("Extending it") and `CONTRIBUTING.md` ("Common contributions"): they list every file
a change touches, and most of that list is enforced by tests that name what is missing. Work from
a redacted capture of what the tool really wrote, not from its documentation.

If the user only wants it supported and is not going to maintain a fork, open an issue instead
with the "Support another tool's usage data" template. What it asks for is what you can find out
here: where the tool keeps its data, the format, one **redacted** record, and which of input,
output, cache and reasoning counts it records. Show the user the redacted record before it is
posted anywhere: these logs hold prompts, paths and sometimes keys.

A missing or wrong price is data, not code: say which model and which provider's published
price, and point at the same repository.
