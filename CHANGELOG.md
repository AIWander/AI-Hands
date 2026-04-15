# Changelog

## [1.2.1] - 2026-04-15 — Phase C Fix3

### Fixed
- **Meta-tool nested dispatch** — inline atomics pattern replaces nested meta-tool calls that caused double-dispatch errors
- **Async Send bound** in `execute_step` — fixes compiler error when meta-tool futures cross await points
- **Phase C instrumentation** — corrected tracing spans and error propagation in meta-tool dispatch path

### Added
- **`notify` tool** — Windows toast notifications (parity with local/manager servers)
- **Meta-tool layer** (Phase A–C) — 10 meta-tools (`hands_navigate`, `hands_click`, `hands_read_page`, `hands_capture`, `hands_find`, `hands_type`, `hands_fill_form`, `hands_verify`, `hands_script`, `hands_app_action`) providing high-level orchestration over 87 browser/UIA/vision primitives with escalation ladders, session tracking, and self-healing retry logic
- **QR/TOTP integration** — `hands_scan_qr` for 2FA registration flows
- **Login recovery** — `hands_login_recovery` with template-based credential filling and submit detection

## v1.1.1 — Initial Public Release

### Browser Automation
- Full Playwright CDP integration with Chrome/Edge
- Escalation ladder: `browser_http_scrape` → `browser_smart_browse` → `browser_extract_content` → full Chrome session
- Accessibility-first interaction via auto-cached a11y snapshots and `a11y_ref` targeting
- `browser_batch` for collapsing multi-step sequences into one round-trip
- `browser_learn_api` — analyze captured network traffic to discover and extract API patterns
- `browser_a11y_find` — fast search of cached accessibility snapshots
- `retry_click` for flaky element resilience
- `file_upload` via DataTransfer API
- `browser_scroll_collect` for infinite-scroll pages
- Stealth mode and persistent browser profiles

### Windows Desktop Automation (UIA)
- Native UI Automation control of Windows applications
- Element interaction: find, click, type, key press, shortcuts, read values, scroll
- Window management: list, focus, snap, move, resize, state control, app launch
- `uia_batch` for multi-action sequences
- `uia_watch` / `uia_poll_event` for event monitoring
- Full ARM64 Windows support — no emulation

### Vision / OCR
- `vision_screenshot` — full screen, region, or monitor capture
- `vision_ocr` / `vision_screenshot_ocr` — text extraction with bounding boxes
- `vision_find_template` — template matching for visual element location
- `vision_diff` — image comparison with difference highlighting
- `vision_analyze` — AI-powered image analysis

### Combo Tools
- `find_and_click` — OCR screen, find text, click it
- `read_screen_text` — screenshot + OCR in one call
- `wait_for_visual` — poll screen until text or template appears
- `window_screenshot` — focus window + screenshot, works on obscured windows
- `type_into_window` — focus, click, type in one call
- `drag` / `element_drag` — mouse drag operations

### Platform
- Single Rust binary, zero runtime dependencies
- ~87 tools across 4 categories
- x64 and ARM64 Windows builds
- MCP protocol over stdio
