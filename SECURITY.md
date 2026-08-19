# Security Policy

## Supported Versions

Only the latest release receives security fixes.

## Reporting a Vulnerability

Report privately through GitHub's private vulnerability reporting:
**https://github.com/SophanaSok/ai-usage-tui/security/advisories/new**

If that is unavailable to you, email **sokdevelopment@gmail.com** with `ai-usage-tui security` in
the subject. Expect an acknowledgement within 7 days.

**Do not include credentials, prompts, completions, API keys, or private database contents in a
report — public or private.** A reproduction against `tests/fixtures/opencode_test.db`, or a
redacted excerpt, is enough. If a report requires real data to reproduce, say so and wait for a
reply rather than attaching it.

## What this tool guarantees

These are the properties a vulnerability report should be measured against. A defect in any of
them is a security bug, not a feature request:

- **Usage metadata only.** Collectors parse token counts, model identifiers, and timestamps.
  Claude Code session transcripts contain source code and secrets; only the `usage` block of each
  line is read. A test plants a fake `AWS_SECRET_ACCESS_KEY` in a transcript and fails if it
  reaches a usage record.
- **No prompt or completion content is persisted or transmitted**, ever.
- **Working directory paths are recorded** for per-project attribution, and appear in `--json`
  and `--csv` exports. This is the one identifying value the tool stores; it is local-only and
  never transmitted, but review an export before sharing it.
- **The user's OpenCode database is opened read-only** (`SQLITE_OPEN_READ_ONLY`).
- **No telemetry.** Outbound network calls happen only when explicitly requested:
  `--refresh-zen`, `--refresh-pricing`, and a budget webhook you configure yourself.
- **No `unsafe` code.**

## Dependency advisories

`cargo-deny` runs on every push and pull request against the RustSec advisory database, with any
accepted exception recorded by advisory id and reason in `deny.toml`. Dependabot opens updates for
Cargo and GitHub Actions.
