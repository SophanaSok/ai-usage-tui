# systemd user units

Two timers, each running one explicit `ai-usage-tui` command. Neither is installed by any
package; copying the files is the opt-in. Both need an absolute `ExecStart` path: a user unit
does not see the shell's `PATH`, so if the binary is not at `%h/.cargo/bin/ai-usage-tui`
(`command -v ai-usage-tui`), edit the line before `daemon-reload`.

| Unit | Runs | When | Platform |
| --- | --- | --- | --- |
| `ai-usage-omarchy.timer` | `--omarchy-record` | 2 min after login, then every 15 min | Linux/Omarchy |
| `ai-usage-update.timer` | `--check-update` | 5 min after login, then daily | Linux |

## `ai-usage-update.timer` — keep the release notice current

`ai-usage-tui --check-update` asks GitHub for the latest release tag and caches it where the
dashboard header reads it at startup. The dashboard never makes that request itself; this timer
is the only thing that makes it recur, which is why it lives here and not in the config file.
A plain GET of a public endpoint: no usage data, no identifiers, no query parameters.

```bash
cp ai-usage-update.service ai-usage-update.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ai-usage-update.timer
```

Verify:

```bash
systemctl --user list-timers ai-usage-update.timer
systemctl --user start ai-usage-update.service       # run once now
journalctl --user -u ai-usage-update.service         # "Latest release vX.Y.Z — ...; cached at ..."
```

A run with no network fails, and the journal says so; the header keeps the last answer, which
can only understate. Uninstall:

```bash
systemctl --user disable --now ai-usage-update.timer
rm ~/.config/systemd/user/ai-usage-update.{service,timer}
systemctl --user daemon-reload
rm ~/.local/share/ai-usage-tui/update-check.json     # the header goes quiet
```

## `ai-usage-omarchy.timer` — publish into Omarchy's agents panel

`ai-usage-omarchy.service` runs `ai-usage-tui --omarchy-record` once; the timer fires it
2 minutes after login and every 15 minutes after that, matching Omarchy's own collectors.
Linux/Omarchy only.

```bash
cp ai-usage-omarchy.service ai-usage-omarchy.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ai-usage-omarchy.timer
```

Verify:

```bash
systemctl --user list-timers ai-usage-omarchy.timer
systemctl --user start ai-usage-omarchy.service      # run once now
journalctl --user -u ai-usage-omarchy.service        # "Wrote Omarchy record ... (N requests, M budget meters)"
ls ~/.local/state/omarchy/agents/usage/               # opencode.json (one file per [omarchy] records id)
```

The tab appears at Omarchy's next rescan (every 900 s by default) or right after
`omarchy-shell omarchy.agents refresh`. The panel never reads `updatedAt`, so a stopped timer
leaves stale numbers on screen — `systemctl --user status ai-usage-omarchy.timer` is the check.

Uninstall:

```bash
systemctl --user disable --now ai-usage-omarchy.timer
rm ~/.config/systemd/user/ai-usage-omarchy.{service,timer}
systemctl --user daemon-reload
rm ~/.local/state/omarchy/agents/usage/opencode.json   # and ollama.json if configured
```
