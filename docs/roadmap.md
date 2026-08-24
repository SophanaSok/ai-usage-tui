# Roadmap and Outstanding Findings

Working state for continuing the audit-driven work started 2026-08-18, last updated 2026-08-20.
Shipped items are in the versioned sections of `CHANGELOG.md` (new work goes under
`[Unreleased]`); this file is the *remaining* work, with enough evidence attached that each item
can be picked up cold.

## Where things stand

Tests (see the CI test job for the count), `cargo fmt --check` and `cargo clippy -D warnings`
clean, CI across Linux / macOS / Windows with an MSRV job (1.88) and `cargo-deny` — six checks,
all green on every branch since PR #5.

**Every P0 and P1 finding from the original audit has shipped.** What remains is P2 and P3 — depth
and breadth, not correctness.

Sources read today: OpenCode SQLite, Claude Code JSONL, Codex CLI rollouts, the local Ollama/routing journal, and the
Zen pricing table. Verified end to end against ~103MB of real Claude Code logs — 5,879 requests
parsed in 0.27s with **zero unpriced rows**. On a subscription account those rows are `quota`
rather than priced, and carry the list-rate figure as `api_equivalent_cost` instead of `cost`.

The two things worth defending, and the reason to prefer depth over breadth below:

- **Cost provenance.** Unknown cost stays unknown; no competitor refuses to invent a number.
- **Routing analytics.** Retries / escalations / test pass-fail / review defects per model —
  "is Opus actually worth 5× Sonnet on my codebase?" Nobody else answers that.

## Outstanding findings

Numbering follows the original audit. Everything not listed here has shipped.

### P2 — Pricing depth

**Provider-blind pricing keys.** `pricing/zen.toml` keys on bare model id, so the same model at two
providers with different rates is indistinguishable. This is also what blocks classifying
aggregators (OpenRouter, Bedrock, Azure) as `PAID` — see the comment in
`classify.rs::FIRST_PARTY_PAID_PROVIDERS`. Provider-qualified keys unblock both.

**LiteLLM as the pricing source.** The scraper now discovers models rather than filtering them
against a hardcoded list, so a new model on the Zen page gets priced without a release — but it
still parses one vendor's HTML with `find("<table>")` and `split("<tr>")`, and that page is the
only source. The bundled table is ~60 hand-maintained models. LiteLLM's
`model_prices_and_context_window.json` is community-maintained at ~2,200 models and is what every
competitor uses. Adopt it as primary with the Zen TOML kept as an overlay for Zen-specific and
stealth models; vendor a snapshot via `include_str!` so offline still works.

### P2 — Coverage

More agent CLIs behind the existing `Collector` trait, which is already the right seam.
**Codex** shipped (`src/collector/codex.rs`); **Gemini CLI** is next. The trait needs auto-discovery ("which agent CLIs are
installed?") and per-source enable/disable, which `CollectorsConfig` already models.

### P3 — Dashboard

Shipped since the audit: the time-series panel (`g`), burn rate with budget projection (`w`), the
per-project view (`p`), per-session drill-down (`s`), and the subscription-limits panel (`l`),
which reads Omarchy's agents-panel records read-only and also surfaces the fullest fresh window
in the header. `src/ui.rs` was split into `src/ui/` with one module per panel, which is what
made each of those a small independent change. The other direction shipped too: `--omarchy-record`
publishes this tool's usage and budgets as a tab in Omarchy's panel (see Decisions below).

Remaining:

- **Drill from a project into its sessions.** `Enter` on a project row. Deliberately not done
  with the flat session list: it introduces panel *state* — which project am I inside? — that no
  panel currently has, and that is a larger change than any of the panels were.
- **Interactive depth** — `/` search, sortable columns, mouse support.
- **Routing analytics still needs a harness for the half that matters.** Escalations are now
  derived from collected sessions, but pass/fail and retry counts cannot be inferred from usage
  metadata and must not be guessed. A shipped hook or wrapper that emits `--record-routing`
  events from a real agent harness is what would close this, not more derivation.
- **Derived escalations are not exported.** `--json` and `--csv` carry usage rows only, so the
  block is TUI-only.

### P3 — Polish

- Migrate to `clap` derive with subcommands (`daily`, `monthly`, `session`, `blocks`, `live`) and
  generated shell completions. The hand-rolled parser already strains under manual
  mutual-exclusion checks.
- **Resolved.** crates.io publication is wired up: `Cargo.toml` carries `readme`, an `exclude`
  that drops the 670KB of screenshots (the packaged crate is 247KB compressed, 91 files), and
  `[package.metadata.binstall]` overrides mapping every target to its release archive. The
  `publish-crate` job in `release.yml` publishes on a tag once `CARGO_REGISTRY_TOKEN` exists. The
  fixtures were kept in deliberately: the `#[cfg(test)]` modules under `src/` read
  `tests/fixtures/` at runtime, so excluding them would ship a crate whose own tests cannot run.
  What is left is the account step — see `docs/release-process.md`, "First publish".
- **Resolved.** `docs/model-routing.md` no longer duplicates the agent-to-model table; that mapping
  lives in `~/.config/opencode/opencode.json` and `~/.config/opencode/ROUTING.md`, and the repo doc
  now carries only policy (tiers by role, privacy boundary, escalation, evaluation schema).
  The screenshot tooling was reconciled at the same time. `docs/phase-status.md` and
  `docs/execution-log.md` have since been removed: both restated `CHANGELOG.md` from memory and
  had drifted -- phase-status still filed the whole of v0.5.0 under "Unreleased" -- while being
  linked from the README as current contributor documentation.

  *One discrepancy is left for you, in your own config rather than this repo:* `ROUTING.md` lists
  `reviewer` as `north-mini-code-free` while `opencode.json` defines it as
  `opencode/deepseek-v4-flash-free`. Both are free-cloud, so cost is unaffected, but note that
  `reviewer` and `reasoning` then share a model — which weakens rule 3 (prefer a reviewer on a
  different provider from the agent that wrote the code) when `reasoning` did the writing.

## Decisions worth knowing about

**Omarchy integration writes a record rather than shipping a collector.** Omarchy's updater only
runs collectors from the root-owned `$OMARCHY_PATH/bin` (`/usr/share/omarchy`), and `OMARCHY_PATH`
is fixed by Omarchy's environment bootstrap, so a user cannot register one. But the panel tabs any
`*.json` in its usage directory, whoever wrote it — so `--omarchy-record` writes the record itself,
on a user timer (`contrib/systemd/user/`), and nothing else in the tool ever writes there. Budgets
map to `limits[]` because that is the one structure the panel meters: a 0..1 `percent` gets the
90 % alarm glyph and a `resetsAt` gets the countdown, which a soft budget wants too. The panel's
`balance` block is opt-in (`[omarchy] balance`) because it labels the figure "Prepaid credits …
funded", a loose description of a budget. Claude Code and Codex rows are excluded because
Omarchy's own `claude`/`codex` tabs already cover those logs — and a record under either id would
overwrite Omarchy's file, so those ids are refused outright.

**Logging is a ~130-line module, not `tracing`.** The audit called for `tracing`; what the problem
actually needed was "collector errors survive to a file the user can read." `tracing` +
`tracing-subscriber` is a large transitive tree for spans and subscribers this app has no use for,
and `deny.toml` uses an exact license allowlist precisely so that its output stays reviewable by
hand. Revisit if structured fields or per-module filtering ever earn their keep.

**Logging is off by default.** `AI_USAGE_LOG` must be set. A tool whose entire pitch is "reads
usage metadata, writes nothing, transmits nothing" should not quietly accumulate a file on disk.

**Poison recovery can lose one row.** Recovering a poisoned `RwLock` may expose a partially
applied merge — a usage key in the dedup index whose row never made it into the list. That is a
bounded one-row loss against an unbounded silent freeze, which is what the previous
`if let Ok(mut s) = state.write()` produced.

## Conventions worth preserving

Established while fixing the accounting; breaking these is how the bugs came back.

1. **Unknown cost stays unknown.** A missing rate yields `CostStatus::Unavailable`, never `$0.00`.
   An explicit `0.0` in the pricing table is distinct from an absent field — `Option<f64>`
   end-to-end, no `unwrap_or(0.0)`.
2. **The refreshed pricing cache is an overlay, never a replacement.** A lossy or corrupt refresh
   must not be able to delete pricing that shipped in the binary.
3. **Dedup keys on identity, not shape.** `event_id` first, shape *plus timestamp* as fallback.
   Token counts alone are not an identity.
4. **Never read message content.** Claude Code transcripts contain source code and secrets. Parse
   only the `usage` block. There is a test that plants a fake credential and fails if it appears
   in the record.
5. **No I/O or clock reads on the render path.** Derived views are computed once per refresh.
6. **Tests must discriminate.** A regression test that passes against the old buggy code is worse
   than no test. Verify by restoring the bug in a scratch copy and confirming the test fails —
   this is how the dedup and integer-rate bugs were both confirmed.
7. **Pricing comes from the `claude-api` skill, never from memory.** Load it before touching any
   model rate.
8. **A silent failure is a bug, even when the code "handles" it.** `unwrap_or_default()` on a
   config parse, `unwrap_or_default()` on a DB read, `if let Ok(..)` on a poisoned lock, and
   `break` on a panic all read as robustness and all render "broken" as "nothing to report".
   Failure must reach the screen or the log.

## Verifying the release path

`release.yml` used to run only on tag push, so its first execution was also its first test, with
a published release riding on the outcome. It now accepts `workflow_dispatch`:

```sh
gh workflow run release.yml -f tag=v0.0.0-dryrun
gh run watch
```

A dispatch run builds every architecture, asserts each binary's arch with `file`, builds and
inspects the `.deb`/`.rpm`, generates checksums, renders the packaging manifests and fails on any
unsubstituted placeholder — then skips only the publish. Run it after any change to the release
workflow, the packaging templates, or the build matrix.

## How to verify a change end to end

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --locked && cargo test --doc --locked

# Against the committed fixture. `--claude-dir`, `--codex-dir` and `--omarchy-dir` matter:
# without them this reads your real ~/.claude/projects, ~/.codex and Omarchy records and
# stops being a fixture check.
cargo build --release --locked
./target/release/ai-usage-tui --json --all \
  --db tests/fixtures/opencode_test.db --claude-dir /nonexistent --codex-dir /nonexistent \
  --omarchy-dir /nonexistent

# Against real Claude Code logs — the true end-to-end check.
# Watch the unpriced count: it should be zero. Subscription rows are `quota` with
# `cost` null by design, so they are excluded — without that this reports 100%
# unpriced on a Max account.
./target/release/ai-usage-tui --json --db /nonexistent.db --all \
  | python3 -c "import json,sys; d=json.load(sys.stdin); \
      rows=[u for u in d['usage'] if u['provider']=='anthropic']; \
      print(len(rows), 'requests,', sum(1 for u in rows if u['cost'] is None \
        and u['cost_status'] != 'quota'), 'unpriced,', \
        sum(1 for u in rows if u['cost_status'] == 'quota'), 'on quota')"
```

Tests are hermetic — they never read the developer's real `~/.claude/projects`, `~/.claude.json`,
`~/.codex`, or `~/.local/state/omarchy`. Keep it that way: pass an explicit `claude_dir` (the config
document is derived from it) or `claude_json`, an explicit `codex_dir` (`--codex-dir /nonexistent`),
and an explicit `omarchy_dir` (`--omarchy-dir /nonexistent`; the billing decision reads its
`tierLabel`), in any new test that goes through `load_usage` or `print_once`, and in any command in
this file.
