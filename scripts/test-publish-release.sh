#!/usr/bin/env bash
#
# Exercise scripts/publish-release.sh against a fake `gh` and a fake `curl`, so the retry,
# verification and draft-until-confirmed logic is tested without a GitHub release to break.
#
# The fakes keep the release and its assets as files in a scratch directory and misbehave on
# request: `FAULTS` lines of `<asset name> <outcome>...` give the outcome of each successive upload
# of that asset -- `starter` (left half-finished, as GitHub did on v0.17.0), `short` (reported
# uploaded, wrong size, success exit), `drop` (nothing recorded, failure exit) or `ok`. Past the
# listed outcomes an upload succeeds.
set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/publish-release.sh"
failures=0

fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
fresh_dir() { mktemp -d "$SCRATCH/case.XXXXXX"; }

# A workspace shaped like the release job's: two archives, checksums, the bill of materials, the
# rendered manifests.
workspace() {
  local dir="$1"
  mkdir -p "$dir"/artifacts/ai-usage-tui-v9.9.9-x86_64-linux "$dir"/artifacts/ai-usage-tui-v9.9.9-x86_64-windows \
    "$dir"/rendered/homebrew "$dir"/rendered/scoop "$dir"/rendered/chocolatey/tools "$dir"/rendered/aur \
    "$dir"/sbom
  echo '{"bomFormat":"CycloneDX"}' >"$dir/sbom/ai-usage-tui-v9.9.9.cdx.json"
  head -c 4000 /dev/urandom >"$dir/artifacts/ai-usage-tui-v9.9.9-x86_64-linux/ai-usage-tui-v9.9.9-x86_64-linux.tar.gz"
  head -c 3000 /dev/urandom >"$dir/artifacts/ai-usage-tui-v9.9.9-x86_64-windows/ai-usage-tui-v9.9.9-x86_64-windows.zip"
  (cd "$dir/artifacts" && sha256sum ./*/ai-usage-tui-* | sed 's#  \./[^/]*/#  #' >checksums.txt)
  for manifest in homebrew/ai-usage-tui.rb scoop/ai-usage-tui.json chocolatey/ai-usage-tui.nuspec \
    chocolatey/tools/chocolateyinstall.ps1 aur/PKGBUILD; do
    echo "manifest $manifest" >"$dir/rendered/$manifest"
  done
  echo "release notes" >"$dir/body.txt"
}

# The fakes. State: `release` holds `<id> <draft>`; `assets` holds `<id>\t<name>\t<state>\t<size>`.
install_fakes() {
  local bin="$1"
  mkdir -p "$bin"
  cat >"$bin/gh" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
S="$FAKE_STATE"
touch "$S/assets" "$S/calls"
echo "gh $*" >>"$S/calls"
method=GET
args=()
while [ $# -gt 0 ]; do
  case "$1" in
    api|--paginate|--silent) ;;
    -X) method="$2"; shift ;;
    --jq|-f|-F) args+=("$1" "$2"); shift ;;
    *) args+=("$1") ;;
  esac
  shift
done
path=""
for a in "${args[@]}"; do case "$a" in repos/*) path="$a" ;; esac; done
case "$method $path" in
  "GET "*/releases/*/assets*) cat "$S/assets" ;;
  "GET "*/releases\?*)
    [ -f "$S/release" ] && { read -r id draft <"$S/release"; printf '%s\t%s\n' "$id" "$draft"; }
    ;;
  "POST "*/releases) echo "4242 true" >"$S/release"; echo 4242 ;;
  "DELETE "*/releases/assets/*)
    id="${path##*/}"
    awk -F'\t' -v id="$id" '$1 != id' "$S/assets" >"$S/assets.new"
    mv "$S/assets.new" "$S/assets"
    ;;
  "PATCH "*/releases/*) read -r id _ <"$S/release"; echo "$id false" >"$S/release" ;;
  *) echo "fake gh: unhandled $method $path" >&2; exit 1 ;;
esac
FAKE
  cat >"$bin/curl" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
S="$FAKE_STATE"
file="" url=""
while [ $# -gt 0 ]; do
  case "$1" in
    --data-binary) file="${2#@}"; shift ;;
    --max-time|--config|-H) shift ;;
    http*) url="$1" ;;
  esac
  shift
done
name="${url##*name=}"
size="$(wc -c <"$file" | tr -d ' ')"
echo "upload $name" >>"$S/calls"
seen="$(grep -c "^upload $name\$" "$S/calls")"
outcome="$(awk -v n="$name" -v k="$seen" '$1 == n { print $(k + 1) }' "$S/faults" 2>/dev/null)"
id=$((RANDOM + 100000))
case "${outcome:-ok}" in
  ok) printf '%s\t%s\tuploaded\t%s\n' "$id" "$name" "$size" >>"$S/assets" ;;
  starter) printf '%s\t%s\tstarter\t%s\n' "$id" "$name" "$size" >>"$S/assets"; exit 22 ;;
  short) printf '%s\t%s\tuploaded\t%s\n' "$id" "$name" "$((size - 1))" >>"$S/assets" ;;
  drop) exit 28 ;;
esac
FAKE
  chmod +x "$bin/gh" "$bin/curl"
}

# Run the publisher in a fresh workspace with the given faults; leaves `$STATE` for assertions.
run_case() {
  local faults="$1"
  CASE_DIR="$(fresh_dir)"
  STATE="$CASE_DIR/state"
  mkdir -p "$STATE"
  workspace "$CASE_DIR/work"
  install_fakes "$CASE_DIR/bin"
  printf '%s\n' "$faults" >"$STATE/faults"
  (
    cd "$CASE_DIR/work"
    PATH="$CASE_DIR/bin:$PATH" FAKE_STATE="$STATE" GITHUB_REPOSITORY=o/r GH_TOKEN=t \
      PUBLISH_RETRY_DELAY=0 "$SCRIPT" --publish v9.9.9
  ) >"$CASE_DIR/out" 2>&1
}

uploads_of() { grep -c "^upload $1\$" "$STATE/calls" || true; }

# 1. The v0.17.0 failure, and its neighbours: a stuck starter, a short upload that reported success,
#    and a dropped connection -- each recovered by retrying, and published once all are confirmed.
if run_case "ai-usage-tui-v9.9.9-x86_64-linux.tar.gz starter
ai-usage-tui-v9.9.9-x86_64-windows.zip short drop"; then
  [ "$(uploads_of ai-usage-tui-v9.9.9-x86_64-linux.tar.gz)" = 2 ] || fail "starter: expected 2 uploads"
  [ "$(uploads_of ai-usage-tui-v9.9.9-x86_64-windows.zip)" = 3 ] || fail "short+drop: expected 3 uploads"
  [ "$(awk -F'\t' '$3 != "uploaded"' "$STATE/assets")" = "" ] || fail "an asset was left unfinished"
  [ "$(wc -l <"$STATE/assets" | tr -d ' ')" = 9 ] || fail "expected exactly 9 assets, one per name"
  grep -q "4242 false" "$STATE/release" || fail "the release was not published"
else
  fail "a recoverable run failed:"; cat "$CASE_DIR/out"
fi

# 2. An asset that never completes: the run fails and the release stays a draft.
if run_case "ai-usage-tui-v9.9.9-x86_64-linux.tar.gz starter starter starter starter"; then
  fail "a run with an unrecoverable asset succeeded"
else
  grep -q "4242 true" "$STATE/release" || fail "a release with a stuck asset was published"
  grep -q "could not upload ai-usage-tui-v9.9.9-x86_64-linux.tar.gz" "$CASE_DIR/out" || fail "no error names the asset"
  [ "$(uploads_of ai-usage-tui-v9.9.9-x86_64-linux.tar.gz)" = 4 ] || fail "expected 4 attempts"
fi

# 3. A failed plan -- a manifest missing -- stops before any release is created.
CASE_DIR="$(fresh_dir)"; STATE="$CASE_DIR/state"; mkdir -p "$STATE"
workspace "$CASE_DIR/work"; install_fakes "$CASE_DIR/bin"; rm "$CASE_DIR/work/rendered/aur/PKGBUILD"
if (cd "$CASE_DIR/work" && PATH="$CASE_DIR/bin:$PATH" FAKE_STATE="$STATE" GITHUB_REPOSITORY=o/r \
  GH_TOKEN=t "$SCRIPT" --publish v9.9.9) >"$CASE_DIR/out" 2>&1; then
  fail "a run with a missing manifest succeeded"
else
  [ ! -f "$STATE/release" ] || fail "a release was created although the asset list failed its check"
fi

# 3b. So does a missing bill of materials -- and one for another tag does not stand in for it.
CASE_DIR="$(fresh_dir)"; STATE="$CASE_DIR/state"; mkdir -p "$STATE"
workspace "$CASE_DIR/work"; install_fakes "$CASE_DIR/bin"
mv "$CASE_DIR/work/sbom/ai-usage-tui-v9.9.9.cdx.json" "$CASE_DIR/work/sbom/ai-usage-tui-v9.9.8.cdx.json"
if (cd "$CASE_DIR/work" && PATH="$CASE_DIR/bin:$PATH" FAKE_STATE="$STATE" GITHUB_REPOSITORY=o/r \
  GH_TOKEN=t "$SCRIPT" --publish v9.9.9) >"$CASE_DIR/out" 2>&1; then
  fail "a run with no bill of materials succeeded"
else
  grep -q "missing or empty release asset: sbom/" "$CASE_DIR/out" || fail "the error does not name the bill of materials"
  [ ! -f "$STATE/release" ] || fail "a release was created although the bill of materials was missing"
fi

# 4. A re-run against an already published release replaces its assets and leaves it published.
run_case "" || fail "first publish failed"
before="$(cut -f1 "$STATE/assets" | sort | tr '\n' ' ')"
if (cd "$CASE_DIR/work" && PATH="$CASE_DIR/bin:$PATH" FAKE_STATE="$STATE" GITHUB_REPOSITORY=o/r \
  GH_TOKEN=t PUBLISH_RETRY_DELAY=0 "$SCRIPT" --publish v9.9.9) >"$CASE_DIR/out2" 2>&1; then
  grep -q "already published" "$CASE_DIR/out2" || fail "the re-run did not reuse the published release"
  [ "$(cut -f1 "$STATE/assets" | sort | tr '\n' ' ')" != "$before" ] || fail "assets were not replaced"
  [ "$(wc -l <"$STATE/assets" | tr -d ' ')" = 9 ] || fail "a re-run duplicated assets"
else
  fail "a re-run failed:"; cat "$CASE_DIR/out2"
fi

if [ "$failures" -eq 0 ]; then
  echo "publish-release: all cases pass"
else
  echo "publish-release: $failures failure(s)"
  exit 1
fi
