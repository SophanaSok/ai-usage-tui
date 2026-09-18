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
#   - on Linux, takes the static build when the release has one, so the C library is never a
#     reason for the binary not to start
#   - verifies the download against the release's own checksums.txt
#   - checks the build attestation when the GitHub CLI is there to check it with
#   - unpacks into a scratch directory, because the archive also contains README.md and LICENSE
#   - creates the target directory and says so when it is not on PATH
#   - says what it replaced, and warns when another copy earlier on PATH will keep winning
set -eu

REPO="SophanaSok/ai-usage-tui"
# The documentation. This script runs on its own, piped from curl, so it cannot read Cargo.toml;
# tests/docs.rs holds it to `package.homepage` instead.
SITE="https://sophanasok.github.io/ai-usage-tui-site/"
BIN="ai-usage-tui"
VERSION=""
DEST=""
REQUIRE_ATTESTATION=""
NO_ATTESTATION=""
LIBC=""

usage() {
    cat <<EOF
Install a prebuilt $BIN release.

Usage: install.sh [--version vX.Y.Z] [--dir PATH] [--libc musl|gnu]
                  [--require-attestation | --no-attestation]

  --version   Release tag to install. Default: the latest release.
  --dir       Directory to install into. Default: \$HOME/.local/bin,
              or /usr/local/bin when running as root.
  --libc      Linux only. musl is the static build, which runs on any distribution and
              is the default when the release has one. gnu is the build linked against
              glibc, which needs a glibc as new as the release runner's.
  --require-attestation
              Refuse to install unless the GitHub CLI (gh) confirms the download
              was built by this project's release workflow. Without this flag the
              same check runs when gh can make it: a failed check still refuses,
              and only the lack of a usable gh is let through, and said.
  --no-attestation
              Skip that check and install on the checksum alone.
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
        --libc)    [ $# -ge 2 ] || die "--libc requires musl or gnu"; LIBC="$2";   shift 2 ;;
        --require-attestation) REQUIRE_ATTESTATION=1; shift ;;
        --no-attestation)      NO_ATTESTATION=1; shift ;;
        --help|-h) usage; exit 0 ;;
        *)         die "unknown option: $1 (try --help)" ;;
    esac
done

[ -z "$REQUIRE_ATTESTATION" ] || [ -z "$NO_ATTESTATION" ] ||
    die "--require-attestation and --no-attestation contradict each other"
case "$LIBC" in
    ""|musl|gnu) ;;
    *) die "--libc takes musl or gnu, not '$LIBC'" ;;
esac

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

BASE="https://github.com/$REPO/releases/download/$VERSION"

WORK="$(mktemp -d)"
# shellcheck disable=SC2064  # WORK is expanded now on purpose; it never changes.
trap "rm -rf '$WORK'" EXIT INT TERM

# Fetched before the archive, because on Linux it decides which archive. A release without it is
# refused here as it always was further down: an unverified binary is what this script is for
# avoiding.
fetch "$BASE/checksums.txt" > "$WORK/checksums.txt" 2>/dev/null && [ -s "$WORK/checksums.txt" ] ||
    die "could not fetch $BASE/checksums.txt; refusing to install an unverified binary
Check that $VERSION is a published release."

listed() {
    awk -v n="$1" '$2 == n || $2 == "*" n { found = 1 } END { exit !found }' "$WORK/checksums.txt"
}

# --- which Linux build ----------------------------------------------------------------------
# The gnu build needs a glibc at least as new as the runner that linked it -- 2.39 through
# v0.20.0 -- so on Debian 12, Ubuntu 22.04 or RHEL 9 this script installed a binary that answered
# "version \`GLIBC_2.39' not found", and on Alpine one that could not be loaded at all. The
# static build has no such floor, so it is what is installed wherever the release has one.
# Releases before the static builds existed fall back to gnu, and say which.
if [ "$os" = "Linux" ]; then
    case "$LIBC" in
        gnu) ;;
        musl)
            listed "${BIN}-${VERSION}-${SLUG}-musl.${EXT}" ||
                die "$VERSION has no static (musl) build for ${arch}; omit --libc to take the glibc one"
            SLUG="${SLUG}-musl" ;;
        *)
            if listed "${BIN}-${VERSION}-${SLUG}-musl.${EXT}"; then
                SLUG="${SLUG}-musl"
            else
                echo "    $VERSION predates the static builds; installing the one linked against glibc"
            fi ;;
    esac
fi
ARCHIVE="${BIN}-${VERSION}-${SLUG}.${EXT}"

echo "==> $BIN $VERSION for ${os}-${arch} ($SLUG)"

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

echo "==> downloading $ARCHIVE"
fetch_to "$BASE/$ARCHIVE" "$WORK/$ARCHIVE" \
    || die "download failed: $BASE/$ARCHIVE
Check that $VERSION is a published release with an asset for ${SLUG}."

# --- checksum -------------------------------------------------------------------------------
# release.yml publishes checksums.txt with bare filenames precisely so this verifies. A missing
# or mismatched entry is a hard failure: a silently unverified binary is the thing this script
# exists to avoid.
echo "==> verifying checksum"
if [ -s "$WORK/checksums.txt" ]; then
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

# --- provenance ------------------------------------------------------------------------------
# The checksum proves the archive is the one the release lists -- but checksums.txt comes from the
# same place as the archive, so whoever could replace one could replace both. A build attestation
# is signed by the release workflow's own identity and kept by GitHub apart from the release's
# files: it says this exact archive was built by this repository's release.yml. Releases carry one
# from the first release after v0.19.0.
#
# Checking it takes the GitHub CLI, signed in and recent enough, which most machines do not have.
# So nothing is refused for the lack of a tool: the check runs when it can, and says so when it
# cannot. What is refused is a *failed* check. When a working gh says that a release which should
# carry an attestation has none for this file, that is not a missing tool -- it is the case the
# attestation exists to catch -- and the first version of this step printed "do not use this
# download" and then installed it. --require-attestation makes "could not check" fatal as well;
# --no-attestation skips the step, for the one who has read this and has a reason.
ATTESTED_AFTER="0.19.0"

# Whether $VERSION is a release newer than $ATTESTED_AFTER, by its three numbers. A suffix after
# the patch number (`-rc.1`) is ignored; anything that does not parse counts as newer, because
# the safe reading of an unfamiliar tag is that it should be attested.
attested_release() {
    have="${VERSION#v}"; have="${have%%[-+]*}"
    old_ifs="$IFS"; IFS=.
    # shellcheck disable=SC2086
    set -- $have $ATTESTED_AFTER
    IFS="$old_ifs"
    [ $# -eq 6 ] || return 0
    for part in "$1" "$2" "$3"; do
        case "$part" in ''|*[!0-9]*) return 0 ;; esac
    done
    [ "$1" -gt "$4" ] && return 0; [ "$1" -lt "$4" ] && return 1
    [ "$2" -gt "$5" ] && return 0; [ "$2" -lt "$5" ] && return 1
    [ "$3" -gt "$6" ]
}

unchecked() {
    [ -z "$REQUIRE_ATTESTATION" ] || die "build provenance not checked: $1
--require-attestation was given, so nothing was installed."
    echo "    not checked: $1"
}

echo "==> checking build provenance"
if [ -n "$NO_ATTESTATION" ]; then
    echo "    skipped (--no-attestation)"
elif ! need gh; then
    unchecked "the GitHub CLI (gh) is not installed"
elif ! gh attestation verify --help 2>/dev/null | grep -q -- '--source-ref'; then
    # An older gh has no `attestation` command, or one without the flag that ties a file to a
    # tag. Its usage error is not a verdict on the download.
    unchecked "this gh is too old to verify an attestation against a tag (upgrade gh)"
elif ! gh auth status >/dev/null 2>&1; then
    unchecked "gh is not signed in (gh auth login)"
# Three things are pinned, and each closes a door: the repository; the workflow, so that no other
# workflow's attestation will do; and the tag, because the release workflow can also be run by
# hand on any branch, and that run's artifacts are attested too -- as built from that branch.
elif verdict="$(gh attestation verify "$WORK/$ARCHIVE" --repo "$REPO" \
        --signer-workflow "$REPO/.github/workflows/release.yml" \
        --source-ref "refs/tags/$VERSION" 2>&1)"; then
    echo "    ok  built by $REPO's release workflow at $VERSION"
elif attested_release; then
    echo "$verdict" | grep -v '^[[:space:]]*$' | tail -n 2 | sed 's/^/    gh: /' >&2
    die "gh could not confirm that $REPO's release workflow built $ARCHIVE at $VERSION.
Every release after v$ATTESTED_AFTER is attested, so this download is not what that workflow built --
or the check itself failed; gh's own words are above. Nothing was installed.
Re-run to try again, or pass --no-attestation to install on the checksum alone."
else
    unchecked "$VERSION predates build attestations (they start after v$ATTESTED_AFTER)"
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
echo
echo "Docs, data sources and configuration: $SITE"
