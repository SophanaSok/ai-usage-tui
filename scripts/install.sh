#!/bin/sh
# Install a prebuilt ai-usage-tui release.
#
#   curl -fsSL https://raw.githubusercontent.com/SophanaSok/ai-usage-tui/main/scripts/install.sh | sh
#   ... | sh -s -- --version v0.5.0 --dir /usr/local/bin
#
# POSIX sh on purpose: this is the one file that runs before the project is installed, on
# whatever shell a stranger's machine happens to have. It has no bashisms and no dependencies
# beyond curl (or wget), tar and a sha256 tool.
#
# What it does that a hand-pasted curl|tar does not:
#   - refuses to guess on an unsupported platform, and names the source build instead
#   - verifies the download against the release's own checksums.txt
#   - unpacks into a scratch directory, because the archive also contains README.md and LICENSE
#   - creates the target directory and says so when it is not on PATH
#   - says what it replaced, and warns when another copy earlier on PATH will keep winning
set -eu

REPO="SophanaSok/ai-usage-tui"
BIN="ai-usage-tui"
VERSION=""
DEST=""

usage() {
    cat <<EOF
Install a prebuilt $BIN release.

Usage: install.sh [--version vX.Y.Z] [--dir PATH]

  --version   Release tag to install. Default: the latest release.
  --dir       Directory to install into. Default: \$HOME/.local/bin,
              or /usr/local/bin when running as root.
  --help      Show this message.
EOF
}

die() {
    echo "install.sh: $*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version) [ $# -ge 2 ] || die "--version requires a tag"; VERSION="$2"; shift 2 ;;
        --dir)     [ $# -ge 2 ] || die "--dir requires a path";    DEST="$2";    shift 2 ;;
        --help|-h) usage; exit 0 ;;
        *)         die "unknown option: $1 (try --help)" ;;
    esac
done

# --- platform -------------------------------------------------------------------------------
# Kept deliberately in step with the archive-name table in README.md and the build matrix in
# .github/workflows/release.yml. A platform absent from all three is a source build, not a guess.
os="$(uname -s)"
arch="$(uname -m)"
case "${os}-${arch}" in
    Linux-x86_64)            SLUG="x86_64-linux";  EXT="tar.gz" ;;
    Linux-aarch64|Linux-arm64) SLUG="aarch64-linux"; EXT="tar.gz" ;;
    Darwin-arm64)            SLUG="aarch64-macos"; EXT="tar.gz" ;;
    Darwin-x86_64)           SLUG="x86_64-macos";  EXT="tar.gz" ;;
    *)
        die "no prebuilt binary for ${os}-${arch}.
Build from source instead:
    cargo install $BIN --locked
or clone the repository and run \`cargo install --path . --locked\`."
        ;;
esac

need tar || die "tar is required"
if need curl; then
    fetch() { curl -fsSL "$1"; }
    fetch_to() { curl -fsSL -o "$2" "$1"; }
elif need wget; then
    fetch() { wget -qO- "$1"; }
    fetch_to() { wget -qO "$2" "$1"; }
else
    die "curl or wget is required"
fi

# --- version --------------------------------------------------------------------------------
if [ -z "$VERSION" ]; then
    # The redirect target of /releases/latest is the tag, which avoids depending on the API's
    # rate limit or on a JSON parser being present.
    if need curl; then
        location="$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
            "https://github.com/$REPO/releases/latest" 2>/dev/null || true)"
    else
        location="$(wget -qS --max-redirect=10 -O /dev/null \
            "https://github.com/$REPO/releases/latest" 2>&1 \
            | awk '/^ *Location:/ { print $2 }' | tail -1 || true)"
    fi
    VERSION="${location##*/}"
    case "$VERSION" in
        v[0-9]*) ;;
        *) die "could not determine the latest release tag; pass --version vX.Y.Z" ;;
    esac
fi

# --- destination ----------------------------------------------------------------------------
if [ -z "$DEST" ]; then
    if [ "$(id -u)" = "0" ]; then
        DEST="/usr/local/bin"
    else
        DEST="${HOME:?HOME is not set; pass --dir}/.local/bin"
    fi
fi

ARCHIVE="${BIN}-${VERSION}-${SLUG}.${EXT}"
BASE="https://github.com/$REPO/releases/download/$VERSION"

echo "==> $BIN $VERSION for ${os}-${arch}"

# --- what is already here -------------------------------------------------------------------
# Re-running this script to upgrade used to be silent about what it replaced, so an upgrade and
# a fresh install looked identical -- and neither mentioned the case that actually bites: a copy
# installed by some other channel sitting earlier on PATH, which goes on being the one that runs.
# Resolved before the install, because that is the state the user is in.
installed_version() {
    # `--version` prints "ai-usage-tui X.Y.Z". A binary that will not run -- wrong architecture,
    # a half-written file -- yields nothing rather than stopping the install.
    "$1" --version 2>/dev/null | awk 'NR == 1 { print $2 }'
}

was_on_path="$(command -v "$BIN" 2>/dev/null || true)"
if [ -e "$DEST/$BIN" ]; then
    previous="$(installed_version "$DEST/$BIN")"
    if [ -z "$previous" ]; then
        echo "    replacing $DEST/$BIN (its version could not be read)"
    elif [ "$previous" = "${VERSION#v}" ]; then
        echo "    reinstalling $previous over $DEST/$BIN"
    else
        # Deliberately not "upgrading" or "downgrading": ordering two versions correctly needs
        # more than string comparison, and `sort -V` is not POSIX. Both are named instead, which
        # cannot be wrong.
        echo "    replacing $previous at $DEST/$BIN"
    fi
fi

WORK="$(mktemp -d)"
# shellcheck disable=SC2064  # WORK is expanded now on purpose; it never changes.
trap "rm -rf '$WORK'" EXIT INT TERM

echo "==> downloading $ARCHIVE"
fetch_to "$BASE/$ARCHIVE" "$WORK/$ARCHIVE" \
    || die "download failed: $BASE/$ARCHIVE
Check that $VERSION is a published release with an asset for ${SLUG}."

# --- checksum -------------------------------------------------------------------------------
# release.yml publishes checksums.txt with bare filenames precisely so this verifies. A missing
# or mismatched entry is a hard failure: a silently unverified binary is the thing this script
# exists to avoid.
echo "==> verifying checksum"
if fetch "$BASE/checksums.txt" > "$WORK/checksums.txt" 2>/dev/null && [ -s "$WORK/checksums.txt" ]; then
    expected="$(awk -v n="$ARCHIVE" '$2 == n || $2 == "*" n { print $1 }' "$WORK/checksums.txt")"
    [ -n "$expected" ] || die "checksums.txt has no entry for $ARCHIVE"

    if need sha256sum; then
        actual="$(sha256sum "$WORK/$ARCHIVE" | awk '{ print $1 }')"
    elif need shasum; then
        actual="$(shasum -a 256 "$WORK/$ARCHIVE" | awk '{ print $1 }')"
    elif need openssl; then
        actual="$(openssl dgst -sha256 "$WORK/$ARCHIVE" | awk '{ print $NF }')"
    else
        die "no sha256 tool found (sha256sum, shasum or openssl); cannot verify the download"
    fi

    [ "$expected" = "$actual" ] || die "checksum mismatch for $ARCHIVE
  expected $expected
  actual   $actual
Do not use this download."
    echo "    ok  $actual"
else
    die "could not fetch $BASE/checksums.txt; refusing to install an unverified binary"
fi

# --- install --------------------------------------------------------------------------------
# Into a scratch directory: the archive carries README.md and LICENSE beside the binary.
tar xzf "$WORK/$ARCHIVE" -C "$WORK"
[ -f "$WORK/$BIN" ] || die "$ARCHIVE did not contain $BIN"

mkdir -p "$DEST" || die "could not create $DEST"
if [ -w "$DEST" ]; then
    install -m 755 "$WORK/$BIN" "$DEST/$BIN"
elif need sudo; then
    echo "==> $DEST is not writable; using sudo"
    sudo install -m 755 "$WORK/$BIN" "$DEST/$BIN"
else
    die "$DEST is not writable and sudo is not available; pass --dir PATH"
fi

echo "==> installed $DEST/$BIN"

# --- PATH -----------------------------------------------------------------------------------
# Three outcomes, and only the first is the happy one: the name resolves to what was just
# installed; it resolves to a different copy, which is the one that will keep running; or the
# directory is not on PATH at all.
case ":${PATH}:" in
    *":${DEST}:"*)
        now_on_path="$(command -v "$BIN" 2>/dev/null || true)"
        if [ -n "$now_on_path" ] && [ "$now_on_path" != "$DEST/$BIN" ]; then
            other="$(installed_version "$now_on_path")"
            echo
            echo "Warning: \`$BIN\` still runs $now_on_path${other:+ ($other)}, not the copy"
            echo "just installed -- it comes earlier on your PATH. Two copies installed by"
            echo "different channels is the usual cause; \`$DEST/$BIN --doctor\` names the"
            echo "channel each one came from. Remove the one you do not want, or run this one"
            echo "explicitly:"
            echo "    $DEST/$BIN"
        else
            echo "==> run it with: $BIN"
        fi
        ;;
    *)
        echo
        echo "$DEST is not on your PATH. Add it:"
        echo "    export PATH=\"$DEST:\$PATH\""
        echo "(put that in ~/.bashrc, ~/.zshrc, or your shell's rc file)"
        echo
        echo "Until then, run it with: $DEST/$BIN"
        if [ -n "$was_on_path" ]; then
            echo
            echo "Note: \`$BIN\` currently runs $was_on_path, which this did not replace."
        fi
        ;;
esac
