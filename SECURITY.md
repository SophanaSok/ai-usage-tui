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
- **Working directory paths are recorded** for per-project attribution, and appear in
  `--summary-json`, `--json` and `--csv` exports. This is the one identifying value the tool
  stores; it is local-only and never transmitted, but review an export before sharing it — and
  know that an LLM agent you ask to read one (the shipped Claude Code skill does) sends what it
  reads to its own model provider. That flow is the agent's; this tool makes no request for it.
- **The user's OpenCode database is opened read-only** (`SQLITE_OPEN_READ_ONLY`).
- **No telemetry.** Outbound network calls happen only when explicitly requested:
  `--refresh-zen` and `--refresh-pricing` (and the `zen_pricing` collector, off unless enabled),
  `--check-update` and `--doctor` with `[update] check = true` (one GET to GitHub's releases API,
  naming only the tool and its version), and a budget webhook you configure yourself. The dashboard
  process itself never makes a request; it reads what those commands cached.
- **No `unsafe` code**, enforced by `#![forbid(unsafe_code)]` in the library and the binary.

## Release integrity

What a release is built from, and how to check that a download is one:

- **Build attestations.** Every archive and Linux package in a release after v0.19.0 is attested by
  the release workflow (`actions/attest-build-provenance`): a Sigstore-signed statement binding the
  file's digest to this repository, `release.yml` and the tagged commit. Verify with
  `gh attestation verify <file> --repo SophanaSok/ai-usage-tui --signer-workflow
  SophanaSok/ai-usage-tui/.github/workflows/release.yml --source-ref refs/tags/<tag>` -- the last
  flag matters, because a hand-run dry run of the workflow attests what it builds too, as built
  from its branch. `scripts/install.sh` runs the
  check when the GitHub CLI is available and `--require-attestation` makes it mandatory. A file
  that fails it was not built by this project's workflow, whatever its checksum says.
- **A bill of materials.** `ai-usage-tui-<tag>.cdx.json` (CycloneDX 1.5) lists every crate any
  released target links, from `Cargo.lock`, and is attested against the same files.
- **Actions are pinned by commit**, not by tag, with the version in a comment for Dependabot to
  maintain; a test fails the build for any `uses:` that is not. The one tool the release job
  downloads is pinned by version and checked against a digest written in the workflow.
- **Tokens are least-privilege.** Every workflow declares `permissions:`, read-only at the top; a
  write is granted only to the job that performs it, and a test refuses a top-level write.
- **`main` is protected**: changes arrive by pull request with the CI checks passing, and the
  branch cannot be deleted or force-pushed. A release commit goes through the same door.

Not yet: the macOS binaries are unsigned and not notarized, and crates.io is published with a
long-lived token rather than trusted publishing.

## Dependency advisories

`cargo-deny` runs on every push and pull request against the RustSec advisory database, with any
accepted exception recorded by advisory id and reason in `deny.toml`. Dependabot opens updates for
Cargo and GitHub Actions.
