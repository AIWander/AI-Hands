---
name: ai-hands-getting-started
description: Install, connect, and verify the optional AI-Hands Windows MCP plugin and choose a safe monitor scope. Use when setting up AI-Hands in Codex, Claude, Grok, ChatGPT or another MCP host, checking whether hands.exe is discoverable, or validating a new installation before real automation.
---

# Get started with AI-Hands

1. Confirm `hands.exe` resolves on `PATH` and run it only through an MCP host or a bounded smoke test.
2. Install or enable this plugin through the host's supported plugin or marketplace UI. Do not hand-edit another AI's live hook or config surface.
3. Start a fresh host session and confirm the `hands` MCP server appears in the tool list.
4. Call a read-only status tool, then `hands_monitor_scope(action="list")`.
5. For interactive use, select primary scope. For unattended use, select a physical stable ID and lock it.
6. Run a harmless capture or element lookup inside the chosen display and verify the returned bounds stay within scope.
7. Keep optional hooks disabled until their exact definitions have been reviewed, trusted by the host, and proven with a harmless runtime event.

Read `../../instructions/APPLY_TO_YOUR_AI.txt` for host-specific application commands. Plugin prose does not grant tool access; the MCP registration and executable must both be active.
