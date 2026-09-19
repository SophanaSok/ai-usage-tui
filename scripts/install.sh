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
#
# On a terminal the steps are drawn as a boot sequence: one line per step, a scanner sweeping
# while the step really runs. Every line is a step this script took and every figure on it is one
# it measured -- there is no filler and no percentage, because the total is not known. Anywhere
# else (a pipe, a log, CI, TERM=dumb, --plain) the output is the plain log, byte for byte what it
# was before the animation existed; scripts/test-install.sh holds it to that.
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
PLAIN=""
# The drawing state, set here because `die` and the exit trap read it from the first line on.
ANIMATE=""
WORK=""
HUD_OPEN=""
HUD_PID=""
HUD_WATCH=""

usage() {
    cat <<EOF
Install a prebuilt $BIN release.

Usage: install.sh [--version vX.Y.Z] [--dir PATH] [--libc musl|gnu]
                  [--require-attestation | --no-attestation] [--plain]

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
  --plain     The plain log, without the animation or colour. It is also what a pipe, CI,
              or TERM=dumb gets; NO_COLOR keeps the animation and drops the colour.
  --help      Show this message.
EOF
}

die() {
    hud_fail
    echo "install.sh: $*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1
}

# --- the boot sequence ----------------------------------------------------------------------
# Every function here does nothing unless ANIMATE is set, and `plain` prints only when it is not,
# so the two outputs sit side by side below and neither can leak into the other.
plain() {
    [ -n "$ANIMATE" ] || echo "$@"
}

# One finished or waiting line, without its newline:
#   "  LABEL       detail ......... value  STATUS"
# $1 label, $2 detail, $3 ok|warn|fail|"" (the status colour), $4 status, $5 value.
hud_static() {
    case "$3" in
        ok)   c_kind="$C_OK" ;;
        warn) c_kind="$C_WARN" ;;
        fail) c_kind="$C_FAIL" ;;
        *)    c_kind="" ;;
    esac
    c_right="${5:-}"
    [ -z "$4" ] || c_right="${c_right:+$c_right  }$4"
    c_room=$((HUD_W - 17 - ${#c_right}))
    [ "$c_room" -ge 8 ] || c_room=8
    c_detail="$(printf "%.${c_room}s" "$2")"
    c_dots=""
    [ -z "$c_right" ] || c_dots="$(printf "%.$((c_room - ${#c_detail}))s" "$HUD_DOTS")"
    printf '\r  %s%-12.12s%s%s %s%s%s %s%s%s%s%s%s%s%s' \
        "$C_LABEL" "$1" "$C_RESET" "$c_detail" "$C_DIM" "$c_dots" "$C_RESET" \
        "$C_VALUE" "${5:-}" "$C_RESET" "${5:+  }" "$c_kind" "$4" "$C_RESET" "${ESC}[K"
}

# A step that runs in the foreground: drawn waiting, then closed by hud_result -- or by `die`,
# which is why a failure needs no drawing of its own.
hud_open() {
    [ -n "$ANIMATE" ] || return 0
    HUD_OPEN=1; HUD_LABEL="$1"; HUD_DETAIL="$2"
    hud_static "$1" "$2" "" "" ""
}

# Closes the open line, or prints a new one when none is open. Arguments as hud_static.
hud_result() {
    [ -n "$ANIMATE" ] || return 0
    hud_static "$@"
    printf '\n'
    HUD_OPEN=""
    # A detail too long for its line was cut to fit. The plain log says it whole, so this does too.
    [ "${#2}" -le "$c_room" ] || hud_note "$2"
}

hud_fail() {
    [ -z "$HUD_OPEN" ] || hud_result "$HUD_LABEL" "$HUD_DETAIL" fail "[FAIL]"
}

# A detail under the line above it, in the plain log's own words.
hud_note() {
    [ -n "$ANIMATE" ] || return 0
    printf '              %s%s%s\n' "$C_DIM" "$1" "$C_RESET"
}

hud_banner() {
    [ -n "$ANIMATE" ] || return 0
    b_head="$G1$G1 AI-USAGE-TUI $G1$G1 INSTALL SEQUENCE "
    # Counted by hand rather than with ${#b_head}: a shell that counts bytes would count each
    # block glyph as three.
    b_n=$((HUD_W - 38))
    b_tail=""
    while [ "$b_n" -gt 0 ]; do b_tail="$b_tail$G1"; b_n=$((b_n - 1)); done
    printf '  %s%s%s%s\n' "$C_LABEL" "$b_head" "$b_tail" "$C_RESET"
}

# A byte count a person can read. Shell arithmetic, so it costs no process per frame; rounded
# down, because a download that says 2.1 MB has received at least that.
hud_human() {
    if [ "$1" -lt 1024 ]; then HUD_HUMAN="$1 B"
    elif [ "$1" -lt 1048576 ]; then HUD_HUMAN="$(($1 / 1024)) KB"
    else HUD_HUMAN="$(($1 / 1048576)).$((($1 % 1048576) * 10 / 1048576)) MB"
    fi
}

hud_size() {
    h_bytes="$(wc -c < "$1" 2>/dev/null || true)"
    hud_human "$((${h_bytes:-0} + 0))"
}

# The line of a step that is still running: the detail typing itself out over the first frames,
# then a scanner sweeping, and beside it the bytes received when the step has a file to watch.
hud_running() {
    r_period=$((2 * (HUD_SCAN_W - 1)))
    r_pos=$((hud_frame % r_period))
    [ "$r_pos" -lt "$HUD_SCAN_W" ] || r_pos=$((r_period - r_pos))
    r_scan=""
    r_i=0
    while [ "$r_i" -lt "$HUD_SCAN_W" ]; do
        r_d=$((r_i - r_pos))
        [ "$r_d" -ge 0 ] || r_d=$((0 - r_d))
        case "$r_d" in
            0) r_scan="$r_scan$C_S0$G0" ;;
            1) r_scan="$r_scan$C_S1$G1" ;;
            2) r_scan="$r_scan$C_S2$G2" ;;
            3) r_scan="$r_scan$C_S3$G3" ;;
            *) r_scan="$r_scan$C_S4$G4" ;;
        esac
        r_i=$((r_i + 1))
    done
    r_room=$((HUD_W - 17 - HUD_SCAN_W - 11))
    r_typed=$(((hud_frame + 1) * 6))
    [ "$r_typed" -le "$r_room" ] || r_typed="$r_room"
    HUD_HUMAN=""
    [ -z "$HUD_WATCH" ] || [ ! -f "$HUD_WATCH" ] || hud_size "$HUD_WATCH"
    printf "\\r  %s%-12.12s%s%-${r_room}.${r_typed}s %s%s  %s%9s%s%s" \
        "$C_LABEL" "$HUD_LABEL" "$C_RESET" "$HUD_DETAIL" "$r_scan" "$C_RESET" \
        "$C_VALUE" "$HUD_HUMAN" "$C_RESET" "${ESC}[K"
}

# run_step LABEL DETAIL command...
#
# Runs the command with its stdout in $WORK/step.out and its stderr in $WORK/step.err, and returns
# the command's own status: the caller's `|| die` decides what a failure means, exactly as it did
# when these steps ran in the foreground. On a terminal the command runs in the background while
# the line is drawn, and the line is left open for hud_result to close.
#
# The command runs in a subshell in both modes, so that it may `exec` the tool it wraps. That is
# what makes the background process *be* curl or gh rather than a shell waiting on one, and so
# what lets an interrupt stop the download instead of orphaning it. A shell without job control
# starts a background command with SIGINT ignored and its stdin on /dev/null: the first is why
# `cleanup` has to kill it, and the second is why it cannot swallow the rest of this script when
# the script is arriving on a pipe from curl.
run_step() {
    step_label="$1"; step_detail="$2"
    shift 2
    if [ -z "$ANIMATE" ]; then
        ( "$@" ) < /dev/null > "$WORK/step.out" 2> "$WORK/step.err"
        return
    fi
    # A second step on a line that is already open carries on drawing it, not typing it again.
    [ -n "$HUD_OPEN" ] || hud_frame=0
    HUD_OPEN=1; HUD_LABEL="$step_label"; HUD_DETAIL="$step_detail"
    ( "$@" ) < /dev/null > "$WORK/step.out" 2> "$WORK/step.err" &
    HUD_PID=$!
    printf '%s' "${ESC}[?25l"
    while kill -0 "$HUD_PID" 2>/dev/null; do
        hud_running
        hud_frame=$((hud_frame + 1))
        sleep 0.1
    done
    printf '%s' "${ESC}[?25h"
    # The status is the point of this function. Lose it and a failed download, or a failed
    # attestation, installs.
    step_rc=0
    wait "$HUD_PID" || step_rc=$?
    HUD_PID=""
    return "$step_rc"
}

cleanup() {
    [ -z "$HUD_PID" ] || kill "$HUD_PID" 2>/dev/null || true
    if [ -n "$ANIMATE" ]; then
        [ -z "$HUD_OPEN" ] || printf '\n'
        printf '%s' "$C_RESET${ESC}[?25h"
    fi
    [ -z "$WORK" ] || rm -rf "$WORK"
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version) [ $# -ge 2 ] || die "--version requires a tag"; VERSION="$2"; shift 2 ;;
        --dir)     [ $# -ge 2 ] || die "--dir requires a path";    DEST="$2";    shift 2 ;;
        --libc)    [ $# -ge 2 ] || die "--libc requires musl or gnu"; LIBC="$2";   shift 2 ;;
        --require-attestation) REQUIRE_ATTESTATION=1; shift ;;
        --no-attestation)      NO_ATTESTATION=1; shift ;;
        --plain)               PLAIN=1; shift ;;
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
# These `exec`, so they are only ever called through run_step, which gives them a subshell to
# replace. Called directly, one would replace this script.
if need curl; then
    fetch_to() { exec curl -fsSL -o "$2" "$1"; }
    # The redirect target of /releases/latest is the tag, which avoids depending on the API's
    # rate limit or on a JSON parser being present.
    latest_location() {
        exec curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest"
    }
elif need wget; then
    fetch_to() { exec wget -qO "$2" "$1"; }
    latest_location() {
        wget -qS --max-redirect=10 -O /dev/null "https://github.com/$REPO/releases/latest" 2>&1 \
            | awk '/^ *Location:/ { print $2 }' | tail -1
    }
else
    die "curl or wget is required"
fi

WORK="$(mktemp -d)"
trap cleanup EXIT
# `exit` runs the EXIT trap, so an interrupt cleans up once and then really stops. It used to
# remove the scratch directory and carry on, to fail at whatever came next.
trap 'exit 130' INT TERM

# --- how to draw ----------------------------------------------------------------------------
# Animated only where a person is watching a terminal that can redraw a line. NO_COLOR asks for no
# colour, not for no movement, so it keeps the animation: every status is a word as well as a
# colour. The probe is `sleep 0.01` because a fractional sleep is not POSIX, and a frame a second
# is worse than the plain log.
if [ -z "$PLAIN" ] && [ -t 1 ] && [ -z "${CI:-}" ] && [ "${TERM:-dumb}" != "dumb" ] &&
    sleep 0.01 2>/dev/null; then
    # stdin is the pipe from curl, so the terminal is asked for by name. A width that cannot be
    # read -- or reads as 0, as under script(1) -- is taken to be 80.
    cols="$( (stty size < /dev/tty) 2>/dev/null | awk '{ print $2 }' || true)"
    case "$cols" in ''|0|*[!0-9]*) cols=80 ;; esac
    # A line that wraps cannot be redrawn in place.
    if [ "$cols" -ge 64 ]; then
        ANIMATE=1
        HUD_W=$((cols - 2))
        [ "$HUD_W" -le 78 ] || HUD_W=78
    fi
fi

ESC="$(printf '\033')"
HUD_SCAN_W=12
HUD_DOTS="................................................................................"
C_RESET=""; C_DIM=""; C_LABEL=""; C_VALUE=""; C_OK=""; C_WARN=""; C_FAIL=""
# The scanner's five shades. Each resets first, so one cell's dim cannot leak into the next.
C_S0=""; C_S1=""; C_S2=""; C_S3=""; C_S4=""
if [ -n "$ANIMATE" ] && [ -z "${NO_COLOR:-}" ]; then
    C_RESET="${ESC}[0m"; C_DIM="${ESC}[2m"; C_LABEL="${ESC}[1;31m"; C_VALUE="${ESC}[1;37m"
    C_OK="${ESC}[1;32m"; C_WARN="${ESC}[1;33m"; C_FAIL="${ESC}[1;91m"
    case "${COLORTERM:+256color}${TERM:-}" in
        *256color*|*-direct)
            C_S0="${ESC}[0;38;5;196m"; C_S1="${ESC}[0;38;5;160m"
            C_S2="${ESC}[0;38;5;124m"; C_S3="${ESC}[0;38;5;88m" ;;
        *)
            C_S0="${ESC}[0;1;91m"; C_S1="${ESC}[0;91m"; C_S2="${ESC}[0;31m"; C_S3="${ESC}[0;2;31m" ;;
    esac
    C_S4="${ESC}[0;2m"
fi
case "${LC_ALL:-${LC_CTYPE:-${LANG:-}}}" in
    *[Uu][Tt][Ff]-8*|*[Uu][Tt][Ff]8*) G0="█"; G1="▓"; G2="▒"; G3="░"; G4="·" ;;
    *)                                 G0="#"; G1="="; G2="-"; G3="."; G4=" " ;;
esac

hud_banner

# --- version --------------------------------------------------------------------------------
if [ -z "$VERSION" ]; then
    run_step RESOLVE "latest release" latest_location || true
    location="$(cat "$WORK/step.out")"
    VERSION="${location##*/}"
    case "$VERSION" in
        v[0-9]*) ;;
        *) die "could not determine the latest release tag; pass --version vX.Y.Z" ;;
    esac
    hud_result RESOLVE "latest release" ok "[ OK ]" "$VERSION"
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

# Fetched before the archive, because on Linux it decides which archive. A release without it is
# refused here as it always was further down: an unverified binary is what this script is for
# avoiding.
if ! run_step MANIFEST "checksums.txt" fetch_to "$BASE/checksums.txt" "$WORK/checksums.txt" ||
    [ ! -s "$WORK/checksums.txt" ]; then
    die "could not fetch $BASE/checksums.txt; refusing to install an unverified binary
Check that $VERSION is a published release."
fi
hud_result MANIFEST "checksums.txt" ok "[ OK ]"

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
                predates_static=1
            fi ;;
    esac
fi
ARCHIVE="${BIN}-${VERSION}-${SLUG}.${EXT}"

[ -z "${predates_static:-}" ] ||
    plain "    $VERSION predates the static builds; installing the one linked against glibc"
plain "==> $BIN $VERSION for ${os}-${arch} ($SLUG)"
hud_result TARGET "$VERSION  ${os}-${arch}  $SLUG" "" ""
[ -z "${predates_static:-}" ] ||
    hud_note "$VERSION predates the static builds; installing the one linked against glibc"

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
        replacing="replacing $DEST/$BIN (its version could not be read)"
    elif [ "$previous" = "${VERSION#v}" ]; then
        replacing="reinstalling $previous over $DEST/$BIN"
    else
        # Deliberately not "upgrading" or "downgrading": ordering two versions correctly needs
        # more than string comparison, and `sort -V` is not POSIX. Both are named instead, which
        # cannot be wrong.
        replacing="replacing $previous at $DEST/$BIN"
    fi
    plain "    $replacing"
    hud_note "$replacing"
fi

plain "==> downloading $ARCHIVE"
HUD_WATCH="$WORK/$ARCHIVE"
if ! run_step DOWNLOAD "$ARCHIVE" fetch_to "$BASE/$ARCHIVE" "$WORK/$ARCHIVE"; then
    hud_fail
    # curl's own reason, which it used to write straight to the terminal.
    cat "$WORK/step.err" >&2
    die "download failed: $BASE/$ARCHIVE
Check that $VERSION is a published release with an asset for ${SLUG}."
fi
HUD_WATCH=""
hud_size "$WORK/$ARCHIVE"
hud_result DOWNLOAD "$ARCHIVE" ok "[ OK ]" "$HUD_HUMAN"

# --- checksum -------------------------------------------------------------------------------
# release.yml publishes checksums.txt with bare filenames precisely so this verifies. A missing
# or mismatched entry is a hard failure: a silently unverified binary is the thing this script
# exists to avoid.
plain "==> verifying checksum"
hud_open CHECKSUM "sha256"
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
    plain "    ok  $actual"
    hud_result CHECKSUM "sha256" ok "[ OK ]"
    hud_note "$actual"
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

PROVENANCE="release.yml @ $VERSION"

unchecked() {
    [ -z "$REQUIRE_ATTESTATION" ] || die "build provenance not checked: $1
--require-attestation was given, so nothing was installed."
    plain "    not checked: $1"
    hud_result PROVENANCE "$PROVENANCE" warn "NOT CHECKED"
    hud_note "$1"
}

# Whether this gh can make the check at all: 3 when it cannot verify against a tag, 4 when it is
# not signed in. `gh auth status` asks GitHub, which is why this is a step with a line of its own.
gh_usable() {
    # An older gh has no `attestation` command, or one without the flag that ties a file to a
    # tag. Its usage error is not a verdict on the download.
    gh attestation verify --help 2>/dev/null | grep -q -- '--source-ref' || return 3
    gh auth status >/dev/null 2>&1 || return 4
}

# Three things are pinned, and each closes a door: the repository; the workflow, so that no other
# workflow's attestation will do; and the tag, because the release workflow can also be run by
# hand on any branch, and that run's artifacts are attested too -- as built from that branch.
verify_attestation() {
    exec gh attestation verify "$WORK/$ARCHIVE" --repo "$REPO" \
        --signer-workflow "$REPO/.github/workflows/release.yml" \
        --source-ref "refs/tags/$VERSION" 2>&1
}

plain "==> checking build provenance"
gh_state=0
if [ -z "$NO_ATTESTATION" ] && need gh; then
    run_step PROVENANCE "$PROVENANCE" gh_usable || gh_state=$?
fi
if [ -n "$NO_ATTESTATION" ]; then
    plain "    skipped (--no-attestation)"
    hud_result PROVENANCE "$PROVENANCE" warn "SKIPPED"
    hud_note "--no-attestation"
elif ! need gh; then
    unchecked "the GitHub CLI (gh) is not installed"
elif [ "$gh_state" -eq 3 ]; then
    unchecked "this gh is too old to verify an attestation against a tag (upgrade gh)"
elif [ "$gh_state" -ne 0 ]; then
    unchecked "gh is not signed in (gh auth login)"
elif run_step PROVENANCE "$PROVENANCE" verify_attestation; then
    plain "    ok  built by $REPO's release workflow at $VERSION"
    hud_result PROVENANCE "$PROVENANCE" ok "[ OK ]"
    hud_note "built by $REPO's release workflow at $VERSION"
elif attested_release; then
    hud_fail
    grep -v '^[[:space:]]*$' "$WORK/step.out" | tail -n 2 | sed 's/^/    gh: /' >&2
    die "gh could not confirm that $REPO's release workflow built $ARCHIVE at $VERSION.
Every release after v$ATTESTED_AFTER is attested, so this download is not what that workflow built --
or the check itself failed; gh's own words are above. Nothing was installed.
Re-run to try again, or pass --no-attestation to install on the checksum alone."
else
    unchecked "$VERSION predates build attestations (they start after v$ATTESTED_AFTER)"
fi

# --- install --------------------------------------------------------------------------------
# Into a scratch directory: the archive carries README.md and LICENSE beside the binary.
hud_open INSTALL "$DEST/$BIN"
tar xzf "$WORK/$ARCHIVE" -C "$WORK"
[ -f "$WORK/$BIN" ] || die "$ARCHIVE did not contain $BIN"

mkdir -p "$DEST" || die "could not create $DEST"
if [ -w "$DEST" ]; then
    install -m 755 "$WORK/$BIN" "$DEST/$BIN"
elif need sudo; then
    plain "==> $DEST is not writable; using sudo"
    # Closed before sudo runs: its password prompt needs a line of its own.
    hud_result INSTALL "$DEST/$BIN" warn "SUDO"
    hud_note "$DEST is not writable; using sudo"
    sudo install -m 755 "$WORK/$BIN" "$DEST/$BIN"
else
    die "$DEST is not writable and sudo is not available; pass --dir PATH"
fi

plain "==> installed $DEST/$BIN"
hud_result INSTALL "$DEST/$BIN" ok "[ OK ]"

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
            echo "${C_LABEL}==>${C_RESET} run it with: $BIN"
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
