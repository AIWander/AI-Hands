# Hands — Multi-Layer Desktop Automation for AI Agents

[![CI](https://github.com/AIWander/hands/actions/workflows/ci.yml/badge.svg)](https://github.com/AIWander/hands/actions/workflows/ci.yml)

**Hands** is a Rust MCP (Model Context Protocol) server that gives AI agents full desktop control through three automation tiers — not just pixel-guessing from screenshots.

See the [`examples/`](examples/) directory for sample configurations and walkthroughs.

**Part of [CPC](https://github.com/AIWander) (Copy Paste Compute)** — a multi-agent AI orchestration platform. Related repos: [manager](https://github.com/AIWander/manager) · [local](https://github.com/AIWander/local) · [workflow](https://github.com/AIWander/workflow)

## What's New in v1.3.5

- **cargo fmt + clippy cleanup** — 32 clippy errors fixed across 11 lints, regex compilations hoisted out of loops
- **README corrected** — all "Playwright" claims replaced with chromiumoxide (the actual browser engine), install sections consolidated, tool count corrected to 117, full capability inventory added
- **Version alignment** — Cargo.toml, README, and CHANGELOG all at v1.3.5

<details>
<summary>v1.3.4</summary>

- ci: bump GitHub Actions versions to latest (Node.js 20 deprecation)
</details>

<details>
<summary>v1.3.3</summary>

- **Phase D: compile-time ZST AtomicTool dispatch** — Replaced all runtime string-based UIA tool dispatch in meta-tools with zero-sized-type (ZST) `AtomicTool` handles resolved at compile time. 11 UIA tools wrapped. 7 meta-tool files refactored. 27 call sites replaced.
- **`src/atomic.rs`** — New module defining the `AtomicTool` trait and ZST wrappers for all UIA tools.
- **`src/stealth.rs`** — Stealth/anti-detection module for browser automation.
</details>

<details>
<summary>v1.3.2</summary>

- **Clippy + dead_code + unused cleanup** — 3 crate-level allows removed, 60+ targeted allows added with justification, 22 supplemental mechanical fixes in `src/meta/*`
</details>

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

**v1.3.0** (2026-04-16) — Path deps to git tags, Cargo.lock committed, README MIT to Apache-2.0, version sync. First version that builds as standalone public clone.

**v1.2.2** — Phase C Fix3: meta-tool dispatch, async Send bound, notify parity.

**v1.2.1** — Phase C fixes, meta-tool dispatch improvements.

**v1.1.1** — Initial public release with 71 MCP tools across 3 automation tiers.

</details>

## Install

1. Download from the [latest release](https://github.com/AIWander/hands/releases/latest):
   - **Windows x64** → `hands-vX.Y.Z-x64.exe`
   - **Windows ARM64** (Snapdragon X / X Elite / X Plus) → `hands-vX.Y.Z-aarch64.exe`
2. Rename to `hands.exe` and place in `%LOCALAPPDATA%\CPC\servers\`.
3. Add to `claude_desktop_config.json`:
   ```json
   {
     "mcpServers": {
       "hands": {
         "command": "%LOCALAPPDATA%\\CPC\\servers\\hands.exe"
       }
     }
   }
   ```
4. Restart Claude Desktop.

**ARM64 note:** the binary uses native ARM64 UIA bindings — no x64 emulation. If you previously ran the x64 build under emulation, swap to aarch64 for ~3-4x faster screenshot/OCR throughput.

---

### Prerequisites

- Windows 10/11 (x64 or ARM64)
- Chrome installed normally (any recent version). Hands does not download or manage browser binaries — it talks to your existing Chrome over CDP.
- Claude Desktop or any MCP-compatible client

For full per-machine setup (paths, skills, credentials), see [`docs/per_machine_setup.md`](./docs/per_machine_setup.md).

---

## Why Hands?

Anthropic's [Claude Computer Use](https://docs.anthropic.com/en/docs/agents-and-tools/computer-use) takes screenshots and clicks pixel coordinates. It works, but it's slow (screenshot after every action), imprecise (guessing where to click), and blind (no DOM, no accessibility tree, no structured data).

Hands takes a different approach: **use the right automation layer for each task**.

| Layer | What it does | When to use it |
|-------|-------------|---------------|
| **Browser** (chromiumoxide CDP) | Full DOM access, JS eval, network interception, form fill, multi-tab | Web apps, scraping, testing |
| **UIA** (Win UI Automation) | Accessibility tree, named elements, window management, app launch | Native Windows apps |
| **Vision** (OCR + template match) | Screenshot, OCR, image diff, visual analysis | Anything else, verification |

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
| Platform | Anthropic-hosted sandbox or managed OS image | Windows (UIA + browser + vision) on your own machine |
| Setup | Zero (built into Claude) | MCP server binary |

## 117 Tools

### Browser Automation (67 tools)
Navigate, click, type, screenshot, extract content, fill forms, eval JS, manage tabs/contexts, intercept network, scroll-and-collect, accessibility snapshots, smart browse with auto-retry, batch operations, API discovery from traffic.

### Windows UIA (18 tools)
Find elements by name/type/automation ID, click, type, read values, get state, window management (snap, move, resize), app launch, keyboard shortcuts, batch operations, event watching.

### Vision (9 tools)
Screenshot (full/window/region), OCR, template matching, image diff, visual analysis, screenshot+OCR combo.

### Meta-Tools (12 tools)
Smart orchestration layer: reads page, clicks, navigates, captures, finds, types, fills forms, verifies, scans QR, launches apps, runs scripts, recovers login flows — picks the right tier automatically.

### Combo & Utility (11 tools)
Cross-tier tools: find-and-click (OCR→UIA), read screen text, wait for visual, window screenshot, type into window, drag, element drag, retry click, file upload, status, health check.

## Capabilities Beyond the Basics

### Browser Tier

**Accessibility-first targeting.** Every `browser_navigate` auto-caches an accessibility snapshot. Each interactive element gets a stable ref (`ref_0`, `ref_1`, ...) that flows into `browser_click`, `browser_type`, `browser_hover`, and every other interaction tool. Refs survive minor DOM changes — no brittle CSS selectors needed. This is Hands' primary competitive advantage over screenshot-based agents.

**Stealth mode.** `browser_launch(stealth=true)` or `browser_attach(stealth=true)` strips WebDriver indicators, spoofs navigator properties, and adjusts timing to bypass common bot-detection checks (Cloudflare, Akamai, navigator.webdriver). Not foolproof against enterprise anti-bot, but handles the common cases.

**Multi-context isolation.** `browser_context_create` spins up isolated cookie jars — separate login sessions, multi-account flows, A/B testing, all in one Chrome instance without cross-contamination.

**Multi-tab management.** `browser_new_tab`, `browser_list_tab`, `browser_switch_tab`, `browser_close_tab` — full tab lifecycle for workflows that span multiple pages simultaneously.

**Network interception.** `browser_route` intercepts requests with block/mock/log actions. Three sources of network truth: route logs (what you intercepted), Performance API logs (`browser_get_performance_log`), and the merged view via `browser_get_all_network`.

**API discovery.** `browser_learn_api` extracts endpoint patterns from captured network traffic — URLs, methods, headers, auth tokens, body templates. Feed the output to `workflow:api_store` and never open Chrome for that task again.

**Auto-escalation reading.** `browser_smart_browse` and `hands_read_page` auto-escalate from HTTP fetch → linkedom parse → jsdom → full Chrome, stopping at the cheapest rung that returns content.

**Iframe extraction** with cross-origin OCR fallback. **Trace recording** (`browser_trace_start/stop/save`) for debugging. **Screenshot bursts** (`browser_screenshot_burst`) for state-change tracking.

### UIA Tier

**Window management.** `uia_window_snap` (left/right/top-left/top-right/center), `uia_window_move`, `uia_window_resize`, `uia_window_state` (minimize/maximize/restore/close) — full multi-window orchestration from AI agents.

**Event watching.** `uia_watch` monitors for focus changes, window-list changes, or element-value changes. `uia_poll_event` drains events without blocking.

**Compile-time-safe dispatch.** Typed ZSTs in `src/atomic.rs` guarantee every UIA tool name matches the canonical MCP tool name at compile time — no runtime "Unknown tool" errors.

### Vision Tier

**Template matching.** `vision_find_template` locates UI elements by reference image instead of selector — works on games, canvas apps, custom-drawn UIs where DOM and UIA are useless.

**Image diff.** `vision_diff` detects screen changes between two captures.

**Zoom + OCR.** `vision_zoom` for tiny or low-contrast text before running `vision_ocr`.

**User-input detection.** `vision_check_user_input` detects whether the user has typed during a tool sequence — for polite mid-operation interruption.

### Meta-Tier (hands_*)

**6-rung escalation ladder.** `hands_click`, `hands_find`, and other meta-tools try: a11y ref → fuzzy text match → CSS selector → coordinates → UIA → OCR, automatically stepping up until one works.

**`hands_navigate`** auto-launches Chrome if not running, and is multi-monitor aware.

**`hands_verify`** — 5-rung verification ladder with configurable polling and named templates.

**`hands_login_recovery`** — 5-stage pipeline: detect login page → fill credentials → handle 2FA (including TOTP via `workflow:totp_generate`) → verify success → retry on failure.

**`hands_scan_qr`** — decodes QR codes on screen and feeds results to `workflow:totp_register_from_uri` for automatic TOTP vault seeding.

**`hands_script`** — multi-step orchestration with `{{var}}` substitution across tool calls.

### Cross-Server Hooks

**Graduation pipeline (hands → workflow).** `browser_learn_api` extracts API patterns during a browser session. `workflow:api_store` saves them. `workflow:api_call` replays direct HTTP forever — ~50-200ms vs 3-5s browser cycle. Automate once in Chrome, replay at API speed indefinitely.

**Unattended 2FA (hands + workflow).** `hands_scan_qr` decodes a TOTP registration QR from the screen, calls `workflow:totp_register_from_uri` to seed the OS keyring (Windows Credential Manager, target `totp:<name>.cpc-workflow`). On every subsequent login, `hands_login_recovery` generates the current TOTP via `workflow:totp_generate` and types it in. Neither password nor TOTP seed ever enters chat context.

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

## Compatible With

`hands` runs standalone — one binary, one client, and you have browser + UIA + vision automation. Pair with other CPC servers when an automation task needs orchestration, file I/O, or credential-backed HTTP replay.

- Pair with [manager](https://github.com/AIWander/manager) to delegate long-running browser chores to a coding agent and monitor via breadcrumbs.
- Pair with [workflow](https://github.com/AIWander/workflow) to graduate discovered API calls from browser automation to direct-HTTP replay (`browser_learn_api` feeds `api_store`).
- Pair with [local](https://github.com/AIWander/local) for filesystem and shell steps before or after automation runs.

Host clients: Claude Desktop, Claude Code, OpenAI Codex CLI, Gemini CLI, or any MCP-compatible host.

### First-run tip for Claude clients

`hands` exposes 117 tools spanning browser, UIA, and vision. Enable **tools always loaded** in your Claude client's tool settings before the first call — a lazy-loaded client sometimes misses layers on initial discovery and you'll get "tool not found" surprises mid-session.

## Architecture

```
hands.exe (MCP server, stdin/stdout JSON-RPC)
├── browser.rs    — chromiumoxide CDP automation
├── uia.rs        — Windows UI Automation COM
├── vision.rs     — Screenshot + OCR + template match
└── tools.rs      — Tool definitions + dispatch
```

Single binary, no runtime dependencies beyond Chrome.

### Dependencies

- Browser automation powered by [chromiumoxide](https://github.com/mattsse/chromiumoxide) (Apache-2.0/MIT) — a pure-Rust Chrome DevTools Protocol client. Hands attaches to a Chrome instance you've already installed; use `browser_debug_launch` to start Chrome with the debug port, or `browser_attach` to connect to an already-running `chrome.exe --remote-debugging-port=9222`. No browser binaries are downloaded or managed by Hands.
- Windows automation layer uses native UIA COM interfaces — no third-party dependency.
- OCR is done via an embedded Rust OCR crate (not Tesseract binaries) — no external install needed.
- Shared libraries: [browser-mcp](https://github.com/AIWander/browser-mcp), [uia-mcp](https://github.com/AIWander/uia-mcp), [vision-core](https://github.com/AIWander/vision-core), [cpc-paths](https://github.com/AIWander/cpc-paths).

## When to Use What

```
Is it a web page?
  → Yes → Browser layer (fast, structured, reliable)
  → No  → Is it a Windows app?
    → Yes → UIA layer (named elements, accessibility tree)
    → No  → Vision layer (screenshot + OCR fallback)
```

## Build from Source

```bash
git clone https://github.com/AIWander/hands.git
cd hands
cargo build --release
```

Binary appears at `target/release/hands.exe`. Requires Rust stable toolchain — nightly is not required.

## Requirements

- **Windows 10/11** (x64 or ARM64) — required for UIA (Windows UI Automation) and CDP browser automation
- Rust stable toolchain (build from source only)
- Chrome installed normally (any recent version). Hands does not download or manage browser binaries — it talks to your existing Chrome over CDP.

Hands is Windows-only. The UIA automation layer depends on Windows COM interfaces, and the vision layer uses Windows-specific screen capture APIs.

## Failure modes

Automation across three different layers (browser, UIA, vision) means each layer has its own characteristic failures:

- **Browser profile locked** — a previous Chromium process still holds the profile. `browser_launch` returns `profile_locked`; close the stuck Chrome or use a fresh context via `browser_context_create`.
- **UIA element not found** — selector name drift after an app update. Call `uia_find` with a broader query, or snapshot the accessibility tree with `browser_a11y_snapshot` / UIA equivalents to see current names.
- **OCR misreads on tiny or low-contrast text** — vision layer returns its best guess. Use `vision_zoom` before `vision_ocr`, or fall back to `browser_extract_content` if the target is a web page with real text.
- **Chrome not found or debug port not open** — Hands connects to Chrome over CDP. Use `browser_debug_launch` to start Chrome with `--remote-debugging-port=9222`, or ensure Chrome is running with that flag before calling `browser_attach`.
- **Popup or OS dialog steals focus mid-sequence** — UIA actions target the wrong window. Use `uia_focus_window` before sensitive sequences, or batch via `uia_batch` which rechecks focus between steps.

## Contributing

Issues welcome; PRs considered but this is primarily maintained as part of the CPC stack.

## License

Apache License 2.0 — see [LICENSE](LICENSE).

Copyright 2026 Joseph Wander.

---

## Contact

Joseph Wander
- GitHub: [github.com/AIWander](https://github.com/AIWander/)
- Email: [josephwander@gmail.com](mailto:josephwander@gmail.com)
