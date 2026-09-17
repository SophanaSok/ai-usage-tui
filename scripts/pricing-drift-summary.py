#!/usr/bin/env python3
"""Summarise what changed between two copies of `pricing/litellm.tsv`, as Markdown.

    scripts/pricing-drift-summary.py OLD NEW

For the monthly pricing-drift job's issue body, and for anyone about to commit a regenerated
table: `git diff` on 3,400 tab-separated lines says *that* it changed, not what. Header lines
(`#`) carry a date and are ignored, as `refresh-litellm-pricing.py --check` ignores them.
"""

import sys

SHOWN = 15


def entries(path):
    """`key -> its rates`. A long-context tier line repeats its key, so it is its own entry."""
    table = {}
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            key, _, rest = line.partition("\t")
            fields = rest.split("\t")
            if fields and fields[0].startswith("tier="):
                key = "%s (%s)" % (key, fields.pop(0).replace("=", " "))
            table[key] = " ".join(fields)
    return table


def section(title, keys, describe):
    if not keys:
        return []
    lines = ["", "**%s: %d**" % (title, len(keys)), ""]
    lines += ["- `%s`%s" % (key, describe(key)) for key in keys[:SHOWN]]
    if len(keys) > SHOWN:
        lines.append("- ... and %d more" % (len(keys) - SHOWN))
    return lines


def main():
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    old, new = entries(sys.argv[1]), entries(sys.argv[2])
    added = sorted(set(new) - set(old))
    removed = sorted(set(old) - set(new))
    repriced = sorted(key for key in set(old) & set(new) if old[key] != new[key])

    print("%d keys before, %d after." % (len(old), len(new)))
    out = []
    out += section("Repriced", repriced, lambda k: ": `%s` -> `%s`" % (old[k], new[k]))
    out += section("Added", added, lambda k: ": `%s`" % new[k])
    out += section("Removed", removed, lambda k: "")
    print("\n".join(out) if out else "\nNo model entry changed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
