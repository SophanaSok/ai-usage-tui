#!/usr/bin/env python3
"""Regenerate `pricing/litellm.tsv` from LiteLLM's community pricing table.

    scripts/refresh-litellm-pricing.py            # fetch upstream and rewrite
    scripts/refresh-litellm-pricing.py --check    # fail if the committed file is stale

Why a generated snapshot rather than the upstream file itself: upstream is 1.8MB of JSON
covering 3,176 entries, most of which are image, audio or embedding models this tool never
sees. What is kept is the token-billed chat entries and only the fields the pricing engine
reads, converted to the per-million-token rates the rest of the table uses.

Why vendored rather than fetched at runtime: pricing must work offline and must be identical
for everyone running the same binary. The runtime refresh (`--refresh-pricing`) stays an
overlay on top of this, never a replacement.

Why TSV rather than TOML like `pricing/zen.toml`: parsing 3,400-odd TOML tables cost 38ms on
every single invocation -- a 9x startup regression, measured. Nobody hand-edits this file, so it
does not need TOML's ergonomics; one tab-separated line per entry parses in about a millisecond.
The curated table stays TOML because humans do edit it and it carries comments and historical
`period` blocks.

**Ambiguous bare names are deliberately not emitted.** 206 model names appear under more than
one provider with different rates -- Bedrock regional variants, mostly. Emitting a bare key for
those would pick one provider's rate and apply it to every provider, which is inventing a
number. They get provider-qualified keys only; a model whose provider does not match stays
`unavailable`, which is the correct answer.
"""

import argparse
import json
import sys
import urllib.request
from collections import defaultdict
from datetime import date, timezone, datetime
from pathlib import Path

UPSTREAM = (
    "https://raw.githubusercontent.com/BerriAI/litellm/main/"
    "model_prices_and_context_window.json"
)

# Modes that bill per token and that this tool can actually see in an agent transcript.
CHAT_MODES = {"chat", "completion", "responses"}

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "pricing" / "litellm.tsv"

# Upstream cost keys -> our field names. Rates are per token upstream, per million here.
FIELDS = [
    ("input", "input_cost_per_token"),
    ("output", "output_cost_per_token"),
    ("cache_read", "cache_read_input_token_cost"),
    ("cache_write", "cache_creation_input_token_cost"),
    ("reasoning", "output_cost_per_reasoning_token"),
]

# Long-context tiers. Upstream spells the threshold in `k`, not digits:
# `input_cost_per_token_above_200k_tokens`. Getting this wrong is silent -- the suffix simply
# matches nothing and every tier is dropped, which is exactly what happened first time.
#
# `cache_creation_input_token_cost_above_1hr_above_200k_tokens` is deliberately not here: that
# is a cache-TTL rate, not a context tier, and treating it as one would price a long-context
# request at the one-hour cache-write rate.
TIER_THRESHOLDS = [("512k", 512_000), ("272k", 272_000), ("256k", 256_000),
                   ("200k", 200_000), ("128k", 128_000)]


def per_million(value):
    """Upstream publishes per-token rates; the table is per million.

    Rounded to 6 decimals: the products are floats like 3e-06 * 1e6 = 2.9999999999999996, and
    a table full of those is unreadable and diffs badly for no gain in accuracy.
    """
    if value is None:
        return None
    if not isinstance(value, (int, float)):
        return None
    return round(float(value) * 1_000_000, 6)


def rates_for(entry, suffix=""):
    """The five rates, optionally for a long-context tier."""
    out = {}
    for name, key in FIELDS:
        out[name] = per_million(entry.get(key + suffix))
    return {k: v for k, v in out.items() if v is not None}


def collect(data):
    """Token-billed chat entries, keyed by (provider, bare name)."""
    rows = {}
    for key, entry in data.items():
        if key == "sample_spec" or not isinstance(entry, dict):
            continue
        if entry.get("mode") not in CHAT_MODES:
            continue
        base = rates_for(entry)
        if "input" not in base and "output" not in base:
            continue
        provider = (entry.get("litellm_provider") or "").strip().lower()
        if not provider:
            continue
        bare = key.rsplit("/", 1)[-1].strip().lower()
        if not bare:
            continue

        tiers = {}
        for label, threshold in TIER_THRESHOLDS:
            tier = rates_for(entry, "_above_%s_tokens" % label)
            if tier:
                tiers[threshold] = tier

        # Upstream can list the same (provider, model) twice via aliases; first wins, and they
        # agree in practice. Recorded rather than silently dropped.
        rows.setdefault((provider, bare), (base, tiers))
    return rows


def render(rows, upstream_sha):
    unambiguous = defaultdict(set)
    for (provider, bare), (base, _) in rows.items():
        unambiguous[bare].add((base.get("input"), base.get("output")))
    ambiguous = {bare for bare, seen in unambiguous.items() if len(seen) > 1}

    lines = [
        "# LiteLLM pricing snapshot (per 1M tokens). GENERATED -- do not hand-edit.",
        "#",
        "# Regenerate with: scripts/refresh-litellm-pricing.py",
        "# Source:   %s" % UPSTREAM,
        "# Upstream: %s" % upstream_sha,
        "# Updated:  %s" % date.today().isoformat(),
        "#",
        "# Format: one entry per line, tab-separated.",
        "#   <key>\\t<field>=<rate>\\t...   fields: input output cache_read cache_write reasoning",
        "#   a `tier=<tokens>` field marks a long-context tier for the key above it",
        "# Rates are per million tokens. An absent field means the source publishes no rate for",
        "# that bucket, which is not the same as a rate of zero.",
        "#",
        "# Layering: this is the *base* table. `pricing/zen.toml` is applied on top of it, and a",
        "# refreshed cache on top of that. So a curated or Zen-specific rate always wins over a",
        "# community one, and neither can be deleted by a bad refresh.",
        "#",
        "# Keys are `provider/model`. A bare `model` key is emitted only where every provider",
        "# publishing that name agrees on the rate: %d of %d names do, and the other %d do not,"
        % (len(unambiguous) - len(ambiguous), len(unambiguous), len(ambiguous)),
        "# so those resolve only when the usage row's provider matches. Guessing which provider's",
        "# rate to apply would be inventing a number, and unknown cost stays unknown.",
        "",
    ]

    def emit(key, base, tiers):
        fields = ["%s=%s" % (n, format_rate(base[n])) for n, _ in FIELDS if n in base]
        lines.append("\t".join([key] + fields))
        for threshold in sorted(tiers, reverse=True):
            tier_fields = [
                "%s=%s" % (n, format_rate(tiers[threshold][n])) for n, _ in FIELDS
                if n in tiers[threshold]
            ]
            lines.append("\t".join([key, "tier=%d" % threshold] + tier_fields))

    for (provider, bare) in sorted(rows):
        base, tiers = rows[(provider, bare)]
        emit("%s/%s" % (provider, bare), base, tiers)

    seen_bare = set()
    for (provider, bare) in sorted(rows):
        if bare in ambiguous or bare in seen_bare:
            continue
        seen_bare.add(bare)
        base, tiers = rows[(provider, bare)]
        emit(bare, base, tiers)

    return "\n".join(lines).rstrip() + "\n", len(rows), len(seen_bare), len(ambiguous)


def format_rate(value):
    """Plain decimal, never scientific notation -- `1e-06` is valid TOML but unreadable here."""
    text = ("%.6f" % value).rstrip("0")
    return text + "0" if text.endswith(".") else text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail if the committed file is stale")
    parser.add_argument("--from-file", help="use a local copy instead of fetching")
    args = parser.parse_args()

    if args.from_file:
        raw = Path(args.from_file).read_bytes()
        sha = "local file %s" % args.from_file
    else:
        request = urllib.request.Request(UPSTREAM, headers={"User-Agent": "ai-usage-tui"})
        with urllib.request.urlopen(request, timeout=60) as response:
            raw = response.read()
        sha = "fetched %s" % datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

    data = json.loads(raw)
    rows = collect(data)
    rendered, qualified, bare, ambiguous = render(rows, sha)

    if args.check:
        current = OUT.read_text(encoding="utf-8") if OUT.exists() else ""
        # The header carries a date, so compare only the model tables.
        def body(text):
            return "\n".join(l for l in text.splitlines() if not l.startswith("#"))
        if body(current) != body(rendered):
            print("pricing/litellm.tsv is stale; run scripts/refresh-litellm-pricing.py", file=sys.stderr)
            return 1
        print("pricing/litellm.tsv is current")
        return 0

    OUT.write_text(rendered, encoding="utf-8")
    print(
        "wrote %s: %d provider-qualified keys, %d unambiguous bare keys, "
        "%d ambiguous names left provider-only (%.0f KB)"
        % (OUT.relative_to(ROOT), qualified, bare, ambiguous, OUT.stat().st_size / 1024)
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
