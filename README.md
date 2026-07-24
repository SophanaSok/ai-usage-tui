# ai-usage-tui

A btop-inspired terminal dashboard for AI token usage across OpenCode providers.

## Current scope

- Reads assistant message usage from the OpenCode SQLite database.
- Groups usage into local, cloud, free, paid, and unknown categories.
- Tracks input, output, reasoning, cache-read, and cache-write tokens.
- Separates paid estimated cost from local and free usage.
- Supports today, 7-day, 30-day, and all-time views.
- **Background collectors** with configurable intervals (TUI stays responsive).
- **Budget tracking** with TUI alerts and webhook dispatch.
- **Model-routing analytics** with cost/retry/defect rate aggregation.
- `--refresh-pricing` scrapes live Zen pricing table.
- Includes a development model-routing policy in [`MODEL_ROUTING.md`](MODEL_ROUTING.md).
- Provides production-readiness, architecture, provider, and release documentation under [`docs/`](docs/).
- Tracks implementation progress in [`docs/phase-status.md`](docs/phase-status.md).

Local includes any provider or model whose recorded identity indicates a local endpoint, including Ollama, LM Studio, llama.cpp, vLLM, localhost, and loopback endpoints.

## Installation

### From source
```sh
cargo install --path . --locked
```

### Linux (.deb)
```sh
# Download from GitHub Releases, then:
sudo dpkg -i ai-usage-tui-*.deb
```

### macOS (Homebrew)
```sh
brew install ai-usage-tui
```

### Windows (Scoop)
```sh
scoop install ai-usage-tui
```

## Run

```sh
cargo run --release
```

For the installed CLI:

```sh
ai-usage-tui
ai-usage-tui --help
ai-usage-tui --version
ai-usage-tui --once --json
ai-usage-tui --once --csv usage.csv
ai-usage-tui --refresh-pricing
ai-usage-tui --check-budgets
ai-usage-tui --record-routing
ai-usage-tui --routing-json
ai-usage-tui --once --days 14 --provider opencode
```

The database defaults to `~/.local/share/opencode/opencode.db`. Override it with:

```sh
OPENCODE_DB_PATH=/path/to/opencode.db ai-usage-tui
ai-usage-tui --db /path/to/opencode.db
```

The dashboard refreshes every 30 seconds by default. Change it with `--refresh-interval N`.
Use `--days N`, `--provider NAME`, and `--model NAME` to filter non-interactive output and the dashboard.

## Configuration

An optional TOML configuration file is loaded from `~/.config/ai-usage-tui/config.toml`. Use `--config PATH` to select another file. Command-line options take precedence.

An annotated example is available at [`examples/config.toml`](examples/config.toml).

```toml
db = "/home/user/.local/share/opencode/opencode.db"
journal = "/home/user/.local/share/ai-usage-tui/usage.db"
refresh_interval = 30
days = 7
provider = "opencode"

[collectors.opencode]
enabled = true
interval = 30

[[budgets.entry]]
scope = "global"
period = "monthly"
limit = 50.0
```

## Ollama Journaling

Record a completed Ollama response from a non-streaming request:

```sh
curl -s http://localhost:11434/api/generate \
  -d '{"model":"qwen3-coder:30b","prompt":"hello","stream":false}' \
  | ai-usage-tui --record-ollama
```

For streaming responses, pipe the newline-delimited response stream. The journal records only the final completed event. Set `AI_USAGE_JOURNAL_PATH` or pass `--journal PATH` to override the default journal at `~/.local/share/ai-usage-tui/usage.db`.

## Budgets

Configure spend limits per provider, model, or globally. Alerts appear in the TUI banner and can be dispatched to a webhook.

```toml
[[budgets.entry]]
scope = "global"
period = "monthly"
limit = 50.0
```

Check budgets from CLI: `ai-usage-tui --check-budgets` (exits 1 if thresholds exceeded).
See [`examples/config.toml`](examples/config.toml) for full config.

## Routing Analytics

Track which agents and models were used for each task. Capture routing events:

```sh
echo '{"agent":"@heavy","model":"glm-5.2:cloud","task":"refactor","tokens":15000,"cost":0.02,"test_result":true}' | ai-usage-tui --record-routing
```

Export analytics: `ai-usage-tui --routing-json` or `ai-usage-tui --routing-csv routing.csv`.
View in TUI with the `t` key. See [`docs/routing-analytics.md`](docs/routing-analytics.md).

## Keys

`1` today, `2` 7 days, `3` 30 days, `4` all time, `r` refresh, `b` budgets, `t` routing, `j/k` navigate, `q` or `Esc` quit.

## Project Documentation

- [`architecture.md`](docs/architecture.md) — system architecture and data flow
- [`background-collectors.md`](docs/background-collectors.md) — background collector design
- [`data-model.md`](docs/data-model.md) — data model and schema
- [`provider-support.md`](docs/provider-support.md) — provider support matrix
- [`routing-analytics.md`](docs/routing-analytics.md) — routing analytics design
- [`release-notes.md`](docs/release-notes.md) — release notes
- [`phase-status.md`](docs/phase-status.md) — implementation phase status