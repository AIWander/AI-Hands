#!/usr/bin/env python3
"""
AI-Hands PreToolUse safety gate (Grok / Claude plugin).

Reads PreToolUse JSON from stdin. Denies high-risk hands click/submit shapes
when allow_destructive is not true and the target looks irreversible.

Exit 0 always with decision JSON on stdout (except deny → exit 2).
Fail-open on parse errors.
"""
from __future__ import annotations

import json
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

if os.environ.get("AI_HANDS_HOOK_ACTIVE") == "1":
    print(json.dumps({"decision": "allow"}))
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
AUDIT_LOG = LOG_DIR / "pre_tool_safety.jsonl"

# Tools that can cause irreversible UI side effects
RISKY_TOOLS = {
    "AI-Hands__hands_click",
    "AI-Hands__hands_fill_form",
    "AI-Hands__hands_login_recovery",
    "AI-Hands__find_and_click",
    "AI-Hands__browser_click",
    "AI-Hands__uia_click",
    "AI-Hands__browser_submit_form",
}

# Full-desktop window enumeration can hang on hung HWNDs. Not banned — rate-limited.
# Prefer title focus; if list is needed, allow sparingly so agents don't poll/list in a loop.
# Bypass rate limit: AI_HANDS_ALLOW_UIA_LIST=1
STUCK_PRONE_TOOLS = {
    "AI-Hands__uia_list_window",
}
LIST_WINDOW_COOLDOWN_S = float(os.environ.get("AI_HANDS_LIST_COOLDOWN_S", "45"))
LIST_WINDOW_STATE = Path(os.environ.get(
    "AI_HANDS_LIST_STATE",
    str(LOG_DIR / "list_window_last.json"),
))
LIST_WINDOW_RATE_REASON = (
    "AI-Hands anti-stuck: uia_list_window rate-limited (cooldown "
    f"{LIST_WINDOW_COOLDOWN_S:.0f}s). Cache the previous list or use "
    "uia_focus_window(title=…) / hands_app_action(focus|open, window_match.title=…). "
    "Need another list now: set AI_HANDS_ALLOW_UIA_LIST=1, or wait for cooldown."
)

DESTRUCTIVE_RE = re.compile(
    r"\b("
    r"pay|buy|purchase|checkout|place\s*order|confirm\s*purchase|"
    r"delete|remove\s*account|transfer|wire|send\s*money|"
    r"approve\s*payment|cancel\s*subscription|unsubscribe|"
    r"destroy|wipe|format|factory\s*reset"
    r")\b",
    re.I,
)

CONFIRMATION_RE = re.compile(
    r"\b("
    r"submit|confirm|sign\s*up|register|create\s*account|"
    r"send|approve|authorize|install|grant"
    r")\b",
    re.I,
)


def _tool_name(payload: dict) -> str:
    return (
        payload.get("toolName")
        or payload.get("tool_name")
        or payload.get("name")
        or ""
    )


def _tool_input(payload: dict) -> dict:
    raw = payload.get("toolInput") or payload.get("tool_input") or payload.get("input") or {}
    if isinstance(raw, str):
        try:
            return json.loads(raw)
        except json.JSONDecodeError:
            return {}
    return raw if isinstance(raw, dict) else {}


def _nested_args(inp: dict) -> dict:
    """use_tool may wrap as {tool_name, tool_input}."""
    if "tool_input" in inp and isinstance(inp.get("tool_input"), dict):
        return inp["tool_input"]
    if "arguments" in inp and isinstance(inp.get("arguments"), dict):
        return inp["arguments"]
    return inp


def _target_text(inp: dict) -> str:
    parts = []
    for key in ("target", "text", "match_text", "selector", "natural_text", "url"):
        val = inp.get(key)
        if isinstance(val, str):
            parts.append(val)
    fields = inp.get("fields")
    if isinstance(fields, dict):
        parts.extend(str(k) for k in fields.keys())
    submit_labels = inp.get("submit_label")
    if isinstance(submit_labels, list):
        parts.extend(str(x) for x in submit_labels)
    return " ".join(parts)


def _audit(entry: dict) -> None:
    try:
        with AUDIT_LOG.open("a", encoding="utf-8") as f:
            f.write(json.dumps(entry, ensure_ascii=False) + "\n")
    except OSError:
        pass


def main() -> int:
    try:
        payload = json.load(sys.stdin)
    except Exception:
        print(json.dumps({"decision": "allow"}))
        return 0

    name = _tool_name(payload)
    # Also catch use_tool dispatching to AI-Hands
    inp_outer = _tool_input(payload)
    if name in ("use_tool", "CallMcpTool") or name.endswith("use_tool"):
        qn = inp_outer.get("tool_name") or inp_outer.get("name") or ""
        if isinstance(qn, str) and qn.startswith("AI-Hands__"):
            name = qn
            inp = _nested_args(inp_outer)
        else:
            print(json.dumps({"decision": "allow"}))
            return 0
    else:
        inp = _nested_args(inp_outer)

    if not name.startswith("AI-Hands__") and name not in RISKY_TOOLS:
        print(json.dumps({"decision": "allow"}))
        return 0

    allow_destructive = bool(inp.get("allow_destructive"))
    auto_submit = bool(inp.get("auto_submit"))
    target = _target_text(inp)

    decision = "allow"
    reason = None

    # --- Anti-stuck: rate-limit full window list (do not ban) ---
    if name in STUCK_PRONE_TOOLS:
        allow_list = os.environ.get("AI_HANDS_ALLOW_UIA_LIST", "").strip() in (
            "1",
            "true",
            "TRUE",
            "yes",
            "YES",
        )
        now = datetime.now(timezone.utc)
        last_ts = None
        try:
            if LIST_WINDOW_STATE.is_file():
                last_ts = json.loads(LIST_WINDOW_STATE.read_text(encoding="utf-8")).get("ts")
        except (OSError, json.JSONDecodeError, TypeError):
            last_ts = None
        within_cooldown = False
        if last_ts and not allow_list:
            try:
                last = datetime.fromisoformat(last_ts.replace("Z", "+00:00"))
                within_cooldown = (now - last).total_seconds() < LIST_WINDOW_COOLDOWN_S
            except (TypeError, ValueError):
                within_cooldown = False
        if within_cooldown:
            decision = "deny"
            reason = LIST_WINDOW_RATE_REASON
            entry = {
                "ts": now.isoformat(),
                "event": "pre_tool_use",
                "tool": name,
                "decision": decision,
                "reason": reason,
                "target_snippet": "rate_limit",
                "allow_destructive": allow_destructive,
                "session": os.environ.get("GROK_SESSION_ID") or os.environ.get("CLAUDE_SESSION_ID"),
            }
            _audit(entry)
            print(json.dumps({"decision": "deny", "reason": reason}))
            return 2
        # Allow this call; record timestamp so rapid re-list is throttled
        try:
            LIST_WINDOW_STATE.parent.mkdir(parents=True, exist_ok=True)
            LIST_WINDOW_STATE.write_text(
                json.dumps({
                    "ts": now.isoformat(),
                    "session": os.environ.get("GROK_SESSION_ID") or os.environ.get("CLAUDE_SESSION_ID"),
                }),
                encoding="utf-8",
            )
        except OSError:
            pass
        _audit({
            "ts": now.isoformat(),
            "event": "pre_tool_use",
            "tool": name,
            "decision": "allow",
            "reason": "list_window_allowed" + ("_forced" if allow_list else ""),
            "target_snippet": "",
            "allow_destructive": allow_destructive,
            "session": os.environ.get("GROK_SESSION_ID") or os.environ.get("CLAUDE_SESSION_ID"),
        })
        print(json.dumps({"decision": "allow"}))
        return 0

    if name in RISKY_TOOLS or name.startswith("AI-Hands__hands_click"):
        if DESTRUCTIVE_RE.search(target) and not allow_destructive:
            decision = "deny"
            reason = (
                "AI-Hands safety: target looks destructive "
                f"({target[:120]!r}). Set allow_destructive=true only after "
                "explicit user approval, or rename/reselect a safer control."
            )
        elif name == "AI-Hands__hands_fill_form" and auto_submit and CONFIRMATION_RE.search(
            " ".join(str(x) for x in (inp.get("submit_label") or ["submit"]))
        ):
            labels = " ".join(str(x) for x in (inp.get("submit_label") or []))
            if DESTRUCTIVE_RE.search(labels or "submit"):
                decision = "deny"
                reason = (
                    "AI-Hands safety: hands_fill_form auto_submit with destructive "
                    "submit label blocked. Confirm with user first."
                )

    entry = {
        "ts": datetime.now(timezone.utc).isoformat(),
        "event": "pre_tool_use",
        "tool": name,
        "decision": decision,
        "reason": reason,
        "target_snippet": target[:200],
        "allow_destructive": allow_destructive,
        "session": os.environ.get("GROK_SESSION_ID") or os.environ.get("CLAUDE_SESSION_ID"),
    }
    _audit(entry)

    if decision == "deny":
        print(json.dumps({"decision": "deny", "reason": reason}))
        return 2

    print(json.dumps({"decision": "allow"}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
