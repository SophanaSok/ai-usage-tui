#!/usr/bin/env python3
"""Redact a Codex CLI rollout into something that can be committed under tests/fixtures/.

A rollout holds the prompt, tool arguments and output, the CLI's base instructions, the working
directory and the time zone. The collector reads none of that, and a fixture needs none of it.

What survives, byte for byte as the CLI wrote it: every line's envelope (`timestamp`, `ordinal`,
`type`), and the whole payload of the kinds the collector reads -- `token_count` events and
`token_usage_record` lines. `session_meta` and `turn_context` keep their keys with every path
replaced and the free text dropped. Every other line keeps its envelope, its `payload.type` and
a marker saying the rest was removed, so the fixture still has the real file's line kinds, in
the real order, around the lines that are read.

    scripts/redact-codex-rollout.py ROLLOUT.jsonl > tests/fixtures/codex_capture/sessions/.../rollout-....jsonl

Capture recipe: docs/provider-support.md, "Capturing a Codex rollout without an account".
"""
import json
import sys

PROJECT = "/home/user/project"
# Free text, or text that is not ours to republish.
DROPPED = {"base_instructions", "instructions", "user_instructions", "developer_instructions", "summary"}
PATH_KEYS = {"cwd", "runtime_workspace_roots", "workspace_roots"}


def scrub(value, key=None):
    if key in DROPPED:
        return "[redacted]"
    if key in PATH_KEYS:
        return [PROJECT] * len(value) if isinstance(value, list) else PROJECT
    if key == "timezone":
        return "Etc/UTC"
    if isinstance(value, dict):
        return {k: scrub(v, k) for k, v in value.items()}
    if isinstance(value, list):
        return [scrub(v) for v in value]
    return value


def redact(line):
    record = json.loads(line)
    payload = record.get("payload")
    kind = record.get("type")
    inner = payload.get("type") if isinstance(payload, dict) else None
    if kind == "token_usage_record" or (kind == "event_msg" and inner == "token_count"):
        return record
    if kind in ("session_meta", "turn_context"):
        record["payload"] = scrub(payload)
        return record
    kept = {"type": inner} if inner is not None else {}
    kept["redacted"] = True
    record["payload"] = kept
    for extra in [k for k in record if k not in ("timestamp", "ordinal", "type", "payload")]:
        del record[extra]
    return record


def main():
    with open(sys.argv[1], encoding="utf-8") as handle:
        for line in handle:
            if line.strip():
                print(json.dumps(redact(line), separators=(",", ":"), ensure_ascii=False))


if __name__ == "__main__":
    main()
