#!/usr/bin/env bash
set -euo pipefail

# Pre-flight checklist before tagging a release.
# Usage: scripts/release.sh 0.2.0

VERSION="${1:?Usage: scripts/release.sh <version>}"

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

# 3. Run tests
echo "==> Running tests..."
cargo test --all-targets

# 4. Run clippy
echo "==> Running clippy..."
cargo clippy --all-targets --all-features -- -D warnings

# 5. Build release
echo "==> Building release..."
cargo build --release

# 6. Verify version matches in Cargo.toml
CARGO_VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
if [ "$CARGO_VERSION" != "$VERSION" ]; then
  echo "ERROR: Cargo.toml version is '$CARGO_VERSION', expected '$VERSION'"
  echo "Update Cargo.toml version first: sed -i 's/^version = .*/version = \"$VERSION\"/' Cargo.toml"
  exit 1
fi

# 7. Check CHANGELOG.md has the version
if ! grep -q "## $VERSION" CHANGELOG.md 2>/dev/null; then
  echo "WARNING: CHANGELOG.md may not have a '$VERSION' section"
fi

echo ""
echo "==> All checks passed!"
echo "==> To release, run:"
echo "    git tag v${VERSION}"
echo "    git push origin main"
echo "    git push origin --tags"
echo ""
echo "    CI will build artifacts for all platforms automatically."