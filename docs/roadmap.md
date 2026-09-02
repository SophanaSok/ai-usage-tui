# Roadmap and Outstanding Findings

Working state for continuing the audit-driven work started 2026-08-18, reconciled against v0.11.0.
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

### Resolved — the Copilot schema is validated against a real account

The collector shipped in v0.12.0 without a Copilot install to test against: the store's
filename, the `assistant_usage_events` column set and the unit of
`modelMetrics.<model>.requests.cost` were all derived from Copilot's published behaviour and
from other readers of the same files. It is now checked against bytes the CLI actually wrote —
Copilot CLI 1.0.82, driven non-interactively twice, once with a tool-using prompt.

**The schema probe did its job**: nothing produced a wrong number, and an absent column
degraded to `NULL` exactly as designed. But three assumptions were wrong, and two of them cost
data:

- **A turn is not a request.** `event_id` was `copilot:{session}:{turn_index}`, and the capture
  has a `user` row and an `agent` row sharing `turn_index` 0 inside one session — a tool-using
  prompt produces several requests per turn, and `turn_index` was `0` on every row of every
  session captured. Against the real store the collector reported **2 rows where Copilot
  recorded 3**, discarding the extra rows' tokens with them. Identity is now the table's own
  autoincrement `id`. This is the same defect the Gemini validation found, in a different
  spelling: there it was keying six `api_response` records on a shared `prompt_id`.
- **`cwd` and `repository` are not columns of `assistant_usage_events`.** They are on
  `sessions`. The probe found neither, selected both as `NULL`, and every Copilot row came out
  with no project at all. The select list now looks on `sessions` too and joins when it must,
  still preferring the usage table so a build that moves them onto it needs no join.
- **`created_at` is declared `TEXT` and written as RFC 3339**, while the incremental read bound
  an `i64`. SQLite orders every integer before every string, so `created_at >= <integer>` was
  *always true* and the cursor filtered nothing — every poll re-read the whole table. The cursor
  now also keeps the store's own spelling and compares text with text.

**`requests.cost` is a premium-request count, not dollars, so the billing decision stands.** The
capture settles the question the previous entry left open: a `session.shutdown` reporting
`"totalPremiumRequests": 1` carries `modelMetrics.<model>.requests = {"count": 1, "cost": 1}`,
and the request table spells the same thing as `request_multiplier` (`1.0`). The real money
figure is `total_nano_aiu` — `231429600` nano AI Units for the run the CLI summarised as
"AI Credits 0.23" — which is a Copilot billing unit and not a dollar amount. So `cost: None`
with `CostStatus::Unavailable` is right and stays right, and nothing here reaches a budget.

**What the capture confirmed.** `session-store.db` is the real filename (the first candidate
tried). The inclusive-token convention holds in the bytes: a shutdown reporting
`usage.inputTokens: 13864` alongside `cacheReadTokens: 1152` also reports
`tokenDetails.input.tokenCount: 12712`, which is the subtraction this collector does — the
v0.12.0 cache double-count fix was right. The legacy `session.shutdown` shape parses as written,
including the empty-`modelMetrics` case a session that made no model call produces. The store is
in WAL mode and the read-only open handles it.

`tests/fixtures/copilot_home/session-store.db` is the redacted capture — the real DDL, with
identifiers and paths replaced and a `FIXTURE_SECRET` planted in `turns.user_message` and
`sessions.summary` so the content-leak assertion has something to catch.

Left open, and needing a Copilot user rather than this machine: whether a store that has been
through several CLI upgrades still matches, and whether a seat on a **paid** plan spells
`request_multiplier` as something other than `1.0` — every model reachable here multiplies to 1.

### Resolved — `claude-review` reviewed nothing, three ways over

Two independent faults, both now understood, both fixed; what is left is to *verify* the fix
against a pull request carrying real defects.

**Fault 1: the workflow denied its own review skill.** `--allowedTools` **replaces** the default
allowlist rather than extending it, and the generated workflow named
`mcp__github_inline_comment__create_inline_comment` and nothing else — while the `prompt:` was
`/code-review:code-review`, a slash command the model invokes through the **`Skill`** tool. First
move denied, then `"subtype": "success"` and silence. Four runs against a pull request carrying
two deliberate defects:

| allowlist | turns | denials | outcome |
| --- | --- | --- | --- |
| comment tool only (generated) | 6 | 1 — `Skill` | never reviewed |
| flag removed, defaults | 21 | 7 | reviewed, no way to post |
| `+ Read,Grep,Glob,Bash` | 7 | 1 — `Skill` | never reviewed |
| `+ Skill` (PR #75) | 7 | **0** | skill ran, spawned 2 subagents |

**Fault 2: the review fans out to background tasks, and the parent does not wait.** With `Skill`
allowed the review starts and then ends having posted nothing. Run 33568706427 carries
`"subtype": "background_tasks_changed"`, `"task_started"`, `"task_progress"` and finishes at

```
"num_turns": 6, "permission_denials": [],
"result": "I'll wait for the background agent's completion notification before continuing."
```

Six turns is nowhere near a cap: **the parent stopped voluntarily**, waiting for a notification a
`-p` invocation never delivers. Raising `--max-turns` — which an earlier version of this entry
proposed as the cheapest fix — buys nothing, and that is worth not re-deriving.

The plugin's command fans out by design: a Haiku eligibility agent, five parallel Sonnet
reviewers, a Haiku confidence scorer per issue. Reading it also showed **the posting path never
matched either**: step 8 posts one summary comment with `gh pr comment` (its frontmatter allows
`Bash(gh pr comment:*)`) and it never creates an inline comment, so
`mcp__github_inline_comment__create_inline_comment` and the action's buffered-comment step were
looking for something that was never going to be produced. Its command body has no `$ARGUMENTS`
either, so the `--comment <repo>/pull/N` the workflow passed was noise.

**The fix** replaces the slash command with a direct prompt that reviews in-process — no
subagents, no background tasks — and posts with `gh pr comment`, with an allowlist naming exactly
the tools that prompt uses. It also posts a "No issues found" comment rather than staying silent,
because silence is indistinguishable from a review that never ran, which is what made the first
eight runs so expensive to read.

**Why none of this can be tested on the pull request that changes it.** `claude-code-action`
validates the workflow file before it runs anything:

```
##[warning]Skipping action due to workflow validation: Workflow validation failed. The workflow
file must exist and have identical content to the version on the repository's default branch.
```

It then exits `Exiting due to workflow validation skip` — and **the job still reports success**.
Run 33571283816 did exactly this in 14 seconds. So every pull request that edited this workflow
(#67, #68, #69, #71, #72, #75 and the one carrying this note) had its review silently skipped,
which is a second, entirely separate way to get a green check and no review. It also means the
measurement loop is: merge the workflow change to `main` first, *then* open a separate pull
request that does not touch `.github/` and read the run on that. `show_full_output` has to be on
in `main` for the measuring run and taken back off afterwards, which is what #72 followed by #76
was doing.

**It works, and the first thing it reviewed it caught.** On PR #81 the in-process review posted
a finding within a minute: the new per-agent journal cursor was built as
`claude-code:{session}:{agent_id}:` while `event_id` was still written as
`claude-code:{session}:{tool_use_id}`, so the prefix matched nothing in SQL and every subagent run
re-attributed from the start of its transcript. Real, ours, and not caught by `fmt`, `clippy`, the
suite or the author. Fixed in the same pull request, with the reply posted back on the thread.
`show_full_output` is off again.

**How this was found, and the methodology error to avoid repeating.** The denial was invisible
until `show_full_output: true` (#72) printed
`{"tool_name":"Skill","tool_input":{"skill":"code-review:code-review"}}`. Three prior guesses at
the allowlist cost about $0.75 and settled nothing. Turn that input back on before changing
anything else, and turn it off again afterwards — this repository is public and so are the logs.

Separately: the first test bed was titled `DO NOT MERGE — smoke test` and said so in its body. The
reviewer read the PR metadata, applied its own stop condition for a PR that "does not need code
review", and additionally flagged the body's instruction-shaped text as a prompt-injection attempt
— correctly. **A test PR must look like ordinary work**, or it measures the reviewer's refusal
rather than its ability.

**Reproduction.** Two defects in `cursor_is_installed()` in `src/main.rs`, chosen because no test
covers that function — `cargo fmt --check`, `clippy -D warnings` and the whole suite pass with
both in place, so the reviewer is the only thing that can catch them. Present them as an ordinary
refactor, with no mention of testing or reviewers:

```diff
 fn cursor_is_installed() -> bool {
-    let Some(home) = ai_usage_tui::utils::home_dir() else {
-        return false;
-    };
+    let home = ai_usage_tui::utils::home_dir().unwrap();
     [
         home.join(".cursor"),
         home.join(".config/Cursor"),
         home.join("Library/Application Support/Cursor"),
         home.join("AppData/Roaming/Cursor"),
     ]
     .iter()
-    .any(|path| path.exists())
+    .all(|path| path.exists())
 }
```

The first makes `--doctor` panic rather than degrade when `HOME` is unset; the second requires
Cursor at all four platform paths at once, so the notice can never fire. Revert both before
merging anything.

## Outstanding findings

Numbering follows the original audit; findings since take the priority they would have had
there. Everything not listed here has shipped.

### P0 — Resolved. Routing analytics reported a cost it could not defend

**Resolved.** Found while planning v0.9.0, and the reason the arc is ordered the way it is.
`RoutingAggregates.cost` is a floor now, read with `priced_tasks` / `unpriced_tasks` /
`quota_tasks` / `free_tasks`; the `$/SUCCESS` cell reports its basis in the escalations block's
vocabulary; and only a real figure or a genuine zero takes part in the sort. What follows is kept
because the shape of the bug is worth not reintroducing.

`routing.rs::aggregate` does `entry.cost += event.cost.unwrap_or(0.0)` and `RoutingAggregates.cost`
is a bare `f64`, so an unpriced or subscription-billed model reaches `cost_per_success` as
`Some(0.0)`. The routing panel's **default sort is `$/SUCCESS` ascending**, so that row ranks as
the cheapest work on the machine, and `cost_per_success_cell` renders it green as **`free`**. On a
Max account, Opus work arrives as `cost: null, cost_status: quota` and prints as free.

The defence is already there and fires one layer too late: `sort_rows_by_cost`'s `cost_order`
holds unknown costs at both ends precisely so they cannot "appear free" — but by then the unknown
has been laundered into `0.0` upstream. This is convention 1 broken in the panel that carries the
project's pitch.

It is latent only because a human typing `--record-routing` JSON usually types a `cost`. **An
automated emitter cannot**, having no rate table in hand — so this must be fixed *before* any
harness ships, or the harness's first act is to make every subscription user's headline metric a
lie. Fix by copying `escalation::Transition`'s shape (`cost_after` + `unpriced_after` +
`quota_after`) and classifying with `is_billable()` / `is_quota_billed()` / `needs_price()`, which
`escalation::derive` already does.

### P1 — Resolved. Routing counters read "never measured" as "never needed"

**Resolved** in the v0.10.0 arc, alongside the budget engine's copy of the same mistake. The
paragraph that follows sat under the P0 heading above, after the words "what follows is kept
because the shape of the bug is worth not reintroducing", and a cold reader took the whole section
as done. That heading is how it survived a release; it has its own now.

Two more of the same class in the same file: `retry_rate` and `defect_rate` returned `0.0` both
when a model never retried and when nothing reported a count, because `RoutingEvent.retries` was a
bare `u32` — `success_rate` immediately above returns `Option` for exactly this reason. And
`retry_rate` was `retries / tasks` rendered `{:.0}%`, so an emitter writing `retries: 3` on one
task rendered `300%`.

The fix is one `ObservedCount` — sum, tasks that reported, tasks affected — with one `rate()`,
rather than three more copies of the guard; `None` renders as `—` and sorts to the end both ways.
The journal's columns were `NOT NULL` and are rebuilt nullable in place, keeping the zeros already
recorded. Worth knowing before touching `--record-routing` again: it now **refuses** a counter or
`test_result` it cannot read rather than storing `0` or `null` under a success message, because the
round-trip test had been sending `"test_result":"pass"` since it was written and asserting the
three counters beside it and not the result.

### P1 — Resolved. A pricing refresh never reached the running dashboard

`CollectorState.pricing` is loaded once in `CollectorHandle::spawn` and never replaced, while
`pricing_refresh::poll` writes a refreshed cache to disk and returns no rows. So on a running
dashboard a successful refresh changes nothing until restart: rows stay `UNKNOWN COST` although
the rate is now known, the log says the refresh succeeded, and the screen silently disagrees.
Nothing documents this. Convention 8.

**Resolved.** `Collector::refreshes_pricing` declares that a source's work re-prices everything
else -- only `zen_pricing` answers true -- and the loop rebuilds the engine and re-prices the
collected rows. The rebuild happens *before* the write lock is taken, because parsing ~3,450 keys
while holding it would block `snapshot()` on the render thread.

**Still open, and now with a constraint attached:** `apply_pricing` re-prices all accumulated
history inside the write lock on every poll. Narrowing it to newly merged rows is the obvious fix
and is only correct if the reload path above keeps re-pricing *everything* -- the rows collected
before a refresh are exactly the ones whose price was missing. Whoever does the incremental work
has to leave `reload_pricing` whole.

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

- **Resolved.** Drill from a project into its sessions: `Enter` on a project row scopes the
  sessions view to it, `Backspace` (or `Esc`) returns to the row it started from, and the panel
  title names the project so the two views cannot be confused.

  The panel *state* this entry warned about is one field, `App::drilldown`, holding the project
  and the row to return to. The narrowing happens in `recompute`, not in the draw call, so the
  render path stays free of computation. `Esc` now means "back" when there is somewhere to go and
  "quit" otherwise, which is the one documented binding whose meaning became contextual.

  It also surfaced a bug that predated it: `recompute` clamped the cursor against the *model
  table* whatever panel was showing, so on a machine with few model groups and many projects the
  later projects were unreachable. Cosmetic while nothing acted on the row; wrong the moment
  `Enter` did. Both the clamp and `visible_rows` are panel-aware now, with a regression test.
- **Interactive depth** — mouse support. Sortable columns shipped: `<`/`>` move the sort column,
  `o` reverses, each panel keeps its own, and the sorted column is marked in its header. The
  defaults reproduce the orders the lists already had, so nothing moves until a key is pressed.

  Two things it turned up. The routing panel **re-sorted inside its draw call**, so it recomputed
  a ranking on every frame (invariant 4) and would have silently discarded any sort applied
  upstream; that ordering is now the panel's default sort, computed once per refresh. And the
  sessions list ordered by `last_seen` while its time column displayed `first_seen`, so the
  column a reader saw was not the column the rows were in — sorting by STARTED now means what it
  says, which is a small deliberate change to the default order.

  `/` search shipped: it filters what
  the visible panel lists (model and provider names, project paths, session ids, the models a
  session used) while deliberately leaving the totals, the coverage figure and the budgets
  computed from the whole range. The footer carries the query and a "showing N of M", because a
  list that silently shortened is how a filtered view gets read as a smaller bill.

  The footer's key hints were the **fifth** hand-written copy of the bindings, the one
  `ui::keys` did not end because the footer abbreviates and reflows by width. **Resolved:** each
  binding carries its footer spelling (`Binding::hint`), `keys::footer_forms` derives the three
  forms, and the footer takes the widest that *measures* as fitting. The `width >= 120` it
  replaced was the full line's width on the day it was written — the same kind of drift that
  hid `q quit` on 80 columns once — and a sweep test now renders every width from 16 to 200.
- **Resolved for Claude Code.** Escalations are derived from collected sessions, but pass/fail
  and retry counts cannot be inferred from usage metadata and must not be guessed; what closed
  this was a shipped hook, not more derivation. `--claude-code-hook` (`src/harness/`) reads a
  `PostToolUse`/`PostToolUseFailure` payload and journals a test run's pass or fail, attributed
  to the model that ran it and priced as the dashboard prices its rows;
  `contrib/claude-code/settings.json` registers it.

  Four things are worth knowing before touching it. **The docs were wrong about the payload**
  — a non-zero Bash exit fires `PostToolUseFailure`, and `PostToolUse` carries no exit code —
  which is why the snippet registers both events and why the fixtures in the tests are captured
  payloads rather than the reference's. **The transcript lags the hook by one request:** the
  line that issued the tool call is appended after the hook has run, so the attempt is bounded
  by a cursor over requests already attributed (`journal::attributed_requests`), never by
  time — the first version used a time window and silently lost that request from every
  attempt, which one real run showed in a `cost_basis` of `unpriced` on a Max account. **A
  pipe is not an observation:** `cargo test | tail`
  exits with `tail`'s status, so `harness::shell` withholds the result whenever the line's
  status is not the runner's own, in either direction, and says so. That rule is in one place
  and one test table on purpose. And **counters are never sent**, so RETRY, ESC and DEFECT read
  `—` for this agent; a harness that can count them is a different harness.

  **Subagent attribution is now verified, and it was wrong.** Driving a real Claude Code session
  that delegates `make test` to a `general-purpose` subagent showed that the payload names the
  *parent's* `transcript_path` and the parent's `session_id`, while the subagent's own turns go
  to `<project>/<session_id>/subagents/agent-<agent_id>.jsonl` with every line marked
  `isSidechain: true`. So the attempt was read from the parent's transcript: the run came out as
  **3 requests and 65,598 tokens** when the agent that ran it had spent 2 requests and about 318,
  and the model recorded was the parent's — an Opus parent would have priced a Haiku subagent's
  attempt at Opus. The harness now prefers the nested transcript when the payload carries an
  `agent_id`, and keys the journal cursor on the agent as well as the session so a subagent and
  its parent stop sharing one counter. `agent_id` had been parsed by a test and read by nothing.

  Worth knowing: the *usage* collector was never affected. Its walk is recursive, so it already
  reads `subagents/*.jsonl`, and those requests carry their own `requestId`s — so subagent tokens
  were counted exactly once in the dashboard all along. Only the harness's attribution was wrong.

  Still open: Codex and Gemini have no equivalent hook surface today.
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

  *The discrepancy this entry used to leave open is gone:* `ROUTING.md` and `opencode.json` now
  both give `reviewer` as `openrouter/z-ai/glm-5.2:free`, against `opencode/nemotron-3-ultra-free`
  for `build`, `spec` and `reasoning` — so rule 3 (a reviewer on a different provider family from
  the agent that wrote the code) holds whichever of them did the writing.

### P2 — The update story

`--doctor` reports which channel this binary came from and the exact command to upgrade it, and
`[update] check = true` opts in to a release-tag check.

- **Resolved. A new release is surfaced outside `--doctor`.** The opt-in check writes its answer
  to `update-check.json`; the dashboard reads it *once at startup* (`App::update_notice`, beside
  `pricing`, for the same reason) and names the release in its header. The cache is the whole
  point: the header redraws several times a second and convention 5 keeps both the network call
  and the clock read off that path. The verdict is deliberately **not** stored — only the tag —
  so a cache written before an upgrade stops claiming an update after it, and a stale cache can
  only understate. `--doctor` discloses a cached answer even when the check is off, because
  otherwise a user who turned the check off and still sees a notice has nowhere to look.

  Two things worth knowing. **The notice broke the header the way the footer broke once:** it is
  11 columns, an 80-column header fitted exactly before it, and a `Paragraph` truncates in
  silence — so the collector status, the one thing that must never vanish quietly, went off the
  end. `LIVE PROVIDER MONITOR` now yields to it, measured rather than thresholded. And **only
  `--doctor` writes the cache**, so a user who never runs it still never learns. Closing that
  needs a periodic writer: a collector on the `zen_pricing` shape, or the `contrib/` timer.
  Deliberately not added here — it would turn an explicit "ask when I run `--doctor`" opt-in
  into a recurring background request, which is a different consent than the one given.

  It was also the third thing to leak this machine into the README images: `render-screenshots`
  builds a real `App`, so a cached answer put `↑ vX.Y.Z` in all seven headers. Cleared in the
  renderer. Anything `App::new` reads from a real path is a candidate for the same mistake — the
  first two were the OpenCode database and Omarchy's records.
- **Resolved. `scripts/install.sh` detects an existing install.** It names the version it is
  replacing (or says the version could not be read), says "reinstalling" when the tag matches,
  and — the case that actually bites — warns after installing when `command -v` still resolves
  to a different copy earlier on `PATH`, naming both. Deliberately does not say "upgrading" or
  "downgrading": ordering two versions needs more than string comparison and `sort -V` is not
  POSIX, so both are named instead, which cannot be wrong.
- **Channel detection is inference, not fact.** A binary copied out of `~/.cargo/bin` reports as
  cargo. Recording the channel at install time would be exact, but only `install.sh` and the
  packaging manifests could write it, and a file the binary trusts about itself is a new thing to
  keep honest. The current guess is conservative -- an unrecognised path says so rather than
  naming a command that would install a second copy.

### P3 — Small, known, unclaimed

Each open item here is an afternoon at most, and each is a reasonable first outside contribution.

One that was listed here is resolved, differently from how it was written: `src/collector/zen.rs`
stays untested, because there was nothing to split — `refresh_zen_catalog`'s entire "parse" is
`to_vec_pretty` on a `Value` — and a test that the path ends in `zen-models.json` is what
convention 6 forbids. The real defect was that `--doctor`'s `zen_pricing` line described that
catalog, which nothing prices from, and told a user with `UNKNOWN COST` rows to run
`--refresh-zen`. It now reports the pricing cache `--refresh-pricing` writes and
`PricingEngine::load` reads, and a `PRICING` section carries the engine's warnings, which had no
production reader at all.

- **Resolved.** The README's "Data sources" section was the longest one left, at roughly 175
  lines with the per-provider parsing detail inside it. That detail — Codex's token arithmetic
  and fork dedup, Gemini's bucket rules, Ollama's stream semantics, the provider-qualified
  pricing keys — now lives in `docs/provider-support.md`, which gained a "Pricing tables"
  section for the last of those. The README keeps one subsection per registered source (which
  `tests/docs.rs` requires), each reduced to its default path, its override flags, one sentence
  on what is not read, and the billing paragraph the previous move kept on purpose.
- **`src/ui/theme.rs` has no tests** and probably wants none — it is colour constants. Noted so
  the next person does not re-derive that.
- **Resolved in v0.8.0.** Derived escalations are exported: `--json` carries an `escalations`
  object. Deliberately not in `--routing-json` or `--csv`; the reasoning is under *Dashboard*
  above.

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
