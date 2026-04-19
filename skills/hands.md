---
name: hands
description: |
  Browser automation, Windows desktop automation, vision/OCR, and workflow
  graduation pipeline -- all through one MCP server. Teaches the escalation
  ladder, a11y_ref pattern, batch operations, API discovery pipeline, UIA
  desktop control, and vision-as-verification. For hands v1.1.1+.
---

# Hands MCP Server -- Skill Reference

Hands is a single Rust binary that gives you browser automation (Playwright CDP),
Windows desktop automation (UI Automation), vision (OCR + template matching), and
a workflow subsystem (API discovery, flow recording, credential storage) -- all over
MCP. 116 tools, zero runtime dependencies, one process.

This skill teaches you how to use it well, not just what buttons exist.

---

## Core Philosophy: The Escalation Ladder

Hands tools are arranged from cheap to expensive. **Always start at the cheapest
rung that can do the job.** Moving up costs more tokens, more latency, and more
fragility.

```
Rung 1: browser_http_scrape     -- raw HTTP fetch, parse with linkedom. No browser.
Rung 2: browser_smart_browse     -- JS-capable fetch via jsdom. Still no Chrome.
Rung 3: browser_extract_content  -- Chrome headless, extracts clean text/markdown.
Rung 4: browser_launch + navigate -- full interactive Chrome session.
Rung 5: Vision (screenshot + OCR) -- last resort, or verification layer.
```

**Rules of thumb:**
- If you just need text from a page: try `browser_http_scrape` first.
- If the page needs JS to render: try `browser_smart_browse`.
- If you need to interact (click, fill, scroll): launch Chrome.
- If you need to verify a visual state or read a native app: use vision.
- If it's a Windows desktop app (not a browser): skip straight to UIA tools.

The whole point of the ladder is that most tasks don't need a full Chrome session.
A `browser_http_scrape` call costs ~50ms and zero browser overhead. A full
`browser_launch` + navigate costs seconds and leaves a Chrome process running.

---

## The a11y_ref Pattern

This is the single most important interaction pattern in Hands.

**Problem:** CSS selectors break when sites change. Coordinates break when windows
resize. Both require you to know implementation details of the page.

**Solution:** The accessibility tree. After any `browser_navigate`, Hands
auto-caches an accessibility snapshot. Each element gets a stable ref like
`ref_0`, `ref_1`, etc. You can then click/type/hover/select by ref instead of
selector.

### The workflow

```
1. browser_navigate -> page loads, a11y snapshot auto-cached
2. browser_a11y_snapshot -> returns the tree with refs (or use the auto-cached one)
3. browser_click(a11y_ref: "ref_12") -> click the element
4. browser_type(a11y_ref: "ref_15", text: "hello") -> type into it
```

### Why this beats selectors

- Refs survive minor DOM changes (class renames, wrapper div additions).
- Refs are human-readable in the snapshot (you see "Submit button" not `#btn-x7q`).
- The snapshot gives you the full interactive surface in one call -- you see every
  clickable/typeable element without inspecting the DOM.

### When to re-snapshot

The cached snapshot goes stale after navigation, SPA route changes, or dynamic
content loads. If you navigate or trigger a major state change, call
`browser_a11y_snapshot` again to refresh the cache. Clicking a tab that loads new
content? Re-snapshot. Scrolling a static page? The existing snapshot is fine.

### Searching the snapshot

Use `browser_a11y_find` to search the cached snapshot by name, role, or text
without re-fetching the full tree. Faster than a full snapshot when you're looking
for one specific element.

---

## Batch Operations

Every MCP round-trip has latency -- especially in Claude Desktop where each tool
call is a full request/response cycle. Hands provides batch tools to collapse
multiple actions into one call.

### browser_batch

```json
{
  "actions": [
    {"tool": "browser_navigate", "arguments": {"url": "https://example.com/login"}},
    {"tool": "browser_fill_form", "arguments": {"fields": [
      {"selector": "#email", "value": "user@example.com"},
      {"selector": "#password", "value": "hunter2"}
    ]}},
    {"tool": "browser_click", "arguments": {"selector": "#login-btn"}},
    {"tool": "browser_screenshot", "arguments": {}}
  ]
}
```

Actions execute sequentially. If one fails and `continue_on_error` is false
(the default), the batch stops and returns the error. Set `continue_on_error: true`
for best-effort sequences where individual failures are acceptable.

### uia_batch

Same pattern for desktop automation:

```json
{
  "actions": [
    {"type": "click", "name": "File", "control_type": "MenuItem"},
    {"type": "click", "name": "Save As", "control_type": "MenuItem"},
    {"type": "type_text", "text": "report.pdf"},
    {"type": "key_press", "key": "Enter"}
  ]
}
```

### When to batch vs. individual calls

- **Batch** when the steps are predictable and don't need intermediate inspection
  (login flows, form fills, menu navigation).
- **Individual** when you need to read intermediate state to decide the next step
  (conditional flows, error recovery, dynamic content).

---

## Browser Tools -- Reference

### Lifecycle
| Tool | Purpose |
|------|---------|
| `browser_launch` | Start Chrome. Params: `headless` (bool), `stealth` (bool), `profile_path` (string). |
| `browser_close` | Close the browser session. Always clean up when done. |
| `browser_navigate` | Go to URL. Auto-caches a11y snapshot on load. |
| `browser_back` / `browser_forward` / `browser_reload` | History navigation. |
| `browser_new_tab` / `browser_switch_tab` / `browser_close_tab` | Multi-tab management. |

### Reading Content
| Tool | Purpose |
|------|---------|
| `browser_get_text` | Extract text from a selector. Clean, no HTML. |
| `browser_get_html` | Raw HTML of an element or page. |
| `browser_extract_content` | Smart content extraction -- strips nav/ads/chrome, returns clean text. **Use this over get_text for article/page content.** |
| `browser_http_scrape` | Lightweight HTTP fetch + parse. No browser needed. Cheapest read. |
| `browser_smart_browse` | JS-capable lightweight fetch. Middle ground. |
| `browser_js_extract` | Run JS to extract structured data. Returns JSON. |
| `browser_scroll_collect` | Auto-paginate: scroll, collect, repeat. For infinite-scroll pages. |
| `browser_get_url` | Current page URL. |

### Interaction
| Tool | Purpose |
|------|---------|
| `browser_click` | Click by `a11y_ref`, `selector`, or `match_text`. Prefer a11y_ref. |
| `browser_type` | Type text into focused or targeted element. |
| `browser_hover` | Hover over element (triggers tooltips, dropdowns). |
| `browser_focus` | Focus an element without clicking. |
| `browser_select` | Select option in a `<select>` element. |
| `browser_press` | Press a keyboard key (Enter, Tab, Escape, etc.). |
| `browser_scroll` | Scroll page or element. |
| `retry_click` | Click with automatic retry (up to `max_attempts`). For flaky elements. |

### Forms
| Tool | Purpose |
|------|---------|
| `browser_get_forms` | Discover all forms and their fields on the page. |
| `browser_fill_form` | Fill multiple fields at once. |
| `browser_submit_form` | Submit a form by selector or index. |
| `file_upload` | Upload file(s) to `input[type=file]` via DataTransfer API. |

### Accessibility
| Tool | Purpose |
|------|---------|
| `browser_a11y_snapshot` | Full accessibility tree with refs. Params: `root_selector`, `max_depth`, `incremental`. |
| `browser_a11y_find` | Search cached snapshot by name/role/text. Fast lookup without re-fetching. |

### JavaScript & Network
| Tool | Purpose |
|------|---------|
| `browser_evaluate` / `browser_eval` | Execute arbitrary JS in page context. |
| `browser_inject_script` | Inject a script tag into the page. |
| `browser_get_network_log` | Recent network requests (XHR, fetch, etc.). |
| `browser_get_all_network` | Full network log since page load. |
| `browser_get_performance_log` | Performance timing data. |
| `browser_cookies` | Read/manage cookies. |

### Screenshots & Waiting
| Tool | Purpose |
|------|---------|
| `browser_screenshot` | Capture page or element screenshot. |
| `browser_wait_for` | Wait for selector to appear/disappear. Params: `visible`, `exists`, `timeout`. |
| `browser_wait_idle` | Wait for network idle (no pending requests). |
| `browser_wait_stable` | Wait for DOM to stop changing. |
| `browser_exists` | Check if selector exists without waiting. |

### API Discovery
| Tool | Purpose |
|------|---------|
| `browser_learn_api` | Analyze captured network traffic to discover API endpoints. Feed it network logs, get back structured API patterns -- URLs, methods, headers, payloads. This is the graduation step from browser to HTTP. |

---

## The Graduation Pipeline (Pairs With: workflow MCP server)

Browser automation is expensive -- Chrome processes, page loads, fragile selectors.
But every browser session makes real HTTP calls under the hood. If you capture
those calls, you can replay them directly: no Chrome, no rendering, no breakage.

This is the **graduation pipeline**: hands discovers the APIs, workflow remembers
and replays them. The two servers ship as paired releases -- separate binaries,
separate repos, but designed as one system.

### Hands' Role: Discovery

Hands contributes three tools to the pipeline:

| Tool | Purpose |
|------|---------|
| `browser_get_all_network` | Capture all HTTP traffic from a browser session -- every request URL, method, headers, and body the page sent while you were interacting with it. |
| `browser_learn_api` | Analyze captured network traffic to extract API patterns. Takes raw network logs, filters out analytics/tracking noise, and returns structured endpoints -- URLs, methods, required headers, auth tokens, body templates, URL placeholders. This is the graduation step. |
| `browser_http_scrape` | Fast non-Chrome HTTP fetch. For cases where you already know the endpoint and just need to hit it without spinning up a browser. |

The pattern is always the same:
1. **Do the task in the browser.** Use `browser_launch`, `browser_navigate`,
   `browser_click`, `browser_type` -- whatever the flow requires.
2. **Capture the traffic.** Call `browser_get_all_network` to pull every HTTP
   request the page made during your session.
3. **Extract the API surface.** Feed the captured traffic to `browser_learn_api`.
   It returns the endpoints that actually matter -- the ones that do the work,
   not the analytics pings.

At this point, hands' job is done. You have structured API patterns. What you
do with them next is workflow's territory.

### Pairs With: workflow MCP server

The **workflow** MCP server handles storage, credentials, and replay. It turns
hands' one-time API discoveries into reusable, headless operations.

Key workflow tools (see the workflow repo for full documentation):
- **`api_store`** -- Save a discovered API pattern as a named, reusable entry
  (endpoint, method, headers, body template, credential references).
- **`api_call`** -- Replay a stored pattern with dynamic parameters. Resolves
  credentials, fills placeholders, makes the HTTP request. No browser needed.
- **`api_list`** / **`api_test`** -- Manage and verify stored patterns.
- **`credential_store`** / **`credential_get`** / **`credential_list`** /
  **`credential_delete`** / **`credential_refresh`** -- Encrypted credential
  vault. Secrets are referenced by name in API patterns and resolved at call time.
- **`flow_record`** / **`flow_replay`** -- Experimental macro layer for flows
  that resist API extraction (complex auth, WebSocket state, CSRF rotation).

### Example: Full Pipeline

```
# 1. Discovery (hands)
browser_launch -> browser_navigate("https://app.example.com/dashboard")
browser_click("#export-btn") -> browser_fill_form({format: "csv", range: "30d"})
browser_click("#download")

# 2. Capture & extract (hands)
browser_get_all_network -> returns 47 requests
browser_learn_api(network_logs) -> extracts 3 real endpoints:
  POST /api/v2/exports  (creates export job)
  GET  /api/v2/exports/{id}/status  (polls until ready)
  GET  /api/v2/exports/{id}/download  (fetches the file)

# 3. Store & replay (workflow -- no browser needed next time)
api_store(name="example_export", endpoints=[...])
credential_store(name="example_token", value="Bearer ...")
api_call(name="example_export", params={format: "csv", range: "30d"})
  -> 200ms instead of 5 seconds, no Chrome process
```

### The Big Picture

```
Browser session (expensive, fragile)        <- hands
    ↓ browser_get_all_network + browser_learn_api
Structured API patterns (discovered)        <- hands output
    ↓ api_store + credential_store
Stored replay (no browser, milliseconds)    <- workflow
    ↓ api_call
Production automation                       <- workflow
```

Hands is the discovery tool. Workflow is the production tool. Install both
for the full pipeline.

---

## Desktop Automation (UIA) -- Reference

The UIA tier controls native Windows applications through the accessibility tree.
It works with any app that exposes UIA elements -- which is most standard Windows
apps (Office, File Explorer, Settings, etc.).

### When to use UIA vs. Browser

- **Browser tools** for anything running in Chrome/Edge/Firefox.
- **UIA tools** for native Windows apps, Electron apps outside the browser, system
  dialogs (file picker, print dialog), or any non-web UI.
- UIA can also control browser windows at the OS level (window position, minimize,
  snap) while browser tools control content inside the page.

### Element Interaction
| Tool | Purpose |
|------|---------|
| `uia_find` | Find element by name, control_type, automation_id. Returns element info. |
| `uia_click` | Click element by name/role or coordinates. |
| `uia_type` | Type text into focused element or find-and-type by name. |
| `uia_key_press` | Press a single key. |
| `uia_shortcut` | Press key combo (Ctrl+S, Alt+F4, etc.). |
| `uia_read_value` | Read current value of a control (text box, slider, etc.). |
| `uia_get_state` | Read checkbox/toggle/radio state. |
| `uia_scroll` | Scroll within a control. |

### Window Management
| Tool | Purpose |
|------|---------|
| `uia_list_window` | List all top-level windows. |
| `uia_focus_window` | Bring window to front by title. |
| `uia_window_snap` | Snap to left/right/top-left/top-right/center. |
| `uia_window_move` | Move window to coordinates. |
| `uia_window_resize` | Resize window. |
| `uia_window_state` | Minimize/maximize/restore/close. |
| `uia_app_launch` | Launch app by path, name, or URI. |

### Advanced
| Tool | Purpose |
|------|---------|
| `uia_batch` | Multiple UIA actions in one call. |
| `uia_watch` | Watch for UIA events (element changes). |
| `uia_poll_event` | Poll for watched events. |

### UIA Tips

- **Always `uia_list_window` first** to verify the target app is running and
  find its exact title string.
- **Control types matter.** `uia_find(name: "Save")` might match a menu item and
  a button. Add `control_type: "Button"` to disambiguate.
- **Some apps don't expose UIA.** Games, custom-drawn UIs, and some Electron apps
  have poor UIA support. Fall back to vision tools for those.
- **ARM64 native.** UIA works on both x64 and ARM64 Windows -- no emulation needed.
  This is a key differentiator vs. browser-only tools.

---

## Vision Tools -- Reference

Vision tools handle screenshots, OCR, template matching, and image comparison.
They work at the pixel level -- no DOM, no accessibility tree.

### Core Tools
| Tool | Purpose |
|------|---------|
| `vision_screenshot` | Capture full screen, region, or monitor. Params: `region`, `monitor`, `quality`. |
| `vision_ocr` | OCR an image file. Returns text + bounding boxes. |
| `vision_screenshot_ocr` | Screenshot + OCR in one call. Most common vision tool. |
| `vision_find_template` | Find an image (template) on screen. Returns coordinates. |
| `vision_diff` | Compare two images, highlight differences. |
| `vision_analyze` | AI-powered image analysis (uses vision model). |
| `vision_load_image` | Load image for inspection. |
| `vision_check_user_input` | Detect if user has typed anything (for interruption checks). |

### The Verification Rule

**Vision tools are for verification, not primary perception.**

Use them to *confirm* a state, not to *drive* decisions. The browser and UIA tiers
give you structured, reliable data. Vision gives you pixels that need
interpretation.

**Good uses of vision:**
- Confirm a page loaded correctly after a batch operation.
- Verify a dialog appeared that UIA can't see.
- Check that a chart rendered (template matching).
- OCR a native app that doesn't expose UIA elements.
- Diff before/after screenshots to verify a change.

**Bad uses of vision:**
- OCR a web page to read its text (use `browser_get_text` or `browser_extract_content`).
- Screenshot to find where to click (use `browser_a11y_snapshot` or `uia_find`).
- Repeated screenshot loops to track state changes (use `browser_wait_for` or
  `wait_for_visual`).

The exception: when you literally cannot reach the content through DOM or UIA
(games, canvas elements, PDF viewers, custom-drawn UI), vision is correct.

---

## Combo Tools -- Reference

These cross tier boundaries for common workflows.

| Tool | Purpose |
|------|---------|
| `find_and_click` | OCR screen -> find text -> click it. Tries other windows if not found. Params: `text`, `window_title`, `button`, `double_click`, `offset_x/y`. |
| `read_screen_text` | Screenshot + OCR in one call. Optional `window_title` targeting. |
| `wait_for_visual` | Poll screen until text (OCR) or template image appears. Params: `text` or `template_path`, `timeout_ms`, `poll_interval_ms`. |
| `window_screenshot` | Focus window by title, screenshot it. Optional OCR. Uses PrintWindow API for obscured windows. |
| `type_into_window` | Focus window, optionally click a position, then type text. |
| `drag` | Mouse drag between coordinates. Smooth, duration configurable. |
| `element_drag` | Drag between CSS-selector elements (or with offset). |
| `status` | Health check -- reports state of all subsystems. |

---

## The Graduation Pipeline

This is the workflow pattern that makes Hands more than just another browser tool.
The idea: **automate with the browser once, then graduate to cheap HTTP replays.**

### Step 1: Do it in the browser

Use browser tools to navigate, log in, click through a flow. This is the
exploration/recording phase.

### Step 2: Capture the API calls

While the browser session is active, `browser_get_all_network` captures every
HTTP request the page made. This includes API calls, auth tokens, payload formats.

### Step 3: Analyze with browser_learn_api

Feed the network log to `browser_learn_api`. It analyzes the captured traffic and
extracts:
- API endpoint URLs and methods
- Required headers (auth tokens, content types)
- Request/response payload structures
- Which calls are the actual data fetches vs. analytics/tracking noise

### Step 4: Replay via HTTP

Now you have the API pattern. Future runs skip the browser entirely -- use
`browser_http_scrape` or direct HTTP calls to hit the API endpoint. No Chrome
process, no rendering, no waiting for page loads. Milliseconds instead of seconds.

### Why this matters

Most web automations are doing the same thing every time: log in, navigate to a
page, extract some data. The browser is just the discovery mechanism. Once you
know the underlying API, you don't need it anymore.

This is especially powerful for:
- Scheduled data collection (check a dashboard daily)
- Monitoring (poll an endpoint for changes)
- Bulk operations (process 100 items via API instead of 100 browser sessions)

---

## Stealth Mode

Some sites detect headless browsers and block them. `browser_launch` accepts a
`stealth` parameter that applies anti-detection measures:

```json
{"tool": "browser_launch", "arguments": {"stealth": true}}
```

This modifies browser fingerprints, removes automation indicators, and adjusts
timing patterns. It's not foolproof -- sophisticated anti-bot systems will still
catch it -- but it handles the common checks (navigator.webdriver, headless
detection, etc.).

**When to use stealth:**
- Sites that return different content to headless browsers.
- Sites with Cloudflare, Akamai, or similar bot detection.
- When `browser_http_scrape` fails with 403 or returns a challenge page.

**When NOT to use stealth:**
- Internal tools, admin panels, APIs you control.
- Sites that don't do bot detection (most of them).
- Stealth mode adds startup time. Don't use it by default.

---

## Browser Profiles

`browser_launch` accepts `profile_path` to persist cookies, localStorage, and
session data across browser launches:

```json
{"tool": "browser_launch", "arguments": {"profile_path": "C:/profiles/mysite"}}
```

This means you can log in once, save the profile, and subsequent launches are
already authenticated. Combine with the graduation pipeline: use a profile for the
initial exploration, capture the auth tokens, then graduate to HTTP replays.

---

## Common Patterns

### Pattern: Scrape a page (cheapest path)

```
1. browser_http_scrape(url: "https://example.com/data")
   -> If this returns good content, you're done.

2. If page needs JS: browser_smart_browse(url: "...")
   -> If this works, you're done.

3. If interactive or JS-heavy:
   browser_launch() -> browser_navigate(url: "...") -> browser_extract_content()
   -> browser_close()
```

### Pattern: Fill a web form

```
1. browser_launch() -> browser_navigate(url: "https://example.com/form")
2. browser_get_forms()  -> discover field names and types
3. browser_fill_form(fields: [...])
4. browser_click(a11y_ref: "ref_for_submit")  or  browser_submit_form()
5. browser_screenshot()  -> verify success
6. browser_close()
```

### Pattern: Automate a Windows app

```
1. uia_app_launch(name: "notepad.exe")
2. uia_list_window()  -> confirm it's running, get exact title
3. uia_focus_window(title: "Untitled - Notepad")
4. uia_type(text: "Hello world")
5. uia_shortcut(keys: "Ctrl+S")
6. wait_for_visual(text: "Save As")  -> wait for save dialog
7. uia_type(text: "myfile.txt")
8. uia_key_press(key: "Enter")
```

### Pattern: Visual verification after automation

```
1. (... do your browser or UIA automation ...)
2. vision_screenshot_ocr(region: {x: 100, y: 200, width: 400, height: 50})
   -> confirm expected text appears
3. If unexpected: vision_diff(image_a: "before.png", image_b: "after.png")
   -> see what changed
```

### Pattern: Graduate browser flow to HTTP

```
1. browser_launch(profile_path: "C:/profiles/mysite")
2. browser_navigate(url: "https://app.example.com/login")
3. browser_fill_form(...) -> browser_click(...) -> (complete the flow)
4. browser_get_all_network()  -> capture all HTTP traffic
5. browser_learn_api()  -> extract API patterns from the traffic
6. browser_close()

Future runs:
7. browser_http_scrape(url: "https://api.example.com/data",
     headers: {"Authorization": "Bearer ..."})
   -> no browser needed
```

### Pattern: Multi-window desktop workflow

```
1. uia_app_launch(name: "excel.exe")
2. uia_app_launch(name: "notepad.exe")
3. uia_window_snap(title: "Excel", position: "left")
4. uia_window_snap(title: "Notepad", position: "right")
5. uia_focus_window(title: "Excel")
6. (... read data with uia_read_value ...)
7. uia_focus_window(title: "Notepad")
8. uia_type(text: "data from excel")
```

---

## Anti-Patterns

### Don't: Launch Chrome for every web read

If you just need text from a URL, `browser_http_scrape` is 100x cheaper. Only
launch Chrome when you need interaction or JS rendering.

### Don't: Use vision to read web content

`browser_get_text` and `browser_extract_content` are faster, more accurate, and
cost zero vision tokens. Vision is for verification and non-DOM content.

### Don't: Use CSS selectors when a11y_ref is available

After `browser_navigate` or `browser_a11y_snapshot`, you have refs. Use them.
They're more stable and more readable than `#div > button.submit-btn:nth-child(3)`.

### Don't: Forget to close the browser

`browser_launch` starts a Chrome process. If you don't `browser_close`, it stays
running and leaks memory. Always close when done.

### Don't: Use individual calls for predictable sequences

If you're doing login -> navigate -> extract -> screenshot, that's 4 round-trips.
Use `browser_batch` for 1 round-trip.

### Don't: Re-snapshot the a11y tree unnecessarily

The tree is auto-cached on navigate. Only call `browser_a11y_snapshot` again after
a state change (navigation, tab click, dynamic content load). Don't re-snapshot
between every click on a static page.

### Don't: Ignore the wait tools

After clicking something that triggers a page load or XHR, use `browser_wait_for`,
`browser_wait_idle`, or `browser_wait_stable` before reading the result. Race
conditions are the #1 cause of flaky browser automation.

### Don't: Use UIA coordinates when names are available

`uia_click(x: 450, y: 320)` is fragile. `uia_click(name: "Save", control_type: "Button")`
survives window moves and resolution changes.

---

## Troubleshooting

### Browser won't launch

- Playwright binaries auto-download on first use. If behind a proxy, set
  `HTTPS_PROXY` env var or install manually: `npx playwright install chromium`.
- Check that no other Hands process is holding the browser lock.

### a11y_ref returns "element not found"

- The snapshot is stale. Re-call `browser_a11y_snapshot` after navigation or
  dynamic content changes.
- The element may be in a different frame/tab. Check `browser_switch_tab`.

### UIA can't find elements

- `uia_list_window()` first -- verify the app is running and the title matches.
- Some apps use custom rendering with no UIA support. Try `vision_screenshot_ocr`
  as fallback.
- On ARM64: UIA works natively. If elements are missing, it's the app, not the
  platform.

### browser_http_scrape returns empty or 403

- The site may require JS rendering. Step up to `browser_smart_browse`.
- The site may block non-browser user agents. Step up to `browser_launch` with
  `stealth: true`.
- The site may require auth. Use `browser_launch` with `profile_path` to log in
  first.

### wait_for_visual times out

- Increase `timeout_ms` (default 10000). Some pages are slow.
- Check the `text` or `template_path` matches exactly. OCR can be sensitive to
  font rendering.
- Use `vision_screenshot_ocr` to see what's actually on screen.

### Batch operation fails partway through

- By default, batches stop on first error. Check the error to see which step
  failed.
- Set `continue_on_error: true` if you want best-effort execution.
- The return value includes results for all completed steps plus the error.

### Stealth mode still gets blocked

- Sophisticated anti-bot systems (Cloudflare Enterprise, PerimeterX) may still
  detect automation. Stealth handles common checks but isn't a silver bullet.
- Try adding realistic delays between actions.
- Consider the graduation pipeline -- capture the API and skip the browser entirely.

---

## Version Notes (v1.1.1)

- 116 tools across 5 tiers (browser, UIA, vision, combo, meta)
- Accessibility snapshot auto-caching on navigate
- `browser_learn_api` for API discovery from network traffic
- `browser_a11y_find` for fast cached snapshot search
- `retry_click` for flaky element resilience
- `file_upload` via DataTransfer API
- Full ARM64 Windows support for UIA tier
- Single binary, no runtime dependencies
