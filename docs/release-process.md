# Release Process

1. Update `CHANGELOG.md` with the new version section.
2. Update the version in `Cargo.toml`: `version = "X.Y.Z"`. Also update the `VERSION=vX.Y.Z`
   quick-start lines and the `.deb`/`.rpm` example names in `README.md` — `tests/docs.rs` fails
   if they disagree with `Cargo.toml`.
3. Run `scripts/release.sh X.Y.Z` — this pre-flight checklist verifies:
   - On `main` branch with clean working tree
   - `cargo test --all-targets --locked` passes
   - `cargo clippy --all-targets --all-features --locked -- -D warnings` passes
   - `cargo build --release --locked` succeeds
   - Cargo.toml version matches the requested version
4. Dry-run the release workflow before tagging — it builds, packages, and verifies everything
   and skips only the publish:
   `gh workflow run release.yml -f tag=v0.0.0-dryrun && gh run watch`.
5. Tag and push: `git tag vX.Y.Z && git push origin main && git push origin --tags`.
6. GitHub Actions (`release.yml`) automatically builds nine artifacts:
   - Linux: `-x86_64-linux.tar.gz`, `-aarch64-linux.tar.gz`
   - macOS: `-x86_64-macos.tar.gz`, `-aarch64-macos.tar.gz` (both cross-compiled on
     `macos-latest` with an explicit `--target`; v0.2.0 shipped an arm64 binary labelled x86_64
     because the build had no `--target`)
   - Windows: `-x86_64-windows.zip`
   - Packages: `-amd64.deb`, `-arm64.deb`, `-amd64.rpm`, `-arm64.rpm`
7. CI generates `checksums.txt` (SHA256) and creates a GitHub Release with all artifacts.
8. Packaging manifests (Homebrew, Scoop, Chocolatey) are **rendered by the release job** from the
   real artifact names and checksums and attached to the release. They are not hand-edited; a
   missing checksum fails the job rather than shipping a placeholder. The Chocolatey pair keeps
   its `tools/` layout, because the nuspec's `<file src="tools/**">` glob matches nothing without
   it. Two follow-on jobs then run on a tag push: `publish-crate` (crates.io) and `update-taps`
   (Homebrew tap, Scoop bucket). Both skip with a notice when their secret is unset — see
   *First publish* below.
9. Verify the published artifacts independently: architecture with `file`, checksums, `.deb`/`.rpm`
   contents with `bsdtar`, and the Homebrew sha256 against the downloaded tarball.

Release artifacts must include the binary, README, and LICENSE. The project should not require Rust to run a published binary.

## First publish

Three one-time steps that need accounts and tokens rather than code. Everything in the repository
is already wired for them: each job checks for its secret and prints a notice instead of failing,
so the release path stays green whether or not these have been done.

1. **crates.io.** Done — published since v0.6.0, and `CARGO_REGISTRY_TOKEN` is set, so
   `publish-crate` runs on each tag. Kept here because it is the recipe for verifying a release
   candidate before tagging. Verify the package first —
   `cargo publish --dry-run --locked`, and `cargo package --list` to confirm `exclude` keeps the
   screenshots out (the tarball should be well under 1 MB). Then:

   ```sh
   cargo login                       # paste a token from https://crates.io/settings/tokens
   cargo publish --locked
   ```

   Add the same token as the repository secret `CARGO_REGISTRY_TOKEN`. From then on the
   `publish-crate` job publishes each tagged release automatically, and it refuses to run if the
   tag and `Cargo.toml` disagree. Publishing also makes `cargo binstall ai-usage-tui` work — the
   `[package.metadata.binstall]` overrides in `Cargo.toml` already map every target to its
   release archive.

2. **Homebrew tap and Scoop bucket.** Create two public repositories,
   `SophanaSok/homebrew-tap` and `SophanaSok/scoop-bucket`. Create a fine-grained personal access
   token with **Contents: read and write** on both, and add it as the repository secret
   `TAP_TOKEN`. The `update-taps` job then pushes `Formula/ai-usage-tui.rb` and
   `bucket/ai-usage-tui.json` on each tagged release, and
   `brew install sophanasok/tap/ai-usage-tui` works. Done — both repositories exist and have been
   pushed for every release since v0.6.0.

   The README's notices claiming neither channel existed stayed in place for three releases after
   both became false, because nothing checked them.
   `tests/docs.rs::no_stale_publication_notes` now bans the phrase, and `identity.yml` checks each
   release channel the README names actually exists. See *Repository metadata* below.

3. **AUR (optional).** The Omarchy integration makes Arch the project's showcase platform, and it
   currently has no native install path. An `ai-usage-tui-bin` PKGBUILD pointing at the published
   `x86_64-linux` and `aarch64-linux` tarballs is about twenty lines and needs no build
   infrastructure.

Chocolatey is rendered and attached to each release but is not pushed anywhere; packing and
pushing it needs a Chocolatey account. The rendered files keep the layout `choco pack` expects
(`chocolatey/ai-usage-tui.nuspec` beside `chocolatey/tools/chocolateyinstall.ps1`), so it can be
packed by hand from the release assets.

## Repository metadata

`Cargo.toml` is the single source of truth for the project's identity — the description, the
keywords, and `[package.metadata.identity]`'s `topics` and `github_homepage`.

Everything else derives from it or is checked against it. crates.io needs no mechanism at all:
`cargo publish` reads the manifest verbatim, so that consumer is correct structurally. The
packaging manifests carry `__DESCRIPTION__` and `__TOPICS__` and are rendered at release time by
the same `sed` loop that renders the tag and the checksums. `src/cli.rs` sets clap's `about` from
`env!("CARGO_PKG_DESCRIPTION")`, so `--help` and the man page cannot diverge. `tests/docs.rs`
pins the README's tagline, its source list and its packaging prose.

GitHub is the one consumer that has no manifest, so `.github/workflows/identity.yml` enforces it.
It runs on pushes to `main` that touch `Cargo.toml`, weekly, and on demand.

**It needs a secret to push, and deliberately does not need one to fail.** Updating a repository's
description, homepage or topics requires **Administration: write**, which the built-in
`GITHUB_TOKEN` can never hold — there is no `administration` key among the scopes a workflow's
`permissions:` block accepts, so `permissions: write-all` does not help either. Reading them needs
only Metadata: read, which every token has.

So the job pushes with a PAT and *verifies with `GITHUB_TOKEN`*. There is no state in which the
sync quietly stops working:

| | |
| --- | --- |
| Secret absent | nothing pushed, verify fails — **red**, with the one command to fix it |
| Secret expired | push fails naming 401/403, verify fails too — **red** |
| Secret valid, nothing changed | `PATCH` is idempotent, verify passes — green, no churn |
| Someone edits the description in the web UI | next Monday's scheduled run goes **red** |

That last property is why this is not modelled on `publish-crate` and `update-taps`, which skip
with a notice and go green when their secret is missing. A silently expiring `TAP_TOKEN` is a
known hazard of that pattern: the release still succeeds and the tap simply stops updating.

To set it up: create a fine-grained personal access token scoped to **this repository only**, with
**Administration: read and write** and nothing else, and add it as the repository secret
`REPO_METADATA_TOKEN`. Do not reuse `TAP_TOKEN` — it is Contents:write on two *other* repositories
and is embedded in a clone URL by `update-taps`, so widening it would give that job settings-write
here, and one expiry would break two unrelated things ambiguously.

Without the secret, set it once by hand and let CI keep it honest afterwards:

```sh
scripts/identity.sh --check      # what GitHub says, against what Cargo.toml says
scripts/identity.sh --apply      # push it (needs the Administration: write token)
scripts/identity.sh --channels   # every release channel the README names actually exists
```

Rehearse the workflow without writing anything — this needs no secret, and against a drifted
repository it is expected to come back red:

```sh
gh workflow run identity.yml -f apply=false
gh run watch
```

## Deferred

- AppImage (requires Docker, niche Linux use case)
- `.msi` (requires WiX, enterprise use case)
- macOS notarization (requires Apple Developer ID, $99/year)
- GPG/cosign signing (checksums sufficient for v1)
- SBOM/Syft (compliance tool, no demand yet)
