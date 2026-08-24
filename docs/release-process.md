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

1. **crates.io.** The name `ai-usage-tui` is unclaimed. Verify the package first —
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
   `brew install sophanasok/tap/ai-usage-tui` works. Remove the "Not published yet" notes from
   the README's Package managers section once both exist.

3. **AUR (optional).** The Omarchy integration makes Arch the project's showcase platform, and it
   currently has no native install path. An `ai-usage-tui-bin` PKGBUILD pointing at the published
   `x86_64-linux` and `aarch64-linux` tarballs is about twenty lines and needs no build
   infrastructure.

Chocolatey is rendered and attached to each release but is not pushed anywhere; packing and
pushing it needs a Chocolatey account. The rendered files keep the layout `choco pack` expects
(`chocolatey/ai-usage-tui.nuspec` beside `chocolatey/tools/chocolateyinstall.ps1`), so it can be
packed by hand from the release assets.

## Deferred

- AppImage (requires Docker, niche Linux use case)
- `.msi` (requires WiX, enterprise use case)
- macOS notarization (requires Apple Developer ID, $99/year)
- GPG/cosign signing (checksums sufficient for v1)
- SBOM/Syft (compliance tool, no demand yet)
