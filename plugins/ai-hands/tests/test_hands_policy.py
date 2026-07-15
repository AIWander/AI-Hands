from __future__ import annotations

import io
import json
import os
import sys
import tempfile
import time
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

PLUGIN_ROOT = Path(__file__).resolve().parents[1]
POLICY_DIR = PLUGIN_ROOT / "hooks" / "opt-in" / "shared" / "policy"
sys.path.insert(0, str(POLICY_DIR))

import universal_policy as policy


class HandsPolicyTests(unittest.TestCase):
    def test_hands_names_normalize_to_one_ability_owner(self) -> None:
        for raw in (
            "hands__hands_click",
            "AI-Hands__hands_click",
            "mcp__hands__hands_click",
        ):
            self.assertEqual(("hands", "hands_click"), policy.canonicalize_tool_name(raw))

    def test_model_booleans_do_not_self_confirm_risky_action(self) -> None:
        args = {
            "target": "Delete account",
            "allow_destructive": True,
            "confirmed_by_user": True,
        }
        self.assertEqual("deny", policy.evaluate("hands", "hands_click", args)[0])

    def test_exact_call_host_token_is_short_lived_and_argument_bound(self) -> None:
        secret = "test-only-consent-key-with-at-least-32-characters"
        args = {"target": "Delete account"}
        token = policy.create_host_consent_token(
            secret,
            "codex",
            "hands__hands_click",
            args,
            expires_at=int(time.time()) + 60,
        )
        payload = {
            "toolName": "hands__hands_click",
            "toolInput": args,
            policy.CONSENT_FIELD: token,
        }
        with (
            tempfile.TemporaryDirectory() as tmp,
            patch.dict(
                os.environ,
                {
                    "AIWANDER_POLICY_DATA": tmp,
                    policy.CONSENT_KEY_ENV: secret,
                },
                clear=False,
            ),
        ):
            stdout = io.StringIO()
            with redirect_stdout(stdout):
                policy.run("PreToolUse", "codex", payload)
            self.assertEqual("allow", json.loads(stdout.getvalue())["decision"])

            payload["toolInput"] = {"target": "Delete every account"}
            stdout = io.StringIO()
            with redirect_stdout(stdout):
                policy.run("PreToolUse", "codex", payload)
            self.assertEqual("deny", json.loads(stdout.getvalue())["decision"])

    def test_plaintext_secret_and_network_to_volumes_are_denied(self) -> None:
        self.assertEqual(
            "deny",
            policy.evaluate(
                "hands", "hands_fill_form", {"password": "not-a-real-secret"}
            )[0],
        )
        self.assertEqual(
            "deny",
            policy.evaluate(
                "hands",
                "browser_get_network_log",
                {"save_path": "C:\\ProtectedData\\Volumes\\capture.json"},
                host_consent=True,
            )[0],
        )

    def test_fragments_do_not_inject_consent_or_claim_auto_install(self) -> None:
        for path in (
            PLUGIN_ROOT / "hooks" / "opt-in" / "codex-hooks.fragment.json",
            PLUGIN_ROOT / "hooks" / "opt-in" / "claude-grok-hooks.fragment.json",
        ):
            text = path.read_text(encoding="utf-8")
            self.assertNotIn(policy.CONSENT_FIELD, text)
            self.assertNotIn(policy.CONSENT_KEY_ENV, text)


if __name__ == "__main__":
    unittest.main()
