# What a Max subscription bought: one machine, nineteen days

*Written 2026-09-02 from the author's own machine, with `ai-usage-tui` at commit `6f97237`
(v0.13.0 plus the unreleased changes) and the pricing snapshot refreshed from LiteLLM the same
day. Every figure below comes out of `ai-usage-tui --json --all` and `git`; the commands are at
the end so the numbers can be re-derived, and they will drift as the transcripts grow.*

## The question this does not answer yet

The roadmap asked for one measurement: *was Opus worth five times Sonnet on this codebase?* —
dollars per passing test, per model, from the routing journal. That is the panel the tool was
built around, and it cannot be filled in from this machine today, for two reasons that are
worth stating before any number.

First, the routing journal here holds one event, hand-recorded in July. The
[`--claude-code-hook`](routing-analytics.md) that records a test run's pass or fail was never
installed on this machine — the author built it and did not eat it. Second, on a Max
subscription there are no dollars per request. Every Claude Code request here is billed
against a quota, and the tool's own rule ([README, *What it shows*](../README.md#what-it-shows))
is that such a row is `quota`, never `$0.00` and never a number invented from a list price. So
even with the hook installed, `$/SUCCESS` on this machine reads `on quota` by design.

What the transcripts *do* support is the other half of the picture: what the subscription was
used for, at what API-equivalent rate, on which projects, and what came out the other end of
the repository this tool lives in. That is this piece. The Opus-versus-Sonnet piece waits on
hook data, which starts accumulating the day the hook is installed.

## Method, and one word about the money

`ai-usage-tui --json --all` reads every Claude Code transcript under `~/.claude/projects`
(835 sessions here), OpenCode's message store, and the local Ollama and llama.cpp journals,
and prints one row per request. The Claude rows carry `cost_status: "quota"` and a field
called `api_equivalent_cost`: what the same request would have cost at Anthropic's published
API list rate — input, output, cache write and cache read priced separately. It is kept beside
the row and never summed into cost or budgets, because it is a counterfactual, not spend.
Everything below that is denominated in dollars is that counterfactual, and it is labelled as
such. The subscription's actual price is on
[Anthropic's pricing page](https://claude.com/pricing) — Max plans "from $100 per month"; this
machine is on the 20x tier, the dearer of the two.

Two things this method cannot see. It cannot see sessions that ran anywhere but this machine:
Claude Code's web and cloud sessions leave no local transcript. And it prices cache reads at
the list rate, which is the honest thing to do and also the thing that makes the totals large:
an agentic session re-reads its context on every turn.

## What was used

Nineteen days, 2026-08-15 to 2026-09-02, fifteen of them with any Claude traffic.

| Model | Requests | Sessions | Output tokens | Cache reads | API-equivalent |
|---|---:|---:|---:|---:|---:|
| Claude Opus 5 | 18,889 | 78 | 11.5M | 4.56B | $2,944.58 |
| Claude Fable 5 | 3,532 | 38 | 2.6M | 580M | $1,007.12 |
| Claude Fable 5.1 (from 09-02) | 994 | 6 | 383K | 84M | $112.00 |
| Claude Sonnet 5 | 216 | 5 | 28K | 21M | $10.07 |
| Claude Opus 4.8 | 10 | 3 | 12K | 1.8M | $5.91 |
| Claude Haiku 4.5 | 249 | 13 | 8K | 9.7M | $1.81 |
| **All Claude** | **23,890** | | | | **$4,081.49** |

Three things stand out.

**Cache reads are the bill.** Of Opus 5's $2,945, roughly $2,280 is cache reads — 4.56 billion
tokens at $0.50 per million — against about $290 of output and $380 of cache writes. The
model wrote 11 million tokens and re-read 400 times that. That is what a long agentic session
looks like at list price, and it is the number a per-token budget would have to reckon with.

**Sonnet was barely used.** 216 requests in five sessions, $10. There is no Opus-versus-Sonnet
comparison to make on this machine because there was no Sonnet period; the question the
roadmap asked presupposes a routing policy that was not in force here.

**The newest model was unpriced for a day.** Claude Fable 5.1 arrived on 09-02 and the bundled
pricing table, nine days old, did not know it: 994 requests sat as `quota` with no
API-equivalent figure until the snapshot was refreshed. The tool showed the gap rather than a
zero — which is the point of the provenance rule — but a reader of the day's numbers would
have seen $3,970 where $4,081 was true.

### By project

Every project the transcripts name, folded to the top ten; the remaining forty-odd paths —
config directories, scratch checkouts, one-request sessions in `$HOME` — add up to $178.73.

| Project | Requests | Sessions | API-equivalent | Opus | Fable | Sonnet |
|---|---:|---:|---:|---:|---:|---:|
| `Projects/games/oneplusone` | 8,233 | 10 | $1,529.36 | $1,390 | $139 | $0 |
| `Projects/md-viewer` | 4,819 | 16 | $776.14 | $602 | $173 | $1.23 |
| `Projects/ai-usage-tui` | 4,665 | 14 | $733.90 | $413 | $321 | $0 |
| `Projects/games/algebraic` | 1,891 | 6 | $263.08 | $254 | $0 | $7.88 |
| `Projects/json-data-drift-analyzer` | 671 | 3 | $210.08 | $10 | $201 | $0 |
| `~` (home directory) | 958 | 28 | $154.02 | $50 | $104 | $0 |
| `models` | 666 | 10 | $120.09 | $63 | $57 | $0 |
| `Projects/games/wildkin` | 287 | 1 | $44.67 | $45 | $0 | $0 |
| `Projects/kickstart.nvim` | 256 | 4 | $41.98 | $27 | $15 | $0 |
| `Projects/marquee-site` | 231 | 2 | $29.44 | $23 | $7 | $0 |

### By day

| Day | Requests | Sessions | API-equivalent | of which Opus |
|---|---:|---:|---:|---:|
| 08-15 | 48 | 1 | $4 | $2 |
| 08-16 | 751 | 3 | $229 | $5 |
| 08-17 | 378 | 6 | $50 | $19 |
| 08-18 | 222 | 3 | $36 | $13 |
| 08-19 | 2,035 | 5 | $534 | $446 |
| 08-20 | 1,725 | 11 | $213 | $181 |
| 08-21 | 978 | 8 | $143 | $99 |
| 08-22 | 241 | 2 | $26 | $26 |
| 08-25 | 2,551 | 17 | $410 | $97 |
| 08-26 | 1,216 | 13 | $261 | $110 |
| 08-27 | 2,590 | 6 | $516 | $516 |
| 08-28 | 5,431 | 6 | $915 | $864 |
| 08-31 | 1,508 | 5 | $206 | $203 |
| 09-01 | 1,346 | 12 | $155 | $147 |
| 09-02 | 2,873 | 16 | $382 | $224 |

The heaviest day, 08-28, was six sessions and $915 of API-equivalent — more than a fifth of
the whole range in one day, and fifty times everything Sonnet has ever cost here. 08-23, 08-24,
08-29 and 08-30 are missing from the table because no transcript on this machine carries a
request on those days; see *The holes* below for why that matters.

### Escalations

The tool derives one more thing from the transcripts without any setup: sessions that reached
for a pricier model than they opened with. Over the whole range, 11 of 102 examined sessions
did (10.8%) — ten went from Opus 5 to Fable 5, one from Sonnet 5 to Opus 5 — and every one
of them is `on quota after`, because there is no per-request price to attach. Over the last
seven days it was 2 of 42 (4.8%).

## What came out: this repository

`ai-usage-tui` is the one project in the table whose output is public and countable, so it is
the one place the two sides can be put next to each other. It cost $733.90 of API-equivalent
across fourteen sessions: Opus 5 for $412.83 (2,599 requests, 12 sessions), Fable 5 for
$247.57 (1,303 requests, 6 sessions), Fable 5.1 for $73.50 on the last day.

Over the same nineteen days the repository gained 130 non-merge commits, 80 merged pull
requests, and thirteen tagged releases (v0.3.0 through v0.13.0). 125 of the 130 commits carry a
model in their co-author trailer: 93 name Opus 5, 26 name Fable 5, 6 name Fable 5.1.

That is a ratio a reader can form an opinion about — $5.65 of list-rate compute per commit,
$9.17 per merged pull request — with the caveat that it is a delivery count, not a
delivery *quality* measure, and that the model per commit is who wrote the trailer, not who
did the thinking. The measure the tool was built to give, tests passed per dollar per model,
is not in this table because no one on this machine had turned it on.

The days line up, mostly. The repository's spend concentrates on 08-19 ($245, the first big
session), 08-25 ($278) and 09-02 ($173); the commit history concentrates on 08-19 (30 commits),
08-24 (24), 08-25 (18) and 09-01 (33). One of those four days has no local transcript at all.

## The holes

**Thirty-one commits with no transcript.** 08-23 and 08-24 carry 31 commits, 18 merged pull
requests and four release tags, and there is not a single Claude Code transcript on this
machine dated either day — not for this repository, not for any. Seven of those commits carry
a Claude Code session link in their trailer. The likeliest reading is that the work happened in
sessions whose transcripts live elsewhere (Claude Code on the web, or another machine); the
tool reports what the local files hold and has no way to know what they do not. Whatever the
subscription bought on those two days is not in any figure above, and the per-commit ratio is
correspondingly generous.

**One day of the newest model at no price.** Described above. The fix was a refresh of the
pricing snapshot; the lesson is that a bundled table ages, and a `quota` row with no
API-equivalent figure is what an aged table looks like from the outside.

**Cache reads at list.** Anthropic prices cache reads at a tenth of input on these models and
the tool uses exactly that rate, but a subscription is not a metered API and the reader should
not mistake the API-equivalent for a bill avoided. It is the cost of the same requests made a
different way.

## What comes next

The hook. [`contrib/claude-code/README.md`](../contrib/claude-code/README.md) is one merged
settings block; from then on every test run Claude Code executes lands in the routing journal
as a pass or a fail with the model that ran it, and the routing panel begins to fill. The piece
the roadmap asked for — Opus against Sonnet, per passing test — needs that journal and a
period in which Sonnet actually did some of the work. Neither exists here yet. This piece is
what could be said honestly in the meantime.

## Reproducing the figures

Every table above is a `jq` expression over the export. The export is large — thirty thousand
rows here — so write it once.

```sh
ai-usage-tui --json --all > usage.json

# By model (Claude rows only): requests, sessions, tokens, API-equivalent.
jq -r 'def s(f): (map(f)|add)//0;
  .usage | map(select(.provider=="anthropic")) | group_by(.model)
  | map({m: .[0].model, req: length, sessions: (map(.session_id)|unique|length),
         out: s(.output_tokens), cache_read: s(.cache_read_tokens),
         api: s(.api_equivalent_cost//0)})
  | sort_by(-.api)[] | "\(.m)\t\(.req)\t\(.sessions)\t\(.out)\t\(.cache_read)\t\(.api)"' usage.json

# By project, with the model split.
jq -r 'def s(f): (map(f)|add)//0;
  .usage | map(select(.provider=="anthropic")) | group_by(.project)
  | map({p: (.[0].project // "(none)"), req: length, api: s(.api_equivalent_cost//0),
         opus: s(select(.model|startswith("claude-opus"))|.api_equivalent_cost//0),
         fable: s(select(.model|startswith("claude-fable"))|.api_equivalent_cost//0),
         sonnet: s(select(.model|startswith("claude-sonnet"))|.api_equivalent_cost//0)})
  | sort_by(-.api)[] | "\(.p)\t\(.req)\t\(.api)\t\(.opus)\t\(.fable)\t\(.sonnet)"' usage.json

# By day.
jq -r 'def s(f): (map(f)|add)//0;
  .usage | map(select(.provider=="anthropic")) | group_by(.created|todate|.[:10])[]
  | "\(.[0].created|todate|.[:10])\t\(length)\t\(map(.session_id)|unique|length)\t\(s(.api_equivalent_cost//0)|round)"' usage.json

# Provenance totals and the derived escalations, as the export prints them.
jq '.provenance, .escalations' usage.json

# The repository's side, from git.
git log --since=2026-08-15 --no-merges --format='%ad' --date=short | sort | uniq -c
git log --since=2026-08-15 --no-merges --format=%B | grep -o 'Co-Authored-By: [^<]*' | sort | uniq -c
gh pr list --state merged --limit 200 --json mergedAt -q '[.[] | select(.mergedAt >= "2026-08-15")] | length'
```

The cache-read arithmetic uses the rates in `pricing/litellm.tsv` on the day of writing: Opus 5
at $5.00 input, $25.00 output, $6.25 cache write and $0.50 cache read per million tokens.
