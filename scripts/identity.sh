#!/usr/bin/env bash
#
# Keep GitHub's About box equal to Cargo.toml, and check the release channels the README names.
#
# The project's description of itself lived in seven files plus GitHub's settings, in five
# wordings, and the README claimed for three releases that crates.io and the Homebrew tap did not
# exist. Cargo.toml is the single source of truth now: crates.io reads it verbatim at publish, the
# packaging manifests render `__DESCRIPTION__` from it at release time, and `tests/docs.rs` pins
# the README and clap's `about` to it. GitHub has no manifest, so this fills that gap.
#
#   scripts/identity.sh --check      compare, report, exit 1 on drift   (needs no write access)
#   scripts/identity.sh --apply      push description, homepage, topics (needs a PAT, see below)
#   scripts/identity.sh --channels   check each release channel the README names actually exists
#
# `--apply` needs Administration: write, which a workflow's GITHUB_TOKEN cannot hold: there is no
# `administration` key among the scopes `permissions:` accepts, and both `PATCH /repos/{o}/{r}`
# and `PUT /repos/{o}/{r}/topics` require it. `--check` needs only Metadata: read, which every
# token has -- which is what lets CI verify with a token that cannot write, so a missing or
# expired PAT fails the build instead of silently skipping.
set -euo pipefail

MODE="${1:---check}"
REPO="${GITHUB_REPOSITORY:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"

field() { python3 -c "
import tomllib
m = tomllib.load(open('Cargo.toml','rb'))['package']
print($1)
"; }

DESCRIPTION="$(field "m['description']")"
HOMEPAGE="$(field "m['metadata']['identity']['github_homepage']")"
TOPICS="$(field "' '.join(sorted(m['metadata']['identity']['topics']))")"
NAME="$(field "m['name']")"

[ -n "$DESCRIPTION" ] || { echo "Cargo.toml has no package.description" >&2; exit 1; }
[ -n "$TOPICS" ] || { echo "Cargo.toml has no package.metadata.identity.topics" >&2; exit 1; }

live_topics() { gh api "repos/$REPO/topics" --jq '[.names[]] | sort | join(" ")'; }

case "$MODE" in
  --apply)
    gh api -X PATCH "repos/$REPO" -f description="$DESCRIPTION" -f homepage="$HOMEPAGE" >/dev/null
    python3 -c "
import json, sys
print(json.dumps({'names': sys.argv[1].split()}))
" "$TOPICS" | gh api -X PUT "repos/$REPO/topics" --input - >/dev/null
    echo "pushed description, homepage and topics to $REPO"
    ;;

  --check)
    have_desc="$(gh api "repos/$REPO" --jq '.description // ""')"
    have_home="$(gh api "repos/$REPO" --jq '.homepage // ""')"
    have_topics="$(live_topics)"
    drift=0
    [ "$have_desc" = "$DESCRIPTION" ] || { drift=1
      echo "description:"; echo "  GitHub     [$have_desc]"; echo "  Cargo.toml [$DESCRIPTION]"; }
    [ "$have_home" = "$HOMEPAGE" ] || { drift=1
      echo "homepage:"; echo "  GitHub     [$have_home]"; echo "  Cargo.toml [$HOMEPAGE]"; }
    [ "$have_topics" = "$TOPICS" ] || { drift=1
      echo "topics:"; echo "  GitHub     [$have_topics]"; echo "  Cargo.toml [$TOPICS]"; }
    if [ "$drift" -ne 0 ]; then
      echo
      echo "GitHub's About box disagrees with Cargo.toml. Either add the REPO_METADATA_TOKEN"
      echo "secret so identity.yml can push it, or set it once by hand:"
      echo "  scripts/identity.sh --apply"
      exit 1
    fi
    echo "GitHub matches Cargo.toml"
    ;;

  --channels)
    # The claims tests/docs.rs cannot check, because checking them needs the network. This is what
    # would have caught the README saying crates.io and the tap did not exist for three releases
    # after both did.
    fail=0
    # crates.io rejects a request with no User-Agent identifying the caller: without this the
    # check 403s and reports the crate missing when it is published.
    curl -fsS -A "ai-usage-tui-identity-check (+https://github.com/$REPO)" \
      "https://crates.io/api/v1/crates/$NAME" >/dev/null \
      || { echo "::error::the README documents \`cargo install\` but $NAME is not on crates.io"; fail=1; }
    gh api "repos/${REPO%/*}/homebrew-tap/contents/Formula/$NAME.rb" >/dev/null 2>&1 \
      || { echo "::error::the README documents \`brew install\` but the tap has no Formula/$NAME.rb"; fail=1; }
    gh api "repos/${REPO%/*}/scoop-bucket/contents/bucket/$NAME.json" >/dev/null 2>&1 \
      || { echo "::error::the README documents \`scoop install\` but the bucket has no bucket/$NAME.json"; fail=1; }
    [ "$fail" -eq 0 ] && echo "every release channel the README names exists"
    exit "$fail"
    ;;

  *)
    echo "usage: scripts/identity.sh [--check|--apply|--channels]" >&2
    exit 2
    ;;
esac
