# AI-Hands — Multi-Layer Desktop Automation for AI Agents

[![CI](https://github.com/AIWander/AI-Hands/actions/workflows/ci.yml/badge.svg)](https://github.com/AIWander/AI-Hands/actions/workflows/ci.yml)

**AI-Hands** is a Rust MCP (Model Context Protocol) server that gives AI agents full desktop control through three automation tiers — not just pixel-guessing from screenshots.

> **Renamed from [AIWander/hands](https://github.com/AIWander/hands) on 2026-05-15.** Same Rust codebase, fresh versioning. The `hands.exe` binary name and `hands:*` MCP tool prefix are unchanged — existing MCP configs in Claude Desktop, Claude Code, Cowork, Codex CLI, Gemini CLI, and LM Studio keep working without edits.

See the [`examples/`](examples/) directory for sample configurations and walkthroughs.

**Part of [CPC](https://github.com/AIWander) (Copy Paste Compute)** — a multi-agent AI orchestration platform. Related repos: [manager](https://github.com/AIWander/manager) · [local](https://github.com/AIWander/local) · [workflow](https://github.com/AIWander/workflow)

## Safe Use / Permission Model

AIWander tools are local, user-authorized MCP capability surfaces. They do not grant an AI new permissions by themselves. They expose tools the user explicitly installs and enables. Sensitive actions should be confirmed by the user, credentials should stay in the OS keyring or local vault, and demos should use mock data.

## What's New in v1.0.1

- **Security: 3 Dependabot alerts resolved** — `openssl` 0.10.78 → 0.10.79 (fixes [GHSA-xp3w-r5p5-63rr](https://github.com/advisories/GHSA-xp3w-r5p5-63rr) HIGH OCSP UB and [GHSA-xv59-967r-8726](https://github.com/advisories/GHSA-xv59-967r-8726) MODERATE AES key-wrap heap overflow); `lru` 0.12.5 → 0.16.4 (fixes [GHSA-rhfx-m35p-ff5j](https://github.com/advisories/GHSA-rhfx-m35p-ff5j) LOW IterMut Stacked Borrows) via `rqrr` 0.7 → 0.10. Binary size: x64 22.55 MB (−1.10 MB vs v1.0.0), ARM64 19.01 MB (−0.94 MB vs v1.0.0).

<details>
<summary>v1.0.0 — AI-Hands launch</summary>

- **New tool: `vision_screenshot_hidden_window`** — always-PrintWindow API captures a window's pixels without bringing it to the foreground. Replaces the `behind=true` mode of `window_screenshot`.
- **`window_title` parameter on `hands_capture`** — focus a named window via UIA before routing the capture.
- **`offset_x`/`offset_y` on `hands_click`** — when non-zero, every rung of the 7-rung click ladder resolves the element by its native method then coord-clicks at bbox.center + offset. When both zero, ref/selector click is preserved on rungs 1-4 for reliability.
- **Deprecation markers** on `find_and_click`, `retry_click`, `read_screen_text`, `type_into_window` (handlers preserved for backward compat), and `window_screenshot` (default mode).

</details>

The entries below are pre-rename `AIWander/hands` lineage notes kept for context; AI-Hands restarted public release numbering at `v1.0.0` on 2026-05-15.

<details>
<summary>v1.3.4</summary>

- ci: bump GitHub Actions versions to latest (Node.js 20 deprecation)
</details>

<details>
<summary>v1.3.3</summary>

- **Phase D: compile-time ZST AtomicTool dispatch** — Replaced all runtime string-based UIA tool dispatch in meta-tools with zero-sized-type (ZST) `AtomicTool` handles resolved at compile time. 11 UIA tools wrapped. 7 meta-tool files refactored. 27 call sites replaced.
- **`src/atomic.rs`** — New module defining the `AtomicTool` trait and ZST wrappers for all UIA tools.
- **Browser compatibility module** — Optional browser compatibility adjustments for authorized automation testing.
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

**v1.3.0** (2026-04-16) — Path deps to git tags, Cargo.lock committed, README license metadata aligned to Apache-2.0, version sync. First version that builds as standalone public clone.

**v1.2.2** — Phase C Fix3: meta-tool dispatch, async Send bound, notify parity.

**v1.2.1** — Phase C fixes, meta-tool dispatch improvements.

**v1.1.1** — Initial public release with 71 MCP tools across 3 automation tiers.

</details>

## Install

> **winget submission pending.** The `microsoft/winget-pkgs` PR is in flight — once it merges, `winget install AIWander.AI-Hands` works against the published index. Until then, [`installers/winget/manifests/`](installers/winget/manifests/) in this repo is the source of truth (use the `--manifest` form below). The manual download path is unaffected.

### Option A — winget (recommended once the PR lands)

```powershell
winget install AIWander.AI-Hands
# Until the upstream PR lands, install from this repo's manifest directly:
# winget install --manifest https://raw.githubusercontent.com/AIWander/AI-Hands/main/installers/winget/manifests/a/AIWander/AI-Hands/1.0.1/AIWander.AI-Hands.installer.yaml

# Then wire it into Claude Desktop (writes a timestamped .bak first):
.\installers\scripts\register-hands.ps1
```

Open a new shell after the install so winget's portable-shim directory (`%LOCALAPPDATA%\Microsoft\WinGet\Links`) is on PATH, then run the registration script. See [`installers/README.md`](installers/README.md) for `-Force` / `-DryRun` flags and rollback.

### Option B — Scoop

```powershell
# Direct URL (no bucket required):
scoop install https://raw.githubusercontent.com/AIWander/AI-Hands/main/installers/scoop/ai-hands.json

# Then register with Claude Desktop:
.\installers\scripts\register-hands.ps1
```

### Option C — Manual download (always works)

1. Download from the [latest release](https://github.com/AIWander/AI-Hands/releases/latest):
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
- Chrome installed normally (any recent version). AI-Hands does not download or manage browser binaries — it talks to your existing Chrome over CDP.
- Claude Desktop or any MCP-compatible client

For full per-machine setup (paths, skills, credentials), see [`docs/per_machine_setup.md`](./docs/per_machine_setup.md).

---

## Why AI-Hands?

*Renamed from AIWander/hands on 2026-05-15. Codebase, binary name, and MCP tool prefix are unchanged.*

Anthropic's [Claude Computer Use](https://docs.anthropic.com/en/docs/agents-and-tools/computer-use) takes screenshots and clicks pixel coordinates. It works, but it's slow (screenshot after every action), imprecise (guessing where to click), and blind (no DOM, no accessibility tree, no structured data).

AI-Hands takes a different approach: **use the right automation layer for each task**.

| Layer | What it does | When to use it |
|-------|-------------|---------------|
| **Browser** (chromiumoxide CDP) | Full DOM access, JS eval, network interception, form fill, multi-tab | Web apps, scraping, testing |
| **UIA** (Win UI Automation) | Accessibility tree, named elements, window management, app launch | Native Windows apps |
| **Vision** (OCR + template match) | Screenshot, OCR, image diff, visual analysis | Anything else, verification |

## Comparison

AI-Hands is a **capability surface** — a local MCP server that lets your chosen AI model (Claude, GPT, Gemini, local LLM) drive your browser, Windows apps, and screen. It does not bundle an AI; you bring the model. The tables below set that apart from (1) other BYOM capability surfaces and (2) AI-bundled computer-use products. This comparison is a May 2026 snapshot; verify third-party pricing, availability, and benchmarks before relying on them.

### vs. other BYOM capability surfaces

| | **AI-Hands** | Playwright MCP |
|---|---|---|
| Runtime | Single Rust binary (`hands.exe`) | Node.js + Playwright runner |
| Procs per session | 1 | 18+ (Node procs + workers) |
| RAM per session (measured) | **~184 MB** | **~320 MB** (5.83× heavier full-stack) |
| Browser control | CDP attach to your existing Chrome | Spawns its own Chromium |
| Persistent auth | YOUR logins, YOUR cookies | Fresh browser each session |
| DOM access | ✓ via CDP | ✓ via Playwright API |
| Native Windows UIA | ✓ | ✗ |
| Vision/OCR (local) | ✓ Tesseract | ✗ DOM only |
| `file://` protocol | ✓ | ✗ blocked by default |
| Screenshot save path | any | restricted to `.playwright-mcp` and `C:\` |
| Smart cross-tier routers | ✓ `hands_click` runs a 7-rung ladder (a11y → fuzzy → CSS → snapshot refresh → clickables → UIA → OCR) under one tool entrypoint | ✗ pick the primitive yourself |
| Chain depth | Claude → `hands.exe` → Chrome (2 hops) | Claude → Node MCP → Playwright API → Chrome (3 hops) |

*Per-session memory measured side-by-side loading example.com with the same Chrome attached.*

### vs. AI-bundled computer-use products

These bundle a specific AI model with their own UI surface. AI-Hands does neither — it gives your model a surface.

| | **AI-Hands + your model** | Claude Computer Use | OpenAI Operator (CUA) | Google Mariner / Gemini Agent | Perplexity Comet |
|---|---|---|---|---|---|
| **Surface** | Your Chrome + your Windows apps | Anthropic-hosted VM or your container | OpenAI-hosted browser sandbox | Chrome extension in your browser | Standalone Chromium-based browser |
| **Cost (May 2026)** | ~$0 marginal (BYOM) | API or Claude subscription | ChatGPT Pro **$200/mo**, or API $3/$12 per 1M tokens (research preview, tiers 3-5) | Free w/ Google account (Mariner-standalone shut down May 4, 2026; capabilities folded into Gemini Agent) | Free since Mar 18, 2026; Comet Plus +$5/mo |
| **Privacy** | All local; your model provider sees what you send it | Anthropic sees screen pixels | OpenAI sees screen pixels | Google sees browser actions; you stay logged in | Perplexity sees pages; you stay logged in |
| **DOM access** | ✓ | ✗ (pixel only) | ✗ (pixel only) | ✓ (extension API) | ✓ (native) |
| **Native app / UIA** | ✓ Windows | ✓ (full OS sandbox) | ✗ (browser only) | ✗ (browser only) | ✗ (browser only) |
| **Vision/OCR** | ✓ local Tesseract | implicit (vision model) | implicit (vision model) | implicit | implicit |
| **Element identification** | CSS, XPath, UIA names, a11y tree, OCR text, template match | Screenshot → guess coordinates | Screenshot → guess coordinates | DOM + screenshot | DOM + screenshot |
| **Persistent auth** | YOUR Chrome cookies (via debug-port attach) | sandbox VM cookies | sandbox VM cookies | YOUR Chrome | OWN browser, OWN profile |
| **Local memory** | ~184 MB | 0 (remote) | 0 (remote) | ~50-100 MB ext + browser | 500+ MB (full browser app) |
| **Bring your own model** | ✓ any | ✗ Claude only | ✗ OpenAI only | ✗ Gemini only | ✗ Perplexity model stack |
| **OSWorld benchmark** | n/a (capability layer) | varies by Claude model | 38.1% (CUA in research preview) | n/a post-shutdown | n/a (browser-only) |
| **Public availability** | v1.0.1 (this repo) | GA | Research preview API; ChatGPT Pro UI | Standalone shut down May 4 2026; lives inside Gemini Agent / Chrome Auto-Browse | Public, free, Windows + macOS |

### TL;DR

- **AI-Hands wins on capability surface** (DOM + UIA + Vision in one binary) and **local resource cost** (~5.8× lighter than Playwright MCP).
- **Bundled-AI products win on plug-and-play onboarding** (no model selection, no setup) but lock you into one provider and surrender screen contents to them.
- **If you already pay for a Claude / GPT / Gemini API subscription**, AI-Hands lets you reuse that model with full DOM + native-app reach for ~$0 marginal cost.

## 117 Tools

### Browser Automation (67 tools)
Navigate, click, type, screenshot, extract content, fill forms, eval JS, manage tabs/contexts, intercept network, scroll-and-collect, accessibility snapshots, smart browse with auto-retry, batch operations, API discovery from traffic.

### Windows UIA (18 tools)
Find elements by name/type/automation ID, click, type, read values, get state, window management (snap, move, resize), app launch, keyboard shortcuts, batch operations, event watching.

### Vision (9 tools)
Screenshot (full/window/region), OCR, template matching, image diff, visual analysis, screenshot+OCR combo.

### Meta-Tools (12 tools)
Smart orchestration layer: reads page, clicks, navigates, captures, finds, types, fills forms, verifies, handles authorized setup flows, launches apps, runs scripts, recovers login flows — picks the right tier automatically.

### Combo & Utility (11 tools)
Cross-tier tools: find-and-click (OCR→UIA), read screen text, wait for visual, window screenshot, type into window, drag, element drag, retry click, file upload, status, health check.

## Capabilities Beyond the Basics

### Browser Tier

**Accessibility-first targeting.** Every `browser_navigate` auto-caches an accessibility snapshot. Each interactive element gets a stable ref (`ref_0`, `ref_1`, ...) that flows into `browser_click`, `browser_type`, `browser_hover`, and every other interaction tool. Refs survive minor DOM changes — no brittle CSS selectors needed. This is AI-Hands' primary competitive advantage over screenshot-based agents.

**Browser compatibility mode.** Launch and attach flows can apply compatibility adjustments for authorized automation testing in environments you control or have permission to test. Users are responsible for site terms and permissions.

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

**`hands_login_recovery`** — 5-stage pipeline: detect login page → fill approved credentials → handle user-authorized MFA steps → verify success → retry on failure.

**`hands_scan_qr`** — supports authorized on-screen setup flows without placing secrets or recovery codes in prompts.

**`hands_script`** — multi-step orchestration with `{{var}}` substitution across tool calls.

### Cross-Server Hooks

**Graduation pipeline (hands → workflow).** `browser_learn_api` extracts API patterns during a browser session. `workflow:api_store` saves them. `workflow:api_call` replays direct HTTP forever — ~50-200ms vs 3-5s browser cycle. Automate once in Chrome, replay at API speed indefinitely.

**Credential and MFA workflows (hands + workflow).** Credential and MFA workflows require explicit user authorization and local keyring storage. Public docs intentionally avoid implementation details. Do not place passwords, tokens, recovery codes, or MFA setup material in chat context.

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
git clone https://github.com/AIWander/AI-Hands.git
cd AI-Hands
cargo build --release
```

Binary appears at `target/release/hands.exe`. Requires Rust stable toolchain — nightly is not required.

## Requirements

- **Windows 10/11** (x64 or ARM64) — required for UIA (Windows UI Automation) and CDP browser automation
- Rust stable toolchain (build from source only)
- Chrome installed normally (any recent version). AI-Hands does not download or manage browser binaries — it talks to your existing Chrome over CDP.

AI-Hands is Windows-only. The UIA automation layer depends on Windows COM interfaces, and the vision layer uses Windows-specific screen capture APIs.

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
- Contact: [GitHub Issues](https://github.com/AIWander/AI-Hands/issues) or contact@aiwander.ai
