# Contributing

Thanks for looking. This is a small, focused Rust project and the codebase is designed to be
readable end to end in an afternoon.

## What this project is

A terminal dashboard that reads AI token usage from local sources — OpenCode's SQLite database,
Claude Code's session logs, a local journal — and reports what it cost. It is a **read-only,
local-first** tool: no server, no telemetry, no account.

Two things make it different from the several similar tools, and both are worth preserving:

- **Cost provenance.** Every figure carries where it came from — reported by the provider,
  calculated, estimated, free, local, or unavailable. Unknown cost is never rendered as `$0.00`.
- **Routing analytics.** Retries, escalations, test pass/fail and review defects per model, so
  you can answer "is the expensive model actually earning its cost on my work?"

## Setup

Install Rust via [rustup](https://rustup.rs/) (1.88+ required — set by the dependency graph, not
by this crate), then:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --locked
cargo test --doc --locked
cargo build --release --locked
```

`--locked` is not decoration. The lockfile is committed and the project ships release binaries,
so a build that silently resolved a different dependency set would test one thing and publish
another. CI passes it on every command above, which also makes a dependency PR that edits
`Cargo.toml` without regenerating `Cargo.lock` fail rather than merge. `cargo fmt` takes no
`--locked` because it never resolves dependencies.

**Run it without a TTY** — the dashboard needs a real terminal, so for scripted checks use:

```sh
cargo run --locked -- --json --db tests/fixtures/opencode_test.db --all --claude-dir /nonexistent
```

The fixture database has 2023-era timestamps, so `--today` and `--week` show nothing. Use
`--all`. Pass `--claude-dir` explicitly or you will read your own `~/.claude/projects`.

## Where things live

| Path | What's in it |
| --- | --- |
| `src/collector/` | One module per data source, plus the supervisor that polls them |
| `src/pricing.rs` | The pricing table, model-id resolution, and cost estimation |
| `src/classify.rs` | Deciding whether usage is local, free, paid, cloud, or unknown |
| `src/model.rs` | `Usage`, `CostStatus`, `Category` — the shared vocabulary |
| `src/ui/` | Dashboard. `app.rs` state, `aggregate.rs` pure math, `panels/` one file per panel |
| `src/budget.rs` | Budget limits and alert dispatch |
| `src/routing.rs` | Routing analytics aggregation |
| `pricing/zen.toml` | The bundled pricing table |
| `docs/roadmap.md` | What's left, with enough evidence attached to pick up cold |

## Common contributions

### Add support for another tool's usage data

The most valuable contribution. Implement `Collector` in a new `src/collector/yours.rs`:

```rust
pub trait Collector: Send + 'static {
    fn name(&self) -> &str;
    fn interval(&self) -> Duration;
    fn poll(&mut self) -> Result<Vec<Usage>>;
}
```

Then register it in `main.rs::build_collectors` and add a `[collectors.yours]` section to the
config. Read `src/collector/claude_code.rs` first — it is the fullest example, including
incremental tailing and how to parse a file that contains things you must not read.

### Add a dashboard panel

Write `src/ui/panels/yours.rs` with a single `draw_yours(frame, area, app)`, add a variant to
`Panel` in `src/ui/app.rs`, a key binding in `src/ui/mod.rs`, and a match arm in `draw`. Nothing
else needs to know it exists. Anything the panel needs should be computed once per refresh into
`DerivedView`, never inside the draw call.

### Correct or add pricing

Edit `pricing/zen.toml`. Rates are per million tokens. If a rate changed on a date, add a
`[[model."x".period]]` block with a `through` date rather than overwriting — otherwise every
historical event silently re-prices at the new rate. Cache rates are not independent: a read is
0.1x input, a five-minute write is 1.25x.

## Invariants

These are not style preferences. Each one exists because breaking it produced a wrong number
that looked right.

1. **Unknown cost stays unknown.** A missing rate yields `CostStatus::Unavailable`, never
   `$0.00`. `Option<f64>` end to end; an explicit `0.0` is distinct from an absent field.
2. **Never read message content.** Session transcripts contain source code and secrets. Parse
   only the usage block. A test plants a fake credential and fails if it reaches a usage record.
3. **Dedup on identity, not shape.** Token counts alone are not an identity — agent loops
   routinely produce distinct requests with identical counts.
4. **No I/O or clock reads on the render path.** The dashboard redraws several times a second.
5. **Failure must be visible.** `unwrap_or_default()` on a parse, `if let Ok(..)` on a poisoned
   lock, and `break` on a panic all read as robustness and all render "broken" as "nothing to
   report".

## Testing

- **Tests must be hermetic.** Anything going through `load_usage` or `print_once` needs an
  explicit `claude_dir`, or it reads the developer's real `~/.claude/projects` — and their real
  `~/.claude.json` for the billing decision. The config document is derived from `claude_dir`, so
  a fixture root resolves to a file that does not exist; pass `claude_json` to plant one.
- **Tests must discriminate.** A regression test that also passes against the buggy code is
  worse than no test. Restore the bug in a scratch copy and confirm the test fails.

## Pull requests

Say what changes for a user, which data sources are touched, any privacy impact, and the
commands you ran. Keep them focused; avoid unrelated reformatting.

Not every change needs to be perfect to be worth sending. A bug report with a reproduction is a
real contribution, and so is a doc fix.
