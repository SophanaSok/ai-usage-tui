# Any other agent: Codex, Cursor, OpenCode, Gemini CLI, Aider…

Claude Code has a [skill](../claude-code/README.md#skill). Every other agent that can run a shell
command needs only to be told the tool exists, because the instructions ship inside the binary.
Paste this into the file your agent reads — `AGENTS.md`, `.cursorrules`, `GEMINI.md`, a system
prompt:

```markdown
## AI usage and cost questions

When asked about AI token usage, cost, prompt-cache efficiency, model routing, rate limits or
budgets, use the `ai-usage-tui` CLI (it reads local agent logs; it sends nothing anywhere):

1. Run `ai-usage-tui --agent-guide` and follow it.
2. Run `ai-usage-tui --summary-json` (add `--today`, `--month`, `--days N` or `--all`; default is
   the last 7 days). Narrow with `--project`, `--session`, `--model`, `--provider`.
3. Do not run `ai-usage-tui --json` un-narrowed: it prints one object per request.
4. `null` is unknown, never zero. Subscription usage has no dollar cost; `api_equivalent_cost`
   was never charged.

To set the tool up, track a tool it does not read, or build a script on its data, the last
section of `ai-usage-tui --agent-guide` names the guide for that. Never estimate a token count,
and show me any change to a file outside the tool's own config before making it.
```

That is the whole integration. `--agent-guide` carries the reading rules and what to look for,
`--schema` defines every key, and both always match the installed version — which is why the
block above says so little.

If the agent asks for permission on every command, allow-list `ai-usage-tui` in whatever form it
supports. The commands named in the block only read. A few others write a file of the tool's own
or use the network — the recorders, `--refresh-pricing`, `--check-update` — and the setup guide
that `ai-usage-tui --agent-guide` points to lists exactly which; allow-list more narrowly if that
matters.

What the agent reads — token counts, model names, costs, project paths, session ids — goes to the
model provider it runs on, like anything else in its context. Nothing is read from your prompts or
transcripts, and `ai-usage-tui` itself transmits nothing.
