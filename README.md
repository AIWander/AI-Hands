# Hands MCP Server

Browser automation, Windows desktop automation, vision/OCR, and API discovery — all through one MCP server. Single Rust binary, single-digit MB, zero runtime dependencies.

## What It Does

**Browser Automation** — Chrome DevTools Protocol (CDP) via `chromiumoxide`, a pure-Rust CDP client. HTTP scraping, headless Chrome, interactive sessions, form filling, accessibility-first element targeting, batch operations, network traffic capture, and API discovery.

**Windows Desktop Automation** — Native UI Automation (UIA) control of any Windows app. Click, type, read values, manage windows, snap layouts, launch apps. Works on x64 and ARM64 natively.

**Vision / OCR** — Screenshots, OCR, template matching, image diffing. Use for verification or when DOM/UIA can't reach the content.

**Graduation Pipeline** — Automate a flow in the browser once, capture the underlying API calls with `browser_learn_api`, then replay them via direct HTTP. No browser needed on future runs.

## Install

### Download

Grab the binary for your platform:

- **x64 Windows**: [hands_x64_windows.exe](https://github.com/josephwander-arch/hands/releases/latest/download/hands_x64_windows.exe)
- **ARM64 Windows**: [hands_arm64_windows.exe](https://github.com/josephwander-arch/hands/releases/latest/download/hands_arm64_windows.exe)

Rename to `hands.exe` and place wherever you keep your MCP server binaries.

### Claude Desktop Config

Add this to your `claude_desktop_config.json`:

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

See `claude_desktop_config.example.json` for the full snippet with both architecture options.

### Prerequisites

- **Chrome or Edge** — browser tools need a Chromium-based browser installed.
- **Windows 10/11** — UIA desktop automation requires Windows.
- No Node.js, no Python, no runtime dependencies.

## Quickstart

### Scrape a page (no browser needed)

```
browser_http_scrape(url: "https://example.com")
```

If the page needs JavaScript:

```
browser_smart_browse(url: "https://example.com")
```

If you need interaction:

```
browser_launch()
browser_navigate(url: "https://example.com/login")
browser_a11y_snapshot()  # see all interactive elements
browser_click(a11y_ref: "ref_5")  # click by accessibility ref
browser_close()
```

### Automate a Windows app

```
uia_app_launch(name: "notepad.exe")
uia_list_window()
uia_focus_window(title: "Untitled - Notepad")
uia_type(text: "Hello from Hands")
uia_shortcut(keys: "Ctrl+S")
```

### Visual verification

```
vision_screenshot_ocr()  # screenshot + OCR in one call
```

## The Escalation Ladder

Always start cheap and escalate only when needed:

```
Rung 1: browser_http_scrape     — raw HTTP fetch, no browser
Rung 2: browser_smart_browse    — JS-capable fetch, still no Chrome
Rung 3: browser_extract_content — headless Chrome, clean text output
Rung 4: browser_launch + tools  — full interactive Chrome session
Rung 5: Vision (OCR/screenshot) — last resort or verification
```

Most tasks don't need a full Chrome session. Don't launch one unless you have to.

## Documentation

Full skill reference with every tool, usage patterns, anti-patterns, and the graduation pipeline:

- [`skills/hands.md`](skills/hands.md)

Recommended CLAUDE.md behavioral instructions:

- [`skills/hands-recommended-instructions.md`](skills/hands-recommended-instructions.md)

## Architecture

- **Browser tier**: Chrome DevTools Protocol via `chromiumoxide` to Chrome/Edge
- **UIA tier**: Windows UI Automation COM interop, native on both x64 and ARM64
- **Vision tier**: Screenshot capture + OCR + template matching
- **Combo tier**: Cross-tier convenience tools (find_and_click, read_screen_text, etc.)

All tiers run in one process. No sidecar services, no daemon, no port binding.

## Preflight Check

Run the diagnostic script to verify your setup:

```powershell
.\doctor.ps1
```

Checks for the binary, Chrome/Edge installation, and screenshot capability.

---

### Prerequisites: log into your browser first

Before using hands for any site that needs authentication, open Chrome (or your preferred Chromium-based browser) manually and log into whatever you plan to automate — Gmail, GitHub, your bank, Twitter, whatever. Hands attaches to your existing browser profile via CDP, which means it inherits all your cookies and active sessions. If you're already logged in, hands is already logged in. If you're not, hands will see a login page just like a fresh browser would. This single step eliminates most `hands can't click past the login wall` friction.

## Compatible With

Works with any MCP client. Common install channels:

- **Claude Desktop** (the main chat app) — add to `claude_desktop_config.json`. See `claude_desktop_config.example.json` in this repo.
- **Claude Code** — add to `~/.claude/mcp.json`, or point your `CLAUDE.md` at `skills/hands.md` to load it as a skill instead.
- **OpenAI Codex CLI** — register via Codex's MCP config, or load the skill directly.
- **Gemini CLI** — register via Gemini's MCP config, or load the skill directly.

**Two install layouts:**

1. **Local folder** — clone or download this repo, then point your client at the local directory or the extracted `.exe` binary.
2. **Installed binary** — grab the `.exe` from the [Releases](https://github.com/josephwander-arch/hands/releases) page, place it wherever you keep your MCP binaries, then register its path in your client's config.

**Also ships as a skill** — if your client supports Anthropic skill files, load `skills/hands.md` directly. Skill-only mode gives you the behavioral guidance without running the server; useful for planning, review, or read-only workflows.

### First-run tip: enable "always-loaded tools"

For the smoothest experience, enable **tools always loaded** in your Claude client settings (Claude Desktop: Settings → Tools, or equivalent in Claude Code / Codex / Gemini). This ensures Claude recognizes the tool surface on first use without needing to re-discover it every session. Most users hit friction on day one because this is off by default.

## License

Apache 2.0 — see [LICENSE](LICENSE)

Copyright 2026 Joseph Wander

## Contact

- Email: protipsinc@gmail.com
- GitHub: [josephwander-arch](https://github.com/josephwander-arch/)
- Donations: **$NeverRemember** (Cash App)
