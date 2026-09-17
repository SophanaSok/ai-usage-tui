# Stability

What a release number promises, and what it does not.

`ai-usage-tui` follows [Semantic Versioning](https://semver.org) for **the command-line tool** —
the things a script, a timer, a status bar or a hook depends on. From 1.0.0, a change that breaks
one of the surfaces below needs a major version; adding to one is a minor version; a fix that
leaves them intact is a patch. Before 1.0.0, as Cargo reads versions, a minor release may still
break them, and says so in `CHANGELOG.md`.

## Stable

| Surface | The promise |
| --- | --- |
| **Command-line flags** | A flag's name and meaning are kept. New flags may be added. The list is the README's CLI reference, which a test checks against the parser. |
| **Exit codes** | `0` on success, including a reader closing stdout (`\| head`). Non-zero on failure. `--check-budgets` exits non-zero when any budget has reached its `warn` threshold, and `--check-update` when it could not ask or could not cache. |
| **Config file** | Keys in `config.toml` keep their names and meanings. New keys may be added. Unknown keys are rejected rather than ignored, so a config written for a newer release fails loudly on an older one instead of being half-applied. `ai-usage-tui --print-config` prints the annotated example. |
| **JSON output** | `--json`, `--routing-json` and `--check-budgets` each print one object carrying `"schema_version": 1`. Within a version keys are only added, never removed, renamed, or given a different meaning or type. Ignore keys you do not know. An absent value is `null`, not `0` — `cost` especially — and that is part of the meaning. |
| **CSV output** | `--csv` and `--routing-csv` columns are appended, never inserted, reordered or removed, so a consumer reading by position keeps working. |
| **Journal** | `usage.db` stores its schema version in `PRAGMA user_version` (currently `1`). A release can read every journal an earlier release wrote. A writer refuses a journal stamped with a *newer* version than it knows rather than writing into it. See [`data-model.md`](data-model.md). |
| **Environment variables** | The variables in the README's table keep their names and meanings. |
| **Omarchy record** | `--omarchy-record` writes Omarchy's own record format (`schemaVersion` 1). It follows Omarchy's format, not this project's release numbers. |

## Not stable

These change whenever the tool needs them to, in any release, and are recorded in `CHANGELOG.md`
when they do:

- **The Rust library API.** The crate publishes a library so the binary, its tests and the screenshot
  renderer can share code. It is not an interface. Depend on the command-line tool.
- **Human-readable output.** The dashboard's layout, panels, wording and key bindings; `--once`;
  `--doctor`; `--statusline`; the status line; error messages. Parse `--json`, not these.
- **Files the tool keeps for itself.** `zen-pricing.toml`, `zen-models.json`, `update-check.json`,
  `statusline-limits.json` and the `AI_USAGE_LOG` diagnostic log.
- **Figures that come from outside.** Pricing rates are data. A release that updates a rate or adds a
  model changes a computed cost, and that is not a breaking change. Which of these a figure rests on
  is what `cost_status` and `provenance` report.
- **What an upstream tool writes.** Claude Code, Codex CLI, GitHub Copilot, Gemini CLI and OpenCode
  change their own formats. The tool follows them as well as it can, and reports what it could not
  read rather than guessing (see `collector::skipped`). No release number can promise another
  tool's file format.
