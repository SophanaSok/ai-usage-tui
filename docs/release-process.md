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
5. GitHub Actions (`release.yml`) automatically builds for all platforms:
   - Linux: `ai-usage-tui-vX.Y.Z-x86_64-linux.tar.gz`
   - macOS: `ai-usage-tui-vX.Y.Z-x86_64-macos.tar.gz`
   - Windows: `ai-usage-tui-vX.Y.Z-x86_64-windows.zip`
6. CI generates `checksums.txt` (SHA256) and creates a GitHub Release with all artifacts.
7. Update package manager formulas (Homebrew, Scoop, Chocolatey) with the new SHA256 hashes.
8. Verify installation from a clean environment on each platform.

Release artifacts must include the binary, README, and LICENSE. The project should not require Rust to run a published binary.

## Deferred

- AppImage (requires Docker, niche Linux use case)
- `.msi` (requires WiX, enterprise use case)
- macOS notarization (requires Apple Developer ID, $99/year)
- GPG/cosign signing (checksums sufficient for v1)
- SBOM/Syft (compliance tool, no demand yet)
