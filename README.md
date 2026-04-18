# Hands -- Multi-Layer Desktop Automation for AI Agents

**Hands** is a Rust MCP (Model Context Protocol) server that gives AI agents full desktop control through three automation tiers -- not just pixel-guessing from screenshots.

**Part of [CPC](https://github.com/josephwander-arch) (Cognitive Performance Computing)** — a multi-agent AI orchestration platform. Related repos: [manager](https://github.com/josephwander-arch/manager) · [local](https://github.com/josephwander-arch/local) · [workflow](https://github.com/josephwander-arch/workflow) · [cpc-paths](https://github.com/josephwander-arch/cpc-paths) · [cpc-breadcrumbs](https://github.com/josephwander-arch/cpc-breadcrumbs)

## What's New in v1.3.2

- **Clippy + dead_code + unused cleanup** -- 3 crate-level allows removed, 60+ targeted allows added with justification, 22 supplemental mechanical fixes in `src/meta/*`

<details>
<summary>v1.3.1</summary>

- HTTP dashboard endpoint migrated to tiny_http (smaller binary, simpler stack)
- Duration tracking for tool calls in dashboard status
- Credential redaction in dashboard output
- Field name alignment across dashboard JSON responses
- Metadata cleanup and documentation fixes
</details>

<details>
<summary>Previous releases</summary>

**v1.3.0** (2026-04-16) -- Path deps to git tags, Cargo.lock committed, README MIT to Apache-2.0, version sync. First version that builds as standalone public clone.

**v1.2.2** -- Phase C Fix3: meta-tool dispatch, async Send bound, notify parity.

**v1.2.1** -- Phase C fixes, meta-tool dispatch improvements.

**v1.1.1** -- Initial public release with 71 MCP tools across 3 automation tiers.

</details>

## Installation & Per-Machine Setup

This is a standalone Rust MCP server for Claude Desktop / Claude Code. Each machine that runs the server needs its own copy of the binary plus a few config tweaks.

**Quick install:**
1. Download the right binary from [Releases](https://github.com/josephwander-arch/hands/releases) -- `_arm64.exe` for Windows ARM64, `_x64.exe` for x64.
2. Copy to `C:\CPC\servers\hands.exe`.
3. Edit `%APPDATA%\Claude\claude_desktop_config.json` -- paste the snippet from [`claude_desktop_config.example.json`](./claude_desktop_config.example.json) into your `mcpServers` object.
4. Restart Claude Desktop.

For full per-machine setup (paths, skills, credentials), see [`docs/per_machine_setup.md`](./docs/per_machine_setup.md).

A future `cpc-setup.exe` helper will automate this entire process.

---

## Why Hands?

Anthropic's [Claude Computer Use](https://docs.anthropic.com/en/docs/agents-and-tools/computer-use) takes screenshots and clicks pixel coordinates. It works, but it's slow (screenshot after every action), imprecise (guessing where to click), and blind (no DOM, no accessibility tree, no structured data).

Hands takes a different approach: **use the right automation layer for each task**.

| Layer | What it does | When to use it |
|-------|-------------|---------------|
| **Browser** (Playwright) | Full DOM access, JS eval, network interception, form fill, multi-tab | Web apps, scraping, testing |
| **UIA** (Win UI Automation) | Accessibility tree, named elements, window management, app launch | Native Windows apps |
| **Vision** (OCR + template match) | Screenshot, OCR, image diff, visual analysis | Anything else, verification |

## Comparison with Claude Computer Use

| Capability | Claude Computer Use | Hands |
|-----------|-------------------|-------|
| Element identification | Screenshot -> guess coordinates | CSS selectors, XPath, UIA names, accessibility tree |
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
├── browser.rs    -- Playwright CDP automation
├── uia.rs        -- Windows UI Automation COM
├── vision.rs     -- Screenshot + OCR + template match
└── tools.rs      -- Tool definitions + dispatch
```

Single binary, no runtime dependencies. Playwright browser binaries are auto-managed.

## When to Use What

```
Is it a web page?
  -> Yes -> Browser layer (fast, structured, reliable)
  -> No -> Is it a Windows app?
    -> Yes -> UIA layer (named elements, accessibility tree)
    -> No -> Vision layer (screenshot + OCR fallback)
```

## Build from Source

```bash
git clone https://github.com/josephwander-arch/hands.git
cd hands
cargo build --release
```

Binary appears at `target/release/hands.exe`. Requires Rust stable toolchain — nightly is not required.

## Requirements

- **Windows 10/11** (x64 or ARM64) — required for UIA (Windows UI Automation) and CDP browser automation
- Rust stable toolchain (build from source only)
- Playwright browser binaries are auto-managed on first use

Hands is Windows-only. The UIA automation layer depends on Windows COM interfaces, and the vision layer uses Windows-specific screen capture APIs.

## Contributing

Issues welcome; PRs considered but this is primarily maintained as part of the CPC stack.

## License

Apache-2.0
