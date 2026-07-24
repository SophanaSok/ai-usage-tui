# Provider Support

## OpenCode

Reads assistant usage metadata from the local OpenCode SQLite database. This is the current supported collector.

## Ollama

Ollama response metrics expose prompt and output token counts, but Ollama does not provide a complete historical usage database. `--record-ollama` provides an opt-in local journal for requests made after tracking is enabled.

## Ollama Cloud

Token counts can be observed when returned by the client response. Account quota and GPU-based Cloud billing are not currently exposed through the supported API, so the tool must not invent a dollar cost. Cloud-routed models are displayed as `CLOUD`, never as local usage.

## OpenCode Zen

Zen usage can be read from OpenCode history. `--refresh-zen` fetches and caches the Zen model catalog at `~/.local/share/ai-usage-tui/zen-models.json`. The current cache is informational; pricing snapshot application to historical events is the next pricing milestone.
