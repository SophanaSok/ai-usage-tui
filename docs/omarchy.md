# Omarchy integration

[Omarchy](https://omarchy.org) is an Arch/Hyprland desktop whose bar carries an Agents panel that
meters every AI coding subscription on the machine. `ai-usage-tui` can read what that panel
already derived, and — separately, and only when asked — publish its own usage back into it.

**Both directions are optional and neither is required to use this tool.** On a machine without
Omarchy the records directory simply does not exist: the reader logs its absence once and idles,
the `l` panel stays empty, and `--json` reports an empty `limits` array. Nothing here changes what
any other data source reports.

## Subscription limits

[Omarchy](https://omarchy.org) is an Arch/Hyprland desktop whose bar has an
Agents panel that meters every AI coding subscription on the machine. Omarchy 4
writes one JSON record per agent under
`${XDG_STATE_HOME:-~/.local/state}/omarchy/agents/usage/` (`claude.json`,
`codex.json`, `fireworks.json`), fetched from the vendors' own rate-limit
endpoints with the agents' saved sign-ins. `l` shows those finished records:
one row per rate-limit window (`AGENT | WINDOW | bar | USED | RESETS IN |
TIER`), then one line per agent — `Claude Code · Max 20x · updated 12m ago`.
The header names the fullest fresh window beside the pricing-coverage figure
(`claude session 92%`).

Only six fields of each record are read: `id`, `name`, `updatedAt`, `ready`,
`tierLabel`, `usageStatusText`, and the `limits` list (`label`, `title`,
`percent`, `resetsAt`). Never read: the agents' credentials, Omarchy's probe
cache (`~/.cache/omarchy/agent-usage`), the network, the record's
`authHelpText`, and its token tallies (`modelUsage`, `recentDays`, …). The
reader writes nothing there; the only write is the opt-in `--omarchy-record`
action described next.

The display rules follow Omarchy's panel. A window at or above 90 % is drawn in
the alarm colour, in the panel and in the header; a window whose reset time has
passed shows `reset passed` and does not alarm. A record whose `updatedAt` is
older than 45 minutes (three of Omarchy's 15-minute refreshes) or missing is
stale: its rows are dimmed and never alarm, and the header ignores it. A record
with no windows but a status text (`Sign-in expired`) is shown as a status row;
a record with neither, such as Fireworks' balance record, is skipped. A file
that does not parse is listed as `unreadable: <file>: <error>` in the panel and
on the status line, and the header shows degraded.

The reader is on by default and idle on any machine without the directory: the
panel says so, and one INFO line goes to `AI_USAGE_LOG` when set. Disable it
with `[omarchy] limits = false`, or point it elsewhere with `[omarchy] dir` or
`--omarchy-dir PATH`. `--json` carries the same data under a top-level
`limits` array — present and empty when disabled or absent:

```json
"limits": [{
  "agent": "claude", "name": "Claude Code", "tier": "Max 20x", "status": "",
  "updated_at": 1755950400, "age_secs": 720, "stale": false,
  "windows": [{ "label": "Session (5-hour)", "percent_used": 92.0,
                "resets_at": 1755961200, "resets_in_secs": 10080 }]
}]
```

`percent_used` is 0–100, like `--check-budgets`' `pct`; `updated_at` and
`resets_at` are Unix seconds or `null`. CSV output is unchanged. The record's
plan label (`tierLabel`) is also a billing signal for the Claude Code and Codex
collectors — see [Claude Code billing](#claude-code).

## Publishing to Omarchy's agents panel

The reverse direction is opt-in. `ai-usage-tui --omarchy-record` writes this
tool's own usage and budgets as a record into the same directory, so the bar's
Agents panel gains a tab for the sources Omarchy cannot meter itself. It is a
one-shot action, mutually exclusive with the other actions: it writes
`<id>.json`, prints `Wrote Omarchy record <path> (N requests, M budget
meters)`, and exits non-zero on failure. Nothing else in this tool writes
there — the dashboard and the exports never do, and a test asserts it.

```toml
[omarchy]
records = ["opencode"]            # ids to write: opencode (default), ollama
balance = false                   # also draw a budget as the panel's prepaid ledger
balance_budget = "global/monthly" # which budget, as <scope>/<period>
```

- `opencode` is every OpenCode row, all providers, priced; `ollama` is the
  journal's Ollama rows. `claude`, `codex` and `fireworks` are refused: those
  are Omarchy's own files and a record so named would overwrite them.
- Claude Code and Codex rows are never included — Omarchy's own tabs cover
  those logs. Omarchy's `claude` and `codex` collectors also fold OpenCode's
  anthropic/openai rows into their tabs, so such a row can appear in both the
  `opencode` tab and Omarchy's. Tabs are never summed, so this is display
  overlap, not double counting.
- Every configured budget becomes a meter in the record's `limits` list
  (`Monthly budget` / `Daily budget`, the scope in the label, `percent` =
  spend/limit clamped to 1, `resetsAt` the next local midnight or first of
  next month), so the bar glyph alarms at 90 % like a rate limit and the
  panel shows a reset countdown. The spend is the figure `--check-budgets`
  reports — computed over all sources, not the tab's rows alone.
- `balance = true` additionally draws one budget as the panel's prepaid
  ledger (`remaining`, `funded`, `spent`, `USD`, `estimated: true`).
  `balance_budget` picks it; a missing match falls back to `global/monthly`,
  then `global/daily`, then the first budget. Off by default because the
  panel labels it "Prepaid credits … funded", which describes a soft budget
  loosely.
- `tierLabel` reads `Budget $50/month` or `Pay as you go`. When billable
  rows lack a price the status reads `Spend partly unpriced` and
  `authHelpText` carries the count.
- The record carries token counts, model ids, request and session counts,
  and dollar figures — never content, never a path. The write is atomic
  (temporary `.<id>.<pid>.tmp`, then rename), mode 0600, and no temporary
  file is left on failure.

Schedule it with the bundled user units (Omarchy's own collectors refresh
every 15 minutes; the timer matches):

```bash
cp contrib/systemd/user/ai-usage-omarchy.{service,timer} ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ai-usage-omarchy.timer
```

The service runs `%h/.cargo/bin/ai-usage-tui` at `Nice=19` with idle IO;
edit `ExecStart` if the binary lives elsewhere (`command -v ai-usage-tui`).
See [`contrib/systemd/user/README.md`](../contrib/systemd/user/README.md).

The tab first appears at Omarchy's next rescan — its updater runs every
`refreshIntervalSec` (900 s by default) — or at once after
`omarchy-shell omarchy.agents refresh`; afterwards the panel watches the file.
The panel never reads `updatedAt`, so if the timer stops the tab keeps showing
its last numbers: check `systemctl --user status ai-usage-omarchy.timer`. To
remove the tab, disable the timer and
`rm ~/.local/state/omarchy/agents/usage/opencode.json` (one file per id).
Linux/Omarchy only — the action has no meaning elsewhere.

| Key | Action |
| --- | --- |
| `1` | Show today (local calendar day) |
| `2` | Show the trailing 7 days |
| `3` | Show the trailing 30 days |
| `4` | Show all history |
| `r` | Refresh now |
| `b` | Toggle the budgets panel |
| `t` | Toggle routing analytics |
| `p` | Toggle the project cost panel |
| `g` | Toggle spend over time |
| `w` | Toggle the burn-rate panel |
| `s` | Toggle the sessions panel |
| `l` | Toggle the subscription-limits panel (Omarchy) |
| `?` | Key reference overlay |
| `j` / `Down` | Select the next model |
| `k` / `Up` | Select the previous model |
| `q` / `Esc` / `Ctrl-C` | Quit |
