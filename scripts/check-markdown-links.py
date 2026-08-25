#!/usr/bin/env python3
"""Fail if any relative Markdown link in the repository points at something that is not there.

A doc-only change can break a relative link between the README and `docs/` without touching a
line of Rust, and nothing else in CI looks: the Omarchy split moved a section between files and
left two dangling anchors behind.

Lives here rather than inline in `.github/workflows/ci.yml` so that `just docs` and the CI job
run the same checker instead of two copies of it. Absolute links (`http:`, `https:`, `mailto:`)
are skipped -- proving those resolve needs the network, which `identity.yml` does for the few
that matter.
"""

import os
import re
import sys

# `](path)` or `](path#anchor)`, with the anchor discarded: only the file has to exist.
LINK = re.compile(r"\]\(([^)#\s]+)(#[^)\s]*)?\)")
SKIP_DIRS = {".git", "target", "node_modules"}
SKIP_SCHEMES = ("http://", "https://", "mailto:")


def main() -> int:
    root = sys.argv[1] if len(sys.argv) > 1 else "."
    bad = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for name in filenames:
            if not name.endswith(".md"):
                continue
            path = os.path.join(dirpath, name)
            with open(path, encoding="utf-8", errors="replace") as handle:
                text = handle.read()
            for match in LINK.finditer(text):
                link = match.group(1)
                if link.startswith(SKIP_SCHEMES):
                    continue
                target = os.path.normpath(os.path.join(os.path.dirname(path), link))
                if not os.path.exists(target):
                    bad.append(f"{path} -> {link}")

    for entry in bad:
        # The `::error::` prefix is what makes GitHub annotate the file in the run summary; it is
        # harmless noise in a local `just docs`.
        print(f"::error::broken relative link: {entry}")
    if bad:
        print(f"\n{len(bad)} broken relative link(s)", file=sys.stderr)
        return 1
    print("every relative Markdown link resolves")
    return 0


if __name__ == "__main__":
    sys.exit(main())
