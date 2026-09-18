# Changelog

## [Unreleased]

### Added

- **PowerShell completions ship.** `--completions powershell` always worked and nothing ever ran
  it: the Windows zip was the one archive with no completions in it. It now carries
  `completions/_ai-usage-tui.ps1`, and the unix archives carry it beside bash, zsh and fish.

- **The Limits panel says when a window resets, not only how long.** An `AT` column gives the
  reset on the local clock -- `Fri 14:00`, or `Sep 25 14:00` once a weekday alone would read as
  today. A countdown is already old when it is read off a twenty-minute-old snapshot, and "can I
  start this at three?" is asked of a clock.

- **`--summary-json` has a `build` block**: the version, how the binary was installed, the
  command that upgrades an install of that kind, and the last cached answer of the opt-in update
  check. These were the two things `--doctor` knew that no JSON document said. The check is read
  and never made; `build.update: null` means nobody has asked, not that the build is current.
  There is no `--doctor --json`, by decision: everything else it would hold was already here, and
  a second document is a second stable surface.

- **Codex's rate-limit windows, on every platform.** On a ChatGPT plan Codex writes the
  account's 5-hour and weekly windows into every `token_count` event of its rollouts, and nothing
  read them, so Codex's row in the Limits panel existed only on Omarchy. They are read now -- the
  newest block per limit, from the three most recently written rollouts -- and filed under the id
  Omarchy uses, so the two readings of one plan are one row. `--json`, `--summary-json` and
  `--doctor` (a `codex` row under `LIMITS`) carry them too. An API key gets no such headers and
  shows nothing. The format was taken from bytes this time: no Codex account was needed, because
  `scripts/codex-standin.py` answers the Responses API on localhost and the real CLI writes a
  real rollout (`tests/fixtures/codex_capture`, recipe in `docs/provider-support.md`). That
  confirmed the synthetic fixture the collector was built on and found one thing reading the
  source had not: a response can carry several limit families, the thread keeps one snapshot, and
  the last family parsed replaces the rest -- so "the last `rate_limits` in the file" can be a
  2%-used one-hour window for some other limit, on every line. Readings are keyed on `limit_id`.

- **Codex rollouts the CLI has compressed are read.** With `local_thread_store_compression` on
  (off by default as of codex-cli 0.155.0) every rollout untouched for a week becomes
  `<name>.jsonl.zst` and the plain file is removed. Tried against the real CLI, the collector then
  reported two of six calls and said nothing about the rest; a week of history would have left the
  dashboard at each start. They are decoded as a stream, read once, and one that does not decode
  is counted as unreadable by name. One new dependency, `ruzstd`: decoder only, pure Rust, MIT.

- **A terminal is sent the colours it says it can draw.** The palette is 24-bit and went to
  every terminal as such; one that does not understand the sequence draws whatever it makes of
  it. The depth is now read once at startup -- `COLORTERM`, then `WT_SESSION`, then `TERM` -- and
  anything short of 24-bit has the finished frame mapped down in a single pass, as `NO_COLOR`
  is, so a new panel cannot forget it. 256 colours take each colour's nearest neighbour; this
  tier exists because `ssh` and `sudo` drop `COLORTERM` and keep `TERM`. Sixteen map the named
  palette by meaning, so muted text and borders do not land on one grey, leave the backgrounds
  to the terminal, and draw white text in the default foreground so a light theme can read it.
  `--doctor` prints the depth chosen and the variable it came from. If a terminal that does
  support 24-bit colour looks flatter than it did, set `COLORTERM=truecolor`.

- **`--install-hook` and `--install-statusline` put this tool into Claude Code's settings;
  `--uninstall-hook`, `--uninstall-statusline` and `--uninstall` take it out.** Routing
  analytics, the feature nothing else here has, sat behind a hand-run `jq -s '.[0] * .[1]'`
  merge into `~/.claude/settings.json` -- and `jq`'s `*` replaces arrays, so a user with any
  other `PostToolUse` hook lost it, which three documents had to warn about; and nothing could say
  whether the hook was installed at all. The commands append to the two event lists and never
  replace them, write nothing on a second run, keep every other key in the order it was found,
  keep the file's permission bits -- its `env` block may hold keys, and nothing of the file but
  this tool's own entries is ever printed -- and refuse a file that is not a JSON object, naming
  the file and the position and changing nothing. The command written is the bare `ai-usage-tui`
  when a binary of that name is on `PATH` and the running binary's absolute path when not, and
  the report says which, because the two age differently. `--install-statusline` refuses to
  replace another program's status line and names it; `--uninstall-statusline` leaves one that is
  not this tool's. `--uninstall` removes both entries and the caches `docs/stability.md` calls the
  tool's own -- a test holds the two lists together -- then prints the journal's and the config
  file's paths with the `rm` that would delete them, and does not run it: those are the user's.
  `--doctor` gained a `CLAUDE CODE` section, asked of the same detector the installers use, so
  what it calls installed is exactly what `--uninstall-hook` removes; a hook on one event of the
  two is reported as such. The command is the consent, as for `--check-update`: no config key, no
  prompt, and the dashboard never writes there. The setup guide, the README and
  `contrib/claude-code/README.md` now name the commands first and keep the hand merge as the
  alternative.

- **`--prune-journal DAYS` deletes old journal rows and hands the space back; `--doctor` says
  how big the journal is.** `usage.db` had no retention and no `VACUUM`, and `--doctor` counted
  its usage rows and nothing else -- not its bytes, not its routing events, which no source row
  counts, not how far back it goes. The journal is also the *only* copy of what `--record-*` and
  the hook wrote, so the answer is a command and not a policy: nothing prunes on a timer, at
  startup or from the dashboard, and no config key exists to make it. The command refuses fewer
  than 31 days and never reaches into the current month, because a monthly budget still reads
  those rows. It keeps the routing events of a Claude Code session that has newer ones -- the hook
  sums a session's earlier rows to know which requests it has already attributed, and would
  attribute them again -- and the usage row with the highest id, because SQLite hands a deleted
  top id out again, below the cursor of a dashboard that is open. It reports rows deleted of rows
  present per table, what it kept and why, and bytes before and after; it creates nothing when
  there is nothing to prune; and a `VACUUM` that fails exits `2` with the rows still deleted, to
  be retried by running it again.

### Fixed

- **The Linux downloads did not start on current stable distributions.** A binary linked against
  glibc needs a glibc at least as new as the machine that built it, and that was the release
  runner's: 2.39, measured on the v0.20.0 binary. So `install.sh` installed something that
  answered ``version `GLIBC_2.39' not found`` on Debian 12, Ubuntu 22.04 and RHEL 9; the `.deb`
  declared `libc6 (>= 2.39)` and apt refused it there; the `.rpm` declared nothing, installed,
  and failed when run; and none of it could be loaded on Alpine. Releases now carry **static
  builds** (`-x86_64-linux-musl.tar.gz`, `-aarch64-linux-musl.tar.gz`), checked with `file` and
  run on a bare Alpine in the release build. `install.sh` takes them (`--libc gnu` for the other),
  as do `cargo binstall` on a musl host and the Homebrew formula on Linux, and the `.deb` and
  `.rpm` are built from them, checked to require no C library, and installed and run on Debian
  11, Ubuntu 20.04, Rocky 8 and Fedora before they are published. The glibc archives are still
  published, and the AUR package uses them.

- **A webhook URL was printed in full when a POST failed.** A Slack, Discord or ntfy webhook is
  its URL -- the token is the path -- and `reqwest` puts the URL in every error it returns, so a
  timeout or a 404 wrote the credential to stderr and to the diagnostic log; the bad-scheme error
  quoted it too. Every message now names the host and nothing else. Found while deciding what the
  notice below should print.

- **The journal, the caches and the log are created owner-only.** They were created at the
  umask, which on most systems means readable by every account on the machine: project paths,
  session ids and spend in the journal, a subscription's utilisation in the caches. New files are
  `0600` -- the journal is made before SQLite opens it, because SQLite creates at the umask and
  its side files copy the main file's bits. **A file that already exists keeps what it has**: the
  caches tighten as they are rewritten, and for a journal from an earlier release `--doctor` prints
  its mode and the `chmod 600` that fixes it, and does not run it.

- **A CSV field a spreadsheet would run is written as text.** The text columns of `--csv` and
  `--routing-csv` are other programs' strings -- a model name from a transcript, a project path, an
  `agent` handed to `--record-event` -- and one beginning `=`, `+`, `-` or `@` is a formula to the
  spreadsheet that opens the file. It now carries the leading apostrophe that means "text". A
  number is left a number, and `--json` is unchanged.

- **A subagent's output tokens were counted from a placeholder.** Claude Code writes one
  request as a line per content block, all under one `requestId`, and deduplication kept the
  first line seen. In a subagent's transcript the first line holds `output_tokens` as it stood
  mid-stream -- `3`, where the closing line says `190`. Found by the first real transcript
  committed as a fixture (`tests/fixtures/claude_capture`; until now Claude Code, the largest
  source, was tested against strings written into the tests, which said every line carries the
  same usage). On the machine it was found on: 916 of 16,523 requests, all in subagents, 7.5% of
  all output tokens. **Output totals for Claude Code will rise after upgrading, by whatever share
  of your work ran in subagents; nothing else moves.** The reading with more tokens now wins, in
  the one-shot read, across polls in the dashboard (the row is replaced and re-priced), and in
  the routing hook's attribution, which had the same defect and the same comment.

- **A value that is not a count is no longer read as one.** `-1` was `0`, `1.5` was `1`, and
  `1e308` or anything past `u64::MAX` was 18,446,744,073,709,551,615 tokens, which overflowed the
  first total it met -- a panic in a debug build, a wrapped figure in a release one. A count is
  now a whole number from 0 to 2^53; anything else in a field a source always reports marks the
  row `incomplete`, as an absent one does. Gemini and Copilot each read numbers their own way
  with the same flaw and now share the rule, and a negative or non-finite OpenCode `cost` is
  absent instead of a price. No figure moved on 23,464 real rows. Found by the new mutation
  tests, below.

- **The Zen model catalogue was the last cache written through a shared temporary.** Two
  dashboards refreshing together raced on `zen-models.json.tmp`; it goes through the same
  per-process temporary as every other cache. Found by reading the first coverage table.

- **A poll reads what is new, and the dashboard copies what changed.** Three places re-did all
  of history on a timer. Gemini's collector tracked a byte offset and then read the whole
  telemetry log into memory every thirty seconds to slice its tail off; it now seeks and reads
  the tail, starting over when the file shrank or the tail does not open on a character boundary
  -- the second was a slice panic once, contained by the collector's restart guard, so the symptom
  was Gemini going `Dead` and quietly disappearing. The journal collector ran `SELECT ... FROM
  usage_event` with no `WHERE` every sixty seconds and left deduplication to throw it away; it now
  reads the rows above the highest id it has seen, and starts over when the journal is another
  file or its highest id went down. And the dashboard deep-cloned every row ever collected on
  every refresh, changed or not; the collector state now counts its changes and the dashboard
  copies only when the count moved -- a pricing reload moves it, which is the case that matters,
  since a refresh that never reached the screen is a bug this project has fixed once. Rows are
  still never evicted, by decision: every source loads all of history at startup and ALL TIME and
  the budgets read it, so a dashboard that dropped old rows would disagree with one just started.

- **The diagnostic log is bounded, and quieter.** With `AI_USAGE_LOG` set the file grew for as
  long as the variable stayed set -- and what grew it was not errors: every successful poll of
  every collector logged `poll ok`, six lines every thirty seconds, some seventeen thousand a day.
  A poll is now logged when its row count changes. Past 5 MiB the file is renamed to `<name>.old`,
  replacing the previous backup, and started again. Several processes write the one file -- the
  dashboard, each hook, each status-line redraw -- and the rotation takes no lock: whoever finds
  the path over the cap renames it, and a process whose open handle is over the cap while the path
  is not knows it is holding the backup, and reopens; without that second half a dashboard would
  write into the backup for the rest of its life. `--uninstall` removes the backup with the log.

### Changed

- **Project hygiene.** `CODE_OF_CONDUCT.md` says where to report (the address `SECURITY.md`
  gives), where to go when the report is about the one maintainer (GitHub's own channel), and
  what follows; it had said only what was unwelcome. `LICENSE` names Sophana Sok and the
  contributors, where it named "ai-usage-tui contributors" and everything else named a person.
  Every changelog heading is `## [x.y.z] - YYYY-MM-DD` -- fourteen were unbracketed and `0.1.0`
  undated -- and the release workflow now cuts release notes by `[x.y.z]`, not by substring,
  which would have taken `1.0.0` out of `11.0.0`. Three tests hold all of it.

- **The Chocolatey package is built and checked on every release run**, dry runs included. The
  manifests were rendered and attached to every release and never packed by CI, and the one hand
  run of `choco pack` had produced a package that installed nothing. The push is gated on a
  `CHOCOLATEY_API_KEY` secret, as crates.io's is on its token: until an account exists the job
  proves the package and publishes nothing.

- **Plain `http://` to a budget webhook is remarked on, not refused.** The payload is the
  budget's scope, limit and spend. `--check-budgets` says so on stderr, `--doctor` under the new
  `webhook` row, the dashboard in its log; loopback is exempt. Not refused, by decision: the usual
  plain-HTTP target is a notifier on the user's own network, and a LAN host cannot be told from a
  public one without resolving it.

- **The parsers of other tools' formats are tested by damaging real fixtures.**
  `src/collector/mutation.rs` takes each source's captured file apart one value at a time --
  every key deleted, every value replaced with a null, a negative, a fraction, `1e308`,
  `i64::MAX`, a string, a container -- plants the result in a scratch home and reads it through
  the registry's own `load`, then prices and totals it. It walks the registry, so a new source
  without a corpus fails by name; it is deterministic and adds no dependency. Coverage is
  measured (`just coverage`, and a `Coverage` CI job that prints the table to its summary:
  93.7% of lines) and deliberately not gated.

- **The dashboard uses the screen it has.** The six tiles were seven rows tall for two lines of
  text, and four of them spent the second line repeating the first (`3.3M` over `3.3M tokens`);
  TOKEN FLOW held a third of the width for nine lines with everything under it empty; nothing
  said which of the eight panels was showing; and the default view drew no proportion anywhere,
  so "which category, which model" meant comparing `2.5M` with `688.6K` by eye. Now a tab strip
  under the header names every panel and marks the active one -- built from the footer hints in
  the bindings table, so it cannot drift from them, and in reverse video, so `NO_COLOR` cannot
  unmark it. The tiles are four rows and say each category's share and request count; a category
  with no tokens says `—`, never `0%`, and a share that rounds to nothing says `<1%`. Under them
  one strip divides the width between the categories: a category with usage always gets a cell,
  one without never does. The left pane is a rail -- the token list with a bar per kind, a meter
  per subscription window under the limits panel's own stale-and-alarming rule, and tokens per
  day (tokens, because on a subscription a chart of dollars is a flat line that reads as nothing
  happening). A rail section that does not fit whole is not drawn, last first, instead of being
  squeezed to a border around nothing. Tables colour the CLASS cell, draw a bar beside TOKENS
  when the pane is wide enough to spare it, say how many rows they hold, and show a scrollbar
  only when rows are off screen. Borders are rounded. The layout is not a stable surface
  (`docs/stability.md`); nothing a script reads has changed.
- **`API-RATE EQUIV.` is two lines.** On one it was wider than its pane on most terminals, and
  what a `Paragraph` cut off without saying so was `not billed` -- the half that stops the
  figure reading as a charge.
- **JSON objects keep the order their keys were written in.** `serde_json` now builds with
  `preserve_order`, which the installers need to hand a user's `settings.json` back as they found
  it. The visible effect elsewhere: a document built key by key (`--check-budgets`, the
  `--schema` glossary) prints its keys in the order the code names them rather than
  alphabetically. No promise covered the order, and a reader that parses JSON does not see it.

## [0.20.0] - 2026-09-17

### Added

- **Releases are attested, and ship a bill of materials.** A checksum proves a download is the
  file the release lists, and nothing more: `checksums.txt` comes from the same place as the
  archive, so whoever could replace one could replace both. From the next release every archive
  and every `.deb` and `.rpm` carries a build attestation -- signed by the release workflow's own
  identity and kept by GitHub apart from the release's files -- binding the file's digest to this
  repository, `release.yml` and the tagged commit. `gh attestation verify <file> --repo
  SophanaSok/ai-usage-tui --signer-workflow …/release.yml --source-ref refs/tags/<tag>` checks
  it; the last flag matters, because a hand-run dry run attests what it builds too, as built from
  its branch. Each release also ships `ai-usage-tui-<tag>.cdx.json`, a CycloneDX list of every
  crate any released target links, with version, licence and registry checksum, attested against
  the same files. Both attestations are made before the release is created, so a failure there
  publishes nothing. Tried end to end on a dry run before merging: the tarball, the `.deb` and the
  bill-of-materials predicate verify; a tampered copy, another workflow and the wrong ref do not.
- **`install.sh` checks the attestation when it can, and refuses a download that fails it.** With
  a usable GitHub CLI -- installed, recent enough to tie a file to a tag, signed in -- the
  installer verifies the archive it just downloaded. Nothing is refused for the lack of a tool:
  that is reported as "not checked" and the install goes on. A check that *fails* on a release
  that should be attested is different, and refuses; the first draft of this step printed "do not
  use this download" and then installed it, which the review of this change caught.
  `--require-attestation` makes "not checked" fatal as well, and `--no-attestation` skips the
  step for whoever has a reason to.

### Changed

- **A failure exits `2`; `1` now means only that a budget is over.** Every failure and a
  breached budget shared one exit code, so the scheduled `--check-budgets` this tool tells people
  to run could not tell "you are over" from "your config does not parse" -- the recipe shipped in
  `--agent-guide recipes` parsed stdout with `jq` to find out which it had, and a cron line
  testing the status alone would have raised a budget alarm for a typo. The codes now mean what
  they mean to `grep` and `diff`: `0` fine, `1` the check said no, `2` trouble -- a flag the tool
  does not know, a config or source it could not read, a write that did not happen. The breach
  keeps `1`, the number the README documented, and `docs/stability.md` had promised only
  "non-zero" for a failure, so a script written against either keeps working unless it tested a
  *failure* for `== 1`. This is the last change of its kind before 1.0.0, which freezes it.
  `--help` and the man page gained an `EXIT STATUS` section, and the budget recipe is three lines
  shorter. One exception, from a capture and not from documentation: on Claude Code 2.1.275 a
  `PostToolUse` hook that exits `2` has its stderr given to the model as something to act on, and
  one that exits `1` does not -- so a failed `--claude-code-hook` still exits `1`, including when
  what failed was the config, before the hook's own code ran. A journal that could not be written
  is not the model's to fix.
- **Every action is pinned by commit, and every workflow token is least-privilege.** Actions
  were named by tag -- a pointer its owner can move -- including in the job that holds the
  crates.io token, and the MSRV job tracked a branch. All are now `owner/action@<commit> # version`,
  which Dependabot maintains; the one tool the release job downloads is pinned by version and
  checked against a digest written in the workflow, not the one served beside it. `release.yml`
  granted `contents: write` to all ten jobs; it is read-only at the top, the release job alone can
  write, and the tap job gets no repository token at all. `ci.yml` and the two Claude workflows
  declare their permissions instead of inheriting a setting. Two tests hold this: one fails for
  any `uses:` not pinned to a 40-hex commit with a version comment, one for a workflow with no
  `permissions:` block or a top-level write.
- **A release goes through a pull request.** `main` is now protected -- changes by pull request
  with the seven CI checks passing, no force-push, no deletion, no bypass -- so the release commit
  no longer goes straight to it. `scripts/release.sh` runs on `release/vX.Y.Z` before the pull
  request and again on `main` before the tag, where it also refuses a `main` that is not
  `origin/main`. The ruleset that existed was switched off, and could not have been switched on:
  it required three checks that do not exist and an approving review from a second maintainer
  the project does not have.

### Fixed

- **A release whose asset list failed its check said nothing about why.** `publish-release.sh
  --publish` captures the list `plan` prints, and `plan` printed its errors to the same stream --
  so a missing manifest failed the job with no message at all. The dry run (`--plan`) showed the
  error, which is how it went unnoticed. Errors go to stderr now; found by the test for a missing
  bill of materials, which asserted on a message that never arrived.

## [0.19.0] - 2026-09-17

### Added

- **`--agent-guide recipes` and `--agent-guide extend`: an agent can build on the data, and cover
  what the tool lacks.** `recipes` says what is stable enough to script against and what is not,
  carries the reading rules into code (`.cost // 0` in `jq` turns an unknown into a zero nobody can
  see; SQLite sums an empty CSV field as `0`), and gives worked scripts: a Waybar module, a guard
  that exits non-zero before a plan window runs out, a budget alert, a weekly Markdown digest, a
  per-project table, rows into SQLite. Every recipe that needs only `jq` is *run* by the test
  suite, as written, through a shim that pins every source -- a recipe that fails on real output,
  or prints `null` where it promised a value, fails the build. `extend` routes a request to the
  cheapest thing that answers it: an adapter into `--record-event` (keys, a worked example, why
  re-sending the whole log is the right design), a script, or a change to the source, with the
  rule that outranks all of them -- a tool that does not measure its token counts gets no row.
  The shipped skill and the pasted `AGENTS.md` block are broadened to set-up, build and extend
  requests, and still name no topic: a test refuses one, because an installed skill outlives the
  binary and an older binary rejects a topic.

- **`--agent-guide setup`: an agent can set the tool up, not only read it.** `--agent-guide` takes
  an optional topic. Bare it prints what it always has, byte for byte -- every installed skill and
  pasted `AGENTS.md` block says "run `--agent-guide`", and they outlive the binary they were
  written for; the default guide now lists the topics, and nothing installed names one, because
  an older binary would reject it. `setup` covers the config file and budgets, the Claude Code
  hook and status line, the systemd timers, a tool with no collector, and undoing all of it.
  It is written for an agent on a binary install, where there is no `contrib/` to copy from: the
  hook's JSON and the four units are inside the guide, and a test holds each to the shipped file
  byte for byte. The tool still edits no other program's files and gains no install command --
  the agent makes the change, after showing it, and `--doctor` says whether it took. What an
  agent gets wrong without being told is in there too: which commands write or use the network,
  that `--print-config`'s sample budgets are live, that merging the hook with `jq`'s `*` deletes
  the user's other hooks, and that a budget counts dollars and so watches nothing on a
  subscription plan.

- **`--record-event`: a way in for a tool that has no collector.** The recorders each understood
  one server's response, and a bare response cannot say where it was made -- so usage fed in from
  outside never reached the Projects or Sessions views, had no cache writes, and could not say it
  was billed against a plan. `--record-event` reads usage in this tool's own terms, one JSON
  object per line: `provider`, `model`, `input_tokens`, `output_tokens`, one of `event_id` or
  `created`, and optionally the rest of the token split, `project`, `session_id`, a `cost` the
  tool itself recorded (kept as `reported`, never re-estimated) or `"billing": "subscription"`
  (a `quota` row with `api_equivalent_cost` beside it, on the same path a native collector's
  takes). A few lines of `jq` over a tool's own log is a whole integration.

  It is strict where a collector is tolerant, because an adapter's author -- often an LLM agent
  -- learns from the exit status and nothing else: an unknown key, a count that is not a whole
  number, a line that is not JSON each refuse the *whole* batch, by name, before the journal is
  opened. And it records measured counts only. An event without its token counts is refused,
  never stored as zero, so a tool that keeps no counts cannot be journaled by guessing them; and
  no `cost_status` but `reported` can be supplied, so this tool never vouches for arithmetic it
  did not see. A supplied `event_id` is stored as `event:<provider>:<id>`, since identities share
  one namespace across sources.

  The journal's `usage_event` gains three nullable columns (`session_id`, `project`, `billing`).
  An older build's writer and reader name their columns, so they are unaffected, and the journal
  schema version stays `1`: an old hook and a new dashboard can keep sharing one file.

- **A monthly job keeps the bundled rate table from going stale by neglect.** The tool now tells
  a user when their install's rates are over 90 days old; this is the other end of that promise.
  `pricing-drift.yml` regenerates `pricing/litellm.tsv` when LiteLLM's table has moved, runs the
  pricing engine's tests against it, and opens one issue with what changed and a link that opens
  the pull request -- an issue, because a pull request opened by the workflow token gets no CI, and
  never a red build, because upstream moving is not a failure. If the tests fail against the new
  table it says so and pushes nothing. The community table stays release-bound by decision:
  refreshing it at runtime would give `--refresh-pricing` a new host to contact.

- **Bundled pricing says when it is old.** Rates ship in the binary, and only the *refreshed cache*
  was ever compared to the clock -- so an install six months old priced at six-month-old rates
  without a word, which is a confident number resting on a fact nobody checked. Both tables carry an
  `# Updated:` date; past 90 days the engine now says so, naming both dates and what to do
  (upgrade: `--refresh-pricing` updates the curated Zen rates only). A fresh refreshed cache
  supersedes the curated table's date, so then only the community snapshot's age counts.
- **Pricing warnings reach the dashboard.** A refused cache -- stale, unreadable, invalid -- and the
  age notice were printed by `--doctor` and nowhere a running dashboard could show them, so a
  dashboard pricing from a table it had silently fallen back to looked exactly like one that was
  not. The status line now carries one clause (`pricing: 1 problem(s), see --doctor`, or
  `pricing: bundled rates over 90 days old`); a fault turns the header red, age alone does not.
- **The currency and the table dates are stated.** `--doctor` prints when each bundled table was
  cut, and `--summary-json`'s `pricing` block gains `currency` (`USD`, list price, nothing
  converted), `community_table_date` and `curated_table_date`. No figure anywhere had a unit.

### Changed

- **The contributor's guide says what adding a data source really takes, and tests hold the parts
  of it that were wrong.** `CONTRIBUTING.md` called it "two files" and listed the other six
  thirty lines later; its fixture command and `just run` both called themselves hermetic while
  leaving Copilot and Gemini unpinned, so the documented fixture-only run printed the reader's own
  rows. The section now opens with the two questions that decide whether there should be a
  collector at all (does the tool measure its own counts; would `--record-event` do), requires a
  redacted real capture before a parser, lists every file, and names the tests that will say what
  is missing. `documented_fixture_commands_pin_every_source` runs both documented commands as
  written and asks `--doctor` where each source resolved. `AGENTS.md` is no longer headed as one
  vendor's instructions and gains an "Extending it" section -- cheapest route first, and the rules
  that outrank a request (never invent a number, work from real bytes, a test must fail against
  its bug). A project skill, `.claude/skills/add-data-source/`, gives a coding agent the order to
  work in, including the two places it should stop. The check that agent-facing files name only
  real flags now matches the parser exactly -- it was a substring search that accepted `--record`
  -- and covers `AGENTS.md` and the new skill.

- **The guards a new source or panel trips now ask the code, not a list kept in a test.** Five
  checks each carried a hand-written list -- of source ids, of billing-capable sources, of panels,
  of overlay words, of actions `--once` refuses -- and a list in a test passes for the entry nobody
  added to it: Gemini was billing-capable and outside the billing check; `--record-usage` and
  `--statusline` were outside the `--once` check. They now iterate the registry, `Panel::ALL`
  (generated beside the enum by one macro), the bindings table and the parser's own action list.
  Two new ones: `--doctor` under the test harness must resolve *every* registered source inside
  `tests/fixtures`, which catches a source the harness forgot to pin and one reached through an
  environment variable; and `--schema`'s sentence listing the source ids must match the registry.
  One leak closed on the way: the registry's reachability test defaulted the roots it did not
  name, so it read the developer's real `~/.copilot` and `~/.gemini`.

- **The bundled community rate table is refreshed from LiteLLM** (snapshot of 2026-09-17, the
  first opened from the monthly drift job's issue): 3,975 keys become 4,627 -- 713 added, 61
  removed, 182 repriced -- and the engine prices 4,370 models, up from 3,785. Of the repriced, 59
  only gained a published rate (usually cache reads) or lost a rounding; 63 got cheaper, such as
  `azure/gpt-5.6-sol` from $5/$30 to $4/$20 per million, 52 dearer and 8 moved both ways. A cost
  computed for one of those models changes with this release, in either direction; the curated Zen
  table, which wins where both list a model, is untouched.

### Fixed

- **A free model the rate table lists at `0.0` is `FREE` again.** The rule added earlier in this
  cycle -- a name does not make a model free if the pricing table lists a rate for it -- asked only
  whether a rate was *listed*, and the community table publishes free tiers as an explicit
  `input=0.0 output=0.0`. So `llama-3.3-70b-instruct-turbo-free`, and every other free model the
  table knew about, became `PAID`, priced at an estimated $0.00 and counted as billable. A
  published rate of zero is the table agreeing the model is free; only a rate above zero
  contradicts its name. Found by the pricing-drift job's first dry run, where a regenerated table
  listing OpenRouter's `...:free` models made a classification test fail.
- **A token count a source stopped reporting is no longer read as zero.** The counts are plain
  integers, so an absent `output_tokens` was `0` -- and a field renamed upstream would have priced
  every request as though it produced no output: a confident, low, wrong number, from a tool whose
  first rule is that unknown stays unknown. The two counts every record of a source carries are now
  required, in all five parsers (Claude Code, Codex, OpenCode, Gemini CLI, Copilot's store). A record
  missing one is kept, because what it does say is a fact; marked `incomplete` (a new key on each
  `--json` row); **never priced** from what is left, neither as an estimate nor as a subscription's
  list-rate figure; and counted onto the status line -- `1 record(s) missing a token count, left
  unpriced` -- and into `--doctor`, `--json` and `--summary-json`. A `usage` block whose every field
  was renamed is now a flagged request of unknown size rather than a row silently dropped. Cache and
  reasoning counts stay optional: older builds and other providers genuinely omit them.
- **Records with no timestamp are counted.** They are stored as the epoch, so they appeared under
  `--all` and in no other range, no day and no budget period, with nothing on screen saying so. The
  status line now reads `N record(s) with no timestamp, in no range but --all`.
- **`FREE` is no longer asserted from any `free` in a model's name.** `FREE` means `$0.00` with no
  lookup, and the rule was a `free` token anywhere in the id, so `free-tier-preview` on an
  unrecognised provider was zero-cost on the strength of its spelling. A name now counts only as a
  provider's documented suffix -- Zen's `-free`, OpenRouter's `:free` -- and never against the
  pricing table: a model the table lists a rate for is not free however it is spelled.
- **The recorders say when they stamp the time of recording.** `--record-ollama`, `--record-usage`
  and `--record-routing` date an event that carries no timestamp with the moment it was recorded,
  which is a fair reading for a response piped in as it completes and a wrong one for a replayed
  file. It was done in silence; it is now noted once on stderr.

Checked against real logs before and after: 40,805 of 40,805 Claude Code `usage` blocks carry both
required counts and a timestamp, and no figure moved across 25,659 requests.

## [0.18.0] - 2026-09-17

### Added

- **`--summary-json`: the whole picture in one compact document.** `--json` prints one object per
  request -- 13.3 MB for the 25,000 requests on the machine this was written on, about a hundred
  times what fits in a model's context window -- and no aggregated JSON existed at all: the
  by-model, by-project, by-session and by-day rollups were computed for the dashboard and rendered
  only there. The summary is those, as one line of about 33 KB for the same history: `totals`,
  `by_category`, `by_model`, `by_project`, `by_session`, `by_day`, the trailing-hour `burn` rate,
  every budget including the ones still `OK`, `limits`, `escalations`, `provenance` and `routing`,
  plus what only `--doctor`'s text carried -- `sources` (rows found, status, the billing decision,
  skipped data) and `pricing` warnings. `--top N` (default 10) lists the largest models, projects
  and sessions and folds the rest into `other`, so a truncated list still adds up to the totals.
  Every rollup carries derived figures nothing computed before: `cache_hit_pct`,
  `tokens_per_request`, `cost_per_request`, `output_pct`, `reasoning_pct`, `share_of_tokens_pct`.
  Each `by_model` row carries `list_input_rate`, and each escalation `from_input_rate` and
  `to_input_rate` (also in `--json`): the pricing table's dollars per million input tokens, so
  which model is the expensive one is a number rather than something inferred from a name -- a
  model reading an early build called an escalation to a newer, pricier model a "downgrade".
  They are facts, not advice -- no thresholds, no verdicts -- and unknown stays unknown: a
  percentage nothing recorded is `null`, not `0` (several sources never report cache or reasoning
  tokens), and `cost` is `null` when nothing in a bucket could be priced.
- **`--schema` and `--agent-guide`: the output explains itself, from the binary.** The meanings
  of the JSON lived in README prose, `docs/data-model.md` and Rust doc comments, none of which the
  CLI could hand to whatever was reading its output -- `"cost_basis": "floor"` could be resolved
  only by reading the source, and `docs/data-model.md` spelled the categories in lower case while
  the exports print them in upper. `--schema` prints a JSON glossary of every key of every
  document and every value of every closed vocabulary, with its type, whether it can be `null`,
  and what it means. It cannot drift: a test walks real `--summary-json`, `--json`,
  `--routing-json` and `--check-budgets` output against it and fails on any key, enum value or
  `null` it does not describe, and another holds each vocabulary to the labels the code prints.
  `--agent-guide` prints the guide for an LLM agent: start with the summary, drill down with the
  filters, the reading rules (`null` is never 0, `cost` can be a floor, `quota` is real cost with
  no figure, `api_equivalent_cost` was never charged, a token share is not a cost share), what to
  look for in usage and routing, and what not to claim. Both are compiled in, because no binary
  install ships `docs/`, and both work before the config is read.
- **A Claude Code skill, and a paste-in block for every other agent.** `contrib/` fed data *into*
  the tool -- the hook, the status line, the recorders -- and shipped nothing for reading it back
  out. `contrib/claude-code/plugin/skills/ai-usage/` is a skill that answers "how can I cut my
  token usage?" or "is the expensive model worth it here?" from the tool's own data. It is
  deliberately thin: it sends Claude to `--agent-guide` and `--summary-json`, so the instructions
  always match the installed version and the skill never needs updating; it pre-approves
  `ai-usage-tui` commands and nothing else. Install it as a plugin -- the repository is its own
  marketplace, `/plugin marketplace add SophanaSok/ai-usage-tui` -- or copy the directory into
  `~/.claude/skills/`. `contrib/agents/README.md` is the same three lines for `AGENTS.md`,
  `.cursorrules` or a system prompt. Checked end to end against a real account: Claude loaded the
  skill, ran the guide and the summary and never `--json`, reported 1.47B tokens at a 98.5% cache
  hit as plan-billed with no dollar figure, named context size rather than caching as the lever,
  and declined to judge the routing because only one outcome had been recorded.
- **The README has a section for this** -- "Ask an LLM about your usage" -- and the documents a
  reader meets say what shipped: the routing guide defines every `cost_basis` value and what it
  does to `cost_per_success`, the privacy and security notes say what an agent's provider sees
  (project paths and session ids; never prompts or transcripts, and nothing sent by this tool),
  and the roadmap records why the tool gives facts rather than advice and why there is no MCP
  server in V1.
- **`--project PATH` and `--session ID`** filter every export, so a reader goes from the summary
  to one project or session without pulling every row.
- **`--csv -`** writes the CSV to stdout. It is the compact row format and could only be written
  to a file.
- **`billing` in each `--json` row** (`per_token` / `subscription`), which the data model
  documented and no export carried, and **`success_rate` in `--routing-json`**, which the panel
  showed and the export left to the reader to divide.

### Changed

- **`--routing-json` honours a range flag when one is given.** It was all history or nothing.
  Without a flag it still means all history: the default range elsewhere is a week, and applying
  that unasked would have shrunk every existing script's output.
- The dashboard's model table and the summary's `by_model` are grouped by one function
  (`summary::model_rows`), with a test holding them to each other.
- **Every channel now points at the website.** crates.io and GitHub's About box named the site,
  while the Homebrew formula, the Scoop manifest, the Chocolatey nuspec and the AUR `PKGBUILD` all
  named the source repository as their homepage, and `--help`, the man page, `--doctor` and the
  installer named nothing -- so how the tool was installed decided whether a user ever learned the
  documentation existed. The packaging templates carry `__HOMEPAGE__`, rendered by the release job
  from `Cargo.toml`'s `homepage` like the description is; `--help` and the man page end with a
  `MORE:` block naming the site and the repository; `--doctor` lists it under THIS BUILD; the
  installer prints it when it finishes; the README leads with a badge and a line saying what is
  there. `package.documentation` names the site too, so crates.io's Documentation link goes to the
  user documentation rather than to docs.rs for a library API `docs/stability.md` says not to use.
  A test holds every one of these to the single field.
- **Release assets are uploaded one at a time, each confirmed before the release goes public.**
  `softprops/action-gh-release` uploaded all fifteen at once, and on v0.17.0 GitHub left the
  multi-megabyte ones stuck half-finished (`state: starter`), which a same-name upload cannot
  replace -- each re-run deleted and re-uploaded everything and left more stuck, one then three then
  five, until the draft was deleted by hand. `scripts/publish-release.sh` now creates a draft,
  uploads serially through the REST endpoint, checks every asset's state and size against the API,
  deletes a stuck one before retrying with backoff, and publishes only when all are confirmed. The
  dry run checks the same asset list. It is tested against a fake `gh` and `curl` in CI, where
  removing the stuck-asset deletion or letting a failed asset check fall through both fail the test.

## [0.17.0] - 2026-09-17

### Added

- **A journal schema version.** Writers stamp `PRAGMA user_version` and refuse, by name, a journal a
  newer build has stamped higher -- a hook installed from one channel beside a dashboard from another
  is how two builds come to share one file. See `docs/data-model.md`.

- **`NO_COLOR`.** Any non-empty value draws the dashboard without colour, per no-color.org. Every
  colour was a hard-coded RGB value, backgrounds included, with no way to turn it off. The colour is
  removed from the finished frame in one pass rather than branched in every panel, so a panel added
  later cannot ignore the setting; bold and the rest stay, and the selected row -- which colour
  alone had marked -- is drawn in reverse video.
- **`--print-config`** prints the annotated example configuration, which is now in the binary.
  `--doctor` used to tell a user without a config to "copy examples/config.toml there", a file no
  binary install channel ships. It works before the config is read, so a broken config does not
  stop it. The example's budgets are live samples, so the hint says to edit them rather than
  suggesting a `> config.toml` redirect.

- **`docs/stability.md`: what a version number promises.** Semantic versioning covers the
  command-line tool -- flags, exit codes, config keys, JSON and CSV output, the journal schema,
  environment variables -- and explicitly not the Rust library API, which exists so the binary, its
  tests and the screenshot renderer can share code. Nothing had said either, while the crate
  published nineteen public modules on crates.io. The crate documentation now says the same.
- **`"schema_version": 1` in every JSON document** -- `--json`, `--routing-json` and
  `--check-budgets` -- so a consumer can check what it is reading. Additive: no key moved.

### Changed

- **`#![forbid(unsafe_code)]`** in the library and the binary. `SECURITY.md` promised no `unsafe`
  code; the build now enforces it.

### Fixed

- **A `kill`, a closed terminal window or a logout no longer leaves the terminal broken.** The
  panic hook restored raw mode and the alternate screen; a signal never reached that code, so
  SIGTERM, SIGHUP or an outside SIGINT ended the dashboard with the shell still on the alternate
  screen and echo off. All three now set a flag the event loop checks every 250ms and leave through
  the same exit as `q`; a second signal exits at once. Checked in a real pseudo-terminal against
  the v0.16.0 binary, which died on each signal without leaving the alternate screen.
- **Quitting no longer freezes a raw-mode terminal behind a poll in flight.** The dashboard owned
  the collector handle, and dropping it joined every collector thread *before* the terminal was
  restored -- with no bound, and a poll cannot be interrupted, so pressing `q` during a
  rate-limited `zen_pricing` fetch held a frozen screen for most of a minute. The terminal is now
  restored first, and the join waits at most two seconds before leaving a stuck poll to the
  process exit (`CollectorHandle::join_within`).
- **The `zen_pricing` collector no longer prints into the dashboard.** Its rate-limit retry notice
  went to stderr from a background thread, which lands in the middle of the frame. The collector
  logs it; the one-shot `--refresh-pricing` still prints it.
- **`--record-ollama`, `--record-usage`, `--record-routing` and `--claude-code-hook` survive a
  closed stdout.** Each confirmed with a bare `println!` after journaling, so a caller that closed
  the pipe got a written row, a panic, and a failing exit status that said the recording had not
  happened.

- **Data a collector reads around now shows on the dashboard.** Every tailing reader skipped an
  unreadable file with `Err(_) => continue` and a line that was not JSON as "no usage here", and
  counted neither: a transcript with a bad byte, a Codex rollout written in a new encoding or a
  corrupt OpenCode row made the totals smaller while the header stayed green. Claude Code, Codex,
  OpenCode, Gemini CLI and Copilot's legacy logs now record both through one `collector::skipped`
  type and implement `Collector::warning` -- which until now only the local-model journal did --
  so the live status line reads, say, `claude_code: 1 file(s) unreadable, 2 malformed record(s)
  skipped` and the header is marked degraded. The one-shot status carries the same note plus the
  first unreadable path and error into `--once`, `--json` and `--doctor`, and the log records each
  change. An unreadable file is a current state (it is retried every poll and drops out once it
  reads); a skipped line is permanent for the process, and is counted exactly once -- including
  OpenCode's deliberately re-read boundary row, which would otherwise have grown by one per poll.
  Gemini's existing count had lived only inside a single read, so the incremental dashboard
  reported it for one poll at most.
- **Subscription windows this build does not recognise are reported, not dropped.** The
  `~/.claude.json` reader counted entries of an unknown `kind`, and nothing read the count, so a
  window Claude Code added upstream vanished from the Limits panel, `--json` and `--doctor` alike.
  It is now a limits problem -- on the status line, and in a `problem` row under `--doctor`'s LIMITS
  section, which had never printed the problems the panel flags at all.

- **Concurrent hooks no longer fail on an unmigrated journal.** Opening the journal to write ran
  probe-then-`ALTER` with no lock held between the two, so writers that opened a journal from
  before `event_id` together -- parallel subagents fire parallel hooks -- all saw the column
  missing, and every one but the first died on "duplicate column name". A test with eight writers
  reproduced it on the first round. Migrations now run under `BEGIN IMMEDIATE`, the routing
  table's rebuild runs inside that transaction instead of opening its own, and writers wait up to
  five seconds for the lock instead of 250ms.
- **The update and pricing caches no longer share a temporary file between writers.** Both wrote
  through a fixed `json.tmp` / `toml.tmp` on the belief that only the dashboard wrote them; in fact
  two dashboards each run `zen_pricing`, and a scheduled `--check-update` can land beside an
  opted-in `--doctor`. Writers sharing a temporary race, and the loser's rename moves a half-written
  file into place. All three caches now go through one `helpers::write_atomic`, which names the
  temporary per process and removes it when the rename fails -- the rule `--statusline` already
  followed.

- **A new install no longer opens on a blank table.** With no rows the default panel drew a header
  over nothing beside tiles reading `0` -- a working dashboard with nothing to report, the least
  likely reading of an empty screen. It now says no usage was collected and names
  `ai-usage-tui --doctor`, or, when data exists outside the range or filter, says so and how to widen
  it.
- **A pane shorter than 20 rows says so** (21 while a budget alert's banner is showing). Below the
  height the layout needs, ratatui squeezed the panels to zero height one by one without complaint.
  The dashboard now shows the rows it needs and has, says so when a budget alert is active so a short
  pane cannot hide one, and keeps the key hints -- and how to quit -- on the last line.

- **`SECURITY.md` listed the network calls as `--refresh-zen`, `--refresh-pricing` and the budget
  webhook**, omitting `--check-update` and an opted-in `--doctor`, which have called GitHub's
  releases API since v0.11.0. The security policy and the README's privacy section now agree.
- **The Limits panel was described as "from Omarchy's agents panel"** in `--help`, the `?` overlay
  and the README panel table, although Claude Code's cache and status line have fed it since
  v0.13.0 with no Omarchy at all. The README's privacy section and paths table still said "Ollama
  journaling", and its prerequisites omitted Gemini CLI and llama.cpp.
- **The review workflow's rubric read a `CLAUDE.md` that did not exist.** `CLAUDE.md` now imports
  `AGENTS.md`, Claude Code's documented way to share one instructions file, so the reviewer and a
  local session read the same conventions.

## [0.16.0] - 2026-09-17

### Added

- **`--record-usage PROVIDER`, so llama.cpp usage stops being invisible.** The journal's only
  write path spoke Ollama's API and hardcoded `provider = 'ollama'` in its INSERT, so a machine
  serving its models through llama.cpp's `llama-server` -- or LM Studio, or vLLM, none of which
  speak that format -- had no way in at all. `llamacpp` appeared in this codebase in exactly one
  place, the `LOCAL_HOSTS` list that *labels* such a row once some collector has produced one,
  and no collector ever did: the usage reached the dashboard only when OpenCode happened to be
  proxying it, and `--doctor` reported `journal  found  0 rows` without hinting why. The new
  command reads a completed OpenAI-compatible response from stdin and journals it under a
  provider you name. Three things it will not do: it will not guess the provider, because that
  is what decides local-at-a-genuine-zero against a price it would then have to look up; it will
  not count cached prompt tokens twice, since OpenAI reports them *inside* `prompt_tokens` while
  this tool keeps `input_tokens` and `cache_read_tokens` apart; and it will not journal a row of
  zeros for a streamed response that carried no `usage`, which is what a request that forgot
  `stream_options.include_usage` gets -- it fails and names the flag instead. Raw server-sent
  events pipe in directly, and the response's own id keys the row, so a replay is a no-op.
- **`contrib/codecompanion/`**, which wires that into CodeCompanion -- the way llama.cpp gets
  driven from Neovim. It asks for usage on streamed requests and pipes the chunk that carries it
  into `--record-usage`, fire-and-forget, so a missing binary can never interrupt a chat.

### Changed

- **The `journal` source is now "Local models", not "Ollama".** It was never only Ollama's -- it
  is the local-model journal, and it now has a second recorder feeding it. The Omarchy record
  still writes under the id `ollama`, which is the filename Omarchy's panel reads, but no longer
  filters the journal down to rows whose provider is literally `ollama`: that filter would have
  silently dropped every llama.cpp row from the panel.

### Security

- **rustls 0.23.45, closing RUSTSEC-2026-0285.** Earlier rustls accepted a TLS 1.3 handshake
  message sent at the wrong encryption level when it shared a record with a key-changing message,
  where RFC 8446 §5.1 requires the connection be terminated. The transcript stays authenticated, so
  a peer could not alter or complete a handshake -- only send in plaintext what should have been
  encrypted without being hung up on. This tool reaches rustls through `reqwest`, and only on its
  opt-in outbound calls: `--refresh-pricing` / `--refresh-zen` and the `zen_pricing` collector,
  `--check-update` and an opted-in `--doctor`, and a configured budget webhook. Lockfile only; every
  install channel built from v0.15.0 carries the affected version, which is why this release exists.

## [0.15.0] - 2026-09-03

### Added

- **`--check-update`, and a timer to run it.** The release check used to be reachable only
  from an opted-in `--doctor`, so a user who never ran that never learned a release existed.
  `--check-update` is the check on its own: a one-shot command in the `--refresh-pricing`
  family that asks GitHub for the latest tag, caches it where the dashboard header reads it,
  says whether it is newer than the running build, and exits non-zero when it could neither ask
  nor cache. No config key gates it -- the command is the consent, as it is for the refreshes.
  `contrib/systemd/user/ai-usage-update.{service,timer}` run it daily for anyone who wants the
  header kept current without running anything by hand. The dashboard process itself still
  never makes the request: the periodic writer lives in the schedule the user installed, not in
  a background collector, which is why this was deferred and how it is resolved.
  `--doctor` and the command now share one implementation, `update::check_and_cache`, and the
  doctor's "not checked" line names both ways of opting in.

### Fixed

- **The library's export tests read the developer's own Copilot store and journal.** Their
  `Cli` pinned Claude Code, Codex and Omarchy to paths under a temp directory and left the
  rest at the defaults, so on a machine with `~/.copilot/session-store.db` the JSON export test
  printed that machine's real rows and both export tests opened the real routing journal. This
  is the gap PR #78 closed in `tests/cli.rs` with `hermetic_with`; the in-process tests never
  went through the binary and were missed. They now build on a `pinned_cli` that names every
  source root, and assert that only the fixtures' providers reach the export, so a root added
  later without a pin fails the test instead of leaking silently.

## [0.14.0] - 2026-09-02

### Added

- **A demo you can watch, rendered from data that was never real.** The README opens on
  `docs/assets/demo.gif`: the dashboard walked key by key — the routing panel, a sort and its
  reverse, a project drill-down and back, the limits panel, the key reference. It is not a
  recording. `examples/render-screenshots.rs` takes a `--script` of key tokens and replays them
  through `App::apply`, the dispatch the event loop itself now calls (it was an inline `match`
  in the loop, so the demo would otherwise have carried a second copy of what each key does),
  writing one SVG per key; `scripts/render-readme-screenshots.sh` rasterises the frames and
  assembles the GIF with ImageMagick, skipping it with a warning where that is missing. Every
  frame comes from the invented fixture through the same off-screen path as the stills, so the
  disclaimer under the image stays true of the moving one. The limits panel is the eighth
  still: the fixture generator now writes the `rate_limits` payload Claude Code would push, and
  the script feeds it through the real `--statusline` into the scratch data root, so the panel
  and the header's limits line show windows that were made up for the purpose.

- **`--statusline`: Claude Code's rate limits, pushed.** Claude Code hands a statusline command
  its official `rate_limits` block on every redraw and again when a window reaches its reset, and
  nothing else in this tool is *pushed* at it — `~/.claude.json` and Omarchy's records are polled
  on the dashboard's interval. `ai-usage-tui --statusline` reads that payload from stdin, prints
  a one-line readout for the status bar (`5h 42% (resets 2h 10m) · 7d 63% (resets 3d 4h)`, in
  red past 90%), and caches the windows under the data directory, where `limits::load` reads them
  as a third producer beside the config cache and Omarchy. So the `l` panel and `--json` carry
  them on any platform, and Claude Code gets an always-visible readout in the same change.
  `contrib/claude-code/statusline-settings.json` is the one-line settings entry; it is a separate
  file from the hooks entry so installing one does not install the other.

  **Absence is meaning, four times over.** The block is absent on an API-billed account and in
  every session before its first response: that is "no such thing here", not 0%, so the line is
  empty, the exit is 0 and the cache is left as it was. Each window may be independently absent,
  and the cache is rewritten with exactly what is present, so a window that has gone is cleared
  from the panel rather than frozen at its last figure. Claude Code drops a window once its
  `resets_at` has passed, so a window behind the clock is dropped at *read* time, whichever side
  of the cache it is on — rendering the last-known percentage after the reset would show a full
  bar on an empty window. And a percentage that is not a finite, non-negative number drops its
  window rather than becoming one.

  **What is read, and the guarantee around it.** From the payload only
  `rate_limits.{five_hour,seven_day,spend_limit}.{used_percentage,resets_at}`; the session id,
  transcript path, working directory, model and session cost beside them are never deserialised,
  the three windows are struct fields rather than an iterated map, and a test plants a marker in
  every one of those places and fails if it reaches the cache, the line or a `Debug` rendering.
  `resets_at` here is epoch seconds as a number while `~/.claude.json` spells the same instant as
  RFC 3339 text, and the two readers are kept separate so neither format is accepted where the
  other is meant. The freshness rule is the two-sided one from v0.13.0. One subscription stays one
  row: the statusline files under the same agent as the config cache and the fresher reading
  wins, at the recorded cost that the statusline carries no per-model weekly window.

  **The line is the product and the cache a by-product, and the exit code says so.** Claude Code
  shows stdout only from a command that exited 0 and blanks the status line otherwise, with
  stderr going to its debug log alone — read from the 2.1.258 bundle, not assumed. So a cache
  that cannot be written is said on stderr and in the log and is never an exit code; the non-zero
  exit is reserved for stdin that is not the document. The cache's temporary file is named per
  process, because unlike every other cache this tool writes, this one has a writer per open
  Claude Code session, and two sharing a name would race each other's rename.

  `--doctor` gains a `LIMITS` section naming where each of the three sources was looked for and,
  for the statusline cache, how many windows are live and when the payload arrived; with
  `[collectors.claude_code] enabled = false` it says "disabled" for the two Claude Code rows
  rather than "found" for a file the panel will never read. And the README screenshot renderer
  now requires `XDG_DATA_HOME` to name a scratch directory, because the statusline cache has no
  flag to pin it and would otherwise have put the author's own rate-limit window into every
  image's header — the fourth such leak, caught before it happened rather than after.

- **The first launch write-up.** `docs/what-a-max-subscription-bought.md`: nineteen days of the
  author's own Claude Code use at API-equivalent rates — by model, project and day — beside what
  this repository shipped in the same period, with the derived escalation rate and the two holes
  the method has (sessions with no local transcript; a day of the newest model unpriced). It
  says in its first paragraph why the routing panel's own measure, tests passed per dollar per
  model, is not in it: the hook was never installed on the measuring machine, and on a Max plan
  every attempt is `quota` by this tool's own rule. Linked from the README under *Write-ups*.

### Changed

- **The pricing snapshot is current again.** `pricing/litellm.tsv` was nine days old and did not
  know `claude-fable-5-1`, so every request to it was `quota` with no API-equivalent figure
  beside it — a gap that showed up as a hole in the first day's numbers the launch write-up is
  built from. Regenerated from upstream with `just pricing`: 3,757 keys, of which 347 are new, 219
  re-priced and 42 retired upstream.

- **Two green checks that meant nothing now mean something.** Neither is in the binary; both are
  in the release path a user's install depends on.

  **`update-taps` verifies what it pushed.** The job that keeps the Homebrew tap and the Scoop
  bucket current skipped with a warning when a clone failed, so an expired `TAP_TOKEN` let a
  release succeed while `brew upgrade` and `scoop update` went on serving the previous version
  indefinitely — the hazard `docs/release-process.md` had recorded and left. It still skips with a
  notice when the secret is *unset*, since that is the documented pre-setup state; an expired
  token now fails the clone and the job, and after the push each manifest is read back through
  the API (not the raw CDN, which caches for minutes) and must name the tag.

  **`claude-review` reviews every push.** Its skip rule was "a comment from me already exists",
  so the review ran once per pull request, ever: every push after the first was a green check over
  an unreviewed diff, and a fix made in response to a finding was the one change guaranteed never
  to be looked at. The prompt is now handed the pull request's head sha, each review comment opens
  with `Reviewed <sha>`, and the skip rule is "a comment of mine already names *this* sha" — so a
  push gets its own review and the same commit never gets two. The same sha feeds the permalinks,
  which used to come from `git rev-parse HEAD` on a checkout sitting on the merge commit.

- **A poll prices the rows it merged, not the whole history.** Every poll of every collector
  re-ran the pricing pass over every row ever collected, inside the write lock that `snapshot()`
  needs on the render thread — a walk that grew with the history and never changed a result,
  since the engine is immutable between reloads and a row is skipped once its status is anything
  but unavailable. `merge` now reports where its new rows begin and the poll prices from there.
  The refresh path is deliberately untouched: a pricing refresh still re-prices everything,
  because the rows collected *before* it are exactly the ones whose price was missing, and that
  pass is the one that reaches them. A test holds the two halves together, and the roadmap entry
  that recorded the constraint is closed under it.

### Fixed

- **Two documents that described work as open after it had landed.** `docs/routing-analytics.md`
  spelled the hook's `event_id` without the scope segment the code writes
  (`claude-code:<session_id>:<scope>:<tool_use_id>`); `docs/roadmap.md` still filed the quota P1
  as open, wrote the statusline route in the future tense, said a push to a pull request would
  not be re-reviewed, and recorded the expired-`TAP_TOKEN` hazard #88 removed. Reconciled against
  the tree.

## [0.13.0] - 2026-09-02

### Added

- **Subscription rate limits on every machine, not just Omarchy.** The `l` panel, the header's
  fullest-window line and `--json`'s `limits[]` were fed by exactly one source — Omarchy's agents
  panel — so everywhere else the panel was permanently empty. Claude Code caches its own
  subscription utilisation in `~/.claude.json` on every platform, and `src/limits.rs` now reads
  it: the 5-hour window, the weekly window, and any per-model weekly window, merged with Omarchy's
  records into the one report the panel and the export already speak. Nothing is fetched, no
  credential is read, and no configuration is required.

  **Both call sites now go through `limits::load`.** The dashboard and `--json` previously ran two
  independent reads of the same directory and could disagree about one run.

  **The panel's empty state was gating on the wrong thing.** It short-circuited on "Omarchy's
  directory is absent" *before* it considered the rows, so a second source's windows would have
  been unreachable on exactly the machines it was added for. It gates on having no rows now.

  **What is read, and the guarantee around it.** Only `cachedUsageUtilization.fetchedAtMs` and the
  entries of `cachedUsageUtilization.utilization.limits`; from each entry only `kind`, `percent`,
  `resets_at` and the scoped model's display name. The sibling per-window keys are an open,
  undocumented set and are never iterated — `limits` is indexed as the self-describing array it is
  — and `kind` is matched as a closed vocabulary, so an unrecognised value is dropped and counted
  rather than formatted into a label. A hand-authored fixture (no bytes from any real config)
  plants a credential and a placeholder account id, and two tests fail if anything from the
  document reaches the readout's `Debug` or the process's stdout.

  **The freshness rule is two-sided now, and that is load-bearing.** `omarchy::snapshot` scored
  staleness as `age > stale_after`, which is right for every age a correct reader produces and
  silently wrong for the one an incorrect reader produces: `fetchedAtMs` is *milliseconds* against
  a seconds clock, so a reader that forgets to divide computes an age near -1.79e12 — hugely
  negative, and therefore *fresh*. A unit bug would have presented as a permanently up-to-date
  panel showing an arbitrary moment's numbers. `limits::is_stale` rejects a stamp from the future
  beyond a 60-second skew tolerance, both readers share it, and the test was confirmed
  discriminating by restoring the one-sided form and watching it fail.

  **Merging is keyed on a normalised agent id** (`omarchy::record_id_for_agent`), not a raw
  string: Omarchy's agent id comes from its own record's `id` field, which this tool does not
  control, so comparing raw strings would file one subscription as two rows the day a record
  spells itself `claude_code`. Fresh beats stale; between two fresh readings the newer wins.

### Fixed

- **`hermetic()` did not pin Claude Code's config document.** `config_json_path` consults
  `CLAUDE_CONFIG_DIR` *before* deriving a path from `--claude-dir`, and falls back to
  `CLAUDE_PROJECTS_DIR`, so a developer with either exported had fixture-only CLI tests resolving
  their real `~/.claude.json`. That cost a tier label while billing detection was the only reader;
  with the limits reader consuming the same document it would have put real subscription
  percentages into a fixture-only `--json` run. Both variables are removed in `hermetic_with` now.
  This is the same lesson as the journal: hermetic has to mean every document a source reads.

- **An AUR package.** `packaging/aur/PKGBUILD` builds `ai-usage-tui-bin`, renders with the other
  manifests from the same `sed` loop and the same checksums, and attaches to each release, so
  `curl` the PKGBUILD and `makepkg -si` installs on Arch. It is a `-bin` package: it takes the
  published `x86_64-linux` or `aarch64-linux` tarball rather than compiling, and installs the
  binary, the man page, the licence, the README and all three shell completions where pacman
  expects each of them. Verified end to end against the published v0.12.1 tarball rather than
  reasoned about -- `makepkg -f` built it and the package contents were listed.

  **Rendering it found a bug the template would have shipped.** A PKGBUILD is bash, and the crate
  description contains the literal `$0.00`. Inside a double-quoted `pkgdesc` bash expanded that
  to the script's own path: `makepkg --printsrcinfo` produced "instead of rendering as
  /usr/bin/makepkg.00", and every Arch user would have read it. `pkgdesc` is single-quoted now,
  and because nothing escapes a single quote inside single quotes,
  `tests/docs.rs::crate_description_fits_every_registry` bans one in the description alongside
  the characters that would corrupt the `sed` substitution.

  The render step also asserts two well-formed `sha256sums_*` lines. An empty substitution
  renders as `sha256sums_x86_64=('')`, which the unrendered-placeholder grep cannot catch --
  it looks for tokens that are still there, not for values that went missing.

  Submitting it to the AUR stays manual: it needs an account with an SSH key, and a `.SRCINFO`
  derived by `makepkg --printsrcinfo`, which needs an Arch host while the release runner is
  Ubuntu. Rendering a second template from the same placeholders would be a hand-maintained copy
  of generated data. `docs/release-process.md` carries the commands.

### Fixed

- **The AUR package now follows Arch's packaging guidelines**, checked against `man PKGBUILD` and
  `/usr/share/pacman/PKGBUILD.proto` rather than from memory. Three things were wrong, and all
  three would have surfaced in an AUR review rather than in CI:

  **`pkgdesc` was the crate's full 302-character description.** The man page asks to "keep the
  description to one line of text and to not use the package's name"; that would have rendered as
  three lines in `pacman -Si` and in every AUR search result. It takes the clause before the em
  dash now -- 90 characters, no package name -- derived from the same single source of truth by
  one rule rather than becoming a sixth hand-written wording. `release.yml` spells the rule as one
  parameter expansion and refuses a result over 100 characters, and
  `tests/docs.rs::aur_pkgdesc_is_one_line_and_derived` pins the properties, so a description edit
  that breaks them fails the build instead.

  **`depends` was absent** while the binary dynamically links `libgcc_s`, `libc` and `libm`. Read
  off the shipped binary with `ldd` rather than assumed: `gcc-libs` and `glibc`, and nothing more
  -- rustls means no OpenSSL, and rusqlite is the bundled build, so there is no system sqlite
  link. An undeclared dependency is a namcap error.

  **The `# Maintainer:` comment was below the explanatory block.** The prototype puts it above
  `pkgname`; it is the one comment in that file with a required position, and the test asserts it
  is line 1.

  Verified by rebuilding against the published v0.12.1 tarball: the package's `.PKGINFO` now
  carries the one-line `pkgdesc` and both dependencies. **`namcap` 3.6.0 then ran against both the
  PKGBUILD and the built package and reported no errors.** Its three warnings are recorded in
  `docs/release-process.md` with the reason each stands.

  One of them is a trap worth naming here: namcap suggests replacing the literal `x86_64` in the
  source arrays with `$CARCH`, and **that would break the ARM package**. `$CARCH` is the build
  host's architecture, so inside `source_aarch64` it expands to whatever machine ran makepkg --
  substituting it and running `makepkg --printsrcinfo` on x86_64 pointed `source_aarch64` at the
  x86_64 tarball. `.SRCINFO` is generated once and pushed, so every aarch64 user would download
  the wrong file and fail its checksum. `tests/docs.rs` now asserts both source lines keep their
  literal architecture and contain no `$CARCH`, so the warning cannot be silenced by obeying it.

### Changed

- **`--doctor`'s system-package label names the AUR too.** All three of the `.deb`, the `.rpm`
  and the AUR package install to `/usr/bin`, and `detect_channel` is pure -- it reads the path
  and nothing else -- so it cannot tell them apart. The label reads
  "a system package (.deb/.rpm/AUR)", which is exactly what is known, and the upgrade line still
  falls through to the releases page rather than guessing at an AUR helper. Filed as P3 in
  `docs/roadmap.md`.

- **The pitch leads with the thing no competitor has.** The crate description, the README
  tagline and the README opening described "a btop-inspired terminal dashboard for token usage,
  cost and budgets" — which is precisely what ccusage (13.2k stars), claude-monitor (8.7k),
  CodexBar (20.8k) and Claude Code's own `/usage` already do, and it buried the routing
  analytics, which nothing below a gateway does at all. All three now lead with cost per
  delivered result, and the six sources are supporting detail rather than the headline. The
  provenance line moved up with it, because "no invented numbers" is a claim about this tool and
  not about its inputs. `tests/docs.rs` keeps the four copies from drifting apart.

### Documentation

- **The cost-provenance claim is restated at its true width, and dated.** `docs/roadmap.md` said
  "no competitor refuses to invent a number"; `claude-monitor` 4.0.0 now ships provenance labels
  (`official` / `local_estimate` / `experimental` / `unknown`), so that sentence had expired.
  What survives is narrower and still true: theirs label a *rate-limit* reading, this labels a
  *per-request cost*, and nothing in the field distinguishes "billed against a subscription
  quota" from "we could not price it" — while `ccusage --mode display` still prints `$0.00` for a
  row it cannot price, which is the behaviour the convention exists to refuse.

- **`docs/roadmap.md` gained a `Positioning` section** recording the field as surveyed
  2026-09-01, with the adoption numbers stated plainly, and two findings filed against it: **P1**,
  that quota and reset windows exist only on Omarchy while Claude Code hands official
  `rate_limits` to any statusline script on stdin — table stakes, missing everywhere except this
  developer's desktop; and **P2**, that there is no AUR package, no moving demo, and a name
  collision with an existing PyPI `aiusage`.

## [0.12.1] - 2026-09-02

### Fixed

- **The Copilot collector, validated against a real account, lost requests.** It shipped without
  a Copilot install to test against; driving Copilot CLI 1.0.82 non-interactively produced one.
  The schema probe held — an absent column degraded to `NULL` as designed and nothing produced a
  wrong number — but three assumptions did not.

  **A turn is not a request.** `event_id` was `copilot:{session}:{turn_index}`, and a tool-using
  prompt writes several rows sharing one turn: the capture has a `user` row and an `agent` row
  both at `turn_index` 0, and the shipping build writes `0` on every row of every session. Against
  the real store the collector reported **2 rows where Copilot recorded 3**, discarding the extra
  rows' tokens with them. Identity is now `assistant_usage_events.id`, the table's own
  autoincrement key; a build without it falls back to the session, turn, timestamp and counts,
  which is still content-derived. This is the Gemini `prompt_id` defect in a different spelling.

  **`cwd` and `repository` are on `sessions`, not on the usage table.** The probe found neither,
  selected both as `NULL`, and every Copilot row came out with no project. The select list now
  looks on `sessions` as well and joins when it must, still preferring the usage table so a build
  that moves them onto it needs no join.

  **`created_at` is declared `TEXT` and written as RFC 3339**, while the incremental read bound an
  `i64`. SQLite orders every integer before every string, so `created_at >= <integer>` was always
  true and the cursor filtered nothing — every poll re-read the whole table. The cursor now also
  keeps the store's own spelling and compares text with text.

- **Four more hermeticity gaps, found by having real Copilot data for the first time.** The
  capture is the first `~/.copilot` store this repository has ever seen, and it immediately turned
  three tests red and put real session data through a fourth. `journal_rows` had its own
  hand-written list of collector dirs — Claude Code, Codex and Omarchy, and nothing else — so a
  developer with a Copilot store or Gemini telemetry switched on saw their own rows in a
  journal-only assertion; it now goes through `hermetic_with` like everything else.
  `derived_escalations_are_exported`, `escalations_follow_the_same_filter_as_the_rows`,
  `claude_billing_decides_whether_transcript_rows_carry_dollars` and the Codex export test pin
  the missing dirs directly. This is the same drift v0.12.0 fixed for `--gemini-dir`: a source
  joins the registry and the hand-written lists do not hear about it.

  `tests/fixtures/copilot_home/session-store.db` is the redacted capture, and an integration test
  reads it end to end. What the capture *confirmed* is in `docs/roadmap.md`: `session-store.db` is
  the real filename, the inclusive-token convention holds in the bytes (so the v0.12.0 cache
  double-count fix was right), and `requests.cost` is a premium-request count rather than dollars
  — so `cost: null` with `cost_status: quota` stays, and nothing here reaches a budget.

- **A subagent's test run was charged to its parent.** The roadmap recorded this as unverified;
  driving a real Claude Code session that delegates `make test` to a `general-purpose` subagent
  settled it. Claude Code hands a subagent's hook the **parent's** `transcript_path` and the
  parent's `session_id`, while the subagent's own turns go to
  `<project>/<session_id>/subagents/agent-<agent_id>.jsonl`, every line marked
  `isSidechain: true`. The attempt was therefore read from the parent's transcript: one measured
  run recorded **3 requests and 65,598 tokens** for a `make test` whose agent had spent 2 requests
  and about 318, and the model recorded was the parent's — so an Opus parent would have priced a
  Haiku subagent's attempt at Opus.

  `--claude-code-hook` now prefers the nested transcript when the payload carries an `agent_id`
  (which had been parsed by a test and read by nothing), and keys the journal cursor on the agent
  as well as the session, so a subagent's attempts and its parent's stop sharing one counter. A
  payload without an `agent_id`, or a build that lays subagents out differently, falls back to the
  path the payload names.

  `event_id` gains a scope segment for this: `claude-code:{session}:{scope}:{tool_use_id}`, where
  scope is `main` or `agent-<agent_id>`. Both halves have to be built from the same place — the
  cursor is matched as a literal prefix of `event_id` in SQL, so a prefix that names the agent
  while the id does not matches nothing, and every run re-attributes from the start of the
  transcript. `main` and `agent-<id>` are also chosen so neither is a prefix of the other, which a
  bare `{session}:` prefix was: it would have counted every subagent's requests against the
  parent's cursor. Two tests now pin the coupling, because nothing else does.

  The usage collector was never affected: its walk is recursive, so it already read
  `subagents/*.jsonl`, and those requests carry their own `requestId`s — subagent tokens were
  counted exactly once in the dashboard throughout.

### Changed

- **The pull-request review workflow reviews in-process.** It had never posted a review, for three
  independent reasons, each of which produced a green check. `--allowedTools` replaces the default
  allowlist rather than extending it, so naming only the inline-comment tool denied `Skill` — the
  tool a slash-command prompt is invoked through. With that fixed the review started and still
  posted nothing: the plugin fans out to background agents, and the parent ended its turn at six
  of an uncapped budget waiting for a completion notification a `-p` invocation never delivers.
  And the plugin posts one summary comment with `gh pr comment`, never an inline comment, so the
  action's buffered-comment step was waiting on something nothing produced. The workflow now
  carries a direct prompt that does the review itself, an allowlist naming exactly the tools that
  prompt uses, and posts a "No issues found" comment rather than staying silent — silence is
  indistinguishable from a review that never ran. Also recorded, because it hides all of the
  above: `claude-code-action` skips entirely when the workflow file differs from the default
  branch, and reports success while doing it, so a workflow change can never be tested on the pull
  request that makes it.

## [0.12.0] - 2026-09-01

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

## [0.11.0] - 2026-08-25

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

## [0.10.0] - 2026-08-25

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

## [0.9.0] - 2026-08-25

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


## [0.8.0] - 2026-08-24

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

## [0.7.0] - 2026-08-24

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

## [0.6.0] - 2026-08-24

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

## [0.5.0] - 2026-08-23

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

## [0.4.1] - 2026-08-20

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

## [0.4.0] - 2026-08-20

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

## [0.3.0] - 2026-08-19

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

## [0.2.0] - 2026-07-24

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

## [0.1.0] - 2026-07-24

- Initial btop-inspired OpenCode usage dashboard.