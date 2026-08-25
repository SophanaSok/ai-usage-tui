# Claude Code hook for `--claude-code-hook`

`settings.json` registers `ai-usage-tui --claude-code-hook` on Claude Code's `PostToolUse` and
`PostToolUseFailure` events for the `Bash` tool. Every test run the agent makes — pass or fail —
becomes a routing event, attributed to the model that ran it and to the requests the attempt
took. What is and is not observed is in
[`docs/routing-analytics.md`](../../docs/routing-analytics.md#recording-from-claude-code).

Both events are needed. A Bash command that exits non-zero fires `PostToolUseFailure`, not
`PostToolUse` with an exit code; registering one without the other records only the passes or
only the failures.

## Install

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

## Verify

Run a test command in a Claude Code session, then:

```bash
ai-usage-tui --routing-json          # {"events": N, "aggregates": [{"agent": "claude-code", ...}]}
ai-usage-tui                          # `t` shows the ROUTING block
```

`claude --debug` shows each hook's output: `Recorded a passing test run in …`, `Already
recorded`, or `Nothing to record: …` with the reason. With `AI_USAGE_LOG=1`, a run that could
not be attributed to its transcript is logged with why.

## Uninstall

Remove the two entries from the settings file. Events already journaled stay; they are routing
events like any other and are dropped by deleting the journal (`--doctor` names it).
