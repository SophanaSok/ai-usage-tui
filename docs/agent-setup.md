# Setting ai-usage-tui up from an agent

You are reading this because someone asked you to configure `ai-usage-tui` for them: a budget, a
data source, the Claude Code hook or status line, a timer. It is printed by
`ai-usage-tui --agent-guide setup`. To *read* their usage instead, run `ai-usage-tui --agent-guide`.

The tool never edits another program's files, and it has no install command. You make the
change; the tool tells you whether it worked. That division is deliberate, so keep to it:

- **Show the change and ask before you edit anything outside this tool's own config** --
  `~/.claude/settings.json`, systemd units, a crontab. Say what it does and how to undo it.
- **If you cannot edit a file** (agents are often denied their own settings file), give the user
  the exact block below and where it goes. Do not work around a denial.
- **Say so when something uses the network.** Nothing does by default.

## 1. What reads, what writes, what uses the network

Everything not listed here only reads local files and prints.

| Command | Writes | Network |
| --- | --- | --- |
| `--record-ollama`, `--record-usage`, `--record-event`, `--record-routing`, `--claude-code-hook` | the journal, `usage.db` | no |
| `--statusline` | `statusline-limits.json` in the data directory | no |
| `--omarchy-record` | one record in Omarchy's agents directory | no |
| `--csv PATH`, `--routing-csv PATH` | that file (`-` is stdout) | no |
| `--refresh-pricing`, `--refresh-zen` | a pricing or catalog cache | yes: opencode.ai |
| `--check-update` | `update-check.json` | yes: api.github.com |
| `--doctor` | nothing, unless `[update] check = true`, then as `--check-update` | only then |
| `--check-budgets`, and the dashboard | nothing | only if a budget `webhook` is set: it POSTs alerts there |
| the dashboard, with `[collectors.zen_pricing] enabled = true` (off by default) | the pricing cache | yes: opencode.ai |

`ai-usage-tui --doctor` prints every path in force -- the config file, each data source, the
journal, the caches, and under `THIS BUILD` the binary's own absolute path and how it was
installed. Run it first; you will need those paths below.

## 2. The config file

`--doctor` names it under `CONFIG` (usually `~/.config/ai-usage-tui/config.toml`). No file is a
valid state. `ai-usage-tui --print-config` prints an annotated example of every key.

- **Never write the example out whole.** Its three `[[budgets.entry]]` tables are live samples:
  saved as-is, the user gets a $50 monthly budget they never asked for. Copy the keys you need.
- **Unknown keys are refused**, not ignored, and so is a table name that is not a source. The
  error names the key. Do not invent keys: if `--print-config` does not show it, it does not exist.
- **Validate by running `ai-usage-tui --doctor`.** A config that does not parse fails every
  command with `Error: <path>: <what is wrong>`; a good one shows `loaded` and the budget count.
  Read the file first and edit it in place -- never overwrite a config you have not read.
- Flags beat the file, and the file beats environment variables, for the same setting.

What people ask for:

```toml
days = 30                      # default range, in days

[collectors.gemini]
enabled = false                # stop reading a source, everywhere

[collectors.claude_code]
billing = "api"                # auto | subscription | api -- only when auto gets it wrong

[[budgets.entry]]
scope = "global"               # global | provider | model
period = "monthly"             # daily | monthly (calendar month, local time)
limit = 50.0                   # dollars
# name = "anthropic"           # required for provider and model scopes, refused for global
# warn = 75.0                  # percent of limit; default 75
# critical = 90.0              # default 90; warn must be below it
```

**Before you add a budget, check how the user is billed.** `ai-usage-tui --summary-json` has a
`detail` per source saying how billing was decided. A budget counts dollars, and usage billed
against a subscription (`quota`) has none -- so on a Claude Max or ChatGPT plan a budget counts
none of that work and warns about nothing. Tell the user that instead of setting one up; what
they want is in `limits` (how much of each plan window is used). Do not set `billing = "api"`
to make a budget "work": that prices plan usage at API rates and calls it spend.

To check budgets on a schedule, `ai-usage-tui --check-budgets` exits non-zero when any budget
has reached its `warn` level. Setting `webhook` makes every check POST to that URL.

## 3. Claude Code: routing data from test runs

Optional. With this hook, every test command Claude Code runs in Bash is journaled as pass or
fail against the model that ran it, which is what fills `routing` in the summary. It reads the
hook's payload and, from the session transcript, the model and token counts of the requests
behind the run -- usage blocks only, never message content. Add to `~/.claude/settings.json` (every
project) or a project's `.claude/settings.json`:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "ai-usage-tui --claude-code-hook",
            "timeout": 30
          }
        ]
      }
    ],
    "PostToolUseFailure": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "ai-usage-tui --claude-code-hook",
            "timeout": 30
          }
        ]
      }
    ]
  }
}
```

**Merge, do not replace.** If `hooks.PostToolUse` or `hooks.PostToolUseFailure` already exists,
append this entry to the existing array. (`jq -s '.[0] * .[1]'` replaces arrays, and would delete
the user's other hooks.) If `ai-usage-tui` is not on the PATH Claude Code runs with, use the
absolute path from `--doctor`.

Verify: a new Claude Code session must run a test command *bare* -- `cargo test`, `pytest`,
`npm test`. A run piped into `tail` or `head` is deliberately not recorded, because the pipe hides
the exit status. Then `ai-usage-tui --routing-json` shows `events` one higher. To remove it,
delete the two entries; events already journaled stay until the journal is deleted.

## 4. Claude Code: subscription limits from the status line

Optional, and usually unnecessary: the tool already reads the limits Claude Code caches in
`~/.claude.json`. The status line gives fresher numbers while a session is open.

```json
{
  "statusLine": {
    "type": "command",
    "command": "ai-usage-tui --statusline"
  }
}
```

`statusLine` is a single object. **If the user already has one, stop and ask** -- this would
replace it. It only runs in an interactive session, so you cannot verify it from a headless one:
after the user's next session, `--doctor` shows `statusline  found` under `LIMITS`. To remove it,
delete the key.

## 5. Timers (systemd user units)

Optional. A user unit does not see the shell's PATH, so **replace the `ExecStart` path with the
absolute path `--doctor` prints**. Save both files of a pair in `~/.config/systemd/user/`, then
`systemctl --user daemon-reload` and `systemctl --user enable --now <name>.timer`. Check with
`systemctl --user list-timers` and `journalctl --user -u <name>.service`. To remove:
`systemctl --user disable --now <name>.timer`, delete the two files, reload. On macOS or without
systemd, schedule the same command with launchd or cron.

A daily release check -- **uses the network**, one GET to GitHub; installing it is the consent.
`ai-usage-update.service`:

```ini
# Ask GitHub for the latest ai-usage-tui release tag and cache it for the dashboard header.
#
# This is the one recurring network request in the tool's vicinity, and it is made by this
# unit, not by the dashboard: installing the timer is the consent. A plain GET of a public
# endpoint -- no usage data, no identifiers, no query parameters. ExecStart needs an absolute
# path: a user-session unit does not see ~/.cargo/bin. Adjust it if the binary is installed
# elsewhere (`command -v ai-usage-tui`).
[Unit]
Description=Cache the latest ai-usage-tui release tag for the dashboard header
# Wait for the network rather than fail on the first tick after a resume.
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=%h/.cargo/bin/ai-usage-tui --check-update
Nice=19
IOSchedulingClass=idle
CPUWeight=20
```

`ai-usage-update.timer`:

```ini
# Enable with: systemctl --user enable --now ai-usage-update.timer
# Daily is plenty: releases are not that frequent, and the header only ever names the tag.
[Unit]
Description=Daily ai-usage-tui release check

[Timer]
OnBootSec=5min
OnUnitActiveSec=1d
RandomizedDelaySec=1h
Persistent=true

[Install]
WantedBy=timers.target
```

Omarchy only: keep the agents panel's record current. `ai-usage-omarchy.service`:

```ini
# Regenerate ai-usage-tui's record for Omarchy's agents panel.
#
# ExecStart needs an absolute path: a user-session unit does not see ~/.cargo/bin. Adjust it if
# the binary is installed elsewhere (`command -v ai-usage-tui`). The record is written only by
# this explicit action; nothing else in ai-usage-tui writes into Omarchy's state directory.
[Unit]
Description=Write ai-usage-tui usage and budgets to Omarchy's agents panel

[Service]
Type=oneshot
ExecStart=%h/.cargo/bin/ai-usage-tui --omarchy-record
# A 2-core laptop should not notice this running.
Nice=19
IOSchedulingClass=idle
CPUWeight=20
```

`ai-usage-omarchy.timer`:

```ini
# Enable with: systemctl --user enable --now ai-usage-omarchy.timer
# Omarchy's own collectors refresh every 15 minutes; matching them keeps the tab current.
[Unit]
Description=Refresh ai-usage-tui's record for Omarchy's agents panel

[Timer]
OnBootSec=2min
OnUnitActiveSec=15min
RandomizedDelaySec=60
Persistent=true

[Install]
WantedBy=timers.target
```

## 6. A tool this one does not read

`ai-usage-tui --summary-json` lists what it reads under `sources`. Anything else that logs its own
token counts can be fed in through `--record-event`; `ai-usage-tui --agent-guide extend` has the
keys, a worked adapter and the rule that matters most: **never estimate a token count**. If the
tool does not measure them, there is nothing to record, and saying so is the right answer.

## 7. Undoing all of it

The config file, the journal and the caches are the only things this tool keeps; `--doctor`
prints where. Deleting the data directory forgets journaled usage and nothing else: the agents'
own logs are never touched. Remove hooks, the status line and timers as above.
