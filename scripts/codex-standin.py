#!/usr/bin/python3
"""A local stand-in for the Responses API, so the real Codex CLI writes a real rollout with no
account and no billable call. It answers the first request of a thread with a tool call and every
later one with a message, and sends the rate-limit header family Codex turns into `rate_limits`.

The token counts and percentages are this script's; the bytes in the rollout are the CLI's.

    scripts/codex-standin.py PORT LOGFILE

Recipe: docs/provider-support.md, "Capturing a Codex rollout without an account"."""
import json
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = open(sys.argv[2], "a")
calls = {"n": 0}

def sse(event):
    return f"event: {event['type']}\ndata: {json.dumps(event)}\n\n".encode()

class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a): pass
    def do_GET(self):
        LOG.write(f"GET {self.path}\n"); LOG.flush()
        self.send_response(404); self.send_header("Content-Length", "0"); self.end_headers()
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        try: req = json.loads(body)
        except Exception: req = {}
        tools = [t.get("name") or t.get("type") for t in req.get("tools", [])]
        has_output = any(i.get("type") == "function_call_output" for i in req.get("input", []) if isinstance(i, dict))
        calls["n"] += 1
        n = calls["n"]
        LOG.write(f"POST {self.path} n={n} tools={tools} has_output={has_output} headers={dict(self.headers)}\n"); LOG.flush()
        rid = f"resp_standin_{n}"
        if not has_output and "exec_command" in tools:
            item = {"type": "function_call", "id": f"fc_{n}", "call_id": f"call_{n}", "name": "exec_command",
                    "arguments": json.dumps({"cmd": "echo standin"}), "status": "completed"}
        elif not has_output and "shell" in tools:
            item = {"type": "function_call", "id": f"fc_{n}", "call_id": f"call_{n}", "name": "shell",
                    "arguments": json.dumps({"command": ["echo", "standin"]}), "status": "completed"}
        else:
            item = {"type": "message", "id": f"msg_{n}", "role": "assistant", "status": "completed",
                    "content": [{"type": "output_text", "text": "Done.", "annotations": []}]}
        usage = {"input_tokens": 9000 + 700 * n, "input_tokens_details": {"cached_tokens": 6000 + 500 * n},
                 "output_tokens": 120 + 15 * n, "output_tokens_details": {"reasoning_tokens": 64},
                 "total_tokens": 9120 + 715 * n}
        now = int(time.time())
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        for k, v in {
            "x-codex-primary-used-percent": str(11.0 + n), "x-codex-primary-window-minutes": "300",
            "x-codex-primary-reset-at": str(now + 3 * 3600),
            "x-codex-secondary-used-percent": str(40.5), "x-codex-secondary-window-minutes": "10080",
            "x-codex-secondary-reset-at": str(now + 4 * 86400),
            "x-codex-plan-type": "plus",
        }.items():
            self.send_header(k, v)
        # A second header family. Codex keeps one snapshot and the last one parsed wins, so with
        # this on, the rollout never carries the default `codex` family at all. NO_OTHER=1 turns
        # it off; the committed fixture holds one rollout of each.
        for k, v in {} if os.environ.get("NO_OTHER") else {
            "x-codex-other-primary-used-percent": "2.0", "x-codex-other-primary-window-minutes": "60",
            "x-codex-other-primary-reset-at": str(now + 1800), "x-codex-other-limit-name": "codex_other",
        }.items():
            self.send_header(k, v)
        self.end_headers()
        resp = {"id": rid, "object": "response", "status": "in_progress", "output": []}
        self.wfile.write(sse({"type": "response.created", "response": resp}))
        self.wfile.write(sse({"type": "response.output_item.added", "output_index": 0, "item": item}))
        self.wfile.write(sse({"type": "response.output_item.done", "output_index": 0, "item": item}))
        self.wfile.write(sse({"type": "response.completed",
                              "response": {"id": rid, "object": "response", "status": "completed",
                                           "output": [item], "usage": usage}}))
        self.wfile.flush()
        self.close_connection = True

ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
