#!/usr/bin/env bash
#
# Publish a GitHub Release from the release job's artifacts, one upload at a time.
#
#   scripts/publish-release.sh --plan    TAG   resolve and check the asset list, upload nothing
#   scripts/publish-release.sh --publish TAG   create a draft, upload serially, verify, publish
#
# Run from the directory holding `artifacts/`, `rendered/` and `body.txt`, as the release job does.
#
# Why not `softprops/action-gh-release`: it uploads every asset at once from the runner, and on
# v0.17.0 GitHub left the multi-megabyte ones stuck in `state: starter` -- "Error saving asset", or
# a silent hang. A same-name upload cannot replace a `starter` asset, and each re-run deleted and
# re-uploaded everything, so it got worse per attempt (1, then 3, then 5 stuck); only deleting the
# draft and starting over got it through. The same file uploaded alone took two seconds.
#
# So: one upload at a time, each checked against the API afterwards (`uploaded`, and the size on
# disk) rather than trusted from an exit status, a stuck asset deleted by id before each retry, and
# the release stays a draft until every asset is confirmed -- a partial release is never public,
# and `publish-crate` / `update-taps` only run after this succeeds. Safe to re-run: an existing
# draft is reused, and an existing published release has its assets replaced in place.
set -euo pipefail

MODE="${1:?usage: publish-release.sh --plan|--publish TAG}"
TAG="${2:?usage: publish-release.sh --plan|--publish TAG}"
REPO="${GITHUB_REPOSITORY:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"
ATTEMPTS="${PUBLISH_ATTEMPTS:-4}"
# Seconds before retry n is `RETRY_DELAY * 3^(n-1)`: 5, 15, 45 by default. Tests set 0.
RETRY_DELAY="${PUBLISH_RETRY_DELAY:-5}"
UPLOAD_TIMEOUT="${PUBLISH_UPLOAD_TIMEOUT:-300}"
UPLOADS="${PUBLISH_UPLOADS_URL:-https://uploads.github.com}"

# The asset list, in one place. Named rather than `rendered/*`: the Chocolatey pair is nested, and
# a one-level glob would try to upload a directory.
assets() {
  local archive
  for archive in artifacts/*/ai-usage-tui-*; do
    [ -f "$archive" ] && printf '%s\n' "$archive"
  done
  # The bill of materials, named for the tag. Expanded, not named: the tag is in the filename.
  # Exactly one, or the glob stays a literal and `plan` refuses it as missing.
  printf '%s\n' sbom/ai-usage-tui-*.cdx.json
  printf '%s\n' \
    artifacts/checksums.txt \
    rendered/homebrew/ai-usage-tui.rb \
    rendered/scoop/ai-usage-tui.json \
    rendered/chocolatey/ai-usage-tui.nuspec \
    rendered/chocolatey/tools/chocolateyinstall.ps1 \
    rendered/aur/PKGBUILD
}

size_of() { wc -c <"$1" | tr -d ' '; }

token() { printf '%s' "${GH_TOKEN:-${GITHUB_TOKEN:-$(gh auth token)}}"; }

# Everything that can be known before touching the API: each file exists and is not empty, no two
# share a name (assets are published flat), and every archive is one `checksums.txt` vouches for.
plan() {
  # Its errors go to stderr: `publish` captures this function's stdout as the asset list, and an
  # error printed there was swallowed with it -- the run failed and said nothing about why.
  local files=() file name archives=0
  mapfile -t files < <(assets)
  for file in "${files[@]}"; do
    [ -s "$file" ] || { echo "::error::missing or empty release asset: $file" >&2; exit 1; }
    # Used unescaped in the upload URL's `name=`, so it must need no escaping.
    [[ "$(basename "$file")" =~ ^[A-Za-z0-9._-]+$ ]] ||
      { echo "::error::asset name needs URL escaping: $file" >&2; exit 1; }
    case "$file" in
      artifacts/*/*)
        archives=$((archives + 1))
        name="$(basename "$file")"
        grep -qE "^[0-9a-f]{64}  ${name//./\\.}\$" artifacts/checksums.txt ||
          { echo "::error::$name is not listed in checksums.txt" >&2; exit 1; }
        ;;
    esac
  done
  [ "$archives" -gt 0 ] || { echo "::error::no build artifacts under artifacts/*/" >&2; exit 1; }
  local duplicates
  duplicates="$(printf '%s\n' "${files[@]}" | xargs -n1 basename | sort | uniq -d)"
  [ -z "$duplicates" ] || { echo "::error::two assets would publish as: $duplicates" >&2; exit 1; }
  printf '%s\n' "${files[@]}"
}

# `id<TAB>name<TAB>state<TAB>size`, one line per asset, including half-finished ones -- which
# `gh release view` does not show.
asset_rows() {
  gh api --paginate "repos/$REPO/releases/$1/assets?per_page=100" \
    --jq '.[] | [.id, .name, .state, .size] | @tsv'
}

release_row() {
  gh api --paginate "repos/$REPO/releases?per_page=100" \
    --jq ".[] | select(.tag_name == \"$TAG\") | [.id, .draft] | @tsv"
}

upload_one() {
  local release_id="$1" file="$2" name size attempt row id state remote_size
  name="$(basename "$file")"
  size="$(size_of "$file")"
  for attempt in $(seq 1 "$ATTEMPTS"); do
    # Whatever is there under this name goes first: a finished asset from an earlier run, or a
    # `starter` left by an upload that never completed and cannot be overwritten.
    while IFS=$'\t' read -r id _ state _; do
      echo "removing existing $name (id $id, $state)"
      gh api -X DELETE "repos/$REPO/releases/assets/$id" --silent
    done < <(asset_rows "$release_id" | awk -F'\t' -v n="$name" '$2 == n')

    echo "uploading $name ($size bytes), attempt $attempt/$ATTEMPTS"
    # The documented upload endpoint, addressed by release id: it takes a draft as readily as a
    # published release, which a lookup by tag does not promise. The token is read from the
    # environment inside curl's own config, so it never appears on a command line.
    curl --fail --silent --show-error --max-time "$UPLOAD_TIMEOUT" \
      --config <(printf 'header = "Authorization: Bearer %s"\n' "$(token)") \
      -H "Accept: application/vnd.github+json" \
      -H "Content-Type: application/octet-stream" \
      --data-binary "@$file" \
      "$UPLOADS/repos/$REPO/releases/$release_id/assets?name=$name" >/dev/null ||
      echo "::warning::upload of $name failed on attempt $attempt"

    row="$(asset_rows "$release_id" | awk -F'\t' -v n="$name" '$2 == n' | head -n1)"
    IFS=$'\t' read -r _ _ state remote_size <<<"${row:-}"
    if [ "${state:-}" = "uploaded" ] && [ "${remote_size:-}" = "$size" ]; then
      echo "confirmed $name"
      return 0
    fi
    echo "::warning::$name is ${state:-absent} with ${remote_size:-no} bytes, expected uploaded with $size"
    if [ "$attempt" -lt "$ATTEMPTS" ]; then
      sleep $((RETRY_DELAY * 3 ** (attempt - 1)))
    fi
  done
  echo "::error::could not upload $name after $ATTEMPTS attempts; the release is left as a draft"
  return 1
}

publish() {
  local files=() listing release_id draft file
  # Not `mapfile < <(plan)`: an `exit` inside a process substitution ends only that subshell, so a
  # failed check would have published whatever list it printed before failing. A command
  # substitution in a plain assignment carries the status to `set -e`.
  listing="$(plan)"
  mapfile -t files <<<"$listing"
  [ -f body.txt ] || { echo "::error::body.txt (the changelog section) is missing"; exit 1; }

  IFS=$'\t' read -r release_id draft <<<"$(release_row)"
  if [ -z "${release_id:-}" ]; then
    release_id="$(gh api -X POST "repos/$REPO/releases" \
      -f tag_name="$TAG" -f name="$TAG" -F draft=true -F body=@body.txt --jq .id)"
    draft=true
    echo "created draft release $release_id for $TAG"
  else
    echo "reusing release $release_id for $TAG (draft=$draft)"
  fi

  for file in "${files[@]}"; do
    upload_one "$release_id" "$file"
  done

  # Nothing half-finished may remain under any name, listed or not.
  local stuck
  stuck="$(asset_rows "$release_id" | awk -F'\t' '$3 != "uploaded"')"
  [ -z "$stuck" ] || { echo "::error::assets not in state uploaded:"; echo "$stuck"; exit 1; }

  if [ "$draft" = "true" ]; then
    gh api -X PATCH "repos/$REPO/releases/$release_id" -F draft=false --silent
    echo "published $TAG with ${#files[@]} assets"
  else
    echo "$TAG was already published; its ${#files[@]} assets are replaced and confirmed"
  fi
}

case "$MODE" in
  --plan) plan ;;
  --publish) publish ;;
  *) echo "usage: publish-release.sh --plan|--publish TAG" >&2; exit 2 ;;
esac
