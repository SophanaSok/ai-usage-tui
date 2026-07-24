# Architecture

The application is designed around a provider-neutral usage pipeline:

```text
collector -> normalized usage event -> aggregation -> dashboard/export
```

The current collector reads assistant message records from the OpenCode SQLite database. Future collectors will journal Ollama and other provider responses into the same normalized shape.

## Privacy Boundary

Collectors may read usage metadata, model identifiers, timestamps, and calculated costs. They must not persist or transmit prompts, completions, API keys, or credentials.

## Cost Provenance

Every cost value must be labeled as provider-reported, calculated, estimated, free, local, or unavailable. A missing cost must never be rendered as a paid zero.
