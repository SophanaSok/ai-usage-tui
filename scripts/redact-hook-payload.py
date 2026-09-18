#!/usr/bin/env python3
"""Redact a captured Claude Code hook payload into something that can be committed under
tests/fixtures/hook/.

A `PostToolUse` / `PostToolUseFailure` payload names the session, the transcript and the working
directory, and carries the model's own description of the command. The hook reads the event, the
command, the ids and — for a line whose exit status is not the test runner's — the runner's
summary line in the output. A fixture needs nothing else.

What survives: every key, so the fixture still says what a real payload carries; with the ids
replaced by numbered stand-ins, the paths replaced by `/home/user/project`, and
`tool_input.description` (model-written text) replaced by the word `redacted`. The command and the
output are kept byte for byte: capture them from a scratch project, never from real work, because
output is whatever a program printed.

    scripts/redact-hook-payload.py PAYLOAD.json [N] > tests/fixtures/hook/<name>.json

N (default 1) numbers the stand-in ids, so two fixtures used together stay distinct.

Capture recipe: CONTRIBUTING.md, "Capturing a Claude Code hook payload".
"""
import json
import sys

PROJECT = "/home/user/project"


def main() -> None:
    payload = json.load(open(sys.argv[1], encoding="utf-8"))
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    session = f"00000000-0000-4000-8000-{n:012d}"
    real_cwd = payload.get("cwd", "")

    def scrub(text: str) -> str:
        return text.replace(real_cwd, PROJECT) if real_cwd else text

    payload["session_id"] = session
    payload["transcript_path"] = f"/home/user/.claude/projects/-home-user-project/{session}.jsonl"
    payload["cwd"] = PROJECT
    payload["prompt_id"] = f"00000000-0000-4000-9000-{n:012d}"
    payload["tool_use_id"] = f"toolu_{n:024d}"
    if "description" in payload.get("tool_input", {}):
        payload["tool_input"]["description"] = "redacted"
    response = payload.get("tool_response")
    if isinstance(response, dict):
        for key in ("stdout", "stderr"):
            if isinstance(response.get(key), str):
                response[key] = scrub(response[key])
    if isinstance(payload.get("error"), str):
        payload["error"] = scrub(payload["error"])
    json.dump(payload, sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
