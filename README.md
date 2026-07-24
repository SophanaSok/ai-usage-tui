# ai-usage-tui

A btop-inspired terminal dashboard for AI token usage across OpenCode providers.

## Current scope

- Reads assistant message usage from the OpenCode SQLite database.
- Groups usage into local, cloud, free, paid, and unknown categories.
- Tracks input, output, reasoning, cache-read, and cache-write tokens.
- Separates paid estimated cost from local and free usage.
- Supports today, 7-day, 30-day, and all-time views.
- Includes a development model-routing policy in [`MODEL_ROUTING.md`](MODEL_ROUTING.md).
- Provides production-readiness, architecture, provider, and release documentation under [`docs/`](docs/).
- Tracks implementation progress in [`docs/phase-status.md`](docs/phase-status.md).

Local includes any provider or model whose recorded identity indicates a local endpoint, including Ollama, LM Studio, llama.cpp, vLLM, localhost, and loopback endpoints.

## Run

```sh
cargo run --release
```

For the installed CLI, use:

```sh
ai-usage-tui
ai-usage-tui --help
ai-usage-tui --version
ai-usage-tui --once --json
ai-usage-tui --once --csv usage.csv
ai-usage-tui --refresh-zen
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
```

## Ollama Journaling

Record a completed Ollama response from a non-streaming request:

```sh
curl -s http://localhost:11434/api/generate \
  -d '{"model":"qwen3-coder:30b","prompt":"hello","stream":false}' \
  | ai-usage-tui --record-ollama
```

For streaming responses, pipe the newline-delimited response stream. The journal records only the final completed event. Set `AI_USAGE_JOURNAL_PATH` or pass `--journal PATH` to override the default journal at `~/.local/share/ai-usage-tui/usage.db`.

The journal stores model and usage metadata only. It does not store prompts, completions, or API keys. Historical Ollama usage before journaling was enabled cannot be reconstructed. The same path can be configured with `AI_USAGE_JOURNAL_PATH`.

Keys: `1` today, `2` 7 days, `3` 30 days, `4` all time, `r` refresh, `j/k` navigate, `q` or `Esc` quit.

## Development Routing

The repository includes [`MODEL_ROUTING.md`](MODEL_ROUTING.md) and an example [`examples/model-routing.toml`](examples/model-routing.toml) for assigning local, free, fast, balanced, and strong models to development phases.

## Data caveat

OpenCode records calculated cost for messages when provider pricing is available. Unknown pricing is shown as unknown rather than silently treated as free. Ollama Cloud account-level quota and GPU-based billing are not exposed by its supported API, so this first version reports recorded usage without inventing a cloud cost.

## Project Documentation

- [`docs/architecture.md`](docs/architecture.md)
- [`docs/data-model.md`](docs/data-model.md)
- [`docs/provider-support.md`](docs/provider-support.md)
- [`docs/release-process.md`](docs/release-process.md)
- [`CONTRIBUTING.md`](CONTRIBUTING.md)
- [`SECURITY.md`](SECURITY.md)
