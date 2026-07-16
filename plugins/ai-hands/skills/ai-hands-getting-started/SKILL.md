---
name: ai-hands-getting-started
description: Install, connect, and verify the optional AI-Hands Windows MCP plugin and choose a safe monitor scope. Use when setting up AI-Hands in Codex, Claude, Grok, ChatGPT or another MCP host, checking whether hands.exe is discoverable, or validating a new installation before real automation.
---

# Get started with AI-Hands

1. Confirm `hands.exe` resolves on `PATH` and run it only through an MCP host or a bounded smoke test.
2. Install or enable this plugin through the host's supported plugin or marketplace UI. Do not hand-edit another AI's live hook or config surface.
3. Keep `HANDS_TOOL_PROFILE=default` for the recommended safe-advertised surface: 104 operational tools plus the catalog (105 entries). `full` and `strict` are also safe-advertised at 107 and 109 entries including the catalog.
4. Start a fresh host session and confirm the `hands` MCP server appears in the tool list.
5. Call a read-only status tool, then `hands_monitor_scope(action="list")`.
6. For interactive use, select primary scope. For unattended use, select a physical stable ID and lock it.
7. Run a harmless capture or element lookup inside the chosen display and verify the returned bounds stay within scope.
8. Keep optional hooks disabled until their exact definitions have been reviewed, trusted by the host, and proven with a harmless runtime event.

`compatibility` exposes 144 entries and is an unsafe debug escape hatch. Raw, built-in direct-fetch, or native-plugin calls require the compatibility profile, the matching `HANDS_ALLOW_UNSAFE_RAW_TOOLS=1`, `HANDS_ALLOW_UNSAFE_DIRECT_FETCH=1`, or `HANDS_ALLOW_UNSAFE_PLUGINS=1` process gate, and the matching `allow_unsafe_raw=true`, `allow_unsafe_fetch=true`, or `allow_unsafe_plugin=true` per-call acknowledgement. The composite `browser_script` and `browser_evaluate` tools require both the direct-fetch and raw gate pairs. Whenever monitor scope is active, these vendor composites and aliases fail closed even when every compatibility gate is present: `browser_agent`/`agent`, `browser_batch`/`batch`, `browser_script`/`script`, `browser_evaluate`/`evaluate`, `browser_screenshot_burst`/`screenshot_burst`, `browser_scroll_collect`/`scroll_collect`, `browser_wait_stable`/`wait_stable`, and `retry_click` (with `browser_retry_click` treated defensively as an alias). Their nested vendor steps cannot revalidate the bound browser window. Use individually scoped browser calls or compatibility-gated `hands_script`, which centrally revalidates each nested call. Do not use compatibility as the speed path; use visible browser observation and keep durable/direct API methods with Workflow or another dedicated web/network owner. Hands can still launch external desktop applications and is not a general OS or secrecy sandbox.

Before enabling a monitor fence, clear browser routes and stop any active trace; fence activation refuses while either persistent state is active. Under an active fence, `browser_route` and `browser_trace_start` fail closed, while `browser_route_remove`, `browser_route_clear`, and `browser_trace_stop` remain available so cleanup cannot be trapped.

The Hands HTTP dashboard is disabled by default. Opt in only by setting `HANDS_ENABLE_DASHBOARD=1` after reviewing the local listening boundary.

Read `../../instructions/APPLY_TO_YOUR_AI.txt` for host-specific application commands. Plugin prose does not grant tool access; the MCP registration and executable must both be active.
