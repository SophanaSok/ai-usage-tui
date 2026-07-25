# ai-usage-tui

![CI](https://github.com/SophanaSok/ai-usage-tui/actions/workflows/ci.yml/badge.svg)
![Release](https://github.com/SophanaSok/ai-usage-tui/actions/workflows/release.yml/badge.svg)
![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Version](https://img.shields.io/badge/version-0.2.0-green.svg)

> Track your AI token usage and costs across local and hosted providers — without leaving the terminal.

![TUI Screenshot](docs/screenshot.png)

`ai-usage-tui` is a btop-inspired terminal dashboard that reads usage data from
the OpenCode SQLite database, journals Ollama responses, and estimates costs using
the OpenCode Zen pricing table. It supports budgets, alerts, model-routing analytics,
and background collectors that keep the UI responsive.

## Table of Contents

- [Quick Start](#quick-start)
- [Installation](#installation)
- [Usage](#usage)
  - [Interactive TUI](#interactive-tui)
  - [Non-Interactive CLI](#non-interactive-cli)
- [Configuration](#configuration)
- [Budgets](#budgets)
- [Routing Analytics](#routing-analytics)
- [Ollama Journaling](#ollama-journaling)
- [Categories & Cost Provenance](#categories--cost-provenance)
- [Data Storage & Privacy](#data-storage--privacy)
- [FAQ](#faq)
- [Documentation](#documentation)

## Quick Start

```sh
# Install (pick one):
cargo install --path . --locked     # from source
sudo dpkg -i ai-usage-tui-*.deb     # Linux .deb
brew install ai-usage-tui           # macOS

# Run the dashboard:
ai-usage-tui

# Or get JSON output for scripts:
ai-usage-tui --once --json
```

That's it. The dashboard reads from `~/.local/share/opencode/opencode.db` by default.

## Installation

### From source
```sh
git clone https://github.com/SophanaSok/ai-usage-tui.git
cd ai-usage-tui
cargo install --path . --locked
```

### Linux

| Format | Command |
|--------|---------|
| `.deb` | `sudo dpkg -i ai-usage-tui-*.deb` |
| `.rpm` | `sudo rpm -i ai-usage-tui-*.rpm` |
| `.tar.gz` | `tar xzf ai-usage-tui-*.tar.gz && sudo cp ai-usage-tui /usr/local/bin/` |

### macOS
```sh
brew install ai-usage-tui
# Or download the .tar.gz from GitHub Releases
```

### Windows
```sh
scoop install ai-usage-tui
# Or: choco install ai-usage-tui
# Or: download the .zip from GitHub Releases
```

## Usage

### Interactive TUI

```sh
ai-usage-tui
```

The dashboard shows a live overview of your AI token usage:

- **Header**: current time range, last refresh, data source status
- **Metrics**: total tokens, requests, and per-category breakdowns
- **Token Flow**: input/output/reasoning/cache tokens + estimated paid cost
- **Model Activity**: per-model table with provider, class, tokens, cost, requests

Press a key to switch views:

| Key | Action |
|-----|--------|
| `1` | Today |
| `2` | Last 7 days |
| `3` | Last 30 days |
| `4` | All time |
| `r` | Refresh data |
| `b` | Toggle budgets panel |
| `t` | Toggle routing panel |
| `j` / `↓` | Select next model |
| `k` / `↑` | Select previous model |
| `q` / `Esc` | Quit |

### Non-Interactive CLI

For scripts, cron jobs, and CI pipelines:

```sh
# JSON output (great for piping to jq)
ai-usage-tui --once --json

# CSV export
ai-usage-tui --once --csv usage.csv

# Filter by time range, provider, or model
ai-usage-tui --once --json --days 14 --provider opencode
ai-usage-tui --once --json --today --model gpt-5.6-luna

# Refresh the Zen pricing table from the docs page
ai-usage-tui --refresh-pricing

# Check budget thresholds (exits 1 if any exceeded — perfect for cron)
ai-usage-tui --check-budgets

# Export routing analytics
ai-usage-tui --routing-json
ai-usage-tui --routing-csv routing.csv
```

**Database location**: defaults to `~/.local/share/opencode/opencode.db`. Override with:

```sh
ai-usage-tui --db /path/to/opencode.db
# Or:
OPENCODE_DB_PATH=/path/to/opencode.db ai-usage-tui
```

## Configuration

An optional TOML file at `~/.config/ai-usage-tui/config.toml`. CLI flags override config values.
See [`examples/config.toml`](examples/config.toml) for a full annotated example.

```toml
refresh_interval = 30
days = 7

# Background collectors (TUI mode only)
[collectors.opencode]
enabled = true
interval = 30

[collectors.journal]
enabled = true
interval = 60

[collectors.zen_pricing]
enabled = false
interval = 3600

# Budget alerts
[[budgets.entry]]
scope = "global"           # "global" | "provider" | "model"
period = "monthly"         # "daily" | "monthly"
limit = 50.0               # USD
# warn = 75.0               # warn at 75% (default)
# critical = 90.0          # critical at 90% (default)

[[budgets.entry]]
scope = "model"
name = "gpt-5.6-sol"
period = "monthly"
limit = 20.0
```

## Budgets

Set spend limits per provider, per model, or globally. The TUI shows an alert banner
when thresholds are reached, and you can dispatch alerts to a webhook:

```toml
[budgets]
webhook = "https://hooks.example.com/budget-alerts"
```

Check from CLI (great for cron or CI):

```sh
ai-usage-tui --check-budgets
# Exits 0 if all budgets OK, exits 1 if any threshold exceeded
# Prints JSON: {"budgets": 2, "alerts": [...]}
```

Or override the webhook URL:

```sh
ai-usage-tui --check-budgets --webhook https://my-hook.example.com/alerts
```

## Routing Analytics

Track which agents and models were used for each development task:

```sh
echo '{"agent":"@heavy","model":"glm-5.2:cloud","task":"refactor","phase":"implementation","provider":"ollama","tokens":15000,"cost":0.02,"retries":0,"escalations":0,"test_result":true,"review_defects":0}' \
  | ai-usage-tui --record-routing
```

View in the TUI with the `t` key — see cost per agent, retry rate, and defect rate.
Export for reporting:

```sh
ai-usage-tui --routing-json     # JSON output
ai-usage-tui --routing-csv routing.csv   # CSV file
```

See [`docs/routing-analytics.md`](docs/routing-analytics.md) for the full schema.

## Ollama Journaling

Ollama doesn't keep a usage history. This tool provides an opt-in journal:

```sh
# Record a completed Ollama response (non-streaming):
curl -s http://localhost:11434/api/generate \
  -d '{"model":"qwen3-coder:30b","prompt":"hello","stream":false}' \
  | ai-usage-tui --record-ollama

# For streaming responses, pipe the newline-delimited response stream:
ollama run qwen3-coder:30b "hello" --json \
  | ai-usage-tui --record-ollama
```

The journal stores only token counts and model metadata — no prompts or completions.
Override the journal location with `--journal PATH` or `AI_USAGE_JOURNAL_PATH`.

## Categories & Cost Provenance

Every usage event is classified into one of five categories:

| Category | Meaning | Example |
|---------|---------|---------|
| **LOCAL** | Local provider (Ollama, LM Studio, llama.cpp, vLLM, localhost) | `ollama/qwen3-coder:30b` |
| **FREE** | Explicitly free model | `nemotron-3-ultra-free`, `big-pickle` |
| **PAID** | Usage with known cost (provider-reported or calculated) | `opencode/gpt-5.6-luna` |
| **CLOUD** | Cloud-routed without authoritative cost | `ollama-cloud/glm-5.2:cloud` |
| **UNKNOWN** | No pricing metadata available | unrecognized model |

Every cost is labeled with its provenance:

| Status | Meaning |
|--------|---------|
| `reported` | Cost reported by the provider |
| `calculated` | Cost calculated from token counts × pricing |
| `estimated` | Cost estimated from bundled pricing table |
| `free` | Model is free, cost = $0 |
| `local` | Local model, no cloud cost |
| `unavailable` | No pricing data — shown as "UNKNOWN COST", never as $0 |

**A missing cost is never displayed as a paid zero.**

## Data Storage & Privacy

| What | Where | What's stored |
|------|-------|---------------|
| OpenCode usage | `~/.local/share/opencode/opencode.db` (read-only) | Read by the tool, never written |
| Ollama journal | `~/.local/share/ai-usage-tui/usage.db` | Token counts + model metadata only |
| Routing events | Same journal, `routing_event` table | Agent, model, task, tokens, cost — no prompts |
| Zen pricing cache | `~/.local/share/ai-usage-tui/zen-pricing.toml` | Model pricing per 1M tokens |
| Config | `~/.config/ai-usage-tui/config.toml` | Your settings |

**The tool never stores:**
- Prompts or completions
- API keys or credentials
- Content of any AI interaction

All data stays on your machine. No telemetry, no phone-home, no analytics sent anywhere
(unless you configure a budget webhook URL).

## FAQ

<details>
<summary><b>Why is my cost showing as "UNKNOWN COST"?</b></summary>

The model isn't in the pricing table. Run `ai-usage-tui --refresh-pricing` to fetch
the latest Zen pricing, or check if the model ID matches a known name in
`pricing/zen.toml`.
</details>

<details>
<summary><b>How do I track Ollama usage?</b></summary>

Ollama doesn't keep history. Use `--record-ollama` to journal each completed response
(see [Ollama Journaling](#ollama-journaling)). The journal stores token counts only —
no prompts or completions.
</details>

<details>
<summary><b>Can I use this without OpenCode?</b></summary>

Yes, but you'll only see data from the Ollama journal and routing events. The OpenCode
collector is the primary data source. If you don't use OpenCode, disable it in config:
`[collectors.opencode] enabled = false`.
</details>

<details>
<summary><b>How do I set up budget alerts in cron?</b></summary>

```sh
# Check every hour, alert if any budget exceeded:
0 * * * * ai-usage-tui --check-budgets >> /var/log/budget.log || send-alert.sh
```

Or configure a webhook in your config and the TUI will dispatch alerts automatically.
</details>

<details>
<summary><b>Where is my data stored?</b></summary>

Everything is local. See [Data Storage & Privacy](#data-storage--privacy) above.
No data leaves your machine unless you configure a budget webhook URL.
</details>

<details>
<summary><b>How do I contribute?</b></summary>

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for guidelines. Run `cargo test &&
cargo clippy --all-targets --all-features -- -D warnings` before submitting a PR.
</details>

## Documentation

| Document | Description |
|----------|-------------|
| [Architecture](docs/architecture.md) | System design, data flow, and privacy boundary |
| [Background Collectors](docs/background-collectors.md) | Collector trait, threading model, shutdown |
| [Data Model](docs/data-model.md) | Normalized usage event + routing event schema |
| [Provider Support](docs/provider-support.md) | OpenCode, Ollama, Zen, and cloud providers |
| [Routing Analytics](docs/routing-analytics.md) | Routing event schema and aggregation logic |
| [Release Process](docs/release-process.md) | How releases are built and published |
| [Phase Status](docs/phase-status.md) | Implementation progress tracker |
| [Model Routing](MODEL_ROUTING.md) | Development agent routing policy |
| [Contributing](CONTRIBUTING.md) | Contribution guidelines |
| [Security](SECURITY.md) | Security policy |
| [Changelog](CHANGELOG.md) | Version history |

## License

MIT — see [LICENSE](LICENSE).