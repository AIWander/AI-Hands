#!/usr/bin/env python3
"""Single policy owner for the AIWander portable agent pack.

Host adapters only identify the host and event. All name normalization, safety
decisions, redaction, cooldown state, and auditing live here. Risk consent is
accepted only as a signed, top-level, exact-call-bound token created by a
trusted host integration; tool arguments cannot self-confirm an action.
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import os
import re
import secrets
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


HANDS_PREFIXES = ("mcp__hands__", "AI-Hands__", "hands__")
PROGRAMMER_PREFIXES = (
    "mcp__programmer-wander__",
    "mcp__programmer__",
    "programmer-wander__",
    "programmer__",
)
WRAPPER_NAMES = {"use_tool", "CallMcpTool"}
CONSENT_FIELD = "_aiwander_host_consent"
CONSENT_PURPOSE = "risky-action"
CONSENT_KEY_ENV = "AIWANDER_POLICY_CONSENT_HMAC_KEY"
CONSENT_MAX_FUTURE_SECONDS = 300
CONSENT_TOKEN_KEYS = {
    "version",
    "purpose",
    "host",
    "tool",
    "args_sha256",
    "expires_at",
    "nonce",
    "signature",
}

SENSITIVE_FRAGMENTS = (
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "authorization",
    "cookie",
    "totp",
    "credential",
)
SAFE_REFERENCE_KEYS = {
    "credential_name",
    "credential_ref",
    "totp_name",
    "totp_ref",
}
NON_SECRET_KEY_SUFFIXES = (
    "_selector",
    "_field",
    "_label",
    "_name",
    "_id",
    "_path",
    "_url",
    "_uri",
    "_hint",
    "_pattern",
)
LABEL_KEYS = {"field", "key", "label", "name", "type"}
VALUE_KEYS = {"content", "input", "text", "value"}
METADATA_ONLY_KEYS = {
    "bodies",
    "body",
    "cmd",
    "cmds",
    "command",
    "commands",
    "header",
    "headers",
    "script",
    "scripts",
    "text",
    "texts",
}
METADATA_ONLY_SUFFIXES = (
    "_body",
    "_cmd",
    "_command",
    "_header",
    "_headers",
    "_script",
    "_text",
)

HANDS_ACTION_TOOLS = {
    "browser_agent",
    "browser_batch",
    "browser_click",
    "browser_eval",
    "browser_evaluate",
    "browser_fill_form",
    "browser_hover",
    "browser_inject_script",
    "browser_navigate",
    "browser_press",
    "browser_script",
    "browser_scroll",
    "browser_select",
    "browser_submit_form",
    "browser_type",
    "drag",
    "element_drag",
    "file_upload",
    "find_and_click",
    "hands_app_action",
    "hands_click",
    "hands_fill_form",
    "hands_login_recovery",
    "hands_navigate",
    "hands_script",
    "hands_type",
    "retry_click",
    "type_into_window",
    "uia_app_launch",
    "uia_batch",
    "uia_click",
    "uia_hold_key",
    "uia_key_press",
    "uia_shortcut",
    "uia_type",
}
HANDS_ALWAYS_CONSENT = {
    "browser_route": "security-sensitive network routing",
    "browser_route_clear": "security-sensitive network routing",
    "browser_route_remove": "security-sensitive network routing",
    "browser_submit_form": "external form submission",
    "file_upload": "external file upload",
    "hands_login_recovery": "account recovery or login",
    "hands_plugin_call": "native plugin execution",
    "hands_plugin_load": "native plugin loading",
    "hands_plugin_unload": "native plugin unloading",
}

PROGRAMMER_COMMAND_TOOLS = {
    "bash",
    "chain",
    "powershell",
    "psession_run",
    "run",
    "session_create",
    "shortcut",
    "shortcut_chain",
    "smart_exec",
    "wsl_bg",
    "wsl_run",
}
PROGRAMMER_ALWAYS_CONSENT = {
    "git_push": "external git push",
    "kill_process": "process termination",
    "psession_destroy": "process-session termination",
    "session_clear_recovery": "recovery-state deletion",
    "session_destroy": "process-session termination",
}

FINANCIAL_ACTION_RE = re.compile(
    r"\b(pay|buy|purchase|checkout|place\s+(?:the\s+)?order|confirm\s+(?:the\s+)?purchase|"
    r"transfer|wire|send\s+money|approve\s+payment|withdraw|deposit|donate|bid|trade|invest)\b",
    re.IGNORECASE,
)
DESTRUCTIVE_ACTION_RE = re.compile(
    r"\b(delete|destroy|wipe|erase|factory\s+reset|close\s+account|deactivate|"
    r"cancel\s+subscription|unsubscribe|remove\s+(?:account|user|member|data))\b",
    re.IGNORECASE,
)
EXTERNAL_SEND_RE = re.compile(
    r"\b(send|submit|publish|post|upload|share|email|message|reply|comment|"
    r"invite|notify|webhook)\b|\b(fetch|sendBeacon|XMLHttpRequest)\s*\(",
    re.IGNORECASE,
)
ACCOUNT_ACTION_RE = re.compile(
    r"\b(sign\s*in|log\s*in|login|sign\s*up|register|create\s+(?:an?\s+)?account|account\s+recovery|"
    r"recover\s+account|reset\s+password|change\s+password|link\s+account|"
    r"unlink\s+account)\b",
    re.IGNORECASE,
)
PERMISSION_ACTION_RE = re.compile(
    r"\b(allow|authorize|grant|revoke|permission|access\s+control|approve\s+access|"
    r"make\s+(?:it\s+)?public|"
    r"make\s+(?:it\s+)?private|add\s+(?:a\s+)?(?:user|member|admin)|"
    r"remove\s+(?:a\s+)?(?:user|member|admin)|change\s+role|administrator|"
    r"chmod|chown|set-acl|icacls)\b",
    re.IGNORECASE,
)
SECURITY_ACTION_RE = re.compile(
    r"\b(disable|turn\s+off|bypass|weaken|reset|rotate|regenerate)\b[^\r\n]{0,50}"
    r"\b(mfa|2fa|two.factor|security|firewall|antivirus|defender|certificate|api\s*key)\b|"
    r"\b(document\.cookie|localStorage|sessionStorage|recovery\s+codes?)\b",
    re.IGNORECASE,
)

COMMAND_PREFIX_RE = (
    r"(?:^|[;&|{(]\s*|(?:sudo|command|env)\s+|"
    r"(?:cmd(?:\.exe)?\s+/[ck]|(?:ba)?sh\s+-[lc]+|"
    r"(?:pwsh|powershell)(?:\.exe)?\s+(?:-[a-z]*command|-c))\s+[\"']?)"
)
DELETE_COMMAND_RE = re.compile(
    rf"(?ix){COMMAND_PREFIX_RE}(?:"
    r"remove-item|rm|rmdir|rd|del|erase|unlink"
    r")\b|\bcurl\b[^\r\n]*\s-X\s+DELETE\b|"
    r"\binvoke-(?:restmethod|webrequest)\b[^\r\n]*\s-method\s+delete\b"
)
GIT_IRREVERSIBLE_RE = re.compile(
    rf"(?ix){COMMAND_PREFIX_RE}git\b[^\r\n]*\b(?:"
    r"reset\s+--hard|clean\s+-[a-z]*f|checkout\b[^\r\n]*--force|"
    r"branch\s+-D|push\b"
    r")"
)
PROCESS_KILL_RE = re.compile(
    rf"(?ix){COMMAND_PREFIX_RE}(?:"
    r"kill|killall|pkill|taskkill|stop-process|terminate-process"
    r")\b"
)
DEPLOY_COMMAND_RE = re.compile(
    rf"(?ix){COMMAND_PREFIX_RE}(?:"
    r"docker\s+push|npm\s+publish|cargo\s+publish|twine\s+upload|"
    r"dotnet\s+nuget\s+push|gh\s+release\s+create|"
    r"kubectl\s+(?:apply|create|delete|patch|replace)|"
    r"helm\s+(?:install|upgrade|uninstall)|"
    r"terraform\s+(?:apply|destroy)|pulumi\s+up|"
    r"vercel(?:\s+deploy|\s+--prod)|netlify\s+deploy|"
    r"fly(?:ctl)?\s+deploy|gcloud\b[^\r\n]*\s+deploy|"
    r"aws\b[^\r\n]*\s+deploy|az\b[^\r\n]*\s+deploy"
    r")\b"
)
NETWORK_TOOL_RE = re.compile(
    r"(network|route|trace|learn_api|performance_log)", re.IGNORECASE
)
VOLUMES_PATH_RE = re.compile(r"(?i)(?:^|[\\/])Volumes(?:[\\/]|$)")
AUTHORIZATION_SECRET_RE = re.compile(
    r"(?i)\bauthorization\s*[:=]\s*(?:bearer\s+|basic\s+)?[^\s\"']{3,}"
)
BEARER_SECRET_RE = re.compile(r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{4,}")
NAMED_SECRET_ASSIGNMENT_RE = re.compile(
    r"(?i)(?:--(?:password|passwd|token|api[-_]?key)\s+|"
    r"\b(?:password|passwd|token|api[_-]?key|apikey)\s*[:=]\s*)"
    r"[\"']?[^\s\"';]{1,}|"
    r"[\"'](?:password|passwd|token|api[_-]?key|apikey)[\"']\s*"
    r"(?:\]|:)\s*=*\s*[\"'][^\"']+[\"']"
)


@dataclass(frozen=True)
class ResolvedCall:
    raw_name: str
    args: dict[str, Any]
    parse_error: str | None
    managed_or_wrapper: bool


def utc_now() -> datetime:
    return datetime.now(timezone.utc)


def data_dir() -> Path:
    configured = os.environ.get("AIWANDER_POLICY_DATA")
    root = (
        Path(configured).expanduser()
        if configured
        else Path.home() / ".aiwander-agent-pack"
    )
    root.mkdir(parents=True, exist_ok=True)
    return root


def _payload_tool_name(payload: dict[str, Any]) -> str:
    return str(
        payload.get("toolName") or payload.get("tool_name") or payload.get("name") or ""
    )


def _first_present(
    mapping: dict[str, Any], names: tuple[str, ...], default: Any = None
) -> Any:
    for name in names:
        if name in mapping:
            return mapping[name]
    return default


def _parse_object(value: Any, label: str) -> tuple[dict[str, Any], str | None]:
    if value is None:
        return {}, None
    if isinstance(value, dict):
        return value, None
    if isinstance(value, str):
        try:
            decoded = json.loads(value)
        except json.JSONDecodeError:
            return {}, f"{label} is malformed JSON"
        if isinstance(decoded, dict):
            return decoded, None
        return {}, f"{label} must decode to an object"
    return {}, f"{label} must be an object or a JSON object string"


def resolve_call(payload: dict[str, Any]) -> ResolvedCall:
    raw_name = _payload_tool_name(payload)
    namespace, _ = canonicalize_tool_name(raw_name)
    is_wrapper = raw_name in WRAPPER_NAMES or raw_name.endswith("use_tool")
    outer_raw = _first_present(payload, ("toolInput", "tool_input", "input"), {})
    outer, outer_error = _parse_object(outer_raw, "tool input")
    if outer_error:
        return ResolvedCall(
            raw_name, {}, outer_error, namespace is not None or is_wrapper
        )

    if is_wrapper:
        qualified = str(_first_present(outer, ("tool_name", "name", "tool"), ""))
        server = str(_first_present(outer, ("server_name", "server"), ""))
        server_key = server.strip().lower().replace("_", "-")
        if qualified and canonicalize_tool_name(qualified)[0] is None:
            if server_key in {"ai-hands", "hands"}:
                qualified = f"hands__{qualified}"
            elif server_key in {"programmer", "programmer-wander"}:
                qualified = f"programmer-wander__{qualified}"
        raw_name = qualified
        namespace, _ = canonicalize_tool_name(raw_name)
        nested_raw = _first_present(outer, ("tool_input", "arguments", "input"), {})
        nested, nested_error = _parse_object(nested_raw, "wrapped tool arguments")
        if not raw_name and nested_error is None:
            nested_error = "wrapper target tool name is missing"
        return ResolvedCall(
            raw_name, nested, nested_error, namespace is not None or not raw_name
        )

    if "arguments" in outer:
        nested, nested_error = _parse_object(outer["arguments"], "tool arguments")
        return ResolvedCall(raw_name, nested, nested_error, namespace is not None)
    return ResolvedCall(raw_name, outer, None, namespace is not None)


def unwrap_call(payload: dict[str, Any]) -> tuple[str, dict[str, Any]]:
    """Compatibility helper; enforcement uses resolve_call so parse errors survive."""
    resolved = resolve_call(payload)
    return resolved.raw_name, resolved.args


def canonicalize_tool_name(name: str) -> tuple[str | None, str]:
    for prefix in HANDS_PREFIXES:
        if name.startswith(prefix):
            return "hands", name[len(prefix) :]
    for prefix in PROGRAMMER_PREFIXES:
        if name.startswith(prefix):
            return "programmer-wander", name[len(prefix) :]
    return None, name


def canonical_args_sha256(args: dict[str, Any]) -> str:
    encoded = json.dumps(
        args, sort_keys=True, separators=(",", ":"), ensure_ascii=True
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def _consent_signing_bytes(token: dict[str, Any]) -> bytes:
    signed = {key: token[key] for key in sorted(CONSENT_TOKEN_KEYS - {"signature"})}
    return json.dumps(
        signed, sort_keys=True, separators=(",", ":"), ensure_ascii=True
    ).encode("utf-8")


def create_host_consent_token(
    secret: str,
    host: str,
    raw_tool: str,
    args: dict[str, Any],
    *,
    expires_at: int | None = None,
    nonce: str | None = None,
) -> dict[str, Any]:
    """Create a token for trusted host integrations; shipped adapters never call this."""
    token: dict[str, Any] = {
        "version": 1,
        "purpose": CONSENT_PURPOSE,
        "host": host,
        "tool": raw_tool,
        "args_sha256": canonical_args_sha256(args),
        "expires_at": expires_at if expires_at is not None else int(time.time()) + 60,
        "nonce": nonce if nonce is not None else secrets.token_hex(16),
    }
    token["signature"] = hmac.new(
        secret.encode("utf-8"), _consent_signing_bytes(token), hashlib.sha256
    ).hexdigest()
    return token


def validate_host_consent(
    payload: dict[str, Any], host: str, raw_tool: str, args: dict[str, Any]
) -> tuple[bool, str | None, bool]:
    if CONSENT_FIELD not in payload:
        return False, None, False
    token = payload.get(CONSENT_FIELD)
    if not isinstance(token, dict) or set(token) != CONSENT_TOKEN_KEYS:
        return False, "Host consent token has an invalid shape", True
    secret = os.environ.get(CONSENT_KEY_ENV, "")
    if len(secret) < 32:
        return False, "Host consent verification is not configured", True
    try:
        expires_at = int(token["expires_at"])
    except (TypeError, ValueError):
        return False, "Host consent token has an invalid expiry", True
    now = int(time.time())
    if expires_at < now or expires_at > now + CONSENT_MAX_FUTURE_SECONDS:
        return (
            False,
            "Host consent token is expired or exceeds the short validity window",
            True,
        )
    if token.get("version") != 1 or token.get("purpose") != CONSENT_PURPOSE:
        return False, "Host consent token version or purpose is invalid", True
    if token.get("host") != host or token.get("tool") != raw_tool:
        return (
            False,
            "Host consent token is not bound to this host and exact tool name",
            True,
        )
    if token.get("args_sha256") != canonical_args_sha256(args):
        return False, "Host consent token is not bound to these exact arguments", True
    nonce = token.get("nonce")
    signature = token.get("signature")
    if not isinstance(nonce, str) or len(nonce) < 16 or not isinstance(signature, str):
        return False, "Host consent token nonce or signature is invalid", True
    expected = hmac.new(
        secret.encode("utf-8"), _consent_signing_bytes(token), hashlib.sha256
    ).hexdigest()
    if not hmac.compare_digest(signature, expected):
        return False, "Host consent token signature is invalid", True
    return True, None, True


def _flatten_text(value: Any, depth: int = 0) -> str:
    if depth > 6:
        return ""
    if isinstance(value, dict):
        return " ".join(
            f"{key} {_flatten_text(item, depth + 1)}" for key, item in value.items()
        )
    if isinstance(value, list):
        return " ".join(_flatten_text(item, depth + 1) for item in value[:100])
    return str(value) if value is not None else ""


def _secretish_key(key: Any) -> bool:
    lowered = _normalize_key(key)
    return (
        lowered not in SAFE_REFERENCE_KEYS
        and not lowered.endswith(NON_SECRET_KEY_SUFFIXES)
        and any(fragment in lowered for fragment in SENSITIVE_FRAGMENTS)
    )


def _label_describes_secret(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    lowered = _normalize_key(value).replace(" ", "_")
    return (
        lowered not in SAFE_REFERENCE_KEYS
        and not lowered.endswith(NON_SECRET_KEY_SUFFIXES)
        and any(fragment in lowered for fragment in SENSITIVE_FRAGMENTS)
    )


def _normalize_key(key: Any) -> str:
    raw = str(key).strip()
    snake = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", raw)
    return snake.lower().replace("-", "_")


def _label_key(key: Any) -> bool:
    lowered = _normalize_key(key)
    return lowered in LABEL_KEYS | {
        "aria_label",
        "placeholder",
        "selector",
    } or lowered.endswith(
        ("_field", "_key", "_label", "_name", "_placeholder", "_selector", "_type")
    )


def _value_key(key: Any) -> bool:
    lowered = _normalize_key(key)
    return lowered in VALUE_KEYS | {"val"} or lowered.endswith(
        ("_content", "_input", "_text", "_val", "_value")
    )


def _has_nonempty_scalar(value: Any, depth: int = 0) -> bool:
    if depth > 6:
        return False
    if isinstance(value, dict):
        return any(_has_nonempty_scalar(item, depth + 1) for item in value.values())
    if isinstance(value, list):
        return any(_has_nonempty_scalar(item, depth + 1) for item in value)
    return value is not None and bool(str(value).strip())


def _contains_secret_text(text: str) -> bool:
    return any(
        pattern.search(text)
        for pattern in (
            AUTHORIZATION_SECRET_RE,
            BEARER_SECRET_RE,
            NAMED_SECRET_ASSIGNMENT_RE,
        )
    )


def _has_plaintext_secret(value: Any, depth: int = 0) -> bool:
    if depth > 7:
        return False
    if isinstance(value, dict):
        secret_label = any(
            _label_key(key) and _label_describes_secret(item)
            for key, item in value.items()
        )
        if secret_label and any(
            _value_key(key) and _has_nonempty_scalar(item)
            for key, item in value.items()
        ):
            return True
        for key, item in value.items():
            if _secretish_key(key) and _has_nonempty_scalar(item):
                return True
            if _has_plaintext_secret(item, depth + 1):
                return True
        return False
    if isinstance(value, list):
        return any(_has_plaintext_secret(item, depth + 1) for item in value[:100])
    return isinstance(value, str) and _contains_secret_text(value)


def _metadata_only(value: Any) -> dict[str, Any]:
    if isinstance(value, str):
        return {"storage": "metadata-only", "kind": "string", "length": len(value)}
    if isinstance(value, dict):
        return {"storage": "metadata-only", "kind": "object", "item_count": len(value)}
    if isinstance(value, list):
        return {"storage": "metadata-only", "kind": "array", "item_count": len(value)}
    return {"storage": "metadata-only", "kind": type(value).__name__}


def _metadata_only_key(key: Any) -> bool:
    lowered = _normalize_key(key)
    return lowered in METADATA_ONLY_KEYS or lowered.endswith(METADATA_ONLY_SUFFIXES)


def redact(value: Any, depth: int = 0) -> Any:
    if depth > 7:
        return "[TRUNCATED]"
    if isinstance(value, dict):
        secret_label = any(
            _label_key(key) and _label_describes_secret(item)
            for key, item in value.items()
        )
        out: dict[str, Any] = {}
        for key, item in value.items():
            if _metadata_only_key(key):
                out[key] = _metadata_only(item)
            elif _secretish_key(key) or (secret_label and _value_key(key)):
                out[key] = "[REDACTED]"
            else:
                out[key] = redact(item, depth + 1)
        return out
    if isinstance(value, list):
        return [redact(item, depth + 1) for item in value[:100]]
    if isinstance(value, str):
        if _contains_secret_text(value):
            return "[REDACTED]"
        if len(value) > 1000:
            return value[:1000] + "[TRUNCATED]"
    return value


def _cooldown_decision(args: dict[str, Any]) -> tuple[str, str | None]:
    if os.environ.get("AIWANDER_ALLOW_UIA_LIST", "").lower() in {
        "1",
        "true",
        "yes",
        "on",
    }:
        return "allow", None
    seconds = float(os.environ.get("AIWANDER_UIA_LIST_COOLDOWN_S", "45"))
    state_path = data_dir() / "uia_list_window.json"
    now = utc_now()
    try:
        prior = json.loads(state_path.read_text(encoding="utf-8"))
        last = datetime.fromisoformat(str(prior["ts"]).replace("Z", "+00:00"))
        if (now - last).total_seconds() < seconds:
            return (
                "deny",
                f"uia_list_window is rate-limited for {seconds:.0f}s; reuse the cached list or focus a known title",
            )
    except (OSError, KeyError, TypeError, ValueError, json.JSONDecodeError):
        pass
    try:
        state_path.write_text(
            json.dumps({"ts": now.isoformat(), "args": redact(args)}), encoding="utf-8"
        )
    except OSError:
        return "deny", "uia_list_window cooldown state could not be recorded safely"
    return "allow", None


def _hands_risk_reason(tool: str, args: dict[str, Any]) -> str | None:
    if tool in HANDS_ALWAYS_CONSENT:
        return HANDS_ALWAYS_CONSENT[tool]
    if tool == "hands_monitor_scope" and str(args.get("action", "get")).lower() in {
        "clear",
        "set",
    }:
        return "monitor security-scope change"
    if tool == "browser_cookies" and str(args.get("action", "get")).lower() in {
        "add",
        "clear",
        "delete",
        "import",
        "remove",
        "set",
    }:
        return "browser credential or security-state change"
    if tool not in HANDS_ACTION_TOOLS:
        return None
    text = _flatten_text(args)
    for label, pattern in (
        ("financial action", FINANCIAL_ACTION_RE),
        ("destructive action", DESTRUCTIVE_ACTION_RE),
        ("external send or publish", EXTERNAL_SEND_RE),
        ("account action", ACCOUNT_ACTION_RE),
        ("permission change", PERMISSION_ACTION_RE),
        ("security-setting change", SECURITY_ACTION_RE),
    ):
        if pattern.search(text):
            return label
    compact = re.sub(r"[\s_+-]+", "", text).lower()
    if "ctrlenter" in compact or "controlenter" in compact:
        return "external send shortcut"
    return None


def _programmer_risk_reason(tool: str, args: dict[str, Any]) -> str | None:
    if tool in PROGRAMMER_ALWAYS_CONSENT:
        return PROGRAMMER_ALWAYS_CONSENT[tool]
    lowered_tool = tool.lower()
    if lowered_tool.startswith(("delete_", "remove_", "kill_", "terminate_")):
        return "deletion or process termination"
    if "deploy" in lowered_tool and tool != "deploy_preflight":
        return "external deployment"
    if tool == "http_request":
        method = str(args.get("method", "GET")).upper()
        if method not in {"GET", "HEAD", "OPTIONS"}:
            return f"external HTTP {method} request"
    if tool == "transform_sync_dir" and any(
        args.get(key) is True
        for key in ("delete_extra", "delete_extras", "mirror", "prune")
    ):
        return "synchronization that can delete destination content"
    if tool == "git_checkout" and args.get("force") is True:
        return "forced git checkout"
    if tool == "git_branch" and (
        args.get("delete") is True
        or str(args.get("action") or args.get("operation") or "").lower()
        in {"delete", "remove"}
    ):
        return "git branch deletion"
    if tool == "git_stash" and str(args.get("action") or "").lower() in {
        "clear",
        "drop",
    }:
        return "git stash deletion"
    if tool in PROGRAMMER_COMMAND_TOOLS or tool in {
        "git_branch",
        "git_checkout",
        "git_clean",
        "git_reset",
        "git_stash",
    }:
        command_values: list[str] = []
        for key, value in args.items():
            if _metadata_only_key(key):
                if isinstance(value, str):
                    command_values.append(value)
                elif isinstance(value, list):
                    command_values.extend(
                        str(item) for item in value if isinstance(item, str)
                    )
        if not command_values:
            command_values.append(_flatten_text(args))
        for text in command_values:
            for label, pattern in (
                ("ordinary deletion", DELETE_COMMAND_RE),
                ("force, hard reset, clean, or git push", GIT_IRREVERSIBLE_RE),
                ("process termination", PROCESS_KILL_RE),
                ("external deployment or publication", DEPLOY_COMMAND_RE),
            ):
                if pattern.search(text):
                    return label
    return None


def evaluate(
    namespace: str | None,
    tool: str,
    args: dict[str, Any],
    *,
    host_consent: bool = False,
) -> tuple[str, str | None]:
    if namespace in {"hands", "programmer-wander"} and _has_plaintext_secret(args):
        return (
            "deny",
            "Plaintext secrets are not accepted; use a credential reference owned by Workflow",
        )
    if namespace == "hands":
        if tool == "uia_list_window":
            return _cooldown_decision(args)
        text = _flatten_text(args)
        if NETWORK_TOOL_RE.search(tool) and VOLUMES_PATH_RE.search(text):
            return (
                "deny",
                "Raw network capture must remain ephemeral and cannot be written into Volumes",
            )
        reason = _hands_risk_reason(tool, args)
        if reason and not host_consent:
            return (
                "deny",
                f"Hands {reason} requires a trusted host consent token bound to this exact call",
            )
    elif namespace == "programmer-wander":
        reason = _programmer_risk_reason(tool, args)
        if reason and not host_consent:
            return (
                "deny",
                f"Programmer-Wander {reason} requires a trusted host consent token bound to this exact call",
            )
    return "allow", None


def emit_decision(decision: str, reason: str | None, event: str) -> None:
    specific: dict[str, Any] = {"hookEventName": event, "permissionDecision": decision}
    output: dict[str, Any] = {"decision": decision, "hookSpecificOutput": specific}
    if reason:
        output["reason"] = reason
        specific["permissionDecisionReason"] = reason
    sys.stdout.write(json.dumps(output, ensure_ascii=True))


def audit(
    event: str,
    host: str,
    raw_name: str,
    namespace: str | None,
    tool: str,
    args: dict[str, Any],
    decision: str,
    reason: str | None,
    payload: dict[str, Any],
    consent_status: str = "absent",
) -> bool:
    session = (
        payload.get("session_id")
        or payload.get("sessionId")
        or os.environ.get("CODEX_THREAD_ID")
        or os.environ.get("CLAUDE_SESSION_ID")
        or os.environ.get("GROK_SESSION_ID")
    )
    cwd = payload.get("cwd") or os.environ.get("GROK_WORKSPACE_ROOT")
    entry = {
        "ts": utc_now().isoformat(),
        "event": event,
        "host": host,
        "raw_tool": redact(raw_name),
        "canonical_tool": redact(f"{namespace}:{tool}") if namespace else None,
        "decision": decision,
        "reason": reason,
        "consent": consent_status,
        "input": redact(args),
        "session": redact(session),
        "cwd": redact(cwd),
    }
    try:
        with (data_dir() / "events.jsonl").open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(entry, ensure_ascii=True) + "\n")
    except OSError:
        return False
    return True


def run(event: str, host: str, payload: dict[str, Any]) -> int:
    resolved = resolve_call(payload)
    namespace, tool = canonicalize_tool_name(resolved.raw_name)
    is_pre = event.lower().startswith("pre")
    if resolved.parse_error and resolved.managed_or_wrapper:
        reason = "Managed tool arguments could not be parsed safely"
        audit(
            event, host, resolved.raw_name, namespace, tool, {}, "deny", reason, payload
        )
        if is_pre:
            emit_decision("deny", reason, event)
        return 0
    if namespace is None:
        if is_pre:
            emit_decision("allow", None, event)
        return 0

    consent, consent_error, consent_present = validate_host_consent(
        payload, host, resolved.raw_name, resolved.args
    )
    consent_status = "valid" if consent else "invalid" if consent_present else "absent"
    if is_pre and consent_error:
        decision, reason = "deny", consent_error
    elif is_pre:
        decision, reason = evaluate(
            namespace, tool, resolved.args, host_consent=consent
        )
    else:
        decision, reason = "allow", None
    audit_ok = audit(
        event,
        host,
        resolved.raw_name,
        namespace,
        tool,
        resolved.args,
        decision,
        reason,
        payload,
        consent_status,
    )
    if is_pre and not audit_ok:
        decision = "deny"
        reason = "Policy audit could not be written; refusing the managed call"
    if is_pre:
        emit_decision(decision, reason, event)
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--event", required=True)
    parser.add_argument(
        "--host", default=os.environ.get("AIWANDER_POLICY_HOST", "generic")
    )
    args = parser.parse_args(argv)
    if os.environ.get("AIWANDER_POLICY_ACTIVE") == "1":
        if args.event.lower().startswith("pre"):
            emit_decision(
                "deny",
                "Policy recursion guard is active; refusing the managed call",
                args.event,
            )
        return 0
    os.environ["AIWANDER_POLICY_ACTIVE"] = "1"
    try:
        payload = json.load(sys.stdin)
        if not isinstance(payload, dict):
            raise ValueError("hook payload must be an object")
        return run(args.event, args.host, payload)
    except Exception as error:
        try:
            audit(
                "policy_error",
                args.host,
                "",
                None,
                "",
                {},
                "deny" if args.event.lower().startswith("pre") else "error",
                f"internal policy error: {type(error).__name__}",
                {},
            )
        except Exception:
            pass
        if args.event.lower().startswith("pre"):
            emit_decision(
                "deny", "Policy engine error; refusing the managed call", args.event
            )
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
