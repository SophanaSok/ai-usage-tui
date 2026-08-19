# ai-usage-tui

> A btop-inspired terminal dashboard for AI token usage, cost, and model-routing analytics.

[![CI](https://github.com/SophanaSok/ai-usage-tui/actions/workflows/ci.yml/badge.svg)](https://github.com/SophanaSok/ai-usage-tui/actions/workflows/ci.yml)
[![Release](https://github.com/SophanaSok/ai-usage-tui/actions/workflows/release.yml/badge.svg)](https://github.com/SophanaSok/ai-usage-tui/actions/workflows/release.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[Quick start](#quick-start) · [Install](#installation) · [Configuration](#configuration) · [Docs](#more-documentation) · [Contributing](CONTRIBUTING.md)

`ai-usage-tui` reads OpenCode's local usage database, can journal completed
Ollama responses, and presents the combined data in an interactive TUI or as
JSON, CSV, or plain text. It tracks requests, input/output/reasoning/cache
tokens, cost provenance, budgets, and opt-in model-routing events.

![Dashboard showing token totals, model activity, and cost provenance](docs/assets/dashboard.png)

*Fixture-backed demo data from `tests/fixtures/opencode_test.db`.*

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

- Usage grouped by provider and model, across OpenCode, Claude Code, and Ollama
- Input, output, reasoning, cache-read, and cache-write tokens
- Today (local calendar day), trailing 7-day, trailing 30-day, all-time, or custom-day ranges
- `LOCAL`, `CLOUD`, `FREE`, `PAID`, and `UNKNOWN` classifications
- Provider-reported, calculated, estimated, free, local, or unavailable cost
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

## Prerequisites

- **Data:** An OpenCode SQLite database (default:
  `~/.local/share/opencode/opencode.db`) and/or journaled Ollama usage
- **Build (optional):** Stable Rust via [rustup](https://rustup.rs/)
- **Platforms:** Linux, macOS, and Windows x86_64 prebuilts from
  [GitHub Releases](https://github.com/SophanaSok/ai-usage-tui/releases)

A missing OpenCode database is not fatal. The dashboard starts with no
OpenCode rows and can still display journaled Ollama usage.

## Quick start

Download a prebuilt binary, put it on your `PATH`, and run the dashboard:

```sh
# Linux x86_64 example — replace VERSION with the latest release tag
VERSION=v0.2.0
curl -fsSL "https://github.com/SophanaSok/ai-usage-tui/releases/download/${VERSION}/ai-usage-tui-${VERSION}-x86_64-linux.tar.gz" \
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

| Platform | Archive name pattern |
| --- | --- |
| Linux x86_64 | `ai-usage-tui-VERSION-x86_64-linux.tar.gz` |
| macOS x86_64 | `ai-usage-tui-VERSION-x86_64-macos.tar.gz` |
| Windows x86_64 | `ai-usage-tui-VERSION-x86_64-windows.zip` |

macOS example:

```sh
VERSION=v0.2.0
curl -fsSL "https://github.com/SophanaSok/ai-usage-tui/releases/download/${VERSION}/ai-usage-tui-${VERSION}-x86_64-macos.tar.gz" \
  | tar xz
install -m 755 ai-usage-tui /usr/local/bin/
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
cargo build --release
./target/release/ai-usage-tui
```

## Data sources

```text
OpenCode DB / Claude Code logs / Ollama journal
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

The dashboard refreshes every 30 seconds by default. OpenCode and journal
collectors run in the background at their configured intervals.

The main view combines summary metrics, token-flow breakdown, and per-model
activity. Press `b` for budget status or `t` for routing analytics:

| View | Keys | Screenshot |
| --- | --- | --- |
| Model activity | default | [dashboard](docs/assets/dashboard.png) |
| Budgets | `b` | [budgets](docs/assets/budgets.png) |
| Routing | `t` | [routing](docs/assets/routing.png) |

| Key | Action |
| --- | --- |
| `1` | Show today (local calendar day) |
| `2` | Show the trailing 7 days |
| `3` | Show the trailing 30 days |
| `4` | Show all history |
| `r` | Refresh now |
| `b` | Toggle the budgets panel |
| `t` | Toggle routing analytics |
| `j` / `Down` | Select the next model |
| `k` / `Up` | Select the previous model |
| `q` / `Esc` | Quit |

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
selected range, and usage rows. Usage CSV columns are:

```text
provider,model,category,cost_status,requests,input_tokens,output_tokens,
reasoning_tokens,cache_read_tokens,cache_write_tokens,cost,created
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

[collectors.opencode]
enabled = true
interval = 30

[collectors.journal]
enabled = true
interval = 60

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
spend. Budget periods begin at local midnight for daily budgets and the first
day of the current local month for monthly budgets — the same boundaries the
dashboard's `TODAY` range uses, so the two always agree.

The CLI accepts `--webhook URL` and config accepts `budgets.webhook`, but the
current command and TUI do not dispatch webhook requests.

## Model-routing analytics

Routing events are separate, opt-in records for evaluating model-selection
outcomes. Record an event as JSON on stdin:

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
| `XDG_CONFIG_HOME` | Base directory for the default config path |
| `XDG_DATA_HOME` | Base directory for default database, journal, and cache paths |

## Privacy and network behavior

- OpenCode data is read locally from SQLite in read-only mode.
- Ollama journaling stores usage metadata, not prompt or response content.
- Routing events contain only the JSON fields supplied by the caller.
- Prompts, completions, API keys, credentials, and interaction content are not
  collected.
- Normal dashboard and export operation does not require a network request.
- `--refresh-pricing`, `--refresh-zen`, and an enabled `zen_pricing`
  background collector make outbound requests to OpenCode/Zen endpoints.

Default local storage paths (when the corresponding XDG variable is unset):

| Data | Path |
| --- | --- |
| OpenCode usage, read-only | `~/.local/share/opencode/opencode.db` |
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
  Windows), or pass explicit paths with `--db`, `--journal`, and `--claude-dir`.
- **No Claude Code rows:** confirm `~/.claude/projects` exists and contains
  `.jsonl` files, or point at the right directory with `--claude-dir PATH`.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo build --release
```

Regenerate README screenshots on a machine with an X11 desktop:

```sh
./scripts/capture-readme-screenshots.sh
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
