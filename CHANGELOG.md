# Changelog

## 0.12.0 - 2026-09-01

### Added

- **GitHub Copilot is a data source.** The collector reads `assistant_usage_events` from
  whichever store under `~/.copilot` has that table — `session-store.db`, `session.db` or
  `data.db`, chosen by schema rather than name, because Copilot has moved the filename between
  releases. One row per model request, with real input/output/cache/reasoning counts and its own
  timestamp, so daily attribution needs no inference. Where no such table exists the legacy
  `session-state/<id>/events.jsonl` logs are read instead, and only their `session.shutdown`
  aggregates; those are cumulative, so the collector emits the difference from the last snapshot
  rather than repeating a resumed session's earlier turns. `--copilot-dir`, `copilot_dir`,
  `COPILOT_HOME`, and `[collectors.copilot]`.

  Rows are subscription-billed. A Copilot seat pays for premium requests rather than tokens, so
  there is no per-token rate to report: `cost` stays `null`, the row reaches `quota`, and the
  list-rate figure travels as `api_equivalent_cost` where it cannot reach a budget. Copilot has
  no API-key mode and therefore no environment signal, so `api_env_vars("copilot")` is an
  explicit empty list and an unevidenced decision resolves to subscription rather than falling
  through to per-token — which would have put invented dollars into budgets.

  Two Copilot shapes are deliberately left unread: the legacy per-message records, which expose
  an output count but record input as `0`, and VS Code's transcripts, which carry no counts at
  all. The usual fallback for both is to divide a character count by four and price the result.
  A token count inferred from message length is not a measurement, and priced it is
  indistinguishable in a total from a figure a provider reported.

- **`provenance` in `--json`, and a `PROVENANCE` block in `--doctor`.** `cost_status` has always
  carried this per row, and the header has always reduced it to one pricing-coverage percentage —
  which says how much of a range is priced, not how much of it a provider actually reported. The
  new block is the whole distribution: rows, requests, tokens and dollars for each of the seven
  statuses, plus `reported_share`, `billable_cost`, `quota_requests` and `unpriced_requests`.
  Every status is present whether or not it has rows, so a consumer can key on the shape, and
  `cost` is `null` rather than `0.00` for `quota` and `unavailable` — a zero would assert those
  rows were free. `reported_share` is `null` rather than `0` when there is no billable spend, on
  the same reasoning as `escalation_rate`. Derived from the same filtered rows the export already
  reports, so a `--provider` filter narrows both.

- **`--doctor` says why there is no Cursor collector.** Cursor writes
  `tokenCount.inputTokens`/`.outputTokens` as zero on current builds; its own team calls the
  field best-effort and unreliable and points at the web dashboard. There is no local
  measurement to read, so the only way to produce a Cursor row is to guess one from message
  length. `--doctor` now reports Cursor as installed-and-unread with that reason, and the README
  has a section stating it. A missing row is recoverable; a fabricated one is not.

### Fixed

- **The Copilot legacy path double-counted cached tokens.** `session.shutdown` aggregates use the
  same inclusive convention as the request table — `inputTokens` contains the cache buckets and
  `outputTokens` contains reasoning — and the legacy reader was not subtracting them back out.
  A session reporting 31,000 input including 12,000 cache reads came out as 31,000 input *and*
  12,000 cache read, inflating its total by the cache. Both unit tests covering that path used
  `cacheReadTokens: 0`, so neither saw it; an end-to-end run against a fixture with cache did.
  There is now a test with non-zero cache asserting `total_tokens()` matches what Copilot
  reported.
- **Two hermeticity gaps in the test suite.** `tests/cli.rs`'s `hermetic_with` never pinned
  `--gemini-dir`, so a developer who had switched Gemini CLI's telemetry on saw their own rows
  in a run that is supposed to read fixtures only, and `--doctor`'s hardcoded source list had
  never gained `gemini` — the drift the list exists to catch. Both fixed, and the
  screenshot renderer now requires the same two roots it requires for every other source, after
  twice rendering the author's real spend into README images.
- **`[collectors.<id>]` billing rejection named the wrong tables.** The error said `billing` and
  `config_json` "apply to `[collectors.claude_code]` and `[collectors.codex]`" long after Gemini
  had joined them. It is derived from the registry now, so it cannot drift again.

### Documentation

- **The README's "Data sources" section is the paths and the switches, not the parsers.** It
  was the longest section left, at about 175 lines, and most of them explained how each
  collector keys an event or splits a token bucket — detail a reader on the way to a first run
  does not need. Every registered source keeps its subsection (`tests/docs.rs` insists), reduced
  to the default path, the override flags, and one sentence on what is not read; the parsing
  detail lives in [`docs/provider-support.md`](docs/provider-support.md), which gained a
  "Pricing tables" section for the provider-qualified-key rules and absorbed the Ollama journal
  semantics and OpenCode path it had never stated. The README is 964 lines, down from 1,009.

## 0.11.0 - 2026-08-25

### Added

- **`--claude-code-hook`: Claude Code's own hooks as a routing harness.** Escalations were
  derived from collected usage, but a pass or a fail cannot be inferred from usage metadata and
  the roadmap said so: "a shipped hook or wrapper that emits `--record-routing` events from a
  real agent harness is what would close this, not more derivation." This is that hook.
  `contrib/claude-code/settings.json` registers the flag on `PostToolUse` and
  `PostToolUseFailure` for the `Bash` tool; the payload is read from stdin, and when it observed
  a test run — a recognised runner whose exit status the shell did not discard — one routing
  event is journaled: pass or fail, the model in use, and the attempt's requests, tokens and
  cost — every request in the transcript no earlier event of the session attributed — priced by
  the billing decision and rate table the dashboard already applies. On a Max or Pro account
  the attempt is `on quota`, exactly as its rows are in the model table.

  Three things were checked against what Claude Code 2.1.245 does rather than what its
  reference says, and all three differ. A non-zero exit fires `PostToolUseFailure` with the
  status in `error`, not `PostToolUse` with an `exit_code`; `PostToolUse`'s Bash response
  carries no exit code at all, so the outcome is which event fired and the snippet registers
  both; and the assistant line that issued a tool call is appended to the transcript *after*
  the tool and its hooks have run, so at hook time the transcript ends one request early. The
  first version of the hook bounded the attempt by time and lost that request from every
  attempt — its timestamp is before the hook, its line arrives after. The attempt is bounded
  by a cursor instead, the requests the session's events have already attributed, so each is
  counted once, one run late.

  What the hook withholds is the point. `cargo test 2>&1 | tail -20` exits with `tail`'s
  status, `cargo build && cargo test` fails when the build does, and `cargo test || true` never
  fails: a line whose status is not the runner's own is not an observation in that direction,
  and the reason is printed. No counter is ever sent — a hook cannot count retries — so the
  panel's RETRY, ESC and DEFECT columns read `—` for this agent, not `0%`. The attempt is
  attributed from the transcript with the collector's own `parse_line`, which reads the usage
  block, the model and the timestamp and nothing else; a test plants a credential in the
  content and fails if it reaches the event.

- **A new release is surfaced on the dashboard, not only by `--doctor`.** `[update] check = true`
  has been able to ask GitHub for the latest release tag since v0.9.0, but the answer was printed
  once and forgotten, so a user who never ran `--doctor` never learned a release existed. The
  check now writes its answer to `update-check.json` beside the pricing cache, and the dashboard
  reads it **once at startup** and names a newer release in its header. The indirection is the
  feature: the header redraws several times a second and must never acquire a network call or a
  clock read, so the check stays exactly where it was and only its answer travels.

  What is cached is the tag, never the verdict — a cache written before an upgrade would
  otherwise go on claiming an update after it — so the comparison is made against the running
  binary every time and a stale cache can only understate. `--doctor` reports a cached answer
  even when the check is off, because a user who turned it off and still sees the header notice
  has nowhere else to find out why.

  The notice also broke the header the way the footer broke once. It is 11 columns wide, an
  80-column header fitted exactly without it, and a `Paragraph` truncates in silence — so the
  collector status, the one thing on that bar that must never disappear quietly, went off the
  end. `LIVE PROVIDER MONITOR` is decoration and now yields to it, by measurement rather than at
  a written-down width.

  And it became the third thing to leak the author's machine into the README images. The
  screenshot renderer builds a real `App`, so on any machine with a cached answer every image
  would have carried `↑ vX.Y.Z` — a fact about one install, pinned into the README until the
  next regeneration, where it would read as a claim about the release being documented. It is
  cleared there now, as the clock is pinned; the two before it were the real OpenCode database
  and Omarchy's real rate-limit window.

- **`scripts/install.sh` says what it replaced.** Re-running it to upgrade was silent, so an
  upgrade and a fresh install looked identical. It now names the version it is replacing, says
  "reinstalling" when the tag already matches, and reports when the existing binary's version
  could not be read at all. It also covers the case that actually bites and that nothing
  anywhere caught before: after installing, if the name still resolves to a *different* copy
  earlier on `PATH`, it says so and names both paths — two copies from two channels, with the
  user upgrading one and running the other, is the failure `--doctor`'s channel detection was
  written for. Deliberately never "upgrading" or "downgrading": ordering two versions needs more
  than a string comparison and `sort -V` is not POSIX, so both are named instead.

### Changed

- **The footer reads the bindings table, and fits itself by measuring.** `ui::keys` was written
  to end the hand-written copies of the key bindings; the footer kept its own, because it
  abbreviates (`1-4`, `j/k`) and folds the panel keys into one run below a width. That width was `120` — the full line's width on the day it was written, and a fact
  about the table's contents in disguise: one more panel and the line would have overrun it
  and truncated again, which is exactly how `q quit` vanished from 80-column terminals once
  before. Each binding now carries its footer spelling, `keys::footer_forms` derives the full,
  folded and help-and-quit-only forms from the table — the fold is by `Action::Panel`, so a new
  panel is in it without anyone remembering — and the footer takes the widest form whose
  measured width fits. A test renders every width from 16 to 200 columns and fails if the line
  is ever cut off; another pins that the form changes exactly one column short of fitting, with
  the widths measured rather than written down. The review of this change also retired the
  unchecked key table in `docs/omarchy.md`, which had already drifted (it still said `Esc`
  quits), brought the "add a dashboard panel" recipe in `CONTRIBUTING.md` up to date with where
  the bindings live and what a new one must carry, and dropped `Binding::alias`, a field
  written on every binding and read by nothing.

## 0.10.0 - 2026-08-25

### Fixed

- **A budget over unpriced or subscription-billed work read as untouched.** `spend_for_scope`
  summed the priced rows into a bare `f64` and dropped the rest, so a request whose rate was
  unknown, or one billed against a Max or Pro quota, contributed nothing and was counted nowhere.
  On a subscription account that is every Claude Code row: a `$50` monthly budget read
  `$0.00 / 0% / OK` for as long as it was configured. Convention 1, in the one figure the tool
  *acts* on — the webhook fires on it, `--check-budgets` sets its exit code from it, and the
  Omarchy meter is drawn from it.

  The guard already existed, and this was the one rollup that bypassed it. `accrue` folds a row
  into a cost floor plus `unpriced` and `quota` counters, and its own comment records that four
  copies of that logic were collapsed into one; the budget engine was the fifth copy, written
  before the counters existed. It goes through `accrue` now — moved to `model.rs`, next to the
  `CostStatus` predicates it is written in terms of, because the budget engine is not UI — and
  `Alert` carries `unpriced_requests` and `quota_requests` beside `spend`.

  The panels say what the figure is standing on, in the vocabulary the rest of the dashboard
  already uses: `≥ $2.00` and `≥ 4%` when part of the period is unpriced, `on quota` when the
  period's work is all plan quota, and `—` for a budget nothing has checked yet, which used to
  print `$0.00 / 0% / OK`. The burn panel's projection was the worst case — it rendered the rate
  as `≥ $4.10/hr (37 unpriced)` and, two lines down, computed time-to-exhaust from a spend that
  had dropped those same 37 requests. It now reads `≤ 2h 14m left` when either the rate or the
  spend is a floor, and marks the figure in brackets only when the spend is: the two markers
  say which. Its "too little activity to project (550/5 requests)" — measured on a Max account,
  over a window of 550 quota-billed requests — blamed the request count when the rate was zero
  because nothing was priced per token; it now says which condition failed: `on quota`,
  `unpriced`, or too few requests. The Omarchy record's status names each budget whose meter is
  a floor, not only the tab's own rows — a budget is computed over every source, so the rows
  can be fully priced while the meter is not — and reads `Budget on quota` when a budget's
  period is all plan quota, where the meter draws 0 % over real work and cannot say so itself.

  `--check-budgets` and the webhook payload gain `unpriced_requests` and `quota_requests`. The
  two had been hand-copied literals of the same shape, and are one `Alert::to_json` now so a
  field cannot reach one and miss the other. `BudgetPeriod::label` replaces the five
  hand-written `daily`/`monthly` matches and the budgets panel's lower-cased `Debug` name.

  Deliberately unchanged: the exit code and the webhook still act on the floor, so a budget that
  is `OK` on its priced spend is not reported, however much of it is unpriced or on quota. On a
  Max account `--check-budgets` still prints an empty `alerts` and exits `0`. Emitting every
  partial budget would change what `alerts` means for every script that reads it; that is a
  contract decision, not a rider on a fix, and the panel is where the floor is visible.

- **The routing panel's `RETRY`, `ESC` and `DEFECT` columns read `0%` for an agent nobody had
  measured.** `RoutingEvent.retries`, `escalations` and `review_defects` were bare `u32`s, so an
  emitter that omitted a field stored `0`, and `retry_rate` divided that by the task count: an
  agent whose harness never counted retries and one that never needed any rendered the same
  `0%`. `success_rate` beside them already returned `Option` for exactly this reason. And the
  rate was `retries / tasks`, so an emitter writing `retries: 3` on one task rendered `300%`.

  The first was latent only because no emitter exists yet — a human typing `--record-routing`
  JSON types every field, and an automated one would not, which is why the roadmap's P0 gated
  any harness on this class of bug. The second was live for any event with more than one retry;
  the round-trip test carried one and asserted the sum, never the rate.

  The three counters are `Option<u32>` on the event and one `ObservedCount` — sum, tasks that
  reported, tasks affected — on the aggregate, with a single `rate()` rather than three more
  copies of the guard `success_rate` carries. A rate is the share of *observed* tasks affected,
  so it cannot exceed 100%, and `None` when nothing reported, which the panel renders as `—` and
  sorts to the end in both directions the way it already held an unknown cost. Sorting by `PASS`
  orders by the rate the column shows; it sorted by the raw pass count, which ranked one-of-one
  at 100% below five-of-ten at 50%.

  The journal's three columns were `INTEGER NOT NULL`, so they could not store "not reported".
  `--record-routing` rebuilds an older journal's table in place, once, in one transaction. Rows
  already there keep their zeros — that is what was recorded, and rewriting it as unknown would
  be inventing in the other direction. Only rows written from here on can say nothing was
  reported.

  `--record-routing` refuses what it used to launder: a counter that is not a non-negative
  integer, or a `test_result` that is not a boolean, `0`/`1`, `"pass"` or `"fail"`, is an error
  rather than a `0` or a `null` stored under a success message. The round-trip test had been
  sending `"test_result":"pass"` since it was written and asserting the three counters beside it
  and not the result. A supplied `event_id` is honoured — an empty one is not an identity, as
  `usage_key` already holds — so two events for one task in the same second no longer collapse
  into one. A bad event is refused before the journal is opened, so it neither creates the file
  nor rebuilds the table on its way to the error. And a `cost` sent without a `cost_status`
  records as `reported`: the default was `unavailable` either way, and since v0.9.0's aggregator
  classifies by status rather than trusting the number, the README's own example — `"cost":0.02`,
  no status — recorded a task the panel then called `unpriced`.

  `--routing-json` reports `retries`, `escalations`, `review_defects` and the three rates as
  `null` rather than `0` when nothing was reported, and gains `retries_observed`,
  `escalations_observed` and `review_defects_observed` so a script has the denominator.
  `--routing-csv` leaves those three fields empty and **appends** the three denominators after
  everything else, never between. The `escalation_rate` key carried opposite contracts in the
  two exports — null when unobserved in `--json`, `0` in `--routing-json`; they agree now.

- **The README images showed a retry rate the code no longer produces, and regenerating them
  showed something worse.** The screenshot renderer pinned four of the five sources and not
  Omarchy's records, so the header carried the author's real rate-limit window and the billing
  decision read their plan tier: every Claude row in the demo went `quota`, in images whose
  caption promises no real account appears. `--omarchy-dir` is required now, like the other four,
  and the images are regenerated from the fixture alone.

- **The Projects and Routing panels draw their cursor, and the row it is on.** Both rendered a
  plain table with no highlight and no viewport: `j` moved a cursor nothing showed, past the fold
  where nothing was drawn, and on Projects `Enter` then drilled into a project the user had not
  seen. The model and session tables already did this right; these two now use the same
  highlight and the same `TableState`, so the selected row is marked and scrolls into view. The
  routing cursor was also clamped to the *model* table's length — cosmetic while nothing drew it,
  and a row the user could see and never reach once something did.

- **A budget that could never fire is refused when the config loads.** `deny_unknown_fields`
  caught a misspelled key; nothing caught a missing one. A `provider` or `model` entry with no
  `name` became `Provider("")`, which matches no row, and a `limit` of zero made `pct` zero
  forever — both sat in the panel at `OK` looking configured. `[[budgets.entry]]` is validated
  the way `[collectors.*]` already was: a scoped entry needs its `name`, `global` takes none,
  `limit` must be above zero, `warn` and `critical` are percentages, and `warn` must sit below
  `critical`. The error names the entry by number and says what it needs.

- **A journal row that cannot be read is no longer dropped in silence.** Both journal readers
  ended in `filter_map(Result::ok)`, so a corrupt row — or one written by a newer version —
  vanished and the rest were reported as the whole. Convention 8. The routing reader is strict
  now: its two callers already put an error where it is seen, on the dashboard's status line and
  as a refused export rather than a partial table. The usage reader keeps the rows it can read,
  because one corrupt Ollama row must not take every command down, but counts the ones it
  cannot: the count is on the source's status line — the `--once` header and `--json`'s
  `source` — `--doctor` adds the row's id and the first reason, and the live dashboard marks the
  source degraded in its header with the count, rather than writing it to a log that is off by
  default. A file that cannot be stepped past a point is the whole read's error, distinct from a
  row that steps and does not map.

- **`--doctor` tells the truth about pricing, and a tier that cannot be read prices nothing.**
  `PricingEngine::load` collected warnings — the refreshed cache is unreadable, invalid, or too
  old to trust, so bundled rates are in use — and nothing printed them: the fallback happened in
  silence and an `UNKNOWN COST` row had no explanation anywhere. `--doctor` has a `PRICING`
  section now, with the model count, the cache's path and age, and every warning. Its
  `zen_pricing` line also described the wrong file: the Zen *catalog* that `--refresh-zen`
  writes, which nothing prices from, with a hint to populate it — while the pricing cache that
  `--refresh-pricing` writes and the engine reads was reported nowhere. It reports that file.

  Two tier parsers defaulted an unreadable threshold to `0`: the curated table's `tier-<junk>`
  keys and the scraper's `(> lots tokens)` display names. A tier at zero matches every request
  with a nonzero prompt, so a typo in a hand-edited table applied the long-context rate — roughly
  double — to everything for that model, and nothing said so. Both refuse now: the table
  reports the key in a warning and the scraper skips the row, and the base row survives either
  way. A lowercase `k` in a scraped name is a number, not a typo.

- **The routing panel's default order survives a refresh.** `refresh` read the routing journal
  *after* `recompute`, which is where the panel is sorted and the cursor clamped, so every refresh
  left the table in `aggregate`'s token order until the next key press — and the README's own
  screenshot showed `$0.60` above `$0.07` in a panel titled "cost per delivered result". The
  tests never saw it because they inject aggregates through a path that sorts. It is read before
  `recompute` now, and one test goes through `refresh` with a real journal.

## 0.9.0 - 2026-08-25

### Added

- **`--doctor` says how this copy was installed and how to upgrade it.** Seven install channels
  ship — cargo, binstall, Homebrew, Scoop, Chocolatey, the `.deb`/`.rpm`, and `install.sh` — and
  until now nothing in the tool knew which one it came from or mentioned upgrading at all. It is
  read off the running binary's own path, so it needs no network and works offline.

  A location none of the channels explains reports itself as unrecognised and points at the
  releases page, rather than guessing. Naming the wrong command is worse than admitting ignorance:
  running it installs a *second* copy elsewhere on `PATH`, and the user upgrades a binary they are
  not running.

- **An opt-in check for a newer release.** `[update] check = true` lets `--doctor`, and only
  `--doctor`, ask GitHub for the latest release tag. **Off by default**, never automatic, never on
  the dashboard's refresh path — the same stance `zen_pricing` takes, and for the same reason: a
  tool whose pitch is "reads usage metadata, writes nothing, transmits nothing" does not get to
  contact a server because it would be convenient. A failed check is reported, not swallowed; a
  check that silently returns nothing is indistinguishable from one that found nothing newer.

### Fixed

- **The routing panel ranked unpriced work as the cheapest on the machine and called it free.**
  `aggregate` summed `cost.unwrap_or(0.0)`, so an agent whose spend could not be priced arrived
  with `cost: 0.0`; `cost_per_success` divided that by its passes and returned `Some(0.0)` rather
  than "no figure". The panel's default sort is `$/SUCCESS` **ascending**, so the row sailed into
  first place, rendered green as `free`. On a Max or Pro account that is where all of the Opus
  work lands.

  The guard against exactly this already existed and fired one layer too late: `cost_order` holds
  a row with no figure at the end of the ordering in both directions — but by the time it ran, the
  unknown had been laundered into a zero upstream. Convention 1, broken in the panel that carries
  the project's pitch.

  `RoutingAggregates.cost` is a **floor** now, read with `priced_tasks`, `unpriced_tasks`,
  `quota_tasks` and `free_tasks`, the way `Transition::cost_after` is read with `unpriced_after`
  and `quota_after`. The `$/SUCCESS` cell reports what the figure is standing on, in the
  vocabulary the escalations block already uses — `$0.42`, `$0.42+q`, `on quota`, `≥ $0.42`,
  `unpriced`, `free`, `—` — and only a real figure or a genuine zero takes part in the sort.

  `--routing-json` reports `cost` as `null` rather than `0` when nothing was priced, and gains
  `priced_tasks`, `unpriced_tasks`, `quota_tasks`, `free_tasks`, `cost_per_success` and
  `cost_basis`. `--routing-csv` appends the four counters after its existing columns, never
  between them.

- **A pricing refresh never reached the running dashboard.** The engine was built once when the
  collectors were spawned and never replaced, while the `zen_pricing` collector wrote a refreshed
  cache to disk and returned no rows. So a successful refresh changed nothing until restart: rows
  stayed `UNKNOWN COST` although the rate was now known, and the log reported success while the
  screen disagreed. Convention 8.

  A collector can now declare that its work re-prices everything else; only `zen_pricing` does.
  The engine is rebuilt *before* the write lock is taken — parsing ~3,450 keys while holding it
  would block `snapshot()` on the render thread, which is the mistake the original one-time load
  was written to avoid.

- **One identity, enforced by CI.** The project's description of itself was hand-copied into seven
  files plus GitHub's About box, in five different wordings — and GitHub's copy named two of the
  five sources this tool reads. `Cargo.toml` is the single source of truth now: crates.io reads it
  verbatim at publish, `src/cli.rs` takes clap's `about` from `env!("CARGO_PKG_DESCRIPTION")`, the
  packaging manifests carry `__DESCRIPTION__` and `__TOPICS__` and are rendered at release time,
  and twelve guards in `tests/docs.rs` fail the build when anything else diverges.

  GitHub is the one consumer with no manifest, so `.github/workflows/identity.yml` enforces it —
  and does so without being able to pass silently. Updating a repository's description or topics
  needs Administration: write, which the built-in `GITHUB_TOKEN` can never hold; reading them
  needs only Metadata: read, which it always has. So the job pushes with a PAT and **verifies with
  `GITHUB_TOKEN`**: a missing or expired secret makes the build red rather than skipping it, which
  is the failure mode the `TAP_TOKEN` gate already has.

  Adding a data source now requires naming it — in the crate description, the README's opening, a
  README section and a GitHub topic. Gemini CLI shipped in v0.7.0 and never reached the README's
  first paragraph, because nothing looked.

- **Sortable columns.** `<` and `>` (or `,` and `.`) move the sort to the previous or next column
  of the visible panel, `o` reverses it, and the sorted column carries a marker in its header so
  the order is never a mystery. Models, projects, sessions and routing each keep their own sort —
  a column index means different things on different panels.

  The defaults reproduce the orders these lists have always had, so nothing moves until a key is
  pressed. Unknown cost sorts to one end rather than being interleaved as `$0.00`: a row whose
  price is unknown is not a cheap row.

- **The man page described the parser, not the tool.** `struct Args` in `src/cli.rs` carried a
  `///` doc comment explaining why it is separate from `Cli`. Clap promotes a doc comment on the
  parser struct to `long_about`, so that paragraph was the DESCRIPTION section of
  `ai-usage-tui --man` — shipped in the `.deb` and the `.rpm` and installed to
  `/usr/share/man/man1/`. `man ai-usage-tui` explained the clap migration, `Option<T>` and all.

- **`--help` and `--completions` no longer panic when the reader closes the pipe.**
  `ai-usage-tui --help | head` aborted with "failed printing to stdout: Broken pipe" while
  `--json | head` ended cleanly: `print_help` finished with a bare `println!`, and
  `clap_complete::generate` `.expect()`s on the writer internally, so handing it stdout panicked
  inside clap where this crate could not catch it. Completions render to memory first. A reader
  closing the pipe is a normal end to a pipeline, which `--json` and `--once` already knew.

- **The README no longer says crates.io and the Homebrew tap do not exist.** Both have since
  v0.6.0; the notices survived three releases because nothing checked them. `tests/docs.rs` now
  bans the phrasing, binds every `packaging/` template to the README prose that names it, and
  `identity.yml` checks each release channel the README documents actually exists.

- **A Gemini telemetry file rewritten in place could kill its collector.** The stale-offset guard
  in `src/collector/gemini.rs` covered a file that shrank but not one whose offset landed inside a
  multi-byte character, and slicing on a non-boundary index panics. `catch_unwind` contained it,
  so the symptom was Gemini restart-looping and going `Dead` rather than a crash.

- **A panic in the dashboard now says what happened.** The terminal was restored *after* the
  unwind began, by which point the default panic hook had already written the message to the
  alternate screen — which `LeaveAlternateScreen` then discarded. A user who hit a panic saw a
  clean prompt and nothing else. The hook restores the terminal first.

- **`just check` runs what it says it runs.** It stopped after the doc tests, omitting the `msrv`,
  `docs` and `audit` jobs — three of the seven — while its own comments claimed it ran "everything
  CI runs, in CI's order". The Markdown link checker moved from an inline heredoc in `ci.yml` to
  `scripts/check-markdown-links.py` so both callers run the same code.

- **Leaving a project drilldown finds the project by name, not by the row number it was at.**
  Sorting, a `/` filter or a refresh that adds a project can all move it while the user is
  inside, and returning them to whatever now sits at the old index would put the cursor on the
  wrong project without saying so.

### Changed

- **The release binary is built with `lto = "thin"`, one codegen unit, and stripped.** There was no
  `[profile.release]` at all; CI stripped on Unix as a separate step and not on Windows, so the
  Windows archive shipped a needlessly larger binary. Deliberately not `panic = "abort"`: the TUI
  restores the terminal from a `catch_unwind`, and a panicking collector is caught and restarted.

- **The routing panel no longer re-sorts inside its draw call.** It ranked by cost per delivered
  result on every frame — computation on the render path, which the dashboard forbids, and which
  would have silently discarded the new sort. That ranking is now the panel's default sort,
  computed once per refresh. The visible order is unchanged.

- **The sessions list orders by the time column it displays.** It ordered by `last_seen` while
  showing `first_seen`, so the column a reader saw was not the column the rows were in. Sorting
  by STARTED now means what it says. Sessions that started earlier but ran longer will move; a
  test distinguishes the two orders rather than assuming they agree.


## 0.8.0 - 2026-08-24

### Added

- **`/` filters the rows a panel lists.** Matches model and provider names, project paths,
  session ids and the models a session used, case-insensitively. `Enter` keeps the filter and
  hands the keyboard back; `Esc` clears it; `Backspace` shortens it and, once empty, leaves.

  It changes **what is listed, never what was spent** — the header totals, the pricing-coverage
  figure and the budgets stay computed from the whole range, because "which rows am I looking at"
  and "what did I spend" are different questions and answering the second with the first is how
  two views of one range end up disagreeing about money. `--provider` and `--model` still narrow
  the data itself. The footer carries the query and a "showing N of M", so a shortened list is
  never mistaken for a smaller bill.

  While a filter is being typed every printable key belongs to it rather than the dashboard —
  otherwise typing `budget` toggles four panels and quits. `Ctrl-C` still works mid-search.
  `Esc` now backs out one step at a time: clear a filter, leave a project, then quit.

- **Drill from a project into its sessions.** `Enter` on a project row scopes the sessions view
  to that project; `Backspace` (or `Esc`) goes back to the row it started from. The panel title
  names the project it is scoped to, so a project's spend cannot be misread as the whole
  machine's. Usage with no recorded project is filed under one row and drills in like any other.

  This is the first *navigational* state any panel has had — every other answers "show me X" and
  is stateless, while this one answers "show me X, inside Y". It is one field holding the project
  and the row to return to, and the narrowing happens in `recompute`, not in the draw call, so
  the render path stays free of computation.

  `Esc` now means "back" when there is somewhere to go and "quit" otherwise. That is the one
  documented binding whose meaning became contextual; `q` and `Ctrl-C` always quit.

- **Derived escalations are exported.** `--json` gains an `escalations` object — sessions
  examined and escalated, the rate, unclassified changes, and each transition with the spend
  after the move. This was the routing panel's derived block and was visible only in the
  dashboard; `--json` carried usage rows and nothing answering "did sessions move to a pricier
  model, and what did that cost".

  Derived from the same filtered rows the export reports, so a `--provider` or `--days` filter
  narrows both and a script cannot disagree with the dashboard about one run. `escalation_rate`
  is `null` rather than `0` when no session had enough information to examine, and `cost_after`
  is a floor rather than a total whenever `unpriced_after` or `quota_after` is non-zero. The
  block is always present, like `limits`, so a consumer can key on it.

  Not added to `--routing-json`, which reads recorded routing events and nothing else: these are
  *inferred* from usage, and the dashboard labels the two as different things. Not added to
  `--csv` either — the usage CSV is one flat table whose columns are appended-never-inserted, and
  transitions are a different shape.

### Fixed

- **The dashboard cursor was clamped by the model table on every panel.** `recompute` ended with
  `selected.min(view.rows.len() - 1)` regardless of which panel was showing, so on a machine with
  three model groups and ten projects the cursor could never reach the fourth project. Harmless
  while nothing acted on the row under it, and not harmless once `Enter` drills into it. The
  clamp and `visible_rows` are both panel-aware now.

- **The Gemini telemetry format is validated against real output, and two things it got wrong are
  fixed.** The parser was derived by reading `@google/gemini-cli` 0.56.0's serialization code and
  shipped in 0.7.0 unconfirmed. It is now checked against bytes that CLI actually wrote —
  `tests/fixtures/gemini_telemetry.json` is a redacted capture, with two tests pinned to it.

  No Gemini account was needed: `GOOGLE_GEMINI_BASE_URL` points the CLI at a local stand-in for
  Google's API, so the real CLI, its real OpenTelemetry SDK and its real `FileLogExporter` produce
  the file with nothing leaving the machine and nothing billed. The reproduction is in
  `docs/provider-support.md`.

  Everything the format notes claimed held — concatenated pretty-printed JSON, `attributes` as a
  top-level sibling of the OTLP wrapper, the token attribute names, and the cache count sitting
  inside the prompt count. Three things they did not predict, now covered by the fixture:

  - Metric records carry **no `attributes` key at all**; anything indexing `["attributes"]` would
    break on them.
  - `resource` carries the host name, home directory paths and the full command line, prompt
    included. Only `attributes` is read, and there is now a test asserting that against the real
    block rather than a synthetic one.
  - One prompt produced **six `api_response` records sharing a `prompt_id` and an identical
    `total_token_count`** — only the timestamp separated them. Keying identity on `prompt_id`, or
    on `prompt_id` plus the total, would have reported one request instead of six.

## 0.7.0 - 2026-08-24

### Added

- **Shell completions and a man page.** `--completions SHELL` (bash, zsh, fish, elvish,
  powershell) and `--man` generate from the same `Command` that parses the arguments, so they
  cannot describe a flag that does not exist. The `.deb` and `.rpm` install them into
  `usr/share/man/man1/`, `usr/share/bash-completion/completions/`, `usr/share/zsh/site-functions/`
  and `usr/share/fish/vendor_completions.d/`; the release archives carry them too. `just assets`
  produces them locally, and the release job generates them from a **host** build — a man page is
  architecture-independent and an aarch64 binary cannot be run on the x86_64 runner that built it.

- **Gemini CLI collector**, the first source added since the registry landed — a module plus one
  registry line, as advertised. Reads Gemini's OpenTelemetry log and reports usage per API
  response, with cost estimated from the bundled tables (`gemini/gemini-2.5-pro` and friends
  resolve because pricing keys are provider-qualified now).

  **It is opt-in, and the setup is Gemini's, not ours.** Unlike Claude Code and Codex, Gemini CLI
  persists no usage anywhere by default: session totals live in UI state and are lost on exit, and
  saved chats hold conversation history with no token counts. The only durable record is its
  telemetry log, which is off until you add
  `{"telemetry":{"enabled":true,"target":"local","outfile":"~/.gemini/telemetry.json"}}` to
  `~/.gemini/settings.json`. `--doctor` prints that line when the file is missing, so the source
  reads as "not set up" rather than "empty". This tool never edits Gemini's settings.

  Configure with `--gemini-dir`, `--gemini-billing` and `[collectors.gemini]`; Gemini's own
  `GEMINI_TELEMETRY_OUTFILE` is honoured when set.

  Three details the format forced, all documented in `docs/provider-support.md`:

  - The log is **concatenated pretty-printed JSON**, not JSONL, so it cannot be split on newlines.
    The reader consumes only complete top-level objects and advances its offset to the end of the
    last one, because a poll can land mid-record while the CLI is writing.
  - Google reports cached tokens *inside* the prompt count, unlike Anthropic which reports them
    alongside input. They are subtracted so a cached token is not billed as fresh input as well,
    and `toolUsePromptTokenCount` is likewise already inside the prompt count and not added again.
  - One `prompt_id` covers a whole tool-use loop, so several responses share it. Identity is
    `prompt_id` + timestamp + total, because keying on `prompt_id` alone would deduplicate real
    requests away and under-report spend.

  Model output never reaches a usage record: the same telemetry carries `response_text` when
  `telemetry.logPrompts` is on, and a test plants a credential there and fails if it appears.

- **LiteLLM is now the base pricing source: 60 models priced to 1,491.** `pricing/litellm.tsv`
  ships in the binary — ~3,450 keys across 88 providers, generated from
  [LiteLLM's community table](https://github.com/BerriAI/litellm) by
  `scripts/refresh-litellm-pricing.py` (`just pricing`). The curated `pricing/zen.toml` is applied
  on top of it for Zen-specific and stealth models, 13 of which appear in no community table, and
  a refreshed cache on top of that. No network is needed and the existing "an overlay never
  replaces" invariant is unchanged. Costs +35KB to the packaged crate and +5ms to startup.

- **Pricing keys can be provider-qualified, and the provider on the usage row is used.** The same
  model bills differently at Bedrock, `bedrock_converse` and the aggregators — 20% apart for
  Claude Sonnet 4.5 — and that is now priced correctly instead of resolved by bare name. Where
  providers disagree on a name (180 of them) the generated table publishes **no bare key at all**,
  so a model whose provider is not recognised stays `UNKNOWN COST` rather than borrowing another
  provider's rate. Long-context tiers come through too, including the 200k and 272k ones.

  Layering still outranks specificity: a hand-checked rate in `zen.toml` wins over a
  provider-qualified community one, and the dated `period` records only the curated table carries
  are never bypassed.

### Changed

- **The CLI is parsed by `clap` instead of a hand-rolled loop.** The 33-flag `match` in
  `parse_cli` is gone. `clap` handles `--help`/`--version`, reports unknown flags with a
  suggestion, lists the valid values for `--claude-billing` rather than only naming them in prose,
  and expresses the eleven-way action exclusion as a group instead of a hand-counted loop.

  `parse_cli` keeps its signature and still returns the same `Cli`, so `main.rs`, `config.rs`,
  `SourceRoots::from_cli`, the exporters and the UI are untouched. The command line is a separate
  `Args` struct converted into `Cli`, because clap's natural shape is `Option<T>` while `Cli`
  carries the `*_set` booleans `apply_config` reads to decide whether a config value may fill a
  field.

  `tests/docs.rs` now **queries** the parser for its flags rather than regexing `src/cli.rs` for
  `"--flag" =>` match arms. The companion guard comparing `--help` to the parser is deleted: clap
  generates the help from the same definitions, so that invariant is structural rather than
  tested.

  Three behaviours were preserved deliberately, each with a test, because clap's defaults differ:

  - **A repeated flag takes the last value.** Clap rejects repeats by default, which would break
    layering a default in an alias and overriding it — and broke the test harness, which passes
    `--omarchy-dir` itself and again per test.
  - **Range flags combine, last one wins.** A clap `group` would reject `--week --today`, and a
    fixed priority would make `--week --today` and `--today --week` mean the same thing. Neither
    matches a parser that simply assigned as it walked the arguments.
  - **`--once --refresh-pricing` is still accepted.** The old exclusion was two rules, and the
    second omitted `--refresh-pricing`. Transcribed rather than tidied: a group covering all
    eleven would have silently "fixed" an asymmetry that has always worked.

  Two error messages changed wording, and their assertions now check substance rather than
  phrasing: an unknown flag ("unexpected argument" rather than "unknown option", now pointing at
  `--help`) and an invalid billing mode (which now lists the three valid values).

  Costs +1.1 MB to the release binary (9.6 → 10.7 MB) and +18 KB to the packaged crate. Startup is
  unchanged at 10 ms. `clap` and friends need Rust 1.85, under the 1.88 already pinned.

- **Aggregators and clouds are classified as `PAID` rather than `UNKNOWN`.** OpenRouter, Bedrock,
  Azure, Vertex, Fireworks, DeepInfra, Together and Perplexity all bill per token, and the bundled
  pricing table now carries provider-qualified rates for them — 106 keys for OpenRouter, 111 for
  Azure, 82 for Bedrock. An OpenRouter row's `anthropic/claude-3.5-sonnet` reduces to a bare name
  and re-qualifies against `openrouter/`, so these rows both classify and price.

  This never causes a row to be priced: pricing is the table's decision, and a row that gets a
  figure was already promoted to `PAID`. What changed is the category of rows that *cannot* be
  priced — "real spend, rate unknown" instead of "no idea what this is". Such a row keeps `cost`
  unknown and counts against the pricing-coverage figure, so the gap stays visible rather than
  hidden in `UNKNOWN`. The README's category table said `PAID` meant "usage with a known billable
  cost"; it now says what the code does.

### Fixed

- **`tests/cli.rs::hermetic()` was not hermetic: it never pinned `--journal`.** It pinned the
  OpenCode database, the Claude Code root, the Codex home and the Omarchy directory, and left the
  usage journal to resolve from the environment — `AI_USAGE_JOURNAL_PATH`, else
  `$XDG_DATA_HOME/ai-usage-tui/usage.db`. Every CLI test therefore read whatever journal the
  developer's own machine had. CI never caught it because a fresh runner has no journal.

  It was not theoretical: with one journaled Ollama response present, a fixture-only `--json` run
  returns 10 rows instead of 9, and `a_disabled_source_is_disabled_for_the_exports_too` — added in
  v0.6.0 — fails outright. Any contributor with journaled Ollama usage would have hit it on their
  first `cargo test`. The same omission was in the documented commands in `CONTRIBUTING.md`,
  `docs/roadmap.md` and the `justfile`'s `run` recipe; all four now pin the journal.

## 0.6.0 - 2026-08-24

### Changed

- **Key bindings are defined once, in `src/ui/keys.rs`.** They existed in five places — the event
  loop's `match` arms, the `?` overlay's `ROWS`, the `KEYS` block in `--help`, the README's panel
  table, and prose in `AGENTS.md` — with nothing keeping them in step, so adding a panel meant
  remembering five edits. The first three now read one table, `tests/docs.rs` fails the build when
  the README's table disagrees with it, and a test fails if a `Panel` variant has no key at all
  (a panel the user cannot open). `AGENTS.md` points at the table instead of restating it.

- **`src/ui/tests.rs` (1837 lines) is now `src/ui/tests/`, one file per area.** It was the only
  home for the projects, coverage, time-series, burn, sessions, routing, breakdown and limits
  panels plus the SVG renderer and the key reference, with nothing but reading order separating
  them. The shared fixtures stay in `mod.rs`; the largest test file is now 218 lines.

- **The path resolvers in `src/utils.rs` take an injected environment.** Their tests called
  `std::env::set_var`, which mutates state every other test in the process shares — Cargo runs
  tests as threads, not processes — and which is `unsafe` from edition 2024 onward. They now pass
  a fixed lookup, mirroring how `collector::billing::Signals` already injects its environment, and
  gained coverage for the Windows `USERPROFILE`/`HOMEDRIVE` fallbacks and the XDG precedence rules.
  One behaviour change falls out: a variable that is *set but empty* (`OPENCODE_DB_PATH=`) now
  falls back to the default instead of resolving to an empty path that opens nothing.

- **CI gained a docs job and its advisory check got faster and more timely.** `cargo doc` with
  warnings denied, plus a relative-link check across every Markdown file — the kind of breakage
  a doc-only PR causes and nothing caught. `cargo-deny` now runs from a prebuilt action instead
  of a from-source `cargo install` on every run, and on a weekly schedule as well as on push: an
  advisory published against an unchanged dependency produces neither a push nor a PR, so it was
  previously never noticed.

- **One source registry, replacing two hand-maintained wirings.** The set of data sources was
  wired independently in `collector::load_usage` (used by `--json`, `--csv`, `--check-budgets`,
  `--omarchy-record` and the dashboard's own refresh) and in `main::build_collectors` (background
  polling). `CONTRIBUTING.md` documented only the second, so a provider added by following it
  appeared in the dashboard and was silently absent from every export. Both now iterate
  `collector::registry::SOURCES`, and a test fails the build when a source is reachable from one
  path and not the other.

  Adding a provider is a module exposing `ID`, `read` and `collector`, plus one registry entry —
  down from edits in seven files. The five per-source collector adapters move out of
  `background.rs` (now purely the supervisor) and into the modules they wrap, which is what
  `CONTRIBUTING.md` always claimed. Each source owns a canonical `ID` constant used by
  `Collector::name()`, its config table, and the registry, so those can no longer drift.

- **`[collectors.<id>] enabled = false` now switches a source off everywhere.** It governed the
  dashboard's background collectors and was ignored by `--json`, `--csv` and `--check-budgets`,
  which still read the source and still counted its spend against budgets — the shipped example
  config even documented the split. This is a deliberate behaviour change: exports from a
  configuration that disables a source will now omit its rows, and the source line says
  `<id>: disabled` rather than dropping silently. `zen_pricing` is unaffected: it contributes no
  rows, and its flag governs only the background network refresh, so the line reporting whether
  the pricing cache exists is still always shown.

- **`[collectors.*]` is keyed by source id rather than a fixed struct.** `[collectors.opencodee]`
  used to parse into a field nobody read; it is now an error that names the real sources.

### Added

- **A `justfile`.** `just check` runs exactly what CI runs, in CI's order; `just run` starts the
  dashboard against the committed fixture with the hermetic overrides already applied, `just
  doctor`, `just deny` and `just msrv` cover the rest. The check list previously existed in four
  places with three different subsets.
- **`--doctor`.** The answer to "the dashboard is empty and I do not know why". One line per
  source: the id, whether anything was there, the exact path searched, how many rows it produced,
  how billing was decided, and — where a source is absent — the flag or environment variable that
  points it somewhere else. Then the config file in force, the number of budgets configured, and
  whether logging is on. It runs the same traversal the dashboard and the exporters use, so it can
  never describe a set of sources the rest of the tool does not read, and it writes nothing. On a
  machine with none of the four sources it exits 0 and says so, because that is a normal first run
  rather than a fault.
- **Config keys that the parser does not recognise are now errors.** Every config struct carries
  `deny_unknown_fields`, so `dayz = 14`, `[collectors.opencodee]`, `webook` under `[budgets]` and
  `warnn` in a budget entry all fail with the offending key named, instead of parsing into nothing.
  The shipped example config has carried a comment warning about exactly this since the `webhook`
  key silently disabled every budget; the policy now matches what `load_config` already did for
  malformed values.

- **`scripts/install.sh`, and the quick start now leads with it.** One line installs the right
  archive for the platform, verifies it against the release's published `checksums.txt`, unpacks
  it into a scratch directory and installs only the binary — then says how to fix `PATH` when the
  destination is not on it. It refuses to install a download it could not verify, and on a
  platform with no prebuilt binary it names the source build instead of 404ing. POSIX `sh`, curl
  or wget, no other dependencies.
- **crates.io publication is wired up.** `Cargo.toml` gains `readme`, an `exclude` that keeps the
  670KB of README screenshots out of the tarball (247KB compressed, 91 files), and
  `[package.metadata.binstall]` overrides mapping every release target to its archive, so
  `cargo binstall ai-usage-tui` works the moment the crate exists. A `publish-crate` job publishes
  on a tag push and refuses to run when the tag and `Cargo.toml` disagree. The test fixtures are
  deliberately kept in the package: the `#[cfg(test)]` modules under `src/` read
  `tests/fixtures/` at runtime, so dropping them would ship a crate whose own tests cannot run.
- **An `update-taps` job pushes the rendered Homebrew formula and Scoop manifest** to
  `SophanaSok/homebrew-tap` and `SophanaSok/scoop-bucket`, so `brew install
  sophanasok/tap/ai-usage-tui` becomes real rather than a template attached to a release.
- **The Homebrew formula offers Linux aarch64.** The `aarch64-linux` tarball has been built and
  published since v0.2.0, but the formula only had an `on_intel` block under `on_linux`.
- **`docs/release-process.md` has a "First publish" section** listing the account-level steps —
  claiming the crates.io name, creating the tap and bucket, the optional AUR package. Every job
  added here is gated on its secret and prints a notice instead of failing, so the release path is
  green before and after those steps.

### Documentation

- **The two provider "Billing" essays move to
  [`docs/provider-support.md`](docs/provider-support.md#billing-detection).** Thirty lines each,
  on the install-to-first-run path, explaining a detection cascade to a reader who has not yet
  seen a number. The README keeps the paragraph that matters — what the collector decides, how to
  override it, and that `--doctor` and the source line show the answer — and points at the rest.
  The README is 836 lines, down from 911, with the CLI and environment tables untouched where
  `tests/docs.rs` expects them.
- **The Omarchy integration moves to [`docs/omarchy.md`](docs/omarchy.md).** It occupied 136
  contiguous lines in the README's primary usage section — enough that a general-audience tool
  read as an add-on for one Arch/Hyprland desktop. A short pointer stays behind. The behaviour is
  unchanged and was already correct: on a machine without Omarchy the reader logs the absence
  once and idles.
- **`docs/phase-status.md` and `docs/execution-log.md` are removed.** Both restated
  `CHANGELOG.md` from memory and had drifted — phase-status still filed the whole of v0.5.0
  under "Unreleased" — while being linked from the README as current contributor documentation.
- **`MODEL_ROUTING.md` moves to `docs/model-routing.md`.** It is the maintainer's development-time
  model policy, and at the repository root beside README and CONTRIBUTING it read as product
  documentation.
- **A `.mailmap`.** 38 of the first 85 commits were authored as `User <user@localhost>` and the
  maintainer appeared under four identities; `git shortlog` and the contributor graph now show
  one person.

### Fixed

- **`--help` no longer carries an orphaned line, and drift is now caught.** A stray
  `(default: ~/.claude/projects)` sat under `--omarchy-record`, inherited from a `--claude-dir`
  entry three flags above it, because `tests/docs.rs` compared the README table against the parser
  and never looked at the help text. It compares all three lists now, and the OPTIONS block is
  regrouped into data sources, range and filters, dashboard, and one-shot actions.
- **`--refresh-zen` and `--refresh-pricing` honour `--config`.** Both ran before the config was
  loaded, so a mistyped `--config` path was a hard error for every other invocation and silently
  fine for these two.
- **A failed refresh no longer blames OpenCode for the journal.** `App::refresh` reported every
  `load_usage` failure as `OpenCode unavailable`, sending readers to the wrong file when it was
  the journal that could not be read.
- **A supervisor test asserted after a flat 200ms sleep**, on a three-OS matrix, where a loaded
  runner could miss the deadline and fail a correct build. It polls for the outcome now.
- **`scripts/release.sh` printed "All checks passed!" without running two of them.** It skipped
  `cargo fmt --check` and `cargo deny` entirely, and every check in it is path-relative with no
  anchoring, so running it from anywhere but the repository root checked nothing and still
  passed. It now runs the formatting check and the doc tests, anchors itself to the repository,
  and names any check it had to skip instead of claiming a clean run.
- **`tests/docs.rs` guarded environment-variable documentation against a hand-maintained list of
  five files.** A new collector reading its own environment variable — exactly what `codex.rs`
  does for `CODEX_HOME` — escaped the check that exists to catch it. It walks `src/` now.
- **The journal's only write path had no tests.** `--record-ollama` and `--record-routing` are
  the only things in the project that write, and neither was exercised; the three fixtures
  written for them were referenced from nowhere. Round-trip tests now cover a single response, a
  streamed response journaling once from its final line, idempotent re-recording, and a routing
  event read back through `--routing-json`.
- **The rendered Chocolatey package could not be packed.** The release job flattened every
  template with `basename`, so `chocolateyinstall.ps1` was published beside the nuspec — whose
  `<file src="tools/**" target="tools/" />` then matched nothing, producing a package that
  installed nothing. Manifests now render under `rendered/<manager>/` preserving each template's
  layout, and the job asserts the nuspec's glob will resolve before publishing.

- **The documented quick start no longer overwrites the reader's own `README.md` and
  `LICENSE`.** v0.5.0 started packing those two files into every unix tarball for MIT
  compliance, but the README's install snippets still piped the download into a bare
  `tar xz`, which extracts into the current directory. Anyone who pasted the quick start
  while sitting in a project directory had both files replaced, silently. Both snippets now
  unpack into a `mktemp -d` scratch directory and install only the binary from it.
- **The quick start's platform `case` has an `*)` arm.** On any platform without a prebuilt
  binary `$SLUG` expanded empty, the URL 404'd, and the pipeline died on `tar: Unexpected EOF
  in archive`. It now names the platform and points at a source build.
- **The quick start creates `~/.local/bin` and explains `PATH`.** `install` failed outright on
  a machine without the directory, and succeeded-then-`command not found` on a machine where
  it exists but is not on `PATH`.

### Documentation

- **macOS Gatekeeper is documented.** The release binaries are unsigned and unnotarized, so an
  archive downloaded in a browser is quarantined and the binary is refused with "cannot be
  opened because the developer cannot be verified". The Installation and Troubleshooting
  sections now give `xattr -d com.apple.quarantine` and note that a `curl` download never sets
  the attribute.

## 0.5.0 - 2026-08-23

### Added

- **`--omarchy-record` publishes usage and budgets to Omarchy's agents panel.** A one-shot
  action that writes `<id>.json` into `${XDG_STATE_HOME:-~/.local/state}/omarchy/agents/usage/`
  (`--omarchy-dir` / `[omarchy] dir`) so the bar gains a tab for what Omarchy cannot meter
  itself: `[omarchy] records` names the ids — `opencode` (default; every OpenCode row, all
  providers, priced) and `ollama` (the journal's Ollama rows) — while `claude`, `codex` and
  `fireworks` are refused because they would overwrite Omarchy's own files. Claude Code and
  Codex rows are left out since Omarchy's tabs cover them. Every configured budget becomes a
  `limits[]` meter (`Monthly budget` / `Daily budget`, spend/limit clamped to 1, reset at the
  next local midnight or month), so the panel alarms at 90 % and counts down like a rate limit;
  `[omarchy] balance = true` also draws one budget (`balance_budget`, default `global/monthly`)
  as the prepaid ledger. The record carries token counts, model ids, request and session counts
  and dollar figures — never content or paths — and is written atomically with mode 0600.
  Nothing writes there without the flag. `contrib/systemd/user/` ships a 15-minute user timer.
- **Subscription limits from Omarchy's agents panel.** Omarchy 4 meters every AI coding
  subscription on the machine and writes one JSON record per agent under
  `${XDG_STATE_HOME:-~/.local/state}/omarchy/agents/usage/`. A new `l` panel shows those
  records — one row per rate-limit window with % used, a bar and the reset countdown, then a
  line per agent with its plan label and record age — and the header names the fullest fresh
  window beside the pricing-coverage figure (`claude session 92%`, alarm colour at 90 %).
  `--json` gains a top-level `limits` array (present and empty when disabled or absent);
  `[omarchy] dir` / `limits` and `--omarchy-dir` configure it. Six fields per record are read
  (`id`, `name`, `updatedAt`, `ready`, `tierLabel`, `usageStatusText`, `limits`); the agents'
  credentials, Omarchy's probe cache, `authHelpText` and the token tallies are never read, no
  request is made, and nothing is written. Records older than 45 minutes are dimmed and never
  alarm; unreadable ones are named on the status line. The record's `tierLabel` is now the
  fourth billing signal for Claude Code and Codex, after the explicit setting, the API-key
  variables and `~/.claude.json`. Off Omarchy the directory is absent and the panel is idle.
- **Codex CLI collector.** Reads Codex's session logs ("rollouts") under `~/.codex/sessions` and
  `~/.codex/archived_sessions` — `$CODEX_HOME`, or `--codex-dir` / `codex_dir` — tailing each file
  by a cursor that also remembers the model, thread and directory in force there. Only
  `session_meta`, `turn_context`, and the `token_count` event's `last_token_usage` are read;
  prompts, tool output and reasoning summaries in the same file are never parsed. Following the
  CLI's own arithmetic, cached input is split out of `input_tokens` as cache-read and reasoning
  out of `output_tokens`, while cache writes stay inside input because OpenAI bills them at the
  input rate. Re-emitted events with an unchanged running total and post-compaction estimates are
  skipped, and identity is content-based so a forked thread's copied history dedupes. Billing is
  decided like Claude Code's — `[collectors.codex] billing` or `--codex-billing`, else
  `OPENAI_API_KEY` / `CODEX_API_KEY` in the environment, else per-token with a "billing unknown"
  hint; `auth.json` is never opened and `config_json` is rejected under this table. Rows are
  `openai`, priced `estimated` from the bundled `gpt-5` family entries, `unavailable` otherwise.
  Tests and examples pass `--codex-dir` to a nonexistent path to stay hermetic; `.jsonl.zst`
  files are not read.
- **`tests/docs.rs` guards the README against drift.** The quick-start version pins must match
  `Cargo.toml`, and the CLI table must match what `--help` actually accepts, or the test fails.
- **Claude Code billing detection.** Claude Code writes the same transcript on an API key and on
  a Pro/Max plan, and priced at list rates a subscription's traffic read as hundreds of dollars
  that were never charged, tripping budgets on them. The collector now decides once per source —
  `[collectors.claude_code] billing` or `--claude-billing`, else an Anthropic API-key variable in
  the environment, else `oauthAccount` in `~/.claude.json`, else per-token with a visible
  "billing unknown" hint — stamps every row, and names the answer on the source line
  (`· subscription Max 20x`). Only the presence of `oauthAccount` and its rate-limit-tier keys are
  read from that file; the email, name and prompt history beside them are dropped unread, and
  `.credentials.json` and `settings.json` are never opened. Subscription rows become `quota` and
  keep the list-rate figure as `api_equivalent_cost`, shown as `API-RATE EQUIV.` in the breakdown
  and never summed into cost. `config_json` points at the document when it is elsewhere.

### Changed

- **Anthropic rows from a subscription account now export as `quota`, with a new column.** Rows
  that exported `("PAID", "estimated", cost: N)` now export
  `("PAID", "quota", cost: null, api_equivalent_cost: N)` when Claude Code runs on a Pro/Max plan.
  Budgets scoped to `global`, `provider = "anthropic"`, or a Claude model no longer count them, and
  `--check-budgets` no longer exits `1` for them. The CSV gains a fifteenth column,
  `api_equivalent_cost`, appended after `session_id`; JSON rows gain the same key, `null` unless
  the row is subscription-billed. Nothing is removed or renamed and no existing column moves.
  `[collectors.claude_code] billing = "api"` restores the previous accounting.

### Fixed

- **Webhook dispatch was documented wrongly.** The README described it as `--check-budgets` only;
  the dashboard also posts on each refresh, and the per-alert suppression that stops it repeating
  is in-memory, so it resets when the process restarts. The docs now say so.
- **Docs said collectors write the journal.** They never do — the journal is a source written by
  `--record-ollama` / `--record-routing` and read by the journal collector, not a sink.
- **Budget period names were inconsistent across the docs,** and `monthly` was described as a
  30-day window. It is the calendar month; the 30-day window is the `3` / `--month` range.
- **The README quick-start pinned a stale release.** It now pins the current one, and
  `scripts/release.sh` refuses to tag while it does not.
- **`examples/config.toml` had no `[budgets]` header above `webhook`.** Uncommenting the key put it
  in `[collectors.zen_pricing]`, where it was dropped without a word. The header is now present.
- **The dashboard swallowed a failed webhook POST.** It is now logged under `AI_USAGE_LOG` like any
  other collector error, rather than discarded.

## 0.4.1 - 2026-08-20

### Fixed

- **`checksums.txt` could not be checked against the published assets.** Every entry named
  `<artifact-dir>/<file>`, the path inside the CI download directory, but release assets are
  published flat — so `sha256sum -c checksums.txt` failed on all nine lines with "No such file or
  directory". The hashes were correct the whole time; the file was simply unusable for the one
  thing it exists for. Hashing is now done by basename, and the job fails if a path component
  reappears rather than shipping another unverifiable file. Affected every release through 0.4.0.

- **The tarballs and the Windows zip contained only the binary.** No README, no LICENSE, though
  `docs/release-process.md` requires all three and the MIT terms ask that the licence accompany
  copies. The `.deb` and `.rpm` were already correct, which is why this went unnoticed: the
  packages a distribution would audit were fine and the archives most people actually download
  were not. Affected every release through 0.4.0.

## 0.4.0 - 2026-08-20

### Added

- **Escalation analytics, derived from usage already collected.** The routing panel could only
  say anything if you had instrumented `--record-routing` by hand, so for most users it said
  nothing. One part of the same question is directly observable in data already on disk: how
  often a session reached for a model pricier than the one it opened with, and what that cost.
  It appears as its own block above the recorded table, labelled as derived. The two are never
  merged — an inferred transition and a measured pass rate would be indistinguishable in one
  table, which is the failure `CostStatus` exists to prevent one level up. Nothing infers a test
  result, and nothing should.
  Counting is per session, not per model switch. Checked against real collected usage first: a
  session there switched models 20 times, 10 of them upward, and per-switch counting reported
  **$233 of escalated spend for a $29 session** by summing the same tail ten times. Each session
  is now characterised once, so the reported figure cannot exceed what the sessions cost.

- **Sessions panel** (`s`). Individual sessions, most recently active first. A session id is a
  bare UUID and tells a reader nothing, so every column exists to make the row identifiable
  without it: when it started, how long it ran, which project, which model — or `N models` when
  it used several. Data that had been collected since the Claude Code collector landed and never
  shown.

- **Burn-rate panel** (`w`). Tokens/min and spend/hour over a trailing hour, and — the part that
  matters — **how long until each configured budget is exhausted at the current rate**. A rate on
  its own is trivia; a rate measured against a limit you set is an answer, and it is only
  possible because the budget engine and the collectors run in the same process.
  Two refusals are deliberate: a window with fewer than five requests says *too little activity
  to project* rather than extrapolating from noise, and a window containing unpriced usage shows
  `≥ $x/hr` rather than presenting a floor as a rate. Both are the same discipline as never
  rendering unknown cost as `$0.00`.

- **Spend-over-time panel** (`g`). Daily tokens and cost, as a sparkline of the whole visible
  range plus a table of the days that fit. Days with no usage are kept as zero bars — dropping
  them compresses a quiet week to the width of a busy one and reads as steady activity. Bars use
  eighth-block characters so a day below a twelfth of the peak still renders; whole-cell bars
  would make a chart of mostly-small days look empty. The sparkline is drawn right-to-left from
  the newest day, so time runs left to right and truncation drops the oldest days rather than
  the most recent. A partly-priced day shows `≥ $x`; a day with no priced usage says `unpriced`
  rather than the technically-true and useless `≥ $0.00`.

- **The project's first TUI rendering tests**, via ratatui's `TestBackend`. The audit noted
  nothing verified rendering; a panel that computes correct numbers and draws nothing was
  previously indistinguishable from a working one.

- **Contributor onboarding.** `CONTRIBUTING.md` was 31 lines of commands and rules with no map
  of the codebase and no route in. It now covers where things live, the three most likely
  contributions (a collector, a panel, a pricing correction) with concrete steps, and the
  invariants *with the reason each exists* — every one of them is there because breaking it
  produced a wrong number that looked right.

- **Issue and PR templates.** Shaped around this project rather than generic: bug reports steer
  people to reproduce against the committed fixture instead of pasting real session data, and
  the collector request asks for the field *shape* with an explicit redaction reminder. Session
  logs contain source code and secrets, so an issue tracker is the last place they should land.

### Changed

- **README images are now rendered, not photographed.** All seven panels have a current image,
  and `routing.png` no longer shows a layout that stopped existing three commits after it was
  taken. The old approach — drive a real terminal with `xdotool`/`wtype` and screenshot it with
  `scrot`/`grim` — had two defects that could not be engineered away. It captures a screen
  *region*, so anything drawn over the terminal lands in the file; a repository that promises to
  read no message content should not ship pictures of its author's desktop, and a first run
  produced exactly that. And it needs a graphical session, so the images could not be regenerated
  in CI and went stale without anyone noticing.
  `src/ui/svg.rs` renders the same `draw` call through ratatui's off-screen backend and turns the
  cell buffer into SVG; `scripts/render-readme-screenshots.sh` rasterises it. Same code path as
  the real dashboard, no screen involved, and it runs headlessly, so regenerating them is one
  command on any machine rather than an errand on a particular desktop. The two capture scripts
  are removed.
  It also needed a dataset that can fill the panels: the test fixture is nine rows on one day in
  2023 with no session ids and no project paths, so projects, sessions, spend-over-time and burn
  all rendered blank. `scripts/make-demo-fixture.py` generates a deterministic, deliberately
  fictional set of Claude Code transcripts and a stand-in OpenCode store — several days ending
  today, three projects, sessions that escalate, and local, free and quota-billed routes. The
  renderer refuses to start unless every source is passed explicitly, because unset they fall
  back to this machine's real usage data.

- **`cost_status` gains a seventh value, `quota`, in `--json` and `--csv`.** Rows that exported
  `("CLOUD", "unavailable", cost: null)` now export `("CLOUD", "quota", cost: null)`. Nothing is
  removed or renamed and no column moves. A consumer computing "share missing a price" from
  `cost_status == "unavailable"` gets a corrected number, which is the point. An older binary
  reading a newer journal maps the unknown label back to `unavailable`, i.e. exactly its previous
  behaviour.

- **Routing analytics now leads with the question it answers.** The panel is titled *cost per
  delivered result* and sorts by exactly that — dollars spent per passing test, cheapest model
  first — instead of listing agents in arbitrary order and leaving the arithmetic to the reader.
  This is the one view no comparable tool has, and it read like a debug dump.
  An agent that never reported a test result shows `—`, not `0%`: never having been measured is
  not the same as failing everything, and the older rendering made an uninstrumented agent look
  like the worst one on the board. A genuinely free model reads `free` rather than `$0.0000`.
  When there is nothing recorded, the panel explains what it would show and how to record it,
  rather than showing an empty table.

- **Pricing coverage moved to the header**, where it is visible on every panel, and reads
  `all priced` when nothing is missing. It previously appeared only in the project panel's
  title, so a reader could take any other panel's total at face value without learning it
  covered two thirds of the requests. Below 100% it is highlighted — that is the case worth
  noticing. Cost provenance is the thing this project does that the alternatives do not, and it
  had been living in an internal enum.

- **`src/ui.rs` split into `src/ui/`.** It was 1,196 lines in one file — the single largest
  barrier to finding anything, and a merge-conflict magnet for concurrent work. Now `app.rs`
  (state), `aggregate.rs` (pure functions over usage), `theme.rs` (palette and shared widgets),
  and one module per panel under `panels/`. Largest file is 306 lines. Adding a panel is now:
  write `panels/yours.rs`, add a `Panel` variant, a key binding, and a match arm — which is
  also the on-ramp for the dashboard work on the roadmap. Pure refactor: `--json` output is
  byte-identical before and after.

### Fixed

- **The footer hid the quit binding on an 80-column terminal.** It was 77 columns at v0.3.0 and
  fits; the graph, burn and sessions panels added since pushed it to 106, and a `Paragraph`
  truncates without saying so, so `j/k navigate` and `q quit` were simply gone below 110 columns.
  The footer is now sized to the terminal, and a **`?` help overlay** carries the full key
  reference — the permanent answer to having more bindings than fit on one line. No test rendered
  the whole dashboard at any width, which is why nothing caught this; one now runs at 80, 100 and
  120 columns.

- **The pricing-coverage figure counted a deliberate refusal as a failure.** The header reported
  **71.6% priced** against a dataset where **100%** of priceable work was priced. Every "unpriced"
  request was Ollama Cloud usage, which this tool refuses to price on purpose: it is billed against
  an account quota and GPU time, and no supported API exposes a per-request rate
  (`docs/provider-support.md`). Those rows carried `CostStatus::Unavailable` — the same value a
  paid model with no entry in the pricing table carries — so seven panels read "we declined to
  invent a number" as "we failed to produce one": the header percentage, project cost as `≥ $X`,
  timeseries days as `unpriced`, burn rate as `≥ $x/hr`, session cost as a floor, escalated spend as
  a floor, and the breakdown's `PRICING STATUS: partial / unknown`.
  Quota-billed usage now has its own `CostStatus`, stamped where pricing already declines to act,
  which also repairs rows already written to a journal without a migration.
  The obvious one-line fix would have been worse than the bug: dropping those rows from the
  unpriced count leaves a cloud-only day, session or project with no unpriced requests and no
  dollars, so all four cost renderers would have printed **`$0.00`** for usage that genuinely costs
  money — the project's cardinal invariant broken in four places. Every rollup therefore carries a
  quota count alongside, each renderer says `quota` rather than a zero, and the header and breakdown
  disclose the volume so "all priced" is never a percentage taken over a silently shrunken
  denominator.
  Quota-billed rows also stop carrying the `0` OpenCode records for cloud routes: that zero is
  absence of data, not a price, and it was exporting as `cost: 0` — the same claim in the export
  that the status exists to prevent on screen. A cloud row with genuine reported spend keeps its
  figure; observed data still beats the policy rule.

- **The selection clamped to the model table on every panel.** `j`/`k` bounded themselves by the
  model row count regardless of which table was visible, so on any other panel the selection
  either stopped short of the last row or ran past the end. It now follows the visible panel.

## 0.3.0 - 2026-08-19

### Fixed

- **The release workflow's tag variable was silently ignored.** The dry-run plumbing passed the
  tag as a step-level `GITHUB_REF_NAME`, but variables starting with `GITHUB_` are reserved: the
  override is displayed in the run log and then discarded, so the step saw the runner's value —
  `main` on a dispatch — and looked for artifacts named after the branch. Renamed to
  `RELEASE_TAG`. This affected dispatch runs only; a real tag push was unaffected, because there
  the runner's value *is* the tag.

- **The manifest-rendering step could fail with no output at all.** A dry run failed there in
  9ms with nothing logged — no error, no partial output — because `! grep … || { …; exit 1; }`
  swallows any failure earlier in the step. It is an explicit `if` now, each resolved checksum
  is printed, a missing one names the artifact and the build job responsible, and a missing
  packaging template says so. Checksum lookup matches on the path's final component with awk
  rather than a regex, since every artifact filename contains dots.

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