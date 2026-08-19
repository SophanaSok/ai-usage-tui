# Release Process

1. Update `CHANGELOG.md` with the new version section.
2. Update the version in `Cargo.toml`: `version = "X.Y.Z"`.
3. Run `scripts/release.sh X.Y.Z` — this pre-flight checklist verifies:
   - On `main` branch with clean working tree
   - `cargo test --all-targets` passes
   - `cargo clippy --all-targets --all-features -- -D warnings` passes
   - `cargo build --release` succeeds
   - Cargo.toml version matches the requested version
4. Tag and push: `git tag vX.Y.Z && git push origin main && git push origin --tags`.
5. GitHub Actions (`release.yml`) automatically builds nine artifacts:
   - Linux: `-x86_64-linux.tar.gz`, `-aarch64-linux.tar.gz`
   - macOS: `-x86_64-macos.tar.gz`, `-aarch64-macos.tar.gz` (both cross-compiled on
     `macos-latest` with an explicit `--target`; v0.2.0 shipped an arm64 binary labelled x86_64
     because the build had no `--target`)
   - Windows: `-x86_64-windows.zip`
   - Packages: `-amd64.deb`, `-arm64.deb`, `-amd64.rpm`, `-arm64.rpm`
6. CI generates `checksums.txt` (SHA256) and creates a GitHub Release with all artifacts.
7. Packaging manifests (Homebrew, Scoop, Chocolatey) are **rendered by the release job** from the
   real artifact names and checksums and attached to the release. They are not hand-edited; a
   missing checksum fails the job rather than shipping a placeholder.
8. Verify the published artifacts independently: architecture with `file`, checksums, `.deb`/`.rpm`
   contents with `bsdtar`, and the Homebrew sha256 against the downloaded tarball.

Release artifacts must include the binary, README, and LICENSE. The project should not require Rust to run a published binary.

## Deferred

- AppImage (requires Docker, niche Linux use case)
- `.msi` (requires WiX, enterprise use case)
- macOS notarization (requires Apple Developer ID, $99/year)
- GPG/cosign signing (checksums sufficient for v1)
- SBOM/Syft (compliance tool, no demand yet)
