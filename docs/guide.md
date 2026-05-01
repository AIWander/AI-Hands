---
title: "Hands MCP Server — Desktop Automation for AI Agents"
description: "Getting started guide for the Hands Rust MCP server. Gives Claude and other AI agents browser automation via chromiumoxide CDP, Windows UI Automation, and vision (OCR + template matching) through 117 tools over the Model Context Protocol."
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
---

# Getting Started with Hands

Hands is a Rust MCP server that provides 117 tools for desktop automation across five tiers: Browser (chromiumoxide CDP), Windows UI Automation (UIA), and Vision (OCR + template matching). It connects to Claude Desktop, Claude Code, or any MCP-compatible client over standard JSON-RPC on stdin/stdout.

Unlike Claude computer use, which relies on repeated screenshots and pixel-coordinate guessing, Hands gives AI agents direct access to the DOM, the Windows accessibility tree, and dedicated OCR --- each chosen for the task at hand. For a full comparison, see the [README](../README.md).

## Installation

### Prerequisites

- **Rust toolchain** (stable, 2021 edition or later)
- **Windows 10/11** (UIA tools require the Windows accessibility API)
- **Chrome** installed normally (any recent version) — Hands connects to Chrome over CDP, no binaries are downloaded

### Build from source

```bash
git clone https://github.com/your-org/hands.git
cd hands
cargo build --release -p hands
```

The output binary lands at `target/release/hands.exe`. It is a single file with no runtime dependencies.

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

Restart Claude Desktop or Claude Code after editing. The 117 tools will appear in your tool list.

## Architecture Overview

```
hands.exe  (MCP tool server, stdin/stdout JSON-RPC)
  |
  +-- browser-mcp    chromiumoxide CDP: navigate, click, fill, eval JS, intercept network
  +-- uia-mcp        Windows UI Automation COM: find elements, click, type, window mgmt
  +-- vision-core    Screenshot capture, OCR, template matching, image diff
  +-- combo tools    Cross-tier helpers: find_and_click, read_screen_text, wait_for_visual
```

All three tiers compile into one binary. The MCP server reads JSON-RPC requests from stdin, dispatches to the appropriate tier, and returns results on stdout. No sidecar processes, no Node runtime, no Python --- just a Rust binary.

## Tool Tiers and Usage Examples

Every example below shows the raw JSON-RPC call. When using Claude Desktop or Claude Code, the client builds these calls automatically from natural-language requests.

### Browser Tier (60 tools)

The browser tier wraps chromiumoxide over CDP. It handles headless browser sessions, web scraping, form filling, JS evaluation, network interception, multi-tab management, and accessibility snapshots.

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
      {"tool": "browser_navigate", "arguments": {"url": "https://example.com/login"}},
      {"tool": "browser_fill_form", "arguments": {"fields": [
        {"selector": "#email", "value": "user@example.com"}
      ]}},
      {"tool": "browser_click", "arguments": {"selector": "#login-btn"}},
      {"tool": "browser_screenshot", "arguments": {"path": "C:/tmp/after_login.png"}}
    ]
  }
}}
```

Other notable browser tools: `browser_eval` (run arbitrary JS), `browser_get_forms` (discover form fields), `browser_scroll_collect` (paginate and collect), `browser_http_scrape` (lightweight fetch without a full browser), `browser_a11y_snapshot` (accessibility tree dump).

### UIA Tier (12 tools)

The Windows UI Automation tier interacts with native desktop applications through the accessibility tree --- no pixel guessing required. It can find elements by name, control type, or automation ID, then click, type, read values, and manage windows.

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

## Common Workflows

**Scrape a page:** `browser_navigate` then `browser_extract_content` or `browser_js_extract` for structured data. For sites that require scrolling, use `browser_scroll_collect` to paginate automatically.

**Fill a web form:** `browser_get_forms` to discover fields, `browser_fill_form` to populate them, `browser_submit_form` or `browser_click` to submit. Batch the whole sequence with `browser_batch`.

**Automate a Windows app:** `uia_app_launch` to open it, `uia_find` to locate controls, `uia_click` / `uia_type` to interact. Use `uia_get_state` to read checkbox or toggle states. Use `uia_shortcut` for keyboard shortcuts like Ctrl+S.

**OCR a screenshot:** `vision_screenshot` to capture, `vision_ocr` to extract text, or `vision_screenshot_ocr` for both in one call. Use `vision_analyze` for higher-level interpretation.

## Tips and Troubleshooting

**Choose the right tier.** If it is a web page, use the browser tier --- it is faster and more reliable than vision. If it is a native Windows application, use UIA. Fall back to vision only when the other two cannot reach the target.

**Use batch operations.** `browser_batch` and `uia_batch` execute multiple actions in a single MCP call. This is significantly faster than issuing one tool call per action, especially over Claude Desktop where each round-trip adds latency.

**Accessibility-first interaction.** Browser tools that support `a11y_ref` (click, type, hover, focus, select) can target elements by their accessibility tree reference instead of CSS selectors. Use `browser_a11y_snapshot` to get the tree, then pass the ref directly.

**Stealth mode.** The `browser_launch` and `browser_attach` tools accept a `stealth` parameter that applies anti-detection measures for sites that block headless browsers.

**Browser not launching.** Hands connects to Chrome over CDP. Use `browser_debug_launch` to start Chrome with `--remote-debugging-port=9222`, or launch Chrome manually with that flag. If Chrome is not installed, install it from https://www.google.com/chrome/.

**UIA elements not found.** Some applications use custom-drawn UI that does not expose UIA elements. In that case, fall back to the vision tier. Use `uia_list_window` first to verify the app exposes an accessibility tree.

**OCR accuracy.** Vision OCR works best on high-contrast text. For small or low-contrast text, capture a larger region and crop. The `vision_screenshot_ocr` combo tool handles capture and extraction in one step to avoid file-management overhead.

## Further Reading

- [README](../README.md) --- full tool list, comparison with Claude computer use, architecture details
- [examples/](../examples/) --- raw JSON-RPC examples for each tool category
- [CONTRIBUTING.md](../CONTRIBUTING.md) --- how to add new tools or tiers
- [Model Context Protocol specification](https://modelcontextprotocol.io/) --- the protocol Hands implements
