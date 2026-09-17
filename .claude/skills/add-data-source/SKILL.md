---
name: add-data-source
description: Add support for another tool's usage data to ai-usage-tui (a new collector), or decide that it should not be one. Use when asked to support, read, track or add a tool, agent, CLI, editor or provider that ai-usage-tui does not read yet, or when a request in the "Support another tool's usage data" issue template is being worked.
---

# Adding a data source

The order matters more than the code. Each step can end the work, and ending it early is a
correct outcome.

1. **Find what the tool writes.** Locate its store or log on this machine and read a real record.
   If it records no token counts, or records zeros, stop: report that, and point at the README
   section "Why there is no Cursor collector". Do not estimate counts from text length, and do
   not proceed with a plan to "fill them in later".

2. **Try it without a collector.** If a short `jq` filter or script can turn the log into
   `ai-usage-tui --record-event` lines (README, "Local models"), that is the answer for one user,
   today, on any installed version. Offer it. A collector is for a tool many people use.

3. **Capture before you parse.** Drive the real tool, keep what it wrote, redact it (prompts,
   paths, hostnames, keys — `resource` and `cwd` blocks carry more than they look like they do)
   and commit it under `tests/fixtures/`. Write the parser against the capture. Then check the
   rule you are about to rely on against *all* the real records you have: every one carries the
   counts? one id per request, or is it shared across a turn?

4. **Write the module, then register it.** `CONTRIBUTING.md` → "Add support for another tool's
   usage data" is the file-by-file list; `src/collector/gemini.rs` is the smallest complete
   example. Use `helpers::required`, `collector::skipped::Skipped` and an `event_id` from the
   tool's own request identity. Add the privacy test: plant a fake credential in the fixture's
   message content and fail if it reaches a `Usage`.

5. **Let the tests lead.** Run `cargo test --all-targets --locked` and fix what fails, in the
   order it fails. The registry, hermeticity and documentation guards name what is missing. Do
   not edit a guard to make it pass; if one is wrong, say so.

6. **Look at it.** `cargo run --locked -- --doctor` against your own data shows the path
   searched, the rows found and the billing decision. Then check one total by hand against the
   tool's own figures. Run `just check` (or the four cargo commands in `CONTRIBUTING.md`) before
   you call it done.

7. **Write it up.** A `### Added` entry under `## [Unreleased]` in `CHANGELOG.md`, a section in
   `docs/provider-support.md` (path, record shape, identity, billing signal, what is deliberately
   unread, where the fixture came from), and the README section the docs test asks for.

What needs a person rather than a guess: whether the tool bills by subscription or per token
when nothing on disk says; anything that would read message content; any figure you could not
reconcile in step 6.
