---
title: "Hands MCP Server — Desktop Automation for AI Agents"
description: "Getting started guide for the Hands Rust MCP server. Covers safe-advertised browser automation, Windows UI Automation, vision, monitor scope, and the explicit unsafe compatibility boundary over the Model Context Protocol."
keywords:
  - MCP server
  - model context protocol server
  - desktop automation
  - Windows automation
  - AI agent tools
  - Claude tools
  - browser automation rust
  - chromiumoxide rust
  - UI automation Windows
  - accessibility tree automation
  - OCR tool
  - screenshot automation
  - Claude Desktop MCP
  - Claude Code MCP
  - computer use alternative
  - Claude computer use
  - headless browser
  - web scraping MCP
  - Windows UI Automation
  - UIA automation
  - MCP tool server
  - custom MCP server
  - rust mcp server
  - build MCP server rust
toc_block_lines: [280, 292]
toc_generated_at: 2026-04-14
---

# Getting Started with Hands

Hands is a Rust MCP server for browser automation, Windows UI Automation (UIA), vision/OCR, monitor-scoped action, and verification. The safe `default`, `full`, and `strict` profiles advertise 105, 107, and 109 entries including the catalog. It connects to Claude Desktop, Claude Code, or any MCP-compatible client over standard JSON-RPC on stdin/stdout.

Unlike Claude computer use, which relies on repeated screenshots and pixel-coordinate guessing, Hands gives AI agents direct access to the DOM, the Windows accessibility tree, and dedicated OCR --- each chosen for the task at hand. For a full comparison, see the [README](../README.md).

## Installation

### Prerequisites

- **Rust toolchain** (stable, 2021 edition or later)
- **Windows 10/11** (UIA tools require the Windows accessibility API)
- **Chrome** installed normally (any recent version) — Hands connects to Chrome over CDP, no binaries are downloaded

### Build from source

```bash
git clone https://github.com/AIWander/AI-Hands.git
cd AI-Hands
cargo build --release -p hands
```

The output binary lands at `target/release/hands.exe`. It is a single file with no runtime dependencies.

### Pre-built binaries

Download the latest Windows binaries from the [latest release](https://github.com/AIWander/AI-Hands/releases/latest):
- `hands-v1.0.0-x64.exe` — Windows x64
- `hands-v1.0.0-aarch64.exe` — Windows ARM64

### Configure for Claude Desktop

Add the server to your Claude Desktop config at `%APPDATA%\Claude\claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "hands": {
      "command": "C:/path/to/hands.exe",
      "args": []
    }
  }
}
```

### Configure for Claude Code

Add it to `~/.claude/mcp.json` (global) or `.mcp.json` (per-project):

```json
{
  "mcpServers": {
    "hands": {
      "command": "C:/path/to/hands.exe",
      "args": []
    }
  }
}
```

Restart Claude Desktop or Claude Code after editing. The recommended `default` profile will advertise 105 entries including `hands_capability_catalog`.

## Architecture Overview

```
hands.exe  (MCP tool server, stdin/stdout JSON-RPC)
  |
  +-- browser-mcp    chromiumoxide CDP: visible navigation, interaction, extraction, minimized network observation
  +-- uia-mcp        Windows UI Automation COM: find elements, click, type, window mgmt
  +-- vision-core    Screenshot capture, OCR, template matching, image diff
  +-- combo tools    Cross-tier helpers: find_and_click, read_screen_text, wait_for_visual
  +-- meta-tools     Intelligent orchestration: multi-step sequences with escalation (v1.2.1)
```

All three tiers compile into one binary. The MCP server reads JSON-RPC requests from stdin, dispatches to the appropriate tier, and returns results on stdout. No sidecar processes, no Node runtime, no Python --- just a Rust binary.

## Tool Tiers and Usage Examples

Every example below shows the raw JSON-RPC call. When using Claude Desktop or Claude Code, the client builds these calls automatically from natural-language requests.

### Browser Tier (safe-advertised tools)

The browser tier wraps chromiumoxide over CDP. In the safe profiles it handles visible browser sessions, structured text extraction, form interaction, bounded batches, multi-tab management, verification, and minimized current-network observation. Output/persistence minimization and pattern redaction are defense in depth, not a secrecy guarantee; use isolated sessions and least privilege because browser traffic can contain secrets.

**Navigate and extract text:**

```json
{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
  "name": "browser_navigate",
  "arguments": {"url": "https://example.com"}
}}
```

```json
{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
  "name": "browser_get_text",
  "arguments": {"selector": "article"}
}}
```

**Fill and submit a form:**

```json
{"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
  "name": "browser_fill_form",
  "arguments": {
    "fields": [
      {"selector": "#username", "value": "testuser"},
      {"selector": "#password", "value": "hunter2"}
    ]
  }
}}
```

```json
{"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {
  "name": "browser_click",
  "arguments": {"selector": "button[type='submit']"}
}}
```

**Batch multiple browser actions in one call** to cut down round-trips:

```json
{"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {
  "name": "browser_batch",
  "arguments": {
    "actions": [
      {"type": "navigate", "params": {"url": "https://example.com/login"}},
      {"type": "type_text", "params": {"selector": "#email", "text": "user@example.com"}},
      {"type": "click", "params": {"selector": "#login-btn"}},
      {"type": "screenshot", "params": {"path": "C:/tmp/after_login.png"}}
    ]
  }
}}
```

Other safe browser tools include `browser_extract_content`, `browser_get_text`, `browser_scroll_collect`, `browser_iframe_extract`, `browser_wait_stable`, and `browser_verify_state`. Prefer `hands_navigate` and `hands_find` when a meta-tool can keep the interaction scoped and verifiable.

### UIA Tier (safe-advertised tools)

The Windows UI Automation tier interacts with native desktop applications through the accessibility tree --- no pixel guessing required. It can find elements by name, control type, or automation ID, then click, type, inspect bounded structural state, and manage windows.

**Find and click a button in a Windows app:**

```json
{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
  "name": "uia_find",
  "arguments": {"name": "Save", "control_type": "Button"}
}}
```

```json
{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
  "name": "uia_click",
  "arguments": {"name": "Save", "control_type": "Button"}
}}
```

**Launch an app and type into it:**

```json
{"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
  "name": "uia_app_launch",
  "arguments": {"path": "notepad.exe"}
}}
```

```json
{"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {
  "name": "uia_type",
  "arguments": {"text": "Hello from Hands"}
}}
```

**Window management:** `uia_window_snap` (snap to left/right/maximize), `uia_window_resize`, `uia_window_move`, `uia_focus_window`, `uia_list_window`. Batch UIA operations with `uia_batch`.

### Vision Tier (9 tools)

The vision tier handles screenshot capture, OCR, template matching, and image diffing. Use it when neither the DOM nor the accessibility tree can reach what you need, or when you want visual verification.

**Screenshot and OCR a screen region:**

```json
{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
  "name": "vision_screenshot_ocr",
  "arguments": {"region": {"x": 100, "y": 200, "width": 400, "height": 50}}
}}
```

**Find an image on screen (template matching):**

```json
{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
  "name": "vision_find_template",
  "arguments": {"template_path": "C:/templates/submit_button.png"}
}}
```

**Diff two screenshots** to detect changes:

```json
{"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
  "name": "vision_diff",
  "arguments": {
    "image_a": "C:/tmp/before.png",
    "image_b": "C:/tmp/after.png"
  }
}}
```

### Combo Tools

A few tools span multiple tiers for common workflows:

- `find_and_click` --- locate an element visually and click it
- `read_screen_text` --- screenshot a region and OCR it in one step
- `wait_for_visual` --- poll until a visual condition is met
- `window_screenshot` --- capture a specific window by title

### Meta-Tools (safe-advertised selection)

The meta-tool layer (added in v1.2.1) provides higher-level orchestration on top of the primitive tiers. Each meta-tool plans a multi-step sequence, executes it, and handles errors with escalation ladders — reducing a 5-tool workflow to a single call.

Key safe-profile meta-tools:
- `hands_navigate` — navigate with retry, wait, and verification
- `hands_click` — find and click with fallback strategies (selector → a11y → vision)
- `hands_fill_form` — discover form fields and fill them intelligently
- `hands_find` — cross-tier element search (DOM → UIA → OCR)
- `hands_capture` — screenshot with optional OCR and analysis
- `hands_verify` — assert page state with multiple verification strategies
- `hands_app_action` — orchestrate Windows app interactions
- `hands_login_recovery` — handle login walls and session expiry
- `hands_type` — intelligent typing with field detection and verification

`hands_read_page`, `hands_script`, and `hands_scan_qr` are compatibility/debug-only because they can perform direct fetches, expose raw decoded values, or execute free-form steps. Use visible-browser extraction, explicit calls, or bounded `browser_batch` actions in a safe profile.

## Common Workflows

**Read a page:** `hands_navigate` then `browser_extract_content` or `browser_get_text`. For sites that require scrolling, use `browser_scroll_collect` to paginate while the visible browser remains the source of truth.

**Fill a web form:** use `hands_find` to identify the intended fields, `hands_fill_form` or `browser_fill_form` to populate them, and an explicitly confirmed submit action when required. Batch only a fixed, predictable sequence with `browser_batch`.

**Automate a Windows app:** `uia_app_launch` to open it, `uia_find` to locate controls, `uia_click` / `uia_type` to interact. Use `uia_get_state` to read checkbox or toggle states. Use `uia_shortcut` for keyboard shortcuts like Ctrl+S.

**OCR a screenshot:** `vision_screenshot` to capture, `vision_ocr` to extract text, or `vision_screenshot_ocr` for both in one call. Use `vision_analyze` for higher-level interpretation.

## Tips and Troubleshooting

**Choose the right tier.** If it is a web page, use the browser tier --- it is faster and more reliable than vision. If it is a native Windows application, use UIA. Fall back to vision only when the other two cannot reach the target.

**Use batch operations.** `browser_batch` and `uia_batch` execute multiple actions in a single MCP call. This is significantly faster than issuing one tool call per action, especially over Claude Desktop where each round-trip adds latency.

**Structured targeting.** Use `hands_find(return_type="ref")` to obtain a bounded current reference, then pass it to interaction tools. Do not request a raw accessibility-tree dump in a safe profile.

**Browser compatibility mode.** Launch and attach flows can apply compatibility adjustments for authorized automation testing in environments you control or have permission to test. Users are responsible for site terms and permissions.

**Safe profile boundary.** `default`, `full`, and `strict` advertise 105, 107, and 109 entries including the catalog. `compatibility` advertises 144 and is an unsafe debug escape hatch, not a speed path. A compatibility-only raw, built-in direct-fetch, or native-plugin call requires `HANDS_TOOL_PROFILE=compatibility`, the matching process gate (`HANDS_ALLOW_UNSAFE_RAW_TOOLS=1`, `HANDS_ALLOW_UNSAFE_DIRECT_FETCH=1`, or `HANDS_ALLOW_UNSAFE_PLUGINS=1`), and the matching per-call acknowledgement. The composite `browser_script` and `browser_evaluate` tools require both the direct-fetch and raw gate pairs. Whenever monitor scope is active, these vendor composites and aliases fail closed even when every compatibility gate is present: `browser_agent`/`agent`, `browser_batch`/`batch`, `browser_script`/`script`, `browser_evaluate`/`evaluate`, `browser_screenshot_burst`/`screenshot_burst`, `browser_scroll_collect`/`scroll_collect`, `browser_wait_stable`/`wait_stable`, and `retry_click` (with `browser_retry_click` treated defensively as an alias). Their nested vendor steps cannot revalidate the bound browser window. Use individually scoped browser calls or compatibility-gated `hands_script`, which centrally revalidates each nested call. Keep durable/direct API methods in Workflow or another dedicated web/network owner. Safe profiles reduce the built-in first-party surface; Hands can still launch and act through external desktop applications and is not a general OS sandbox.

Before enabling a monitor fence, clear browser routes and stop any active trace; fence activation refuses while either persistent state is active. Under an active fence, `browser_route` and `browser_trace_start` fail closed, while `browser_route_remove`, `browser_route_clear`, and `browser_trace_stop` remain available so cleanup cannot be trapped.

**Dashboard.** The Hands HTTP dashboard is disabled by default. Opt in deliberately with `HANDS_ENABLE_DASHBOARD=1` only after reviewing the local listening boundary.

**Browser not launching.** Hands connects to Chrome over CDP. Use `browser_debug_launch` to start Chrome with `--remote-debugging-port=9222`, or launch Chrome manually with that flag. If Chrome is not installed, install it from https://www.google.com/chrome/.

**UIA elements not found.** Some applications use custom-drawn UI that does not expose UIA elements. In that case, fall back to the vision tier. Use `uia_list_window` first to verify the app exposes an accessibility tree.

**OCR accuracy.** Vision OCR works best on high-contrast text. For small or low-contrast text, capture a larger region and crop. The `vision_screenshot_ocr` combo tool handles capture and extraction in one step to avoid file-management overhead.

## Further Reading

- [README](../README.md) --- full tool list, comparison with Claude computer use, architecture details
- [examples/](../examples/) --- raw JSON-RPC examples for each tool category
- [CONTRIBUTING.md](../CONTRIBUTING.md) --- how to add new tools or tiers
- [Model Context Protocol specification](https://modelcontextprotocol.io/) --- the protocol Hands implements
