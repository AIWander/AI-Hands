#!/usr/bin/env python3
"""
AI-Hands PostToolUse / PostToolUseFailure audit logger (Grok / Claude plugin).

Appends one JSON line per matching tool call. Never blocks.
"""
from __future__ import annotations

import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path

if os.environ.get("AI_HANDS_HOOK_ACTIVE") == "1":
    sys.exit(0)


def _log_dir() -> Path:
    plugin_data = (
        os.environ.get("GROK_PLUGIN_DATA")
        or os.environ.get("CLAUDE_PLUGIN_DATA")
    )
    if plugin_data:
        p = Path(plugin_data) / "logs"
    elif os.environ.get("AI_HANDS_LOG_DIR"):
        p = Path(os.environ["AI_HANDS_LOG_DIR"])
    else:
        p = Path.home() / ".grok" / "plugin-data" / "grok-ai-hands" / "logs"
    p.mkdir(parents=True, exist_ok=True)
    return p


LOG_DIR = _log_dir()
AUDIT_LOG = LOG_DIR / "post_tool_audit.jsonl"

SENSITIVE_KEYS = {
    "password", "passwd", "secret", "token", "api_key", "apikey",
    "authorization", "cookie", "totp", "otp", "credential",
}


def _redact(obj, depth=0):
    if depth > 6:
        return "…"
    if isinstance(obj, dict):
        out = {}
        for k, v in obj.items():
            lk = str(k).lower()
            if any(s in lk for s in SENSITIVE_KEYS):
                out[k] = "[REDACTED]"
            else:
                out[k] = _redact(v, depth + 1)
        return out
    if isinstance(obj, list):
        return [_redact(x, depth + 1) for x in obj[:50]]
    if isinstance(obj, str) and len(obj) > 500:
        return obj[:500] + "…"
    return obj


def _tool_name(payload: dict) -> str:
    return (
        payload.get("toolName")
        or payload.get("tool_name")
        or payload.get("name")
        or ""
    )


def _tool_input(payload: dict):
    raw = payload.get("toolInput") or payload.get("tool_input") or payload.get("input")
    if isinstance(raw, str):
        try:
            return json.loads(raw)
        except json.JSONDecodeError:
            return {"_raw": raw[:300]}
    return raw


def main() -> int:
    try:
        payload = json.load(sys.stdin)
    except Exception:
        return 0

    name = _tool_name(payload)
    inp = _tool_input(payload) or {}

    if name in ("use_tool", "CallMcpTool") or str(name).endswith("use_tool"):
        qn = ""
        if isinstance(inp, dict):
            qn = inp.get("tool_name") or inp.get("name") or ""
        if not (isinstance(qn, str) and qn.startswith("AI-Hands__")):
            return 0
        name = qn
        if isinstance(inp, dict) and isinstance(inp.get("tool_input"), dict):
            inp = inp["tool_input"]

    if not str(name).startswith("AI-Hands__"):
        return 0

    event = (
        payload.get("hookEventName")
        or os.environ.get("GROK_HOOK_EVENT")
        or "post_tool_use"
    )
    err = payload.get("error") or payload.get("toolError") or payload.get("message")

    entry = {
        "ts": datetime.now(timezone.utc).isoformat(),
        "event": event,
        "tool": name,
        "input": _redact(inp) if isinstance(inp, (dict, list)) else inp,
        "error": err,
        "session": os.environ.get("GROK_SESSION_ID") or os.environ.get("CLAUDE_SESSION_ID"),
        "cwd": payload.get("cwd") or os.environ.get("GROK_WORKSPACE_ROOT"),
    }

    try:
        with AUDIT_LOG.open("a", encoding="utf-8") as f:
            f.write(json.dumps(entry, ensure_ascii=False) + "\n")
    except OSError:
        pass

    return 0


if __name__ == "__main__":
    sys.exit(main())
