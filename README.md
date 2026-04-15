# Hands — Multi-Layer Desktop Automation for AI Agents

**Hands** is a Rust MCP (Model Context Protocol) server that gives AI agents full desktop control through three automation tiers — not just pixel-guessing from screenshots.

## Why Hands?

Anthropic's [Claude Computer Use](https://docs.anthropic.com/en/docs/agents-and-tools/computer-use) takes screenshots and clicks pixel coordinates. It works, but it's slow (screenshot after every action), imprecise (guessing where to click), and blind (no DOM, no accessibility tree, no structured data).

Hands takes a different approach: **use the right automation layer for each task**.

| Layer | What it does | When to use it |
|-------|-------------|---------------|
| **Browser** (Playwright) | Full DOM access, JS eval, network interception, form fill, multi-tab | Web apps, scraping, testing |
| **UIA** (Windows UI Automation) | Accessibility tree, named elements, window management, app launch | Native Windows apps |
| **Vision** (OCR + template matching) | Screenshot, OCR, image diff, visual analysis | Anything else, verification |

## Comparison with Claude Computer Use

| Capability | Claude Computer Use | Hands |
|-----------|-------------------|-------|
| Element identification | Screenshot → guess coordinates | CSS selectors, XPath, UIA names, accessibility tree |
| Speed | ~2s per action (screenshot cycle) | Milliseconds (direct API calls) |
| Browser JS execution | No | Full eval, inject, extract |
| Network interception | No | Route, mock, log requests |
| Form handling | Type into coordinates | `fill_form`, `submit_form`, `get_forms` |
| Multi-tab/context | No | Full tab and context management |
| Native app control | Screenshot + click | Full UIA: find, click, type, read values, window snap/resize |
| OCR | Relies on vision model | Dedicated OCR engine |
| Template matching | No | `vision_find_template` |
| Image diff | No | `vision_diff` |
| Batch operations | One action per turn | `browser_batch`, `uia_batch` |
| Tracing | No | `trace_start/stop/save`, metrics |
| Platform | macOS only (consumer) | Windows (UIA + browser + vision) |
| Setup | Zero (built into Claude) | MCP server binary |

## 71 Tools

### Browser Automation (40 tools)
Navigate, click, type, screenshot, extract content, fill forms, eval JS, manage tabs/contexts, intercept network, scroll-and-collect, accessibility snapshots, smart browse with auto-retry.

### Windows UIA (20 tools)
Find elements by name/type/automation ID, click, type, read values, get state, window management (snap, move, resize), app launch, keyboard shortcuts, batch operations, event watching.

### Vision (11 tools)
Screenshot (full/window/region), OCR, template matching, image diff, visual analysis, screenshot+OCR combo, wait-for-visual condition.

## Quick Start

```bash
# Build
cargo build --release -p hands

# Run as MCP server (stdio transport)
./hands.exe

# Add to Claude Desktop config
{
  "mcpServers": {
    "hands": {
      "command": "C:/path/to/hands.exe",
      "args": []
    }
  }
}
```

## Compatible Clients

- Claude Desktop (Chat + Cowork)
- Claude Code
- Codex
- Any MCP-compatible client

## Architecture

```
hands.exe (MCP server, stdin/stdout JSON-RPC)
├── browser.rs    — Playwright CDP automation
├── uia.rs        — Windows UI Automation COM
├── vision.rs     — Screenshot + OCR + template match
└── tools.rs      — Tool definitions + dispatch
```

Single binary, no runtime dependencies. Playwright browser binaries are auto-managed.

## When to Use What

```
Is it a web page?
  → Yes → Browser layer (fast, structured, reliable)
  → No → Is it a Windows app?
    → Yes → UIA layer (named elements, accessibility tree)
    → No → Vision layer (screenshot + OCR fallback)
```

## License

MIT
