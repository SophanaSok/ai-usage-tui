# Development Model Routing

This policy assigns models to development tasks based on risk, reasoning demand, latency, cost, and privacy. It uses the configured OpenCode agents in `~/.config/opencode/agent/`.

## Agent Inventory

| Agent | Model | Provider | Privacy | Mode |
| --- | --- | --- | --- | --- |
| `@explorer` | `ollama/qwen3-coder-agent` | Ollama local | local | subagent, read-only |
| `@local` | `ollama/qwen3-coder-agent` | Ollama local | local | subagent |
| `@junior` | `opencode/nemotron-3-ultra-free` | Zen (free) | hosted, non-secret | subagent |
| `@heavy` | `ollama-cloud/glm-5.2:cloud` | Ollama Cloud | hosted, non-secret | subagent |
| `@heavy2` | `opencode/gpt-5.6-sol` | Zen (paid) | hosted, non-secret | subagent |
| `@reviewer` | `opencode/gpt-5.6-sol` | Zen (paid) | hosted, non-secret | subagent, read-only |
| orchestrator | `opencode/gpt-5.6-luna` | Zen (paid) | hosted, non-secret | primary |

`@reviewer` and `@heavy2` use the same model but `@reviewer` is read-only and cannot delegate, making it safe for independent review.

## Default Assignment

| Phase | Agent | Model | Privacy |
| --- | --- | --- | --- |
| Repository exploration | `@explorer` | Ollama qwen3-coder (local) | local |
| Requirements extraction | `@junior` | nemotron-3-ultra-free | hosted, non-secret |
| Architecture and planning | `@heavy2` | GPT 5.6 Sol (Zen) | hosted, non-secret |
| Routine implementation | `@junior` | nemotron-3-ultra-free | hosted, non-secret |
| Complex implementation | `@heavy` | GLM 5.2 (Ollama Cloud) | hosted, non-secret |
| Debugging unfamiliar failures | `@heavy` then `@heavy2` | GLM 5.2 then GPT 5.6 Sol | hosted, non-secret |
| Test generation | `@junior` or `@local` | free or local | varies |
| Refactoring | `@heavy` | GLM 5.2 (Ollama Cloud) | hosted, non-secret |
| Security and correctness review | `@reviewer` | GPT 5.6 Sol (Zen) | hosted, non-secret |
| Sensitive or private code | `@local` | Ollama qwen3-coder (local) | local |
| Documentation and release notes | `@junior` | nemotron-3-ultra-free | hosted, non-secret |
| Final verification | orchestrator | GPT 5.6 Luna (Zen) | hosted, non-secret |

Model IDs are configurable in `~/.config/opencode/opencode.json` and agent markdown files. Update them when provider catalogs or pricing change.

## Routing Rules

1. Prefer `@explorer` or `@local` for private code, repository exploration, summaries, and routine test work.
2. Prefer `@junior` for low-risk searches, classification, and documentation drafts.
3. Use `@heavy` or `@heavy2` for architecture, difficult debugging, migrations, and complex implementation.
4. Use `@reviewer` for all review tasks — it is read-only, uses a different provider than `@heavy`, and cannot delegate.
5. Escalate when the model reports uncertainty, fails twice, or produces a patch that does not pass validation.
6. Start high-risk work at `@heavy2` instead of repeatedly retrying with `@junior`.
7. Do not send secrets, credentials, production data, or private source to a hosted model unless explicitly approved.
8. Hosted models are permitted for non-secret code; uncertain or sensitive content defaults to `@local`.
9. Use `@reviewer` (different provider, read-only) for correctness and security review.

## Fallback Chain

```text
@local -> @junior -> @heavy -> @heavy2
```

Escalate by invoking the next agent in the chain when the current tier fails or reports uncertainty.

## Escalation Signals

Escalate to the next tier when any of these occur:

- The task touches authentication, authorization, billing, persistence, or data migration.
- The change affects concurrency, security, public APIs, or backwards compatibility.
- Tests fail after two repair attempts.
- The model cannot explain the failure or identifies conflicting requirements.
- `@reviewer` finds a correctness or security defect.

## Evaluation

Record routing decisions alongside usage when possible:

- Task and phase
- Agent and model
- Provider
- Local, free, paid, cloud, or unknown category
- Input, output, reasoning, and cache tokens
- Estimated cost
- Retry and escalation count
- Test result
- Human correction required

The dashboard can then compare models by cost per successful task, token efficiency, retry rate, test pass rate, and review defect rate.
