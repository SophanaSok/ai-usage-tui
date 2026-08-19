## What changes

<!-- What a user would notice. If nothing user-visible changes, say so. -->

## Verification

<!-- The commands you actually ran. -->

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --locked
```

## Checklist

- [ ] Tests cover the change, and a regression test **fails** against the unfixed code
- [ ] No prompts, completions, or credentials are collected, logged, or persisted
- [ ] Unknown cost is still never rendered as `$0.00`
- [ ] Tests pass an explicit `--claude-dir` and do not read real `~/.claude/projects`
- [ ] `CHANGELOG.md` updated if the change is user-visible

<!-- Not every box applies to every PR. Delete what doesn't. -->
