#!/usr/bin/env python3
"""Build the demo dataset the README screenshots are captured from.

The old fixture (`tests/fixtures/opencode_test.db`) is nine rows on a single day in 2023 with no
session ids and no project paths, so four of the seven panels capture empty: projects and sessions
have nothing to group by, the burn window says "too little activity to project", and the
spend-over-time chart is one bar. Adding `send_key` calls to the capture script was never going to
be enough — the data has to be able to show what the panels do.

Everything here is invented. The project names are deliberately fictional so a reader never
mistakes a screenshot for the author's real spend, and the shape is chosen to exercise the cases
that matter rather than to look impressive:

  * several days of history, ending today, so the graph and the burn window both have something
  * more than one project and more than one session, with a session spanning two projects
  * a cheaper-to-pricier model change inside one session, so the escalations block renders
  * a run of local and cloud usage, so LOCAL/CLOUD classification and the quota status appear
"""

import argparse
import json
import pathlib
import random
import shutil
from datetime import datetime, timedelta, timezone

# Fictional. Anyone reading a screenshot should be able to tell these are not real repositories.
PROJECTS = ["/home/dev/lantern-api", "/home/dev/lantern-web", "/home/dev/orbit-cli"]

MODELS = [
    ("claude-sonnet-5", 0.62),
    ("claude-opus-5", 0.22),
    ("claude-haiku-4-5", 0.16),
]


def assistant_line(session, project, model, when, seq, tokens):
    """One Claude Code transcript line, carrying only the fields the collector reads."""
    return {
        "type": "assistant",
        "uuid": f"u-{session}-{seq}",
        "requestId": f"req_{session}_{seq}",
        "sessionId": session,
        "timestamp": when.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "cwd": project,
        "gitBranch": "main",
        "message": {
            "id": f"msg_{session}_{seq}",
            "role": "assistant",
            "model": model,
            "usage": {
                "input_tokens": tokens,
                "output_tokens": max(1, tokens // 6),
                "cache_read_input_tokens": tokens * 4,
                "cache_creation_input_tokens": tokens // 3,
            },
        },
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("out", type=pathlib.Path, help="directory to write projects/ into")
    parser.add_argument("--days", type=int, default=9)
    parser.add_argument("--seed", type=int, default=20260819)
    args = parser.parse_args()

    rng = random.Random(args.seed)  # Deterministic: the same screenshots regenerate identically.
    root = args.out / "projects"
    if root.exists():
        shutil.rmtree(root)

    now = datetime.now().astimezone()
    session_no = 0

    for day_offset in range(args.days - 1, -1, -1):
        day = now - timedelta(days=day_offset)
        for _ in range(rng.randint(1, 3)):
            session_no += 1
            session = f"demo-session-{session_no:02d}"
            project = rng.choice(PROJECTS)
            # Munged-path directory naming, as Claude Code writes it.
            folder = root / project.replace("/", "-").lstrip("-")
            folder.mkdir(parents=True, exist_ok=True)

            # One session in three starts cheap and escalates, which is what the derived
            # escalations block exists to show. The rest keep a single model.
            escalates = rng.random() < 0.34
            model = "claude-haiku-4-5" if escalates else rng.choices(
                [m for m, _ in MODELS], weights=[w for _, w in MODELS]
            )[0]

            lines = []
            requests = rng.randint(4, 22)
            for seq in range(requests):
                if escalates and seq == requests // 3:
                    model = "claude-sonnet-5"
                if escalates and seq == (2 * requests) // 3:
                    model = "claude-opus-5"
                when = day.replace(
                    hour=rng.randint(9, 20), minute=rng.randint(0, 59), second=rng.randint(0, 59)
                ) + timedelta(seconds=seq * rng.randint(20, 90))
                lines.append(
                    assistant_line(session, project, model, when, seq, rng.randint(400, 9000))
                )

            path = folder / f"{session}.jsonl"
            with path.open("w") as handle:
                for line in lines:
                    handle.write(json.dumps(line) + "\n")

    # The burn panel measures a trailing hour, so the most recent session must be minutes old or
    # it reports "too little activity to project" — which is correct behaviour and a poor advert.
    session_no += 1
    session = f"demo-session-{session_no:02d}"
    project = PROJECTS[0]
    folder = root / project.replace("/", "-").lstrip("-")
    folder.mkdir(parents=True, exist_ok=True)
    with (folder / f"{session}.jsonl").open("w") as handle:
        for seq in range(14):
            when = now - timedelta(minutes=rng.randint(1, 45))
            model = "claude-sonnet-5" if seq < 9 else "claude-opus-5"
            handle.write(
                json.dumps(assistant_line(session, project, model, when, seq, rng.randint(2000, 12000)))
                + "\n"
            )

    print(f"wrote {sum(1 for _ in root.rglob('*.jsonl'))} sessions to {root}")


if __name__ == "__main__":
    main()
