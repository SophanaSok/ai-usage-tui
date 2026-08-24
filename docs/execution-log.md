# Execution Log

Frozen at 2026-07-24. Later history is in `CHANGELOG.md`.

## 2026-07-24

### Production Foundation

- Added explicit cost provenance.
- Hardened OpenCode SQLite reads.
- Added automatic refresh and non-interactive output.
- Added open-source project documentation and CI scaffolding.

### Ollama and Zen Milestone

- Added the opt-in Ollama response journal.
- Verified local and `:cloud` Ollama classification.
- Added the Zen catalog refresh command and local cache.

### Routing Policy

- Exploration and test work prefer local models.
- Routine implementation uses balanced models.
- Reviews use an independent provider.
- Hosted models are allowed for non-secret code only.

### CLI and Hardening

- Added arbitrary day ranges and provider/model filters.
- Added TOML configuration and CSV export.
- Added journal idempotency and cross-source deduplication.
- Added independent review checkpoints for accounting and ingestion correctness.
