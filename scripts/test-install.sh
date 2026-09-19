#!/usr/bin/env bash
#
# Exercise scripts/install.sh against a fake `curl`, `gh` and `uname`, with a release that is a
# directory of files, so that what it prints and what it refuses are tested without a network.
#
# Two things are held here. Where nobody is watching a terminal -- a pipe, CI, TERM=dumb, --plain
# -- the output is the plain log, to the byte. And on a terminal, where the slow steps run in the
# background so that a line can be drawn while they do, a step that fails still stops the install:
# the status of a background command is easy to lose, and losing it installs an unverified binary.
#
# The terminal is script(1)'s. Without it those cases are skipped, and said to be.
#
#   scripts/test-install.sh          run the cases
#   scripts/test-install.sh --demo   watch the animation against a slow fake release
#   INSTALL_SH=/tmp/broken.sh scripts/test-install.sh
#                                    run them against another copy: break the installer in a scratch
#                                    file and watch the case that is for that bug fail
set -euo pipefail

SCRIPT="${INSTALL_SH:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/install.sh}"
failures=0
skipped=0
ESC=$'\033'

fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

VERSION="v9.9.9"
ARCHIVE="ai-usage-tui-$VERSION-x86_64-linux-musl.tar.gz"

# A release: the archive (a stub binary that answers --version, beside the README and LICENSE the
# real one carries) and its checksums.txt. `padding` makes the archive big enough to watch.
release() {
  local dir="$1" padding="${2:-0}"
  mkdir -p "$dir/tree"
  printf '#!/bin/sh\necho "ai-usage-tui %s"\n' "${VERSION#v}" >"$dir/tree/ai-usage-tui"
  chmod 755 "$dir/tree/ai-usage-tui"
  head -c "$padding" /dev/urandom >"$dir/tree/README.md"
  echo licence >"$dir/tree/LICENSE"
  tar czf "$dir/$ARCHIVE" -C "$dir/tree" ai-usage-tui README.md LICENSE
  rm -rf "$dir/tree"
  (cd "$dir" && sha256sum "$ARCHIVE" >checksums.txt)
}

install_fakes() {
  local bin="$1"
  mkdir -p "$bin"
  # FAKE_SLOW: seconds before every request. FAKE_SLOW_ARCHIVE: seconds before the archive only.
  # FAKE_TRICKLE: the archive arrives in pieces.
  #
  # It is harder on the installer than the real one, twice. It reads its stdin to the end: a step
  # handed the pipe the script itself arrives on eats the script, and one handed the terminal is
  # refused outright. And it outlives a hangup: when the installer dies, script(1) closes the
  # terminal, and the SIGHUP that follows would otherwise stop a download the installer had left
  # running -- which a terminal that stays open, as a real one does after Ctrl-C, never would.
  cat >"$bin/curl" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
trap '' HUP
if [ -t 0 ]; then echo "fake curl: was handed the terminal as stdin" >&2; exit 97; fi
cat >/dev/null
echo $$ >"$FAKE_STATE/curl.pid"
out="" url="" head=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift ;;
    -w) shift ;;
    -fsSLI) head=1 ;;
    -*) ;;
    *) url="$1" ;;
  esac
  shift
done
sleep "${FAKE_SLOW:-0}"
if [ -n "$head" ]; then
  printf 'https://github.com/SophanaSok/ai-usage-tui/releases/tag/%s' "$FAKE_VERSION"
  exit 0
fi
src="$FAKE_RELEASE/${url##*/}"
if [ ! -f "$src" ]; then
  echo "curl: (22) The requested URL returned error: 404" >&2
  exit 22
fi
case "$url" in *.tar.gz) sleep "${FAKE_SLOW_ARCHIVE:-0}" ;; esac
if [ -z "$out" ]; then
  cat "$src"
elif [ -n "${FAKE_TRICKLE:-}" ]; then
  : >"$out"
  blocks=$(( ($(wc -c <"$src") + 65535) / 65536 ))
  for ((i = 0; i < blocks; i++)); do
    dd if="$src" bs=65536 skip="$i" count=1 2>/dev/null >>"$out"
    sleep 0.04
  done
else
  cp "$src" "$out"
fi
FAKE
  # FAKE_GH: ok | fail (no attestation for the file) | unauth (not signed in) | old (no --source-ref)
  cat >"$bin/gh" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
sleep "${FAKE_SLOW:-0}"
case "$*" in
  "attestation verify --help")
    [ "$FAKE_GH" = old ] && { echo "unknown command"; exit 1; }
    echo "      --source-ref string" ;;
  "auth status") [ "$FAKE_GH" != unauth ] ;;
  "attestation verify "*)
    [ "$FAKE_GH" = ok ] && exit 0
    echo "Error: no attestations found for subject" >&2
    exit 1 ;;
  *) echo "fake gh: unhandled $*" >&2; exit 1 ;;
esac
FAKE
  cat >"$bin/uname" <<'FAKE'
#!/bin/sh
case "$1" in -s) echo Linux ;; -m) echo x86_64 ;; *) exec /usr/bin/uname "$@" ;; esac
FAKE
  chmod 755 "$bin/curl" "$bin/gh" "$bin/uname"
}

# Every run starts from an empty environment: what the installer decides from TERM, CI, NO_COLOR
# and LANG is what is under test, so none of them may come from whoever runs this.
#   new_case; launcher [VAR=value ...] -- [install.sh arguments]
# writes $CASE/run, which runs the installer under $SH with the scratch bin directory on PATH.
new_case() {
  CASE="$(mktemp -d "$SCRATCH/case.XXXXXX")"
  DEST="$CASE/dest"
  mkdir -p "$CASE/tmp" "$CASE/state" "$CASE/home"
  install_fakes "$CASE/bin"
}

launcher() {
  local vars=() feed=""
  while [ "$1" != "--" ]; do
    if [ "$1" = STDIN ]; then feed=1; else vars+=("$1"); fi
    shift
  done
  shift
  {
    echo '#!/usr/bin/env bash'
    echo "echo \$\$ >'$CASE/state/sh.pid'"
    printf 'exec env -i PATH=%q HOME=%q TMPDIR=%q LANG=C.UTF-8 TERM=xterm-256color' \
      "$DEST:$CASE/bin:/usr/bin:/bin" "$CASE/home" "$CASE/tmp"
    printf ' FAKE_STATE=%q FAKE_RELEASE=%q FAKE_VERSION=%q FAKE_GH=%q' \
      "$CASE/state" "${RELEASE_DIR:-$SCRATCH/release}" "$VERSION" "${GH:-unauth}"
    [ "${#vars[@]}" -eq 0 ] || printf ' %q' "${vars[@]}"
    if [ -n "$feed" ]; then
      # As `curl ... | sh` has it: the script arrives on stdin.
      printf ' %s -c %q' "$SH" "cat '$SCRIPT' | $SH -s -- --dir '$DEST' $*"
    else
      printf ' %s %q --dir %q' "$SH" "$SCRIPT" "$DEST"
      [ $# -eq 0 ] || printf ' %q' "$@"
    fi
    echo
  } >"$CASE/run"
  chmod 755 "$CASE/run"
}

# Not a terminal: stdout and stderr are files. Sets STATUS.
run_piped() {
  STATUS=0
  "$CASE/run" >"$CASE/out" 2>"$CASE/err" </dev/null || STATUS=$?
}

# A terminal: everything the installer writes lands in $CASE/out, with the pty's \r\n line ends.
run_tty() {
  STATUS=0
  script -qec "$CASE/run" /dev/null >"$CASE/out" </dev/null || STATUS=$?
}

expected_plain() {
  local sum
  sum="$(awk '{ print $1 }' "${RELEASE_DIR:-$SCRATCH/release}/checksums.txt")"
  cat <<EOF
==> ai-usage-tui $VERSION for Linux-x86_64 (x86_64-linux-musl)
==> downloading $ARCHIVE
==> verifying checksum
    ok  $sum
==> checking build provenance
    not checked: gh is not signed in (gh auth login)
==> installed $DEST/ai-usage-tui
==> run it with: ai-usage-tui

Docs, data sources and configuration: https://sophanasok.github.io/ai-usage-tui-site/
EOF
}

is_plain() { # $1: what the case is called
  if ! diff <(expected_plain) <(tr -d '\r' <"$CASE/out") >"$CASE/diff"; then
    fail "$1: the output is not the plain log"; cat "$CASE/diff"
  fi
  if grep -q "$ESC" "$CASE/out"; then fail "$1: an escape sequence reached the plain log"; fi
}

installed() { [ -x "$DEST/ai-usage-tui" ]; }
cursor_restored() { # the last thing said about the cursor is "show"
  [ "$(grep -ao "$ESC\[?25[lh]" "$CASE/out" | tail -n 1)" = "${ESC}[?25h" ]
}
scratch_removed() { [ -z "$(ls -A "$CASE/tmp")" ]; }

release "$SCRATCH/release"
release "$SCRATCH/tampered"
echo "0000000000000000000000000000000000000000000000000000000000000000  $ARCHIVE" \
  >"$SCRATCH/tampered/checksums.txt"

if [ "${1:-}" = "--demo" ]; then
  release "$SCRATCH/big" 3500000
  SH=sh RELEASE_DIR="$SCRATCH/big" GH=ok
  new_case
  launcher FAKE_SLOW=0.7 FAKE_TRICKLE=1 "TERM=${TERM:-xterm}" "COLORTERM=${COLORTERM:-}" \
    "NO_COLOR=${NO_COLOR:-}" "LANG=${LANG:-C.UTF-8}" -- "${@:2}"
  exec "$CASE/run"
fi

have_tty=1
if ! command -v script >/dev/null 2>&1 || ! script -qec true /dev/null >/dev/null 2>&1 </dev/null; then
  have_tty=""
fi

# `sh` is whatever this machine's is -- dash on the CI runner, bash on others -- and the stricter
# ones are added where they exist. By full path: the installer runs with a PATH of its own.
shells=(sh)
if command -v dash >/dev/null 2>&1; then shells+=("$(command -v dash)"); fi
if command -v busybox >/dev/null 2>&1; then shells+=("$(command -v busybox) sh"); fi

for SH in "${shells[@]}"; do
  RELEASE_DIR="$SCRATCH/release" GH=unauth

  # 1. Not a terminal: the plain log, to the byte, and the binary installed.
  new_case; launcher --; run_piped
  [ "$STATUS" -eq 0 ] || { fail "[$SH] piped: exit $STATUS"; cat "$CASE/err"; }
  is_plain "[$SH] piped"
  installed || fail "[$SH] piped: nothing was installed"
  scratch_removed || fail "[$SH] piped: the scratch directory was left behind"

  # The refusals do not depend on a terminal either.
  new_case; RELEASE_DIR="$SCRATCH/tampered" launcher --; run_piped
  { [ "$STATUS" -ne 0 ] && ! installed; } || fail "[$SH] piped: a checksum mismatch was installed"
  grep -q "checksum mismatch" "$CASE/err" || fail "[$SH] piped: the mismatch was not named"

  new_case; launcher STDIN --; run_piped
  { [ "$STATUS" -eq 0 ] && installed; } || { fail "[$SH] piped, script on stdin: exit $STATUS"; cat "$CASE/err"; }

  if [ -z "$have_tty" ]; then
    echo "SKIP: [$SH] the terminal cases -- script(1) from util-linux is not here"
    skipped=$((skipped + 1))
    continue
  fi

  # 2. A terminal: drawn, in colour, the cursor given back, and the same binary installed.
  new_case; launcher --; run_tty
  [ "$STATUS" -eq 0 ] || { fail "[$SH] tty: exit $STATUS"; cat "$CASE/out"; }
  installed || fail "[$SH] tty: nothing was installed"
  grep -q "$ESC\[[0-9;]*m" "$CASE/out" || fail "[$SH] tty: no colour was drawn"
  cursor_restored || fail "[$SH] tty: the cursor was left hidden"
  scratch_removed || fail "[$SH] tty: the scratch directory was left behind"
  # Nothing the plain log says may go missing from the drawn one.
  for said in "$VERSION" "$ARCHIVE" "$(awk '{ print $1 }' "$SCRATCH/release/checksums.txt")" \
    "NOT CHECKED" "gh is not signed in (gh auth login)" "$DEST/ai-usage-tui" "run it with: ai-usage-tui"; do
    grep -qF -- "$said" "$CASE/out" || fail "[$SH] tty: the drawn log never says '$said'"
  done

  # 3. NO_COLOR keeps the movement and drops the colour; the rest ask for the plain log.
  new_case; launcher NO_COLOR=1 --; run_tty
  { [ "$STATUS" -eq 0 ] && installed; } || fail "[$SH] tty NO_COLOR: exit $STATUS"
  if grep -q "$ESC\[[0-9;]*m" "$CASE/out"; then fail "[$SH] tty NO_COLOR: colour was drawn"; fi
  grep -q "\[ OK \]" "$CASE/out" || fail "[$SH] tty NO_COLOR: the statuses went with the colour"

  new_case; launcher CI=true --; run_tty; is_plain "[$SH] tty CI=true"
  new_case; launcher TERM=dumb --; run_tty; is_plain "[$SH] tty TERM=dumb"
  new_case; launcher -- --plain; run_tty; is_plain "[$SH] tty --plain"

  # 4. The steps run in the background here, and their failures still refuse.
  new_case; RELEASE_DIR="$SCRATCH/tampered" launcher --; run_tty
  { [ "$STATUS" -ne 0 ] && ! installed; } || fail "[$SH] tty: a checksum mismatch was installed"
  grep -q "checksum mismatch" "$CASE/out" || fail "[$SH] tty: the mismatch was not named"
  grep -q "\[FAIL\]" "$CASE/out" || fail "[$SH] tty: the failed step was not marked"

  # 5. A download that fails, and an attestation that a working gh does not find.
  new_case; launcher -- --libc gnu; run_tty
  { [ "$STATUS" -ne 0 ] && ! installed; } || fail "[$SH] tty: a failed download was installed"
  grep -q "download failed" "$CASE/out" || fail "[$SH] tty: the failed download was not named"
  grep -q "curl: (22)" "$CASE/out" || fail "[$SH] tty: curl's own reason was lost"
  cursor_restored || fail "[$SH] tty: a failed download left the cursor hidden"

  new_case; GH=fail launcher --; run_tty
  { [ "$STATUS" -ne 0 ] && ! installed; } || fail "[$SH] tty: a failed attestation was installed"
  grep -q "gh: Error: no attestations found" "$CASE/out" || fail "[$SH] tty: gh's own words were lost"

  new_case; GH=ok launcher --; run_tty
  { [ "$STATUS" -eq 0 ] && installed; } || fail "[$SH] tty: a good attestation was refused"
  grep -q "built by SophanaSok/ai-usage-tui's release workflow" "$CASE/out" ||
    fail "[$SH] tty: the attestation was not reported"

  # 6. As it is really run: the script arriving on stdin, while background steps are running.
  new_case; launcher STDIN FAKE_SLOW=0.2 --; run_tty
  { [ "$STATUS" -eq 0 ] && installed; } || { fail "[$SH] tty, script on stdin: exit $STATUS"; cat "$CASE/out"; }

  # 7. An interrupt mid-download stops the download, removes the scratch directory and gives the
  # cursor back. A background command ignores SIGINT, so this is the installer's doing or nobody's.
  new_case; launcher FAKE_SLOW_ARCHIVE=6 --
  # Under job control, because without it bash starts a background command with SIGINT ignored,
  # and a shell cannot trap a signal that was ignored when it started.
  set -m
  script -qec "$CASE/run" /dev/null >"$CASE/out" </dev/null &
  waiter=$!
  set +m
  for _ in $(seq 1 100); do
    if [ -n "$(ls "$CASE/tmp"/*/*.tar.gz 2>/dev/null)" ] || grep -q DOWNLOAD "$CASE/out" 2>/dev/null; then break; fi
    sleep 0.1
  done
  sleep 0.3
  download="$(cat "$CASE/state/curl.pid")"
  # To the whole foreground process group, as Ctrl-C is. A shell that alone receives SIGINT while
  # it waits on a command takes the command to have handled it, and runs no trap.
  kill -INT -- "-$(ps -o pgid= -p "$(cat "$CASE/state/sh.pid")" | tr -d ' ')"
  wait "$waiter" || true
  sleep 0.2
  if kill -0 "$download" 2>/dev/null; then fail "[$SH] interrupt: the download was left running"; kill "$download"; fi
  if installed; then fail "[$SH] interrupt: it installed anyway"; fi
  scratch_removed || fail "[$SH] interrupt: the scratch directory was left behind"
  cursor_restored || fail "[$SH] interrupt: the cursor was left hidden"
done

if [ "$failures" -eq 0 ]; then
  echo "install: all cases pass (${shells[*]}), $skipped skipped"
else
  echo "install: $failures failure(s)"
  exit 1
fi
