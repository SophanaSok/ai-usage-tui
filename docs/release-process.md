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

3. **AUR.** The Omarchy integration makes Arch the project's showcase platform. `packaging/aur/
   PKGBUILD` is rendered by the same `sed` loop as the other manifests and attached to each
   release, so `curl` + `makepkg -si` works today. It is an `-bin` package: it installs the
   published `x86_64-linux` and `aarch64-linux` tarballs and needs no build infrastructure.

   **Checked against `man PKGBUILD` and `/usr/share/pacman/PKGBUILD.proto`, not from memory**
   (the wiki is behind a bot filter and cannot be fetched; the man page ships with pacman and is
   authoritative). Three things it got wrong, all fixed:

   - **`pkgdesc` was the crate's full description, 302 characters.** The man page asks to "keep
     the description to one line of text and to not use the package's name" -- that would have
     been three lines in `pacman -Si` and in every AUR search result. It renders the clause
     before the em dash now (90 characters), derived from the same single source of truth by one
     rule rather than becoming a sixth hand-written wording. `release.yml` spells the rule as
     `${DESCRIPTION%% — *}` and refuses a result over 100 characters;
     `tests/docs.rs::aur_pkgdesc_is_one_line_and_derived` pins the properties.
   - **`depends` was absent** while the binary dynamically links `libgcc_s`, `libc` and `libm`.
     Read off the shipped binary with `ldd` rather than assumed: `depends=('gcc-libs' 'glibc')`,
     and nothing more -- rustls means no OpenSSL and rusqlite is the bundled build, so there is
     no system sqlite link. An undeclared dependency is a namcap error.
   - **The `# Maintainer:` comment sat below the explanatory block.** The prototype puts it above
     `pkgname`; it is the one comment in the file with a required position, and the test now
     asserts it is line 1.

   **`namcap` 3.6.0 has been run**, against both the rendered PKGBUILD and a package built from
   it. **No errors.** Three warnings, all understood, none acted on -- re-run it after any change
   here (`namcap PKGBUILD` and `namcap *.pkg.tar.zst`) and compare against this list rather than
   re-deriving it:

   - **`Reference to x86_64 should be changed to $CARCH`** -- a false positive, and following it
     breaks the ARM package. `$CARCH` is the *build host's* architecture, so inside
     `source_aarch64` it expands to whatever machine ran makepkg. Substituting it and running
     `makepkg --printsrcinfo` on an x86_64 box produced
     `source_aarch64 = ...-aarch64.tar.gz::.../ai-usage-tui-v0.12.1-x86_64-linux.tar.gz`, and
     `.SRCINFO` is generated once and pushed, so every aarch64 user would fetch the x86_64
     tarball. It fails the checksum rather than installing the wrong binary, which is the safe
     direction, but the package is simply broken on ARM. The arch-specific source arrays exist to
     name an architecture that is *not* necessarily the builder's. `tests/docs.rs` asserts the
     literals stay, so the warning cannot be silenced by taking its advice.
   - **`Unused shared library '/usr/lib64/ld-linux-x86-64.so.2'`** -- the dynamic loader, which
     `ldd` lists for every dynamically linked binary. Not actionable.
   - **`Dependency included, but may not be needed ('gcc-libs')`** -- namcap sees libgcc as
     implicitly satisfied through the dependency tree. Kept anyway: the binary links
     `libgcc_s.so.1` directly, and declaring what you link is more robust than relying on another
     package continuing to pull it in.

   **Two things it cost, both worth not relearning.** `pkgdesc` is single-quoted, and has to
   stay that way: a PKGBUILD is bash, the description contains the literal `$0.00`, and inside
   double quotes bash expanded it to the script's own path -- `makepkg --printsrcinfo` rendered
   "instead of rendering as /usr/bin/makepkg.00". Nothing escapes a single quote inside single
   quotes, so `tests/docs.rs::crate_description_fits_every_registry` bans one in the description
   alongside the characters that would corrupt the `sed` substitution. And the render step
   asserts two well-formed `sha256sums_*` lines, because an empty substitution renders as
   `sha256sums_x86_64=('')`, which the unrendered-placeholder grep cannot see -- it looks for
   tokens still present, not for values that went missing.

   **Submitting it to the AUR is still a manual step**, and deliberately so. It needs an AUR
   account with an SSH key, and `.SRCINFO` -- which is derived from the PKGBUILD by
   `makepkg --printsrcinfo`, a tool that needs an Arch host, while the release runner is Ubuntu.
   Rendering a second template from the same placeholders would be a hand-maintained copy of
   generated data, which is the drift `packaging/` exists to avoid. From an Arch box, against the
   PKGBUILD attached to the release being published:

   ```sh
   git clone ssh://aur@aur.archlinux.org/ai-usage-tui-bin.git
   cp <rendered PKGBUILD> ai-usage-tui-bin/PKGBUILD
   cd ai-usage-tui-bin
   makepkg --printsrcinfo > .SRCINFO
   makepkg -f                     # build it before pushing it; this is the only real check
   git commit -am "ai-usage-tui-bin <version>" && git push
   ```

   When it is submitted, `scripts/identity.sh --channels` needs a fourth check for it and the
   README's "not submitted to the AUR" sentence has to go -- that sentence is the exact shape of
   the claim that stayed false for three releases before `no_stale_publication_notes` existed.

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
