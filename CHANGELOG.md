# Changelog

## [Unreleased]

### Fixed

- **The Intel macOS build depended on a runner that no longer starts.** `macos-13` is being
  retired; a release dry run sat queued on it for 76 minutes without ever being scheduled, which
  on a real tag push would hang the release indefinitely. That target is now cross-compiled from
  the Apple Silicon runner with an explicit `--target`, which Xcode supports directly. The
  `file`-based architecture assertion is unchanged and is what keeps the claim checkable — the
  original mislabeled-artifact bug was the *absence* of `--target`, not cross-compilation.

### Added

- **Claude Code collector.** Reads `~/.claude/projects/**/*.jsonl` — the largest source of
  Anthropic usage on most machines, and previously invisible. Tails each session log by byte
  offset (never re-parsing history), dedupes on `requestId`, and attributes usage to a session
  and project. Only the `usage` block is parsed; transcripts contain source code and secrets, and
  no message content is read or retained. Configurable via `--claude-dir` /
  `[collectors.claude_code]` / `CLAUDE_PROJECTS_DIR`.
- **`session_id` and `project` on `Usage`**, enabling per-project cost attribution — a dimension
  the data model previously could not express.
- **Layered model-ID resolution.** Real-world ids never arrive in table form: Claude Code writes
  `claude-sonnet-4-5-20250929`, aggregators write `anthropic/claude-sonnet-4.5`, Ollama writes
  `glm-5.2:cloud`. Resolution now tries provider-stripped, date-stripped, dotted-version, and
  suffix-stripped spellings before giving up.
- **Incremental ingestion.** The OpenCode collector resumes from a `time_created` high-water mark
  instead of re-reading and re-parsing the entire message table every 30 seconds. The cursor is
  inclusive by design; `event_id` deduplication absorbs the boundary overlap.
- **Missing Anthropic pricing.** `claude-opus-5` was absent from the table entirely — against real
  Claude Code logs that was 2,810 requests and ~1.01B cache-read tokens reporting no cost at all.
  Added alongside `claude-mythos-5`, with tests asserting current Anthropic models resolve and
  that cache rates follow the published 0.1x (read) and 1.25x (5-minute write) multipliers.
- **Per-project cost view** (`p`). `session_id` and `project` had been populated since the Claude
  Code collector landed and nothing rendered them. Shows tokens, cost, requests and distinct
  sessions per project, ranked by spend, with unpriced work marked `≥ $x` rather than folded into
  a confident total. `project` now holds the full working directory, so `~/a/build` and
  `~/b/build` are separate projects instead of one silently merged row; the table shows the
  shortest name that tells them apart.
- **Pricing coverage is visible.** The project panel's title reports what share of billable
  requests actually carry a known cost. Provenance was the project's differentiator and lived
  entirely in an internal enum — a total could cover two thirds of the requests without saying so.
- **`project` and `session_id` in `--json` and `--csv`.** Appended to the CSV, never inserted, so
  a consumer reading by column index keeps working.
- **Collector health, rendered rather than logged.** Each collector now reports a liveness state
  (starting / ok / failing / restarting / dead) and is flagged stale after three missed intervals.
  A degraded source names itself in the header, in red. A monitoring tool that goes quiet used to
  look exactly like one with nothing to report.
- **A diagnostic log.** `AI_USAGE_LOG=1` (or a path) writes collector errors, panics and restarts
  to a file. The dashboard holds the alternate screen, so stderr was invisible; a panicking
  collector left no trace anywhere. Off by default — a usage monitor should not silently
  accumulate a log file.
- **The pricing refresh can discover models.** A scraped row previously had to match one of 65
  hardcoded `(display name, model id)` pairs to survive, so a newly launched model stayed unpriced
  until someone edited Rust source and cut a release — on the one code path whose purpose is to
  pick up pricing changes *without* a release. Ids are now derived from the display name; all 66
  entries of the deleted table are reproduced exactly by that rule, and that claim is a test. The
  deleted table was also where the Claude Opus dash/dot mismatch lived.
- **`.deb` and `.rpm` are now actually built.** `Cargo.toml` carried the packaging metadata and
  the changelog claimed the packages, but no job ever ran `cargo-deb` or `cargo-generate-rpm`.
  Both are built for amd64 and arm64, and each is asserted to contain the binary before publish.
- A real security reporting channel in `SECURITY.md`, replacing "once the project repository is
  published", together with the specific guarantees a report should be measured against.
- CI matrix across Linux, macOS and Windows, plus an MSRV job (`rust-version = "1.88"`),
  doctests, a CLI smoke test against the fixture database, `cargo-deny`, and Dependabot.

### Changed

- **The release workflow can be dry-run.** It only fired on tag push, so its first execution was
  also its first test — with a published release riding on the result. `workflow_dispatch` now
  exercises every build, architecture assertion, package build and inspection, checksum, and
  manifest render, and skips only the publish.
- **Dependabot groups GitHub Actions updates.** Ungrouped, it opened one PR per action; four sat
  open for weeks, went stale against main, and each needed an individual rebase before it could
  merge. Cargo updates were already grouped.
- **A stale pricing cache is ignored rather than trusted forever.** The refreshed cache overrode
  the bundled table with nothing to expire it, so a cache written before a rate change kept
  applying the superseded rate to new events indefinitely — found on a real machine, three weeks
  stale and still winning. Past 30 days the bundled table wins and the status line says why.
  Some models may become `UNKNOWN COST` as a result; that is the intended trade.
- **CI builds with `--locked`.** `AGENTS.md` told agents to always pass it while no CI job did —
  an instruction diverging from actual practice. CI now passes it on clippy, test, doc-test,
  check, and both release builds, so a dependency change that edits `Cargo.toml` without
  regenerating `Cargo.lock` fails rather than resolving a different dependency set than the one
  that was tested. `cargo fmt` is the exception; it resolves nothing.

- **ratatui 0.30, rusqlite 0.40, toml 1.1.** toml 1.1 rejects the pricing table when parsed as
  `toml::Value`, which would have left every model in the catalog unpriced — silently, with the
  dashboard still rendering totals; it parses as `toml::Table`. rusqlite 0.40 removed the `u64`
  impls of `FromSql`/`ToSql` (SQLite integers are signed), so counters round-trip explicitly and
  clamp rather than cast: a corrupt negative row reads as 0 instead of ~1.8e19 tokens in a cost
  total. MSRV stays 1.88. This also resolved the crossterm 0.28/0.29 double-compile.
- `deny.toml` no longer ignores RUSTSEC-2024-0436 — ratatui 0.30 dropped `paste`, so the crate is
  gone and the exception with it. `BSD-2-Clause` left the allowlist for the same reason: it is no
  longer in the tree, and an allowlist entry that matches nothing is a claim that has stopped
  being true.
- Journal fixtures are constructed in-test rather than read from a gitignored binary that did not
  exist on a fresh clone — which had let the pipeline test pass while covering nothing.

### Fixed

- **Pricing was retroactive** (audit finding 1.6). Cost was computed from whatever the table said
  *now*, so correcting a rate after a vendor price change silently re-priced every historical
  event — a request made in August got billed at September's price the moment someone edited a
  number. Rates are now effective-dated: a model entry can carry `[[model."x".period]]` blocks with
  a `through` date, and an event is priced at the rates in effect on the UTC day it happened. On
  real August `claude-sonnet-5` usage this is the difference between $3.27 and $4.91.
- **`claude-sonnet-5`'s introductory-to-list rate change is now encoded rather than pending.**
  Because pricing is effective-dated, both sides of the 2026-08-31 boundary are correct as
  written; there is no dated edit left to make and the calendar-guard test is gone with it.
  A refresh cannot erase a historical period — the scraper reads current rates and has no way to
  know what a rate used to be — but an overlay that supplies its own periods still wins, so a
  wrong recorded history can be corrected.
- **A dated comment was the only thing guarding a pricing deadline.** `claude-sonnet-5` runs on
  introductory rates that lapse after 2026-08-31; nothing read the comment saying so. A test now
  fails the build on 2026-09-01 with the exact replacement rates in its message, and guards the
  other direction too — applying list rates before the lapse date overcharges every request. All
  eleven Anthropic entries were re-verified against the `claude-api` skill; every rate, including
  the 0.1x cache-read and 1.25x cache-write multipliers, was already correct.
- **`--json | head` crashed.** `println!` panics when the write fails and a closed pipe is a
  write failure, so piping any output into `head`, `grep -q`, or a `less` the user quits out of
  aborted with "failed printing to stdout: Broken pipe". A closed pipe is now a clean exit. Fixed
  without `libc` or an `unsafe` block — the usual `SIGPIPE` fix needs both, and this crate has
  neither. The CLI smoke test only ever wrote to a file, so nothing caught it; it now pipes.
- **Two right-hand panels could both be "on".** `show_budgets` and `show_routing` were
  independent booleans and the draw order silently picked a winner. One `Panel` enum now.
- **A panicking collector was retired for the life of the process.** The supervisor recorded the
  panic and `break`, so that source never updated again while the UI kept showing its last
  numbers as current. Collectors now restart with capped exponential backoff and are marked
  `dead` only after five panics.
- **One panic under the state lock froze the dashboard permanently.** `if let Ok(mut s) =
  state.write()` turned every subsequent write into a silent no-op once the `RwLock` was
  poisoned. Poisoned guards are now recovered.
- **Collector threads could outlive their handle.** `shutdown()` only set an `AtomicBool` polled
  once a second and `Drop` never joined, so threads could still be mid-poll — holding a SQLite
  handle — after the handle was dropped. Shutdown is now a condvar and `Drop` joins.
- **The routing panel rendered a failed journal read as "no routing events."** The two are now
  distinguishable: read failures mark the dashboard degraded and name themselves.
- **The config file was parsed three times with three error policies.** `apply_config`
  hard-errored while the collector and budget loaders both `unwrap_or_default()`, so a typo in
  `[budgets]` silently disabled every budget while the same typo in `[collectors]` was reported.
  One read, one policy; parse and read failures are always reported. `[budgets]` with only a
  `webhook` and no entries is now valid rather than a parse error.
- **`--check-budgets` exited via `std::process::exit(1)`**, skipping every destructor including
  the collector join. It now unwinds, preserving the exit code.
- **Under-counted usage.** Deduplication keyed only on token counts, so two distinct requests
  with identical counts — routine in agent loops — silently collapsed into one. Events now carry
  a stable `event_id` (OpenCode message id, journal `event_id`) and fall back to shape *plus*
  timestamp.
- **Claude Opus lost its pricing after `--refresh-pricing`.** The scraper emitted
  `claude-opus-4-8` where the pricing table said `claude-opus-4.8` (likewise 4.5/4.6/4.7 and
  `claude-sonnet-4.6`), so a refresh silently unpriced the whole Opus family. A test now asserts
  every model id the scraper can emit resolves against the bundled table.
- **Whole-dollar rates were charged as $0.00.** The refreshed cache writes `input = 5`, a TOML
  integer, and the parser accepted only floats. Every whole-number rate was skipped and billed at
  zero. Rates now accept integers and floats alike.
- **Unpublished rates no longer become free.** Missing rate fields defaulted to `0.0`; a bucket
  with tokens but no published rate now yields `UNKNOWN COST`, honouring the project's
  never-convert-unknown-cost-to-zero invariant. An explicit `0.0` remains distinct from absent.
- **Reasoning tokens are now billed**, at the output rate unless a model publishes a distinct
  `reasoning` rate. They were counted in totals and displayed but excluded from cost.
- **A corrupt pricing cache can no longer wipe all pricing.** The cache is applied as an overlay
  on the bundled table rather than replacing it, and parse failures are surfaced as warnings
  instead of silently yielding an empty table.
- **`TODAY` and daily budgets agreed on nothing.** The dashboard used a rolling 24h window while
  budgets used a UTC calendar day. Both now use the local calendar day; the clock renders in
  local time.
- **Windows could not start.** Path resolution required `HOME`, which Windows does not set, so
  every lookup failed on a platform with published Scoop and Chocolatey packages. `USERPROFILE`,
  `%LOCALAPPDATA%` and `%APPDATA%` are now honoured.
- **`--webhook` silently did nothing.** `AlertDispatcher` was fully implemented but never
  constructed. Alerts now dispatch from both `--check-budgets` and the TUI, on a background
  thread, with URL scheme validation.
- **Misclassification from substring matching.** Provider `cloudflare` matched "cloud"; any model
  whose name contained "free" was treated as free and excluded from all cost totals. Matching is
  now token-based, and the free-model list is derived from the pricing table instead of a second
  hand-maintained copy.
- **First-party providers classified as `UNKNOWN`.** Anthropic, OpenAI and Google usage fell
  through to `UNKNOWN` and stayed there even after a cost was estimated, so the per-category tiles
  disagreed with the aggregate cost. Estimated rows are now promoted to `PAID`.
- **macOS releases shipped the wrong architecture.** The artifact labelled `x86_64-macos` was
  built on an Apple Silicon runner without `--target` and contained an arm64 binary. Each
  architecture now builds on a matching runner and the workflow verifies the binary before
  packaging. `aarch64-linux` and `aarch64-macos` are now published.
- **Every package-manager template 404'd.** They requested `ai-usage-tui-0.2.0-...` while releases
  publish `ai-usage-tui-v0.2.0-...`, and all carried `PLACEHOLDER_SHA256`. Manifests are now
  rendered at release time from the real artifact names and checksums.

### Changed

- Derived views (filtered set, grouped rows, totals, routing aggregates) are computed once per
  refresh instead of per frame. `draw` no longer clones the dataset ~8 times per frame, opens
  SQLite, or reads the clock per row. Collector merges use a hash index instead of a linear scan
  over a rebuilt key vector, removing quadratic growth on every poll.
- The model table scrolls: a selection past the fold used to disappear.

### Removed

- The unused `proptest` dev-dependency.

## 0.2.0 - 2026-07-24

### Added

- Background collector framework with `Collector` trait, `CollectorHandle`, and `std::thread`-based polling.
- Built-in collectors: `OpenCodeCollector` (30s), `JournalCollector` (60s), `ZenPricingCollector` (3600s, opt-in).
- `[collectors.<name>]` TOML config section with `enabled` and `interval` per collector.
- Budgets and alerts: `BudgetEngine`, `AlertDispatcher`, per-provider/model/global scopes.
- `[[budgets.entry]]` TOML config with `scope`, `period`, `limit`, `warn`, `critical`.
- `--check-budgets` (JSON output, exit 1 if alerts active) and `--webhook URL` CLI flags.
- TUI alert banner (yellow/critical) and budget panel toggle (`b` key).
- Calendar-based period cutoffs (daily at 00:00 UTC, monthly on 1st).
- In-memory alert dedup (1-hour window) for webhook dispatch.
- Model-routing analytics: `RoutingEvent` struct, `routing_event` journal table, `--record-routing` capture.
- `RoutingEngine` with aggregation (cost/task, token efficiency, retry/escalation/defect rates).
- `--routing-json` and `--routing-csv` export flags.
- TUI routing panel toggle (`t` key) with AGENT/MODEL/TOKENS/COST/RETRY%/DEFECTS/TASKS table.
- `--refresh-pricing` command that scrapes the Zen docs page into `~/.local/share/ai-usage-tui/zen-pricing.toml`.
- HTTP retry/backoff for rate-limited Zen pricing fetches.
- Fixture-based HTML parsing tests for the pricing scraper.
- Library crate conversion (`src/lib.rs`) enabling integration testing.
- Integration test suite covering full pipeline, config precedence, export formats, and pricing engine.
- Test fixtures for OpenCode DB, Ollama journal, and Zen pricing HTML.
- Cross-platform packaging: `.tar.gz`, `.deb`, `.rpm` (Linux), `.tar.gz` + Homebrew (macOS), `.zip` + Scoop + Chocolatey (Windows).
- `scripts/release.sh` pre-flight checklist (branch check, tests, clippy, build, version verification).
- Tag-triggered GitHub Actions release workflow with multi-OS matrix build, SHA256 checksums, and auto-generated GitHub Release from CHANGELOG.
- `[package.metadata.deb]` and `[package.metadata.generate-rpm]` Cargo.toml sections.
- Package manager templates: Homebrew formula, Scoop manifest, Chocolatey nuspec + install script.
- `docs/background-collectors.md` and `docs/routing-analytics.md` architecture docs.

### Changed

- TUI now uses background collectors by default; `--once`/`--json`/`--csv` stay synchronous.
- Converted to library crate (`src/lib.rs`) enabling integration testing.
- Graceful shutdown via `AtomicBool` flag; `Drop` impl triggers shutdown automatically.
- Budget spend only counts `ProviderReported`, `Calculated`, and `Estimated` costs.
- Privacy: routing events store only metadata — no prompts, completions, API keys, or credentials.

## 0.1.0

- Initial btop-inspired OpenCode usage dashboard.