---
name: ai-usage
description: Analyze the user's AI coding-agent token usage, cost, cache efficiency and model routing from local data with the ai-usage-tui CLI. Use when asked how many tokens or how much money Claude Code, Codex, Copilot, Gemini CLI, OpenCode or local models used; which project, session or model is heaviest; how to reduce token usage or improve prompt caching; whether the expensive model is worth it or routing/escalation is working; how close a subscription rate limit or a budget is.
allowed-tools: Bash(ai-usage-tui:*)
---

# AI usage analysis

`ai-usage-tui` reads this machine's agent logs locally and prints compact JSON. The binary carries
its own instructions, which match the installed version — read them instead of guessing:

1. Run `ai-usage-tui --agent-guide` and follow it. It says which command to start with, how to
   drill down, the rules for reading the numbers, and what to look for.
2. Run `ai-usage-tui --summary-json` with the range the user asked about (`--today`, `--week`,
   `--month`, `--days N`, `--all`; the default is the last 7 days). Add `--project`, `--session`,
   `--model` or `--provider` with values copied from the summary to look closer.
3. Run `ai-usage-tui --schema` only if a key or value is unclear.

Do not run `ai-usage-tui --json` without narrowing it first: it prints one object per request and
is usually larger than your context window.

Answer with the figures you read and the range they cover. `null` is unknown, never zero;
subscription usage has no dollar cost, and `api_equivalent_cost` was never charged — do not call it
spend or savings. If the command is missing, the install page is
https://sophanasok.github.io/ai-usage-tui-site/ .

$ARGUMENTS
