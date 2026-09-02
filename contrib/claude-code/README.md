# Claude Code integration: the hook and the status line

Two settings files, deliberately separate so installing one does not install the other:
`settings.json` feeds routing analytics from Claude Code's hooks, and
`statusline-settings.json` feeds the limits panel from its status line, giving Claude Code a
one-line rate-limit readout in the same change.

## Hook

`settings.json` registers `ai-usage-tui --claude-code-hook` on Claude Code's `PostToolUse` and
`PostToolUseFailure` events for the `Bash` tool. Every test run the agent makes — pass or fail —
becomes a routing event, attributed to the model that ran it and to the requests the attempt
took. What is and is not observed is in
[`docs/routing-analytics.md`](../../docs/routing-analytics.md#recording-from-claude-code).

Both events are needed. A Bash command that exits non-zero fires `PostToolUseFailure`, not
`PostToolUse` with an exit code; registering one without the other records only the passes or
only the failures.

### Install

Merge the `hooks` block into `~/.claude/settings.json` (every project) or a project's
`.claude/settings.json` (that project, and shareable). Hook entries merge across the two, so an
existing `PostToolUse` list keeps its other entries:

```bash
jq -s '.[0] * .[1]' ~/.claude/settings.json contrib/claude-code/settings.json > /tmp/settings.json \
  && mv /tmp/settings.json ~/.claude/settings.json
```

`jq`'s `*` merges objects but replaces arrays, so if `~/.claude/settings.json` already has a
`PostToolUse` or `PostToolUseFailure` list, add the entry by hand instead. Claude Code reads the
file at start; restart a running session, or check with `/hooks`.

Hooks run with Claude Code's environment, so `ai-usage-tui` must be on that `PATH`. If it is not
(`command -v ai-usage-tui` in a shell Claude Code was launched from), put the absolute path in
`command`.

### Verify

Run a test command in a Claude Code session, then:

```bash
ai-usage-tui --routing-json          # {"events": N, "aggregates": [{"agent": "claude-code", ...}]}
ai-usage-tui                          # `t` shows the ROUTING block
```

`claude --debug` shows each hook's output: `Recorded a passing test run in …`, `Already
recorded`, or `Nothing to record: …` with the reason. With `AI_USAGE_LOG=1`, a run that could
not be attributed to its transcript is logged with why.

### Uninstall

Remove the two entries from the settings file. Events already journaled stay; they are routing
events like any other and are dropped by deleting the journal (`--doctor` names it).

## Status line

`statusline-settings.json` registers `ai-usage-tui --statusline` as Claude Code's status-line
command. Claude Code runs it on every redraw, and again when a rate-limit window reaches its
reset, handing it the official `rate_limits` block on stdin. The command prints one line for
the bar — `5h 42% (resets 2h 10m) · 7d 63% (resets 3d 4h)`, red past 90% — and caches the
windows in this tool's data directory, which is how the `l` panel and `--json` see them on a
machine without Omarchy. Only the three windows' `used_percentage` and `resets_at` are read
from the payload; the session id, transcript path, working directory, model and cost beside
them are never deserialised.

The block appears only for Pro and Max subscribers and only after the session's first API
response, so the line is empty until then — and stays empty on an API-billed account, which
is correct rather than 0%.

### Install

Merge it the same way. `statusLine` is a single object, so `jq`'s `*` replaces one that is
already there:

```bash
jq -s '.[0] * .[1]' ~/.claude/settings.json contrib/claude-code/statusline-settings.json > /tmp/settings.json \
  && mv /tmp/settings.json ~/.claude/settings.json
```

If you already have a status-line script and want to keep it, have it read its stdin once and
hand the same bytes to `ai-usage-tui --statusline` as well; the line this prints is the
readout, and the cache is written either way. Claude Code reads the file at start, so restart a
running session. `claude --settings '{"statusLine":{"type":"command","command":"ai-usage-tui --statusline"}}'`
tries it for one session without touching any file.

### Verify

Send one message in a Claude Code session; the line appears under the prompt after the first
response. Then:

```bash
ai-usage-tui --doctor                 # LIMITS: `statusline  found  2 windows  …/statusline-limits.json`
ai-usage-tui --json | jq .limits      # the "claude" row, with its windows
ai-usage-tui                          # `l` shows the row
```

### Uninstall

Remove the `statusLine` key from the settings file. The cached windows go stale on their own
after 30 minutes and are dimmed rather than alarmed from then on; delete
`statusline-limits.json` from the data directory (`--doctor` names it) to remove the row at once.
