# ai-usage-tui

> A btop-inspired terminal dashboard for AI token usage, cost, and model-routing analytics.

[![CI](https://github.com/SophanaSok/ai-usage-tui/actions/workflows/ci.yml/badge.svg)](https://github.com/SophanaSok/ai-usage-tui/actions/workflows/ci.yml)
[![Release](https://github.com/SophanaSok/ai-usage-tui/actions/workflows/release.yml/badge.svg)](https://github.com/SophanaSok/ai-usage-tui/actions/workflows/release.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[Quick start](#quick-start) · [Install](#installation) · [Configuration](#configuration) · [Docs](#more-documentation) · [Contributing](CONTRIBUTING.md)

`ai-usage-tui` reads OpenCode's local usage database, Claude Code's session
logs, and Codex CLI's session logs, can journal completed Ollama responses, and
presents the combined data in
an interactive TUI or as JSON, CSV, or plain text. It tracks requests, input/output/reasoning/cache
tokens, cost provenance, budgets, and opt-in model-routing events.

![Dashboard showing token totals, model activity, and cost provenance](docs/assets/dashboard.png)

*Invented demo data, rendered off-screen by `scripts/render-readme-screenshots.sh`. No
real account, project, or spend appears in any image here.*

## Contents

- [What it shows](#what-it-shows)
- [Prerequisites](#prerequisites)
- [Quick start](#quick-start)
- [Installation](#installation)
- [Data sources](#data-sources)
- [Interactive dashboard](#interactive-dashboard)
- [Non-interactive output](#non-interactive-output)
- [Configuration](#configuration)
- [Budget checks](#budget-checks)
- [Model-routing analytics](#model-routing-analytics)
- [CLI reference](#cli-reference)
- [Privacy and network behavior](#privacy-and-network-behavior)
- [Troubleshooting](#troubleshooting)
- [Development](#development)
- [More documentation](#more-documentation)
- [License](#license)

## What it shows

- Usage grouped by provider and model, across OpenCode, Claude Code, Codex CLI, and Ollama
- Input, output, reasoning, cache-read, and cache-write tokens
- Today (local calendar day), trailing 7-day, trailing 30-day, all-time, or custom-day ranges
- `LOCAL`, `CLOUD`, `FREE`, `PAID`, and `UNKNOWN` classifications
- Provider-reported, calculated, estimated, free, local, quota-billed, or unavailable cost
- Daily and monthly budget status
- Routing aggregates including retries, escalations, tests, and review defects

Unknown cost is kept unknown rather than displayed as paid usage with a zero
cost. Local and explicitly free usage is excluded from budget spend.

| Category | Meaning |
| --- | --- |
| `LOCAL` | Usage identified as running on a local endpoint |
| `CLOUD` | Hosted or cloud-routed usage without authoritative cost |
| `FREE` | Usage from a model explicitly identified as free |
| `PAID` | Usage with a known billable cost |
| `UNKNOWN` | Usage without enough metadata to classify or price |

Cost status is reported separately from category: `reported` comes from the
provider, `calculated` or `estimated` comes from pricing data, `free` and
`local` are non-billable, and `unavailable` remains unknown.

`quota` is its own case: the usage is billed, but against an account quota or
GPU time rather than per token, so no per-request price exists to report. Ollama
Cloud is one example; Claude Code on a Pro or Max subscription is the other.
For subscription rows the API-list-rate figure is kept as `api_equivalent_cost`
and shown as `API-RATE EQUIV.` in the breakdown, but it is never summed into
cost or budgets. `quota` is deliberately **not** counted as a pricing gap —
doing so reported a correct refusal to invent a number as a failure to produce
one — and deliberately not rendered as `$0.00`. The header shows the volume
alongside the coverage figure so it cannot silently disappear.

## Prerequisites

- **Data:** An OpenCode SQLite database (default:
  `~/.local/share/opencode/opencode.db`), Claude Code session logs under
  `~/.claude/projects`, Codex CLI session logs under `~/.codex`, and/or
  journaled Ollama usage
- **Build (optional):** Stable Rust via [rustup](https://rustup.rs/)
- **Platforms:** Linux and macOS (x86_64 and aarch64) and Windows x86_64 prebuilts from
  [GitHub Releases](https://github.com/SophanaSok/ai-usage-tui/releases)

A missing OpenCode database is not fatal. The dashboard starts with no
OpenCode rows and can still display journaled Ollama usage.

## Quick start

Download a prebuilt binary, put it on your `PATH`, and run the dashboard:

```sh
VERSION=v0.4.1
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  SLUG=x86_64-linux   ;;
  Linux-aarch64) SLUG=aarch64-linux  ;;
  Darwin-arm64)  SLUG=aarch64-macos  ;;
  Darwin-x86_64) SLUG=x86_64-macos   ;;
esac
curl -fsSL "https://github.com/SophanaSok/ai-usage-tui/releases/download/${VERSION}/ai-usage-tui-${VERSION}-${SLUG}.tar.gz" \
  | tar xz
install -m 755 ai-usage-tui ~/.local/bin/   # or sudo install ... /usr/local/bin/

ai-usage-tui
```

If OpenCode stores its database elsewhere:

```sh
ai-usage-tui --db /path/to/opencode.db

# Equivalent environment-variable form
OPENCODE_DB_PATH=/path/to/opencode.db ai-usage-tui
```

See [Installation](#installation) for macOS and Windows archives, source
builds, and packaging templates.

## Installation

### Prebuilt release

Download the archive for your platform from
[GitHub Releases](https://github.com/SophanaSok/ai-usage-tui/releases),
extract it, and place `ai-usage-tui` (or `ai-usage-tui.exe`) on your `PATH`.
Checksums are published with each release.

**Match the archive to your machine's architecture.** Every published binary is verified with
`file` during the release build to confirm it is the architecture its name claims, so an
`x86_64` archive really does contain an x86_64 binary and will not run on Apple Silicon.

| Platform | Archive name pattern |
| --- | --- |
| Linux x86_64 | `ai-usage-tui-VERSION-x86_64-linux.tar.gz` |
| Linux aarch64 | `ai-usage-tui-VERSION-aarch64-linux.tar.gz` |
| macOS Apple Silicon | `ai-usage-tui-VERSION-aarch64-macos.tar.gz` |
| macOS Intel | `ai-usage-tui-VERSION-x86_64-macos.tar.gz` |
| Windows x86_64 | `ai-usage-tui-VERSION-x86_64-windows.zip` |
| Debian/Ubuntu | `ai-usage-tui-VERSION-amd64.deb`, `-arm64.deb` |
| Fedora/RHEL | `ai-usage-tui-VERSION-amd64.rpm`, `-arm64.rpm` |

macOS example (Apple Silicon — use `x86_64-macos` on an Intel Mac):

```sh
VERSION=v0.4.1
curl -fsSL "https://github.com/SophanaSok/ai-usage-tui/releases/download/${VERSION}/ai-usage-tui-${VERSION}-aarch64-macos.tar.gz" \
  | tar xz
install -m 755 ai-usage-tui /usr/local/bin/
```

Linux package example:

```sh
sudo dpkg -i ai-usage-tui-v0.4.1-amd64.deb      # Debian/Ubuntu
sudo rpm -i ai-usage-tui-v0.4.1-amd64.rpm       # Fedora/RHEL
```

On Windows, extract the zip and add the directory containing
`ai-usage-tui.exe` to your `PATH`.

### Package managers

Release packaging templates for Homebrew, Scoop, and Chocolatey live under
[`packaging/`](packaging/). Use GitHub Releases until a formula or manifest is
published to those registries.

### Build or install from source

Install the stable Rust toolchain with [rustup](https://rustup.rs/), clone this
repository, and run one of:

```sh
# Install to Cargo's binary directory
cargo install --path . --locked

# Or build without installing
cargo build --release --locked
./target/release/ai-usage-tui
```

## Data sources

```text
OpenCode DB / Claude Code logs / Codex logs / Ollama journal
        -> background collectors -> TUI or JSON/CSV export
```

### OpenCode

OpenCode collection is automatic. The database is opened read-only and only
assistant-message usage metadata is read. The default path follows
`XDG_DATA_HOME` when set, otherwise it is:

```text
~/.local/share/opencode/opencode.db
```

Select another database with `--db PATH`, the `db` config setting, or
`OPENCODE_DB_PATH`.

### Claude Code

Claude Code collection is automatic. Session logs are read from:

```text
~/.claude/projects/<project>/<session-id>.jsonl
```

Select another directory with `--claude-dir PATH`, the `claude_dir` config
setting, or `CLAUDE_PROJECTS_DIR`. Each log is tailed by byte offset, so a
poll reads only what was appended since the last one.

Only the `usage` block of each assistant message is parsed. Claude Code
transcripts contain prompts, completions, file contents, and anything a tool
printed — including secrets read from a `.env` — and none of that is read or
retained. Usage is attributed to a session and a project (the working
directory's last path segment).

**Billing.** Claude Code writes identical transcripts whether it runs on an API
key or a Pro/Max plan, and nothing on a usage line says which. Priced at list
rates, a subscription's traffic reads as real spend and trips budgets on money
that was never charged, so the collector decides how the account pays before
pricing runs. In order: an explicit `billing` setting; then, if any of
`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_USE_BEDROCK`, or
`CLAUDE_CODE_USE_VERTEX` is set in the environment, per-token; then, if Claude
Code's own `~/.claude.json` has an `oauthAccount` block, subscription, with the
plan named from its rate-limit tier (`default_claude_max_20x` → "Max 20x");
then the plan label in Omarchy's record for the agent, if Omarchy's agents
panel is present (see [Subscription limits](#subscription-limits-omarchy)),
subscription; otherwise per-token, with a visible "billing unknown" hint. Per-token rows are
priced `estimated` as before. Subscription rows carry `cost_status = quota`,
`cost = null`, and the list-rate figure as `api_equivalent_cost`.

Force the answer with `[collectors.claude_code] billing = "subscription"` or
`"api"` (default `"auto"`), or `--claude-billing MODE` on the command line. If
`~/.claude.json` is not at the default location — it follows `CLAUDE_CONFIG_DIR`,
and an overridden `claude_dir` derives it from two levels above the session-log
root — point at it with `config_json`. The decision is printed on the source
line so a wrong guess is visible: `Claude Code: ~/.claude/projects (N sessions)
· subscription Max 20x`, `· api billing`, or `· billing unknown — set
[collectors.claude_code] billing`.

Two caveats. The decision is made once per source, not per request, and it
applies to every Claude Code row in the window, including history from before
the plan or key changed. And a plan with *extra usage* enabled is dollar-billed
at API rates once it passes its limits, and those requests look exactly like
the ones inside the plan, so `api_equivalent_cost` is a ceiling on what such an
account was charged, not a spend figure.

### Codex CLI

Codex collection is automatic. Session logs ("rollouts", one JSONL file per
thread) are read from both of:

```text
~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<thread-id>.jsonl
~/.codex/archived_sessions/...
```

Select another Codex home with `--codex-dir PATH`, the `codex_dir` config
setting, or `CODEX_HOME`; `sessions/` and `archived_sessions/` are scanned
recursively beneath it. Each rollout is tailed by byte offset, so a poll reads
only what was appended since the last one. Files the CLI has compressed to
`.jsonl.zst` are skipped.

Only three line kinds are read: `session_meta` (thread id and working
directory), `turn_context` (the model in force), and the `token_count` event's
`last_token_usage` block. Rollouts also contain prompts, tool-call arguments
and outputs, and reasoning summaries — none of that is read or retained.
Usage is attributed to a session (the thread id) and a project (the working
directory).

Token conventions follow the CLI's own arithmetic. Codex reports
`cached_input_tokens` inside `input_tokens`, so the cached part is split out as
cache-read. It reports `reasoning_output_tokens` inside `output_tokens`, so
that part is split out as reasoning. Prompt-cache writes stay inside input,
because OpenAI bills them as ordinary input. Re-emitted events whose running
total did not move (rate-limit refreshes, resumes) and post-compaction
estimates are skipped. A forked thread copies its ancestor's history into the
new file, timestamps and all, so event identity is content-based
(`timestamp + call tokens + running total`) and the copy deduplicates against
the original.

**Billing.** As with Claude Code, a rollout looks the same on an API key and on
a ChatGPT plan. In order: an explicit `billing` setting; then, if
`OPENAI_API_KEY` or `CODEX_API_KEY` is set in the environment, per-token;
then the plan label in Omarchy's record for the agent, if Omarchy's agents
panel is present, subscription; otherwise per-token, with a visible "billing
unknown" hint. No Codex config
document is read — `~/.codex/auth.json` is a credential file and is never
opened — so `config_json` is rejected under `[collectors.codex]`. Force the
answer with `[collectors.codex] billing = "subscription"` or `"api"` (default
`"auto"`), or `--codex-billing MODE`. The decision is printed on the source
line: `Codex: ~/.codex (N sessions) · api billing` or `· billing unknown — set
[collectors.codex] billing`. The same line appends `· N token events disagree
with running totals` when the CLI's cumulative counter did not advance by a
call's own figure, so a change in what the CLI emits is visible rather than a
silent under-count.

Rows are `openai` / `PAID` and priced `estimated` from the bundled table, which
covers the `gpt-5`, `gpt-5.1`, `gpt-5.2`, `gpt-5.3-codex`, `gpt-5.4`, `gpt-5.5`,
and `gpt-5.6` families (including their `-codex`, `-mini`, `-nano`, and `-pro`
variants where published). A model absent from the table stays `unavailable`;
no rate is invented.

### Ollama

Ollama usage is opt-in. Pipe a completed response into
`--record-ollama`; the command stores token counts in the local journal.

```sh
curl -s http://localhost:11434/api/generate \
  -d '{"model":"qwen3-coder:30b","prompt":"hello","stream":false}' \
  | ai-usage-tui --record-ollama
```

Newline-delimited streaming responses also work:

```sh
curl -s http://localhost:11434/api/generate \
  -d '{"model":"qwen3-coder:30b","prompt":"hello","stream":true}' \
  | ai-usage-tui --record-ollama
```

For a stream, only the final event with `done: true` is recorded. Replaying the
same completed event does not duplicate it. The journal defaults to:

```text
~/.local/share/ai-usage-tui/usage.db
```

Override it with `--journal PATH`, the `journal` config setting, or
`AI_USAGE_JOURNAL_PATH`.

### Zen catalog and pricing

The bundled Zen pricing table is used to estimate cost when authoritative cost
is unavailable. These optional network commands update local caches and exit:

```sh
# Scrape the current Zen pricing table
ai-usage-tui --refresh-pricing

# Cache the OpenCode Zen model catalog
ai-usage-tui --refresh-zen
```

The model catalog is informational. Refreshing it does not create usage data.
Automatic pricing refresh is disabled unless enabled in the collector config.

## Interactive dashboard

Run without an output or action flag:

```sh
ai-usage-tui
ai-usage-tui --month --provider opencode
ai-usage-tui --days 14 --model qwen3-coder:30b
ai-usage-tui --refresh-interval 15
```

The dashboard refreshes every 30 seconds by default. OpenCode, Claude Code,
Codex, and journal collectors run in the background at their configured
intervals.

The main view combines summary metrics, token-flow breakdown, and per-model
activity. One other panel occupies the right-hand pane at a time; `?` lists every
key.

| View | Key | What it answers |
| --- | --- | --- |
| Model activity | default | Where did the tokens and the money go, by model |
| Budgets | `b` | How close am I to a limit I set |
| Routing | `t` | Is the expensive model earning its cost, and how often do sessions escalate |
| Projects | `p` | Which repository is this spend attributable to |
| Spend over time | `g` | What does the trend look like day by day |
| Burn rate | `w` | At this rate, when do I hit my budget |
| Sessions | `s` | Which individual runs cost the most |
| Limits | `l` | Subscription windows from Omarchy's agents panel: % used and reset countdown |

<details>
<summary>Screenshots of each panel</summary>

**Budgets** (`b`) — spend against every configured limit, and which one has gone over.

![The budgets panel](docs/assets/budgets.png)

**Routing** (`t`) — cost per delivered result per agent, above escalations derived from the
sessions themselves.

![The routing panel](docs/assets/routing.png)

**Projects** (`p`) — spend attributed to the working directory it came from. Usage with no
project, and no per-token price, is shown as `quota` rather than as `$0.00`.

![The project cost panel](docs/assets/projects.png)

**Spend over time** (`g`) — one bar per local calendar day, newest first.

![The spend-over-time panel](docs/assets/timeseries.png)

**Burn rate** (`w`) — the trailing hour, projected against each budget.

![The burn-rate panel](docs/assets/burn.png)

**Sessions** (`s`) — individual runs, ranked by cost, with the models each one used.

![The sessions panel](docs/assets/sessions.png)

</details>

### Subscription limits (Omarchy)

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

### Publishing to Omarchy's agents panel

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
See [`contrib/systemd/user/README.md`](contrib/systemd/user/README.md).

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

## Non-interactive output

Use one-shot mode in scripts and scheduled jobs:

```sh
# Human-readable rows
ai-usage-tui --once

# JSON to stdout
ai-usage-tui --json --week

# CSV to a file
ai-usage-tui --csv usage.csv --days 14

# Exact, case-insensitive provider and model filters
ai-usage-tui --json --all --provider opencode --model gpt-5.6-sol
```

`--json` and `--csv` imply `--once`. JSON includes the source description,
selected range, usage rows, and a `limits` array of Omarchy subscription
windows (see [Subscription limits](#subscription-limits-omarchy); empty when
there are none); each usage row also carries `project` and `session_id`
(`null` when unknown). Usage CSV columns are:

```text
provider,model,category,cost_status,requests,input_tokens,output_tokens,
reasoning_tokens,cache_read_tokens,cache_write_tokens,cost,created,project,
session_id,api_equivalent_cost
```

## Configuration

The optional TOML file defaults to:

```text
${XDG_CONFIG_HOME:-~/.config}/ai-usage-tui/config.toml
```

Use `--config PATH` to select another file. An explicitly selected file must
exist. Command-line values override config values; for data paths, config
values override environment variables and defaults.

```toml
refresh_interval = 30
days = 7
# claude_dir = "/home/user/.claude/projects"
# codex_dir = "/home/user/.codex"

[collectors.opencode]
enabled = true
interval = 30

[collectors.claude_code]
enabled = true
interval = 30
billing = "auto"                        # auto | subscription | api
# config_json = "/home/user/.claude.json"

[collectors.codex]
enabled = true
interval = 30
billing = "auto"                        # auto | subscription | api

[collectors.journal]
enabled = true
interval = 60

[omarchy]
# dir = "/home/user/.local/state/omarchy/agents/usage"
limits = true                           # read Omarchy's agents-panel records
# records = ["opencode"]                # what --omarchy-record writes (opencode, ollama)
# balance = false                       # also draw a budget as the panel's prepaid ledger
# balance_budget = "global/monthly"     # which budget, as <scope>/<period>

[[budgets.entry]]
scope = "global"
period = "monthly"
limit = 50.0
```

`warn` and `critical` are percentages of `limit`; they default to 75 and 90.
The complete annotated example — including data paths, filters, collectors, and
budget scopes — is in [`examples/config.toml`](examples/config.toml).

## Budget checks

Configured budgets appear in the TUI. To check them non-interactively:

```sh
ai-usage-tui --check-budgets
```

The command prints JSON and exits with status `1` when any budget is at the
warning, critical, or exceeded threshold. It exits with status `0` when all
budgets are below their warning thresholds or no budgets are configured.

Only usage with a reported, calculated, or estimated cost contributes to
spend. Subscription-billed Claude Code usage is `quota` and does not count, so
a budget scoped to `global`, `provider = "anthropic"`, or a Claude model sees
none of it; `[collectors.claude_code] billing = "api"` restores the per-token
accounting. A daily budget period begins at local midnight — the same boundary the
dashboard's `TODAY` range uses, so those two always agree. A monthly budget
period begins on the first day of the current local month, which is
deliberately **not** the dashboard's `3` / `--month` trailing-30-day range, so
a monthly budget's spend differs from the `30 DAYS` total.

When `--webhook URL` (or `webhook` in the `[budgets]` table) is set, actionable
alerts are POSTed as JSON with this shape:

```text
{tool, timestamp, alerts: [{scope, period, level, spend, limit, pct}]}
```

`--check-budgets` posts synchronously before exiting `1` and prints
`warning: budget webhook dispatch failed: …` on stderr if the POST fails. The
dashboard posts from a background thread on every refresh and logs a failed
POST when `AI_USAGE_LOG` is set. A repeat alert at the same level for the same
scope and period is suppressed for one hour, but that suppression is in-memory
only: a cron-driven `--check-budgets` re-POSTs on every run while a threshold
is breached, so run it no more often than you want to be notified.

## Model-routing analytics

This answers a question a usage total cannot: **is the expensive model actually
earning its cost on your work?** A model that costs 5x more but lands the change
on the first try can be the cheaper one. The panel ranks agent/model pairs by
**cost per delivered result** — dollars spent per passing test — alongside the
retry, escalation, and review-defect rates behind that figure.

A pair that never reported a test result shows `—`, not `0%`. Never having been
measured is not the same as failing everything.

The panel also shows an **escalations** block derived from usage already
collected, which needs no setup at all: how often a session reached for a
pricier model than it opened with, and what that cost. Derived and recorded data
are shown as separate blocks and never merged — nothing this tool inferred
should look like something your harness measured. See
[docs/routing-analytics.md](docs/routing-analytics.md).

Routing events are separate, opt-in records for evaluating model-selection
outcomes. Nothing records them for you — emit one per task from whatever drives
your agents. Record an event as JSON on stdin:

```sh
echo '{
  "agent":"@heavy",
  "model":"glm-5.2:cloud",
  "provider":"opencode",
  "task":"refactor",
  "phase":"implementation",
  "tokens":15000,
  "cost":0.02,
  "retries":1,
  "escalations":0,
  "test_result":true,
  "review_defects":0
}' | ai-usage-tui --record-routing
```

`agent`, `model`, `task`, and `tokens` are the useful minimum fields. Optional
fields include `provider`, `phase`, `category`, `cost_status`, `requests`,
`cost`, `retries`, `escalations`, `test_result`, `review_defects`, and a Unix
`created` timestamp. Missing optional counters default to zero, `requests`
defaults to one, and missing strings use empty or `unknown` values.

View aggregates with `t` in the TUI or export them:

```sh
ai-usage-tui --routing-json
ai-usage-tui --routing-csv routing.csv
```

Routing output is aggregated by agent, model, and provider. JSON also includes
retry, escalation, and defect rates. Routing CSV columns are:

```text
agent,model,provider,tasks,tokens,cost,retries,escalations,test_passes,
test_failures,review_defects
```

See [`docs/routing-analytics.md`](docs/routing-analytics.md) for the underlying
design and [`examples/model-routing.toml`](examples/model-routing.toml) for an
example routing policy. The policy is an orchestration example; the binary
does not load it automatically.

## CLI reference

| Option | Meaning |
| --- | --- |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print the version |
| `--once` | Collect once, print plain text, and exit |
| `--json` | Collect once and print usage JSON |
| `--csv PATH` | Collect once and write usage CSV |
| `--config PATH` | Load a specific TOML config file |
| `--db PATH` | Override the OpenCode database path |
| `--journal PATH` | Override the local journal path |
| `--claude-dir PATH` | Override the Claude Code session-log directory |
| `--claude-billing MODE` | How Claude Code usage is billed: `auto` (default), `subscription`, or `api`; overrides `[collectors.claude_code] billing` |
| `--codex-dir PATH` | Override the Codex home (`$CODEX_HOME`, else `~/.codex`); `sessions/` and `archived_sessions/` are read beneath it |
| `--codex-billing MODE` | How Codex usage is billed: `auto` (default), `subscription`, or `api`; overrides `[collectors.codex] billing` |
| `--omarchy-dir PATH` | Override where Omarchy's agents panel keeps its usage records (default `$XDG_STATE_HOME/omarchy/agents/usage`) |
| `--omarchy-record` | Write usage and budgets as a record for Omarchy's agents panel (`[omarchy] records`, default `opencode`) and exit |
| `--today` | Use today (local calendar day) |
| `--week` | Use the trailing 7 days (default) |
| `--month` | Use the trailing 30 days |
| `--days N` | Use the trailing `N` days; `N` must be greater than zero |
| `--all` | Use all available history |
| `--provider NAME` | Filter by exact provider name, ignoring case |
| `--model NAME` | Filter by exact model name, ignoring case |
| `--refresh-interval N` | Refresh the TUI every `N` seconds |
| `--record-ollama` | Read an Ollama response from stdin and journal it |
| `--refresh-zen` | Refresh the cached Zen model catalog and exit |
| `--refresh-pricing` | Refresh the Zen pricing cache and exit |
| `--check-budgets` | Print actionable budget alerts as JSON |
| `--webhook URL` | POST budget alerts to this URL (overrides `budgets.webhook`) |
| `--record-routing` | Read one routing event from stdin and journal it |
| `--routing-json` | Print aggregated routing analytics as JSON |
| `--routing-csv PATH` | Write aggregated routing analytics as CSV |

Recording, refresh, budget, usage export, and routing export modes are
single-purpose actions; do not combine action flags.

Environment variables:

| Variable | Meaning |
| --- | --- |
| `OPENCODE_DB_PATH` | OpenCode SQLite database path |
| `AI_USAGE_JOURNAL_PATH` | Usage and routing journal path |
| `CLAUDE_PROJECTS_DIR` | Claude Code session-log directory |
| `CLAUDE_CONFIG_DIR` | Claude Code config directory; logs are read from `$CLAUDE_CONFIG_DIR/projects` (`CLAUDE_PROJECTS_DIR` wins when both are set) |
| `CODEX_HOME` | Codex home; session logs are read from `sessions/` and `archived_sessions/` beneath it |
| `AI_USAGE_LOG` | Write diagnostics to a file — `1` for the default location, or a path. Off when unset. |
| `XDG_CONFIG_HOME` | Base directory for the default config path |
| `XDG_DATA_HOME` | Base directory for default database, journal, and cache paths |
| `XDG_STATE_HOME` | Base directory for Omarchy's agents-panel records (`omarchy/agents/usage` beneath it) |

On Windows, `USERPROFILE` (or `HOMEDRIVE` + `HOMEPATH`) stands in for `HOME`,
`LOCALAPPDATA` for `XDG_DATA_HOME`, and `APPDATA` for `XDG_CONFIG_HOME`.

## Privacy and network behavior

- OpenCode data is read locally from SQLite in read-only mode.
- Ollama journaling stores usage metadata, not prompt or response content.
- Routing events contain only the JSON fields supplied by the caller.
- Prompts, completions, API keys, credentials, and interaction content are not
  collected.
- Claude Code session transcripts contain source code and secrets; only the
  `usage` block of each line is parsed. A test plants a fake credential in a
  transcript and fails if it reaches a usage record.
- `~/.claude.json` is read to decide billing: only whether `oauthAccount` is
  present and its `userRateLimitTier` / `organizationRateLimitTier` strings.
  The file also holds the account's email, name, organisation, and per-project
  prompt history; none of that is retained or logged, and the parsed document
  is dropped at once. `.credentials.json` and `settings.json` are never read.
  The environment is checked only for the presence of `ANTHROPIC_API_KEY`,
  `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_USE_BEDROCK`, and
  `CLAUDE_CODE_USE_VERTEX`; their values are not read.
- Codex rollouts contain prompts, tool-call arguments and outputs, and
  reasoning summaries; only `session_meta`, `turn_context`, and the
  `token_count` block are parsed, under the same planted-credential test as
  Claude Code. `~/.codex/auth.json` is a credential file and is never opened;
  the environment is checked only for the presence of `OPENAI_API_KEY` and
  `CODEX_API_KEY`.
- Omarchy's agents-panel records are read-only display data: six fields per
  record (`id`, `name`, `updatedAt`, `ready`, `tierLabel`, `usageStatusText`,
  `limits`). The agents' credentials, Omarchy's probe cache, the record's
  `authHelpText` and token tallies are never read, no network request is made,
  and the reader writes nothing into the directory.
- `--omarchy-record` is the one write into Omarchy's directory, and only that
  explicit action performs it: `<id>.json` (`opencode` by default) holding
  token counts, model ids, request and session counts, and budget figures —
  never content, never a path. Ids that would overwrite Omarchy's own files
  (`claude`, `codex`, `fireworks`) are refused; the file is written
  atomically with mode 0600.
- Per-project attribution records the **working directory path** of each
  session, so `~/a/build` and `~/b/build` stay separate projects. The dashboard
  shows only the shortest name that distinguishes them, but `--json` and
  `--csv` export the full path — worth knowing before pasting an export into a
  ticket.
- Normal dashboard and export operation does not require a network request,
  and nothing is written outside the tool's own data directory unless
  `--omarchy-record` is run.
- `--refresh-pricing`, `--refresh-zen`, and an enabled `zen_pricing`
  background collector make outbound requests to OpenCode/Zen endpoints.

Default local storage paths (when the corresponding XDG variable is unset):

| Data | Path |
| --- | --- |
| OpenCode usage, read-only | `~/.local/share/opencode/opencode.db` |
| Claude Code config document, read-only (billing only) | `~/.claude.json` |
| Codex session logs, read-only | `~/.codex/sessions`, `~/.codex/archived_sessions` |
| Omarchy agents-panel records, read-only | `~/.local/state/omarchy/agents/usage` |
| Omarchy agents-panel record, written only by `--omarchy-record` | `~/.local/state/omarchy/agents/usage/<id>.json` |
| Ollama and routing journal | `~/.local/share/ai-usage-tui/usage.db` |
| Zen pricing cache | `~/.local/share/ai-usage-tui/zen-pricing.toml` |
| Zen model catalog | `~/.local/share/ai-usage-tui/zen-models.json` |
| Configuration | `~/.config/ai-usage-tui/config.toml` |

## Troubleshooting

- **No OpenCode rows:** check that the database exists at the displayed source
  path, or pass `--db PATH`.
- **No Ollama rows:** first pipe a completed response containing `done: true`
  through `--record-ollama`, then verify the journal path.
- **No cost shown:** the model may be local, free, absent from pricing data, or
  missing authoritative cost metadata. Try `--refresh-pricing`; unknown cost
  remains unavailable by design.
- **Config not applied:** verify TOML syntax and the path shown above. A custom
  `--config PATH` fails immediately when the file does not exist.
- **`could not determine a home directory`:** set `HOME` (or `USERPROFILE` on
  Windows), or pass explicit paths with `--db`, `--journal`, `--claude-dir`, and
  `--codex-dir`.
- **No Claude Code rows:** confirm `~/.claude/projects` exists and contains
  `.jsonl` files, or point at the right directory with `--claude-dir PATH`.
- **No Codex rows:** confirm `codex` has written `~/.codex/sessions` (or
  `$CODEX_HOME/sessions`), or point at the right home with `--codex-dir PATH`.
  Compressed `.jsonl.zst` rollouts are not read.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --locked
cargo build --release --locked
```

Regenerate the README images. They are rendered off-screen from invented demo data — no
terminal is opened and no screen is captured — so this works headlessly and gives the same
result on every machine:

```sh
./scripts/render-readme-screenshots.sh   # needs librsvg for rsvg-convert
```

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for contribution guidelines.

## More documentation

**User guides**

- [`docs/provider-support.md`](docs/provider-support.md) — provider support matrix
- [`docs/routing-analytics.md`](docs/routing-analytics.md) — routing analytics

**Contributor docs**

- [`docs/architecture.md`](docs/architecture.md) — architecture and data flow
- [`docs/background-collectors.md`](docs/background-collectors.md) — collector design
- [`docs/data-model.md`](docs/data-model.md) — data model and schema
- [`docs/release-process.md`](docs/release-process.md) — release process
- [`docs/phase-status.md`](docs/phase-status.md) — implementation status
- [`docs/roadmap.md`](docs/roadmap.md) — outstanding findings, conventions, and next steps
- [`docs/execution-log.md`](docs/execution-log.md) — development history
- [`MODEL_ROUTING.md`](MODEL_ROUTING.md) — development agent-routing policy
- [`SECURITY.md`](SECURITY.md) — security policy
- [`CHANGELOG.md`](CHANGELOG.md) — release notes

## License

MIT — see [`LICENSE`](LICENSE).
