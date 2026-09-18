#!/usr/bin/env python3
"""Redact a Claude Code transcript into something that can be committed under tests/fixtures/.

A transcript holds the prompts, the replies, file contents and whatever a tool printed. The
collector reads a `usage` block and a handful of identifiers, and a fixture needs nothing else.

What survives: on every line, the identifying scalars the collector or the hook reads (`type`,
`uuid`, `parentUuid`, `timestamp`, `sessionId`, `isSidechain`, `version`, `requestId`, ...),
with the working directory replaced; on an assistant line, the `message` with its `usage` block
byte for byte as Claude Code wrote it, and its `content` reduced to the type of each block. Every
other key is dropped and *named* in `redacted`, so the fixture still says what a real line
carries. API request and message ids are replaced with numbered stand-ins, consistently, so lines
that shared one still do.

    scripts/redact-claude-transcript.py SESSION.jsonl [FIRST_STAND_IN] > tests/fixtures/claude_capture/<project>/SESSION.jsonl

FIRST_STAND_IN (default 1) is where the numbering starts. Give a subagent's transcript its own
range (101, 201, ...): ids are deduplicated across files, and two files numbered from 1 would
read as one set of requests.

Capture recipe: CONTRIBUTING.md, "Capturing a Claude Code transcript".
"""
import json
import sys

PROJECT = "/home/user/project"
KEPT = (
    "type", "uuid", "parentUuid", "timestamp", "sessionId", "isSidechain", "version", "userType",
    "entrypoint", "requestId", "agentId", "apiBlockIndex", "leafUuid", "operation",
)
MESSAGE_KEPT = ("model", "id", "type", "role", "stop_reason", "stop_sequence", "usage")
stand_ins = {}
FIRST = int(sys.argv[2]) if len(sys.argv) > 2 else 1


def stand_in(value, prefix):
    if not isinstance(value, str):
        return value
    return stand_ins.setdefault(value, f"{prefix}_fixture_{len(stand_ins) + FIRST:03d}")


def redact(line):
    record = json.loads(line)
    out = {key: record[key] for key in KEPT if key in record}
    dropped = [key for key in record if key not in KEPT and key not in ("message", "cwd", "gitBranch")]
    if "requestId" in out:
        out["requestId"] = stand_in(out["requestId"], "req")
    if "cwd" in record:
        out["cwd"] = PROJECT
    if "gitBranch" in record:
        out["gitBranch"] = "main"
    message = record.get("message")
    if isinstance(message, dict):
        kept = {key: message[key] for key in MESSAGE_KEPT if key in message}
        if "id" in kept:
            kept["id"] = stand_in(kept["id"], "msg")
        content = message.get("content")
        if isinstance(content, list):
            kept["content"] = [{"type": block.get("type")} for block in content if isinstance(block, dict)]
        elif content is not None:
            kept["content"] = "[redacted]"
        dropped += [f"message.{key}" for key in message if key not in MESSAGE_KEPT and key != "content"]
        out["message"] = kept
    if dropped:
        out["redacted"] = sorted(dropped)
    return out


def main():
    with open(sys.argv[1], encoding="utf-8") as handle:
        for line in handle:
            if line.strip():
                print(json.dumps(redact(line), separators=(",", ":"), ensure_ascii=False))


if __name__ == "__main__":
    main()
