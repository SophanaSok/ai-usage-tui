# What a Max subscription bought: one machine, nineteen days

*Written 2026-09-18 from the author's own machine, with `ai-usage-tui` 1.0.0 (commit `eb264e8`)
and the pricing table bundled with it, dated 2026-09-17. Every figure below comes out of
`ai-usage-tui --json --all`, `ai-usage-tui --summary-json --all` and `git`; the commands are at
the end so the numbers can be re-derived, and they will drift as the transcripts grow — and,
as it turns out, as they are deleted.*

This is the second edition. The first was written on 2026-09-02, at v0.13.0, over 2026-08-15 to
09-02. Its tables cannot be re-derived any more, which is one of the findings below, so this
edition is measured afresh over the window the machine still holds rather than patched. The
first edition is in this file's `git` history.

## The question this still does not answer

The roadmap asked for one measurement: *was Opus worth five times Sonnet on this codebase?* —
dollars per passing test, per model, from the routing journal. That is the panel the tool was
built around. The first edition could not fill it in because the
[`--claude-code-hook`](routing-analytics.md) that records a test run's pass or fail had never
been installed on the author's own machine. It is installed now — `ai-usage-tui --install-hook`
is one command, and `--doctor` reports `hook installed` — and the honest state of the journal is
three events: one passing run under Opus 5, one under Fable 5.1, and one hand-recorded in July.
Three events are not a measurement. Why there are so few turned out to have an answer, found the
day this was written; it is under *The holes*.

The second reason has not changed and will not. On a Max subscription there are no dollars per
request. Every Claude Code request here is billed against a quota, and the tool's own rule
([README, *What it shows*](../README.md#what-it-shows)) is that such a row is `quota`, never
`$0.00` and never a number invented from a list price. Both hook events land in a
routing aggregate whose `cost_basis` is `"quota"`, so `$/SUCCESS` on this machine reads `on quota` by design.

What the transcripts *do* support is the other half of the picture: what the subscription was
used for, at what API-equivalent rate, on which projects, what came out the other end of the
repository this tool lives in — and, new in this edition, what running the tool for a month
made visible that nothing else on the machine was showing.

## Method, and one word about the money

`ai-usage-tui --json --all` reads every source it knows and prints one row per request. On this
machine `--doctor` says what that means: Claude Code's transcripts under `~/.claude/projects`
(362 sessions), OpenCode's message store, GitHub Copilot's session store (6 rows), the local
Ollama and llama.cpp journals, and Omarchy's agents-panel records for limits. Codex CLI is
installed and has written no session logs; Gemini CLI records nothing until its telemetry is
switched on, and `--doctor` prints the setting that does it; Cursor is detected and deliberately
not read, because it keeps no reliable local token counts.

The tool itself is not the one the first edition used. Between v0.13.0 and v1.0.0 it gained
`--install-hook`, the compact `--summary-json` that the limits and routing figures below come
from, `--prune-journal`, and a written promise ([`stability.md`](stability.md)) that the flags
and JSON keys used at the end of this piece will not break before a 2.0.

The Claude rows carry `cost_status: "quota"` and a field called `api_equivalent_cost`: what the
same request would have cost at Anthropic's published API list rate — input, output, cache
write and cache read priced separately. It is kept beside the row and never summed into cost or
budgets, because it is a counterfactual, not spend. Everything below that is denominated in
dollars is that counterfactual unless it says otherwise. The subscription's actual price is on
[Anthropic's pricing page](https://claude.com/pricing) — Max plans "from $100 per month"; this
machine is on the 20x tier, the dearer of the two.

Two things this method cannot see. It cannot see sessions that ran anywhere but this machine:
Claude Code's web and cloud sessions leave no local transcript. And it prices cache reads at
the list rate, which is the honest thing to do and also the thing that makes the totals large:
an agentic session re-reads its context on every turn.

## What was used

Nineteen days, 2026-08-31 to 2026-09-18, fifteen of them with any Claude traffic: 16,870
requests in 109 sessions across 72 working directories.

| Model | Requests | Sessions | Output tokens | Cache reads | API-equivalent |
|---|---:|---:|---:|---:|---:|
| Claude Opus 5 | 11,780 | 75 | 8.3M | 3.12B | $1,985.69 |
| Claude Fable 5.1 | 4,112 | 45 | 4.3M | 945M | $698.37 |
| Claude Sonnet 5 | 535 | 10 | 193K | 199M | $70.86 |
| Claude Fable 5 (to 09-02) | 196 | 2 | 162K | 30M | $45.57 |
| Claude Haiku 4.5 | 247 | 16 | 34K | 9.5M | $1.90 |
| **All Claude** | **16,870** | | | | **$2,802.39** |

**Cache reads are still the bill.** Of Opus 5's $1,986, about $1,562 is cache reads — 3.12
billion tokens at $0.50 per million — against $207 of output and $217 of cache writes. The
model wrote 8.3 million tokens and re-read 378 times that. Across every source the export's
cache-hit figure is 91.2% and output is 0.33% of all tokens.

**Sonnet was used seven times as much and it is still not a routing policy.** 535 requests in
ten sessions, $71, against 216 requests and $10 in the first edition. It is 2.5% of the total.
The question the roadmap asked presupposes a period in which the cheaper model did a real share
of the work, and this was not one.

**The work moved to the newest model.** Fable 5.1 arrived on 09-02. Over the last five days it
is $401 of $956; on 09-17 and 09-18 it is the larger share.

### By project

The top ten of 72 working directories; the other sixty-two — config directories, scratch
checkouts, one-request sessions in `$HOME` — add up to $274.03.

| Project | Requests | Sessions | API-equivalent | Opus | Fable | Sonnet |
|---|---:|---:|---:|---:|---:|---:|
| `Projects/ai-usage-tui` | 4,161 | 21 | $565.28 | $240 | $325 | $0 |
| `Projects/proof` | 2,431 | 15 | $555.62 | $462 | $70 | $24 |
| `Projects/json-data-drift-analyzer` | 2,298 | 5 | $482.32 | $457 | $25 | $0 |
| `Projects/md-viewer` | 2,078 | 9 | $289.77 | $213 | $75 | $1.23 |
| `Projects/games/algebraic` | 1,891 | 6 | $263.81 | $254 | $0 | $8.04 |
| `Projects/multiverse` | 834 | 8 | $156.24 | $94 | $62 | $0 |
| `Projects/games/brick-breaker` | 498 | 2 | $73.59 | $58 | $14 | $2.18 |
| `Projects/marquee-site` | 516 | 5 | $54.33 | $41 | $14 | $0 |
| `Projects/proof/apps/mobile` | 178 | 4 | $50.00 | $16 | $1.57 | $32.83 |
| `Projects/job-posting-bot` | 194 | 2 | $37.40 | $1.03 | $36 | $0 |

A project is the working directory a session was started in, exactly as the transcript spells
it, so `Projects/proof` and `Projects/proof/apps/mobile` are two rows. The tool does not guess
that they are one repository.

### By day

| Day | Requests | Sessions | API-equivalent | of which Opus | of which Fable |
|---|---:|---:|---:|---:|---:|
| 08-31 | 1,508 | 5 | $206 | $203 | $0 |
| 09-01 | 1,346 | 12 | $155 | $147 | $0 |
| 09-02 | 3,139 | 17 | $422 | $241 | $181 |
| 09-03 | 1,242 | 14 | $189 | $67 | $121 |
| 09-04 | 603 | 4 | $95 | $60 | $33 |
| 09-08 | 1,095 | 3 | $164 | $164 | $0 |
| 09-09 | 1,109 | 3 | $289 | $289 | $0 |
| 09-10 | 900 | 1 | $224 | $224 | $0 |
| 09-11 | 145 | 1 | $13 | $13 | $0 |
| 09-13 | 279 | 1 | $89 | $82 | $7 |
| 09-14 | 1,782 | 18 | $370 | $217 | $96 |
| 09-15 | 557 | 5 | $100 | $77 | $23 |
| 09-16 | 832 | 7 | $137 | $80 | $57 |
| 09-17 | 1,469 | 19 | $244 | $97 | $146 |
| 09-18 | 864 | 8 | $105 | $26 | $79 |

09-18 is a part day: the export was taken mid-afternoon, in a session that is itself in the
table. The heaviest day, 09-02, is 15% of the range. 09-10 is one session and $224. The four
missing days — 09-05 to 09-07 and 09-12 — are a weekend, the Monday after it and a Saturday,
and the repository has no commits on them either, so unlike the first edition's gaps they look like
days off rather than work the machine cannot see.

### Escalations

The tool derives one more thing from the transcripts without any setup: sessions that reached
for a pricier model than they opened with. Over the whole range, 8 of 109 examined sessions did
(7.3%) — seven went from Opus 5 to Fable 5.1, one from Sonnet 5 to Opus 5 — and every request
after the switch is `on quota`, because there is no per-request price to attach. Over the last
seven days it was 7 of 55 (12.7%): all but one of the escalations happened in the last week.

## What came out: this repository

`ai-usage-tui` is the one project in the table whose output is public and countable, so it is
the one place the two sides can be put next to each other. It cost $565.28 of API-equivalent
across twenty-one sessions: Fable 5.1 for $315.77 (2,199 requests, 13 sessions), Opus 5 for
$240.25 (1,883 requests, 14 sessions), Fable 5 for $9.17 and Haiku 4.5 for nine cents.

Over the same nineteen days the repository gained 129 non-merge commits, 70 merged pull
requests, and eleven tagged releases (v0.12.0 through v1.0.0). 126 of the 129 commits carry a
model in their co-author trailer: 77 name Fable 5.1, 47 name Opus 5, 2 name Fable 5.

That is a ratio a reader can form an opinion about — $4.38 of list-rate compute per commit,
$8.08 per merged pull request — with the caveat that it is a delivery count, not a
delivery *quality* measure, and that the model per commit is who wrote the trailer, not who
did the thinking. The measure the tool was built to give, tests passed per dollar per model,
is not in this table because the journal that feeds it holds three events (see *The holes* for
why).

The days line up better than they did. The repository's spend falls on six days — 09-01 to
09-03 ($19, $192, $15) and 09-16 to 09-18 ($18, $234, $87) — and 126 of the 129 commits fall
on the same six. The other three, on 09-09 and 09-15, are a dependency bump with no model
trailer and two commits with no local transcript for this repository on that day.

## What running it showed

The first edition was about the subscription. A month on, the more useful list is what the
author knows about their own machine only because this was running on it. Each of these is a
figure from the export, not an impression.

**Real spend and the counterfactual are different numbers, and they are never added.** The
export's provenance block puts $25.47 of calculated cost — 683 requests through OpenCode to
metered providers — beside 20,030 quota requests, 1,999 free requests and 1,031 local ones, each
under its own status. The $2,802.39 of API-equivalent sits on 16,870 of the quota requests, the
Claude Code ones; the other 3,160, through Ollama's cloud models and Copilot, are `quota` with no
API-equivalent figure, and are not given one. A dashboard that summed them would
report a $2,828 month on a machine that paid a subscription and $25. One that rendered the
quota rows as `$0.00` would report that Opus is free.

**How close the limit is, before reaching it.** The Limits panel reads the windows Claude Code
already caches in `~/.claude.json`: at the time of writing, 4% of the five-hour session window,
51% of the weekly window and 76% of the Fable weekly window, each with the clock time it
resets. The by-day table says why the third number is the high one — the last five days are
where Fable 5.1 took over — and that is a thing to know on a Friday afternoon before starting a
long session, not after the refusal.

**Where the tokens go.** Output is a third of one percent of the tokens on this machine. 91.2%
are cache reads. Anyone reasoning about what agentic coding costs from a per-token output price
is reasoning about the wrong column, and the by-model table makes that visible per model: Opus
5 re-read 378 tokens for every one it wrote.

**Transcripts expire, and the tool can only read what is on disk.** The first edition counted
835 sessions. There are 362 now, and nothing before 08-31. `~/.claude/settings.json` on this
machine has `cleanupPeriodDays: 20`, so Claude Code deletes its own transcripts after twenty
days, and `--all` means "all that is left". That is why this edition covers nineteen days again
rather than thirty-five, and why the first edition's $4,081.49 cannot be checked by anyone,
its author included. The tool does not copy transcripts into its own journal — they hold source
code and secrets, and only the usage block is ever parsed — so the remedy is a longer
`cleanupPeriodDays` or a scheduled `--summary-json`, and the point is that this was invisible
until two exports a fortnight apart disagreed.

**An aged price table looks like a gap, not a zero.** This one is from the first edition and
is kept because it is the rule working: on the day Fable 5.1 arrived, the bundled table did not
know it, and 994 requests sat as `quota` with no API-equivalent figure until the snapshot was
refreshed. Today the export reports `unpriced_requests: 0`, and `--doctor` prints the size and
the date of the table it is using: 4,370 model entries, dated 2026-09-17.

**What is not being measured.** `--doctor` on this machine lists Codex with zero rows and
billing unknown, Gemini CLI as recording nothing until its telemetry is on, Cursor as installed
and not read, and the journal as readable by other accounts (`mode 644`). None of those is a
number, and each is a thing a blank panel would otherwise have left to guesswork — "no usage"
and "not looked at" are different answers.

**Escalation, with no setup.** One session in fourteen reached for a pricier model part-way
through, nearly all of them in the last week and nearly all Opus 5 to Fable 5.1. On a metered
plan that is the line item to look at first; here it is the explanation for the 76%.

## The holes

**Cloud sessions are still invisible.** The first edition found 31 commits on two days with no
transcript anywhere on the machine. This window has nothing that stark — two commits on 09-15
without a local transcript for this repository — but the method has not changed: work done in
Claude Code on the web or on another machine is in no figure above, and the per-commit ratio is
generous by whatever that was.

**Three routing events — and why.** The hook records a test run Claude Code executes through its
Bash tool, and two such events beside 129 commits is far fewer than the work. This piece first
went out calling that an open question. It is answered: the hook was working, and it recorded
every test run its rules allowed it to.

The rule is an honest one. A hook sees the exit status of the whole command line, not of the test
runner inside it, so `cargo test 2>&1 | tail -20` exits with `tail`'s status and a red run would
read as green; the tool records a result only when the status is the runner's own. And every test
command on this machine is trimmed — `cargo test --all-targets --locked 2>&1 | grep -E "^test
result|FAILED" | head -40` — so that the output fits in a tool result. Replaying every Bash call
the local transcripts hold through v1.0.1's hook: **1,074 command lines ran a test runner, and
ten had a status that was the runner's own.** Eight of those ten ran in scratch sessions started
without the user's hooks on purpose, to capture fixtures. The other two are the two events. The
hook had been installed since 09-02, saw a thousand test runs, could honestly speak for two, and
said nothing at all about the rest — which is this tool's own failure mode, *broken rendered as
nothing to report*, in the one place it had not been looked for.

The capture that settled what to do about it: a **failing** `cargo test 2>&1 | grep -E "^test
result|FAILED"` fires Claude Code's *success* hook, because `grep` found its lines. Loosening the
rule would have recorded failures as passes. What the payload does carry is the runner's own
summary line, in its output. The next release reads that line where the status cannot speak —
for the four runners whose real output was captured, believing a failure always and a pass only
when the end of the output is there — and counts, by reason, every run it still cannot record.
Replayed over the same 1,074 lines it records **396** (272 passes, 124 failures) where v1.0.1
recorded 10, and shows the other 678 in `--doctor` instead of nowhere: 331 on a line with `$(…)`
or a heredoc, 197 followed by `;`, 147 piped with no summary left to read. The mechanism and the
rules are in [`routing-analytics.md`](routing-analytics.md).

**Cache reads at list.** The tool uses the cache-read rate in the bundled table for each model,
but a subscription is not a metered API and the reader should not mistake the API-equivalent
for a bill avoided. It is the cost of the same requests made a different way.

**One machine, one person.** Every ratio here is an anecdote with a method attached.

## What comes next

A period in which Sonnet does a real share of the work, with a hook that can now see the test
runs it was blind to. The piece the roadmap asked for — Opus against Sonnet, per passing test —
needs both, and the second only stopped being the blocker the day this was written. Until then
this is what could be said honestly, and at twenty days' retention the last of the transcripts it
was said from will be gone by 2026-10-08.

## Reproducing the figures

Every table above is a `jq` expression over the export. The export is large — twenty-four
thousand rows here — so write it once. The Claude Code rows are the `anthropic` rows with
`cost_status == "quota"`; two further `anthropic` rows here went through OpenCode on a metered
key and are `calculated`. Days in the usage tables are UTC, as `todate` prints them; `git`'s
are the committer's local date.

```sh
ai-usage-tui --json --all > usage.json
ai-usage-tui --summary-json --all > summary.json

# By model (Claude Code rows only): requests, sessions, tokens, API-equivalent.
jq -r 'def s(f): (map(f)|add)//0;
  .usage | map(select(.provider=="anthropic" and .cost_status=="quota")) | group_by(.model)
  | map({m: .[0].model, req: length, sessions: (map(.session_id)|unique|length),
         out: s(.output_tokens), cache_read: s(.cache_read_tokens),
         cache_write: s(.cache_write_tokens), api: s(.api_equivalent_cost//0)})
  | sort_by(-.api)[]
  | "\(.m)\t\(.req)\t\(.sessions)\t\(.out)\t\(.cache_read)\t\(.cache_write)\t\(.api)"' usage.json

# By project, with the model split.
jq -r 'def s(f): (map(f)|add)//0;
  .usage | map(select(.provider=="anthropic" and .cost_status=="quota")) | group_by(.project)
  | map({p: (.[0].project // "(none)"), req: length,
         sessions: (map(.session_id)|unique|length), api: s(.api_equivalent_cost//0),
         opus: s(select(.model|startswith("claude-opus"))|.api_equivalent_cost//0),
         fable: s(select(.model|startswith("claude-fable"))|.api_equivalent_cost//0),
         sonnet: s(select(.model|startswith("claude-sonnet"))|.api_equivalent_cost//0)})
  | sort_by(-.api)[]
  | "\(.p)\t\(.req)\t\(.sessions)\t\(.api)\t\(.opus)\t\(.fable)\t\(.sonnet)"' usage.json

# By day.
jq -r 'def s(f): (map(f)|add)//0;
  .usage | map(select(.provider=="anthropic" and .cost_status=="quota"))
  | group_by(.created|todate|.[:10])[]
  | "\(.[0].created|todate|.[:10])\t\(length)\t\(map(.session_id)|unique|length)\t\(s(.api_equivalent_cost//0)|round)"' usage.json

# Provenance totals and the derived escalations, as the export prints them.
jq '.provenance, .escalations' usage.json

# Limits, routing and the token mix, from the summary.
jq '.limits, .routing, .totals.metrics' summary.json

# What each source resolved to, the hook, and the date of the price table.
ai-usage-tui --doctor

# The repository's side, from git.
git log --since=2026-08-31T00:00 --no-merges --format='%ad' --date=short | sort | uniq -c
git log --since=2026-08-31T00:00 --no-merges --format=%B | grep -o 'Co-Authored-By: [^<]*' | sort | uniq -c
gh pr list --state merged --limit 300 --json mergedAt -q '[.[] | select(.mergedAt >= "2026-08-31")] | length'
```

The cache-read arithmetic uses the rates in `pricing/litellm.tsv` on the day of writing: Opus 5
at $5.00 input, $25.00 output, $6.25 cache write and $0.50 cache read per million tokens.
