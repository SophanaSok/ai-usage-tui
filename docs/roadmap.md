# Roadmap and Outstanding Findings

Working state for continuing the audit-driven work started 2026-08-18, last updated 2026-08-19.
Shipped items are in `CHANGELOG.md` under `[Unreleased]`; this file is the *remaining* work, with
enough evidence attached that each item can be picked up cold.

## Where things stand

105 tests (from 51), `cargo fmt --check` and `cargo clippy -D warnings` clean, CI across Linux /
macOS / Windows with an MSRV job (1.88) and `cargo-deny`. All six checks green on PR #5, including
macOS, Windows and `cargo-deny`, which had never run before that branch.

Sources read today: OpenCode SQLite, Claude Code JSONL, the local Ollama/routing journal, and the
Zen pricing table. Verified end to end against ~103MB of real Claude Code logs — 5,879 requests
parsed in 0.27s with **zero unpriced rows**.

The two things worth defending, and the reason to prefer depth over breadth below:

- **Cost provenance.** Unknown cost stays unknown; no competitor refuses to invent a number.
- **Routing analytics.** Retries / escalations / test pass-fail / review defects per model —
  "is Opus actually worth 5× Sonnet on my codebase?" Nobody else answers that.

## Outstanding findings

Numbering follows the original audit. Everything not listed here has shipped.

### P1 — Robustness

**1.21 The pricing scraper can only re-price models it already hardcodes.** `pricing_refresh.rs`
skips any scraped row whose display name is absent from `known_model_names()`. A refresh can never
discover a new model. Combined with a hand-written `find("<table>")` / `split("<tr>")` parser over
one vendor's docs page, this is the most brittle code in the repo.

*Note:* the overlay-merge fix means a lossy refresh can no longer **delete** pricing, so this is
now a staleness problem rather than a data-loss one.

### P2 — Pricing depth

**1.6 Pricing is retroactive.** `apply_estimated_pricing` re-prices every event at whatever is in
the cache *now*, so a `--refresh-pricing` silently rewrites historical costs. Fix: persist resolved
unit rates onto the journal row when an event is first priced; re-price only rows still
`Unavailable`.

**Provider-blind pricing keys.** `pricing/zen.toml` keys on bare model id, so the same model at two
providers with different rates is indistinguishable. This is also what blocks classifying
aggregators (OpenRouter, Bedrock, Azure) as `PAID` — see the comment in
`classify.rs::FIRST_PARTY_PAID_PROVIDERS`. Provider-qualified keys unblock both.

**LiteLLM as the pricing source.** The bundled table is ~60 hand-maintained models. LiteLLM's
`model_prices_and_context_window.json` is community-maintained at ~2,200 models and is what every
competitor uses. Adopt it as primary with the Zen TOML kept as an overlay for Zen-specific and
stealth models; vendor a snapshot via `include_str!` so offline still works.

**Time-boxed:** `claude-sonnet-5` in `pricing/zen.toml` carries **introductory** pricing that
lapses **2026-08-31** ($2/$10 → $3/$15, cache 0.20/2.50 → 0.30/3.75). There is a dated comment at
the entry. This needs a manual update.

### P2 — Coverage

More agent CLIs behind the existing `Collector` trait, which is already the right seam. Highest
value first: **Codex**, then **Gemini CLI**. The trait needs auto-discovery ("which agent CLIs are
installed?") and per-source enable/disable, which `CollectorsConfig` already models.

### P3 — Dashboard

The `btop-inspired` claim is not yet earned — btop's identity *is* the graph, and there isn't one.

- **Time-series panel** — sparkline / braille chart of tokens and spend per day.
- **Burn rate + window tracking** — tokens/min over a trailing window, projected spend, 5-hour
  block progress. This is `Claude-Code-Usage-Monitor`'s headline feature and the reason it has
  8.6k stars.
- **Per-project view** — `session_id` and `project` are now populated by the Claude Code collector
  but nothing renders them. This is the cheapest high-value view remaining.
- **Interactive depth** — `/` search, sortable columns, `Enter` to drill into a model's sessions,
  `?` help overlay, mouse support.
- **Surface the differentiators** — promote routing analytics from a hidden `t` toggle to a
  first-class view, and add a "pricing coverage: N% of requests priced" indicator so provenance
  becomes visible rather than an internal enum.
- `ui.rs` is ~950 lines in one module and should be decomposed into `src/ui/` alongside this work.

### P3 — Polish

- Migrate to `clap` derive with subcommands (`daily`, `monthly`, `session`, `blocks`, `live`) and
  generated shell completions. The hand-rolled parser already strains under manual
  mutual-exclusion checks.
- Publish to crates.io with `include`/`exclude` — a publish today would ship an 86KB HTML fixture
  and 180KB of PNGs — and support `cargo binstall`.
- **Resolved.** `MODEL_ROUTING.md` no longer duplicates the agent-to-model table; that mapping
  lives in `~/.config/opencode/opencode.json` and `~/.config/opencode/ROUTING.md`, and the repo doc
  now carries only policy (tiers by role, privacy boundary, escalation, evaluation schema).
  `docs/phase-status.md` and `scripts/capture-readme-screenshots.sh` were reconciled at the same
  time.

  *One discrepancy is left for you, in your own config rather than this repo:* `ROUTING.md` lists
  `reviewer` as `north-mini-code-free` while `opencode.json` defines it as
  `opencode/deepseek-v4-flash-free`. Both are free-cloud, so cost is unaffected, but note that
  `reviewer` and `reasoning` then share a model — which weakens rule 3 (prefer a reviewer on a
  different provider from the agent that wrote the code) when `reasoning` did the writing.

## Decisions worth knowing about

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

## How to verify a change end to end

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets && cargo test --doc

# Against the committed fixture
cargo build --release
./target/release/ai-usage-tui --json --db tests/fixtures/opencode_test.db --all

# Against real Claude Code logs — the true end-to-end check.
# Watch the unpriced count: it should be zero.
./target/release/ai-usage-tui --json --db /nonexistent.db --all \
  | python3 -c "import json,sys; d=json.load(sys.stdin); \
      rows=[u for u in d['usage'] if u['provider']=='anthropic']; \
      print(len(rows), 'requests,', sum(1 for u in rows if u['cost'] is None), 'unpriced')"
```

Tests are hermetic — they never read the developer's real `~/.claude/projects`. Keep it that way:
pass an explicit `claude_dir` in any new test that goes through `load_usage` or `print_once`.
