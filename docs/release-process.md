# Release Process

1. Update `CHANGELOG.md` and the version in `Cargo.toml`.
2. Run formatting, Clippy, tests, and a release build.
3. Build binaries for all supported platforms.
4. Generate SHA-256 checksums.
5. Create a signed version tag.
6. Publish release archives and checksums.
7. Verify installation from a clean environment.

Release artifacts must include the binary, README, LICENSE, and CHANGELOG. The project should not require Rust to run a published binary.
