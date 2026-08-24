# Roadmap and Outstanding Findings

Working state for continuing the audit-driven work started 2026-08-18, last updated 2026-08-24 (v0.6.0).
Shipped items are in the versioned sections of `CHANGELOG.md` (new work goes under
`[Unreleased]`); this file is the *remaining* work, with enough evidence attached that each item
can be picked up cold.

## Where things stand

Tests (see the CI test job for the count), `cargo fmt --check` and `cargo clippy -D warnings`
clean, CI across Linux / macOS / Windows with an MSRV job (1.88), a docs job (`cargo doc -D
warnings` plus a relative-link check across every Markdown file) and `cargo-deny` — seven checks,
all green on every branch since PR #5. `cargo-deny` also runs weekly on a schedule: an advisory
published against a dependency that has not changed produces neither a push nor a PR, so a
push-only trigger never noticed it.

`just check` runs exactly what CI runs, in CI's order. Use it before pushing.

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

- **Resolved.** LiteLLM is now the base pricing source. `pricing/litellm.tsv` is generated from
  its community table by `scripts/refresh-litellm-pricing.py` (`just pricing`) and ships in the
  binary: ~3,450 keys across 88 providers, against ~60 in the curated table. Together they price
  1,491 distinct model names. `pricing/zen.toml` is applied on top for Zen-specific and stealth
  models — 13 names it carries are in no community table — and a refreshed cache on top of that,
  so the existing "an overlay never replaces" invariant is unchanged.

- **Resolved.** Pricing keys can be provider-qualified. `resolve` now takes the usage row's
  provider and tries `<provider>/<model>` before the bare name, which is what lets the same model
  bill differently at Bedrock, `bedrock_converse` and the aggregators. Where providers disagree
  on a name — 180 of them — the generated table publishes no bare key at all, so an unrecognised
  provider yields no price rather than borrowing another's rate.

  Two things are worth not relearning. **Layering outranks specificity:** preferring the more
  specific key let the community's `anthropic/claude-sonnet-5` beat a hand-checked bare
  `claude-sonnet-5` and bypass the dated `period` records only the curated table carries. The
  engine tracks which keys came from a layer above the generated table and searches that layer
  first. And **the format is TSV, not TOML, on purpose:** ~3,450 TOML tables cost 38ms on every
  invocation, a 9x startup regression; the compact form costs 5ms. The curated table stays TOML
  because humans edit it and it needs comments and `period` blocks.

- **Resolved.** Aggregators and clouds classify as `PAID`. `classify.rs::PAID_PROVIDERS` (was
  `FIRST_PARTY_PAID_PROVIDERS`) now carries OpenRouter, Bedrock, Azure, Vertex, Fireworks,
  DeepInfra, Together and Perplexity alongside the first-party names, and their rows both
  classify and price — an OpenRouter row's `anthropic/claude-3.5-sonnet` reduces to a bare name
  and re-qualifies against `openrouter/`.

  Two things worth knowing if this list is ever touched again. **It never causes a row to be
  priced**: pricing is the table's decision, and a row that gets a figure is promoted to `PAID`
  regardless of this list. What the list decides is the category of rows we *cannot* price —
  "real spend, rate unknown" rather than "no idea what this is", which keeps the gap visible in
  the coverage figure instead of hidden in `UNKNOWN`.

  And **deriving the list from the pricing table's key prefixes does not work**, though it looks
  more principled. `google` has no keys at all (LiteLLM spells it `gemini` and `vertex_ai`),
  `ollama` has 29 and is emphatically not billable, and LiteLLM's `fireworks_ai` does not match
  the `fireworks-ai` a collector records. Reconciling those needs fuzzy token matching, and a
  token like `ai` matches nearly anything. An explicit, reviewable list is the honest answer.

### P2 — Coverage

More agent CLIs behind the source registry. **Codex** shipped (`src/collector/codex.rs`), and
**Gemini CLI** now too (`src/collector/gemini.rs`) — a module plus one registry line, which is
what the registry was for.

Gemini is worth reading before adding the next one, because it is the awkward case: it persists
no usage at all unless the user enables its OpenTelemetry log, so the collector is opt-in and
`--doctor` has to distinguish "not set up" from "empty". Expect more sources to look like this
than like Claude Code, whose transcripts are simply always there.

**Resolved: the Gemini format is validated against real output.** It was derived by reading
`@google/gemini-cli` 0.56.0's serialization code; it is now confirmed against bytes that CLI
actually wrote. No Gemini account was needed — `GOOGLE_GEMINI_BASE_URL` points the CLI at a local
stand-in for Google's API, so the real CLI, its real OpenTelemetry SDK and its real
`FileLogExporter` produce the file with no billable call. `tests/fixtures/gemini_telemetry.json`
is a redacted capture, and two tests pin the parser to it.

Everything the format notes claimed held: concatenated pretty-printed JSON, `attributes` as a
top-level sibling of the OTLP wrapper, and the token attribute names. Three things the notes did
*not* predict, all now covered:

- **Metric records have no `attributes` key at all.** A parser indexing `["attributes"]` would
  have panicked on them. Ours returns `None`, and the fixture contains one.
- **`resource` carries the host name, home directory paths and the full command line, prompt
  included.** Only `attributes` is read, which is why none of it reaches a usage record — there
  is now a test asserting that against the real block rather than a synthetic one.
- **One prompt produced six `api_response` records sharing a `prompt_id` *and* an identical
  `total_token_count`.** Only the timestamp separated them. Keying identity on `prompt_id`, or on
  `prompt_id` plus the total, would have reported one request instead of six.

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
- **Resolved.** Derived escalations are exported. `--json` carries an `escalations` object —
  sessions examined and escalated, the rate, unclassified changes, and the transitions with their
  spend after the move — derived from the same filtered rows the export reports, so a
  `--provider` filter narrows both and a script cannot disagree with the dashboard about one run.

  Deliberately not added to `--routing-json`, which reads recorded `--record-routing` events from
  the journal and nothing else: these are *inferred* from usage, the dashboard labels the two as
  different things, and a test asserts it. Folding one into the other in an export would undo
  exactly that distinction.

  Deliberately not added to `--csv` either. The usage CSV is one flat table whose columns are
  appended-never-inserted so a consumer reading by index keeps working; transitions are a
  different shape and would need their own file. `--json` is the export for them.

### P3 — Polish

- **Resolved, except the subcommands — which should not be done as described.** The parser is
  `clap` derive now, and shell completions and a man page ship with it: `--completions SHELL`
  and `--man` generate from the same `Command` that parses the arguments, the `.deb`/`.rpm`
  install them, and `just assets` produces them locally.

  `clap` also replaced the hand-rolled eleven-way mutual-exclusion count with a declarative
  group, and `tests/docs.rs` now *queries* the parser instead of regexing `src/cli.rs` for
  `"--flag" =>` match arms. The guard that compared `--help` against the parser is gone
  deliberately: clap generates the help from the argument definitions, so they cannot disagree —
  that invariant is structural now rather than tested.

  **The subcommands (`daily`, `monthly`, `session`, `blocks`, `live`) are a separate question,
  and the case for them is weaker than this entry implied.** Everything it offered them as the
  means to is already delivered without them: the mutual exclusion is declarative, and
  completions and the man page shipped. What is left would be a UX redesign that breaks every
  documented invocation, the systemd unit in `contrib/`, and the CLI reference table — for a
  vocabulary that partly collides with the existing range flags (`daily`/`monthly` against
  `--today`/`--month`) and partly has no referent here at all (`blocks`). If it is done, it wants
  its own design pass and a major version, not a rider on a parser swap.
- **Resolved and shipped.** `ai-usage-tui` is published on crates.io as of v0.6.0, so
  `cargo install ai-usage-tui` and `cargo binstall ai-usage-tui` both work. `Cargo.toml` carries
  `readme`, an `exclude` that drops the 670KB of screenshots (the packaged crate is 267KB
  compressed, 105 files) and `[package.metadata.binstall]` overrides mapping every target to its
  release archive. The `publish-crate` job publishes on each tag and refuses to run when the tag
  and `Cargo.toml` disagree.

  The fixtures are kept in the package deliberately: the `#[cfg(test)]` modules under `src/` read
  `tests/fixtures/` at runtime and `src/config.rs` includes `examples/config.toml`, so excluding
  either would ship a crate whose own tests cannot run.

  A Homebrew tap and Scoop bucket exist too (`SophanaSok/homebrew-tap`,
  `SophanaSok/scoop-bucket`); the `update-taps` job pushes the rendered manifests on each tag.
  Both that job and `publish-crate` skip with a notice if their secret is absent — including when
  a fine-grained `TAP_TOKEN` **expires**, in which case the release still succeeds and the tap
  silently stops updating. The job-log warning is the only signal.
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

### P3 — Small, known, unclaimed

Each of these is an afternoon at most, and each is a reasonable first outside contribution.

- **`src/collector/zen.rs` has no tests.** Thirty-two lines: a path join and a network fetch. The
  join is trivially testable; the fetch wants the same fetch-from-parse split
  `pricing_refresh.rs` already uses (`fetch_with_backoff` does the HTTP, `parse_pricing_html` is
  pure and tested against committed fixtures).
- **The README's "Data sources" section is the longest one left**, at roughly 140 lines. The two
  provider billing essays already moved to `docs/provider-support.md`; the per-provider parsing
  detail is the obvious next candidate, leaving a table and the paths.
- **`src/ui/theme.rs` has no tests** and probably wants none — it is colour constants. Noted so
  the next person does not re-derive that.
- **Derived escalations are TUI-only** (also listed under Dashboard above): `--json` and `--csv`
  carry usage rows only. This is the cheapest of the Dashboard items and the only one that
  needs no new UI.

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
inspects the `.deb`/`.rpm`, generates checksums, renders the packaging manifests (asserting the
Chocolatey pair keeps the `tools/` layout its nuspec glob needs) and fails on any unsubstituted
placeholder — then skips only the publish. Run it after any change to the release workflow, the
packaging templates, or the build matrix.

**What a dispatch run does not cover:** `publish-crate` and `update-taps` are both gated on
`github.event_name == 'push'`, so a dry run always skips them. Their first real execution is on a
tag. That is deliberate — neither can be rehearsed without publishing — but it means a change to
either job is unverified until a release, so keep them boring.

## How to verify a change end to end

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --locked && cargo test --doc --locked

# Against the committed fixture. All four overrides matter: without them this reads your real
# ~/.claude/projects, ~/.codex, Omarchy records and usage journal, and stops being a fixture
# check. `--journal` is the one that is easy to forget -- it defaults to
# $XDG_DATA_HOME/ai-usage-tui/usage.db, so any journaled Ollama usage shows up as `ollama` rows.
cargo build --release --locked
./target/release/ai-usage-tui --json --all \
  --db tests/fixtures/opencode_test.db --claude-dir /nonexistent --codex-dir /nonexistent \
  --omarchy-dir /nonexistent --journal /nonexistent/journal.db

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
`~/.codex`, `~/.local/state/omarchy`, or usage journal. Keep it that way: pass an explicit
`claude_dir` (the config document is derived from it) or `claude_json`, an explicit `codex_dir`
(`--codex-dir /nonexistent`), an explicit `omarchy_dir` (`--omarchy-dir /nonexistent`; the billing
decision reads its `tierLabel`), and an explicit `journal`, in any new test that goes through
`load_usage` or `print_once`, and in any command in this file.

The journal is the one that was missed: `tests/cli.rs::hermetic()` pinned the other three and not
this one, so every CLI test read `$XDG_DATA_HOME/ai-usage-tui/usage.db`. CI never caught it —
a fresh runner has no journal — and it would have surfaced first as `ollama` rows appearing in a
fixture-only assertion on a contributor's machine. Fixed in `hermetic()`; the lesson is that
"hermetic" has to mean *every* source the registry lists, not the ones that were top of mind.
