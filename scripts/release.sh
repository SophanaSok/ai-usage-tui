#!/usr/bin/env bash
set -euo pipefail

# Pre-flight checklist before tagging a release.
# Usage: scripts/release.sh 0.2.0

VERSION="${1:?Usage: scripts/release.sh <version>}"

# Anchored to the repository, not to the caller's working directory: every check below is
# relative (`Cargo.toml`, `README.md`, `CHANGELOG.md`), so running this from anywhere but the
# root silently checked nothing and still printed a pass.
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "==> Running pre-flight checks for v${VERSION}..."

# 1. Check we're on main branch
BRANCH=$(git branch --show-current)
if [ "$BRANCH" != "main" ]; then
  echo "ERROR: Must be on 'main' branch (currently on '$BRANCH')"
  exit 1
fi

# 2. Check working tree is clean
if [ -n "$(git status --porcelain)" ]; then
  echo "ERROR: Working tree has uncommitted changes"
  git status --short
  exit 1
fi

# 3. Formatting. CI checks this first and it is the cheapest thing to fail on; leaving it out
# meant "All checks passed!" could be followed by a red CI run on the tag.
echo "==> Checking formatting..."
cargo fmt --all -- --check

# 4. Run tests
echo "==> Running tests..."
cargo test --all-targets --locked
cargo test --doc --locked

# 5. Run clippy
echo "==> Running clippy..."
cargo clippy --all-targets --all-features --locked -- -D warnings

# 6. Advisories and licence policy. Skipped with a warning rather than failing, because it needs
# a tool that may not be installed -- but the skip is said out loud, not hidden.
if command -v cargo-deny >/dev/null 2>&1; then
  echo "==> Checking advisories..."
  cargo deny check
else
  echo "WARNING: cargo-deny is not installed; skipping the advisory check that CI runs."
  echo "         Install it with: cargo install cargo-deny --locked"
  SKIPPED="${SKIPPED:-}advisories "
fi

# 7. Build release
echo "==> Building release..."
cargo build --release --locked

# 8. Verify version matches in Cargo.toml
CARGO_VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
if [ "$CARGO_VERSION" != "$VERSION" ]; then
  echo "ERROR: Cargo.toml version is '$CARGO_VERSION', expected '$VERSION'"
  echo "Update Cargo.toml version first: sed -i 's/^version = .*/version = \"$VERSION\"/' Cargo.toml"
  exit 1
fi

# 9. Verify README.md quick-start pins this version
if ! grep -qF "VERSION=v${VERSION}" README.md; then
  echo "ERROR: README.md quick-start still pins a different VERSION="
  grep -n '^VERSION=v' README.md || true
  exit 1
fi

# 10. Check CHANGELOG.md has the version
if ! grep -qF "## [$VERSION]" CHANGELOG.md 2>/dev/null; then
  echo "WARNING: CHANGELOG.md may not have a '$VERSION' section"
fi

echo ""
if [ -n "${SKIPPED:-}" ]; then
  echo "==> Checks passed, but these were SKIPPED and CI still runs them: ${SKIPPED}"
else
  echo "==> All checks passed!"
fi
echo "==> To release, run:"
echo "    git tag v${VERSION}"
echo "    git push origin main"
echo "    git push origin --tags"
echo ""
echo "    CI will build artifacts for all platforms automatically."