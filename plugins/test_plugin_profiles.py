from __future__ import annotations

import json
import py_compile
import re
import tempfile
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PLUGINS = ROOT / "plugins"
HOOKS = PLUGINS / "ai-hands"
SKILLS = PLUGINS / "ai-hands-skills"
EXPECTED_SKILLS = {
    "ai-hands",
    "ai-hands-getting-started",
    "ai-hands-safety",
    "ai-hands-workflows",
}


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


class PluginProfileTests(unittest.TestCase):
    def test_exactly_two_current_profiles_are_advertised(self) -> None:
        agents = load_json(ROOT / ".agents" / "plugins" / "marketplace.json")
        claude = load_json(ROOT / ".claude-plugin" / "marketplace.json")
        expected = {"ai-hands", "ai-hands-skills"}
        self.assertEqual(expected, {item["name"] for item in agents["plugins"]})
        self.assertEqual(expected, {item["name"] for item in claude["plugins"]})

    def test_profiles_have_same_lean_skill_pack(self) -> None:
        for root in (HOOKS, SKILLS):
            found = {path.parent.name for path in (root / "skills").glob("*/SKILL.md")}
            self.assertEqual(EXPECTED_SKILLS, found)
            for skill in EXPECTED_SKILLS:
                text = (root / "skills" / skill / "SKILL.md").read_text()
                self.assertTrue(text.startswith("---\nname:"))
                self.assertIn("\ndescription:", text.split("---", 2)[1])

    def test_only_hook_capable_profile_contains_hook_code(self) -> None:
        self.assertTrue((HOOKS / "hooks" / "opt-in").is_dir())
        self.assertFalse((SKILLS / "hooks").exists())

    def test_hook_commands_use_portable_python_three_entrypoint(self) -> None:
        for name in ("codex-hooks.fragment.json", "claude-grok-hooks.fragment.json"):
            fragment = load_json(HOOKS / "hooks" / "opt-in" / name)
            for registrations in fragment["hooks"].values():
                for registration in registrations:
                    for hook in registration["hooks"]:
                        command = hook["command"]
                        self.assertTrue(command.startswith('python "'))
                        self.assertNotIn("py -3.11", command)

    def test_all_hook_python_entrypoints_compile(self) -> None:
        hook_root = HOOKS / "hooks" / "opt-in"
        sources = (
            hook_root / "adapters" / "codex" / "hook_adapter.py",
            hook_root / "adapters" / "claude-grok" / "hook_adapter.py",
            hook_root / "shared" / "policy" / "universal_policy.py",
        )
        with tempfile.TemporaryDirectory() as temp_dir:
            for index, source in enumerate(sources):
                py_compile.compile(
                    str(source),
                    cfile=str(Path(temp_dir) / f"hook-{index}.pyc"),
                    doraise=True,
                )

    def test_hands_policy_does_not_claim_other_tool_servers(self) -> None:
        policy = (
            HOOKS
            / "hooks"
            / "opt-in"
            / "shared"
            / "policy"
            / "universal_policy.py"
        ).read_text()
        self.assertNotIn("programmer", policy.lower())

    def test_both_profiles_retain_full_routing_and_safety_contract(self) -> None:
        required = {
            "ai-hands": (
                "browser_screenshot_burst",
                "browser_route_remove",
                "Safe profiles hide built-in first-party raw/direct-fetch/native-plugin front doors",
                "HANDS_ENABLE_DASHBOARD=1",
                "not a general OS sandbox",
            ),
            "ai-hands-getting-started": (
                "104 operational tools",
                "`compatibility` exposes 144 entries",
                "browser_trace_stop",
            ),
            "ai-hands-safety": (
                "recovery codes",
                "Pattern redaction is defense in depth",
                "separate Windows session or virtual machine",
            ),
            "ai-hands-workflows": (
                "Never let remembered site behavior replace current evidence",
                "hands_login_recovery",
                "## Desktop applications",
            ),
        }
        for root in (HOOKS, SKILLS):
            for skill, phrases in required.items():
                text = (root / "skills" / skill / "SKILL.md").read_text()
                for phrase in phrases:
                    self.assertIn(phrase, text, f"{root.name}/{skill}: {phrase}")

    def test_profile_manifest_names_match_directories(self) -> None:
        self.assertEqual(
            "ai-hands",
            load_json(HOOKS / ".codex-plugin" / "plugin.json")["name"],
        )
        self.assertEqual(
            "ai-hands-skills",
            load_json(SKILLS / ".codex-plugin" / "plugin.json")["name"],
        )

    def test_product_and_plugin_versions_are_aligned(self) -> None:
        cargo_version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"][
            "version"
        ]
        inno = (ROOT / "installers" / "inno" / "AIHands.iss").read_text()
        inno_match = re.search(r'^#define MyAppVersion "([^"]+)"', inno, re.MULTILINE)
        self.assertIsNotNone(inno_match)
        self.assertEqual(cargo_version, inno_match.group(1))
        for root in (HOOKS, SKILLS):
            for manifest in (
                root / ".codex-plugin" / "plugin.json",
                root / ".claude-plugin" / "plugin.json",
            ):
                self.assertEqual(cargo_version, load_json(manifest)["version"])

    def test_skills_only_guide_contains_hooklike_dispatch_and_truth_boundary(self) -> None:
        text = (SKILLS / "instructions" / "APPLY_TO_YOUR_AI.txt").read_text()
        for phrase in (
            "At the start of a task",
            "Before any Hands mutation",
            "For a multi-step automation",
            "re-observe after each short mutation burst",
            "behavioral guidance, not hard enforcement",
        ):
            self.assertIn(phrase, text)


if __name__ == "__main__":
    unittest.main()
