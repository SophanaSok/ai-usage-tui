# systemd user units for `--omarchy-record`

`ai-usage-omarchy.service` runs `ai-usage-tui --omarchy-record` once; `ai-usage-omarchy.timer`
fires it 2 minutes after login and every 15 minutes after that, matching Omarchy's own collectors.
Linux/Omarchy only.

## Install

```bash
cp ai-usage-omarchy.service ai-usage-omarchy.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ai-usage-omarchy.timer
```

`ExecStart` is `%h/.cargo/bin/ai-usage-tui`. A user unit does not see the shell's `PATH`, so the
path must be absolute; if the binary is installed elsewhere (`command -v ai-usage-tui`), edit the
line before `daemon-reload`.

## Verify

```bash
systemctl --user list-timers ai-usage-omarchy.timer
systemctl --user start ai-usage-omarchy.service      # run once now
journalctl --user -u ai-usage-omarchy.service        # "Wrote Omarchy record ... (N requests, M budget meters)"
ls ~/.local/state/omarchy/agents/usage/               # opencode.json (one file per [omarchy] records id)
```

The tab appears at Omarchy's next rescan (every 900 s by default) or right after
`omarchy-shell omarchy.agents refresh`. The panel never reads `updatedAt`, so a stopped timer
leaves stale numbers on screen — `systemctl --user status ai-usage-omarchy.timer` is the check.

## Uninstall

```bash
systemctl --user disable --now ai-usage-omarchy.timer
rm ~/.config/systemd/user/ai-usage-omarchy.{service,timer}
systemctl --user daemon-reload
rm ~/.local/state/omarchy/agents/usage/opencode.json   # and ollama.json if configured
```
