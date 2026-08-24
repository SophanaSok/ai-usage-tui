# Model Routing

Development-time model policy for working on this repository: **which tier of model may see what**,
and **what to record about each routing decision**.

## Source of truth

Agent-to-model assignments are *not* recorded here. They live in the OpenCode workspace config:

```text
~/.config/opencode/opencode.json   # agent definitions (model, mode, description)
~/.config/opencode/ROUTING.md      # tier table, hardware constraint, budget rules
```

This file used to carry its own copy of that table. It drifted — the copy still named
`ollama/qwen3-coder-agent`, `gemma3:4b`, and `qwen2.5-coder:7b` long after the workspace moved to a
single persistent local server plus free cloud, and referenced a `@reasoning` agent it never
defined. Two copies of a fast-moving mapping will always diverge, so this one is gone. Read the
config for models; read this file for policy.

## Tiers by role

Roles are stable even as the model behind each one changes. Cheapest first — never skip a tier.

| Tier | Role | Cost | Use for |
| --- | --- | --- | --- |
| Local primary | `local` | free | Multi-file edits, refactors, debugging, tool loops |
| Local read | `explore-local` | free | Codebase Q&A on the same server (no extra VRAM) |
| Free cloud | `explorer`, `junior` | $0 | Search, boilerplate, routine edits |
| Free cloud | `reviewer` | $0 | Code review and second opinions |
| Free cloud | `reasoning` | $0 | Hard logic, planning, algorithm audits |
| Local vision | `vision` | free | Screenshots and UI work |
| Flat cloud | `heavy` | subscription | Architecture, large refactors |
| Metered | `heavy2` | $$ per token | Last resort only |

`orchestrate` (free cloud) is the default agent; it plans and delegates rather than doing the work
itself.

**Hardware constraint.** 16GB of VRAM fits exactly one capable local model. Route local work to
`local` sequentially — never run two local agents concurrently. Anything that would once have been
a small co-resident local model is now free cloud, which costs nothing and uses no VRAM.

**Budget cap: $20/mo.** Prefer free cloud over flat-rate, and flat-rate over metered. If a task's
estimated metered cost exceeds $2, stop and ask. If metered spend trends past ~$15, restrict to
`heavy` plus local for the rest of the month.

## Privacy policy

Hosted models may process non-secret code. Secrets, credentials, production data, private source,
and security-sensitive changes must use a local tier (`local` or `explore-local`). When sensitivity
is uncertain, route locally — the local tier is free, so there is no cost argument against
defaulting to it.

This mirrors the constraint the application itself enforces: collectors read usage metadata only
and never persist or transmit prompts, completions, or credentials. See
[`docs/architecture.md`](architecture.md#privacy-boundary).

## Routing rules

1. Never skip tiers: local → free cloud → flat cloud → metered.
2. Free cloud is always preferred over paid; `reviewer` and `reasoning` cost $0 and use no VRAM.
3. Use `reviewer` for all review work — read-only, cannot delegate, and safe for independent review.
   Prefer a reviewer on a different provider from the agent that wrote the code.
4. After any metered invocation, follow up with `reviewer` to verify.
5. Escalate when a model reports uncertainty, fails twice, or produces a patch that fails validation.

## Escalation signals

Escalate a tier when any of these hold:

- The task touches authentication, authorization, billing, persistence, or data migration.
- The change affects concurrency, security, public APIs, or backwards compatibility.
- Tests fail after two repair attempts.
- The model cannot explain the failure, or identifies conflicting requirements.
- `reviewer` finds a correctness or security defect.

## Evaluation

Routing decisions are recorded through the application's own routing schema, which mirrors
`RoutingEvent` in [`src/model.rs`](../src/model.rs):

- Task and phase
- Agent, model, provider
- Category — local, free, paid, cloud, or unknown
- Request count
- Total tokens (one `tokens` counter; routing events do not split buckets)
- Estimated cost and its provenance
- Retry and escalation counts
- Test result
- Review defects found
- Created timestamp

Capture with `--record-routing`, view with the `t` panel, export with `--routing-json` or
`--routing-csv`. The dashboard compares models by cost per successful task, token efficiency, retry
rate, test pass rate, and review defect rate — the numbers that answer whether a paid tier is
actually earning its cost over a free one.

Track spend with:

```sh
~/Projects/ai-usage-tui/target/release/ai-usage-tui
```
