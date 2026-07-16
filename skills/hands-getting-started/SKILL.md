---
name: hands-getting-started
description: 'Getting started with Hands -- the safe-profile automation server for browser,

  Windows desktop, and vision/OCR tasks. Use when: first time using hands,

  unsure which hands tool to pick, need a workflow example, or want to

  understand what hands can do vs other approaches.'
---

## What Hands Is

A single MCP server (`hands.exe`) with safe-advertised browser, Windows UIA, vision/OCR, cross-surface, monitor-scope, and health abilities. It replaces pixel-guessing with structured, verifiable automation.

| Profile | Operational tools | Catalog | Advertised entries | Safety |
|---|---:|---:|---:|---|
| `default` | 104 | 1 | 105 | Safe-advertised; recommended |
| `full` | 106 | 1 | 107 | Safe-advertised |
| `strict` | 108 | 1 | 109 | Safe-advertised |
| `compatibility` | 143 | 1 | 144 | Unsafe debug escape hatch |

Compatibility is not the fast path. A raw/debug, built-in direct-fetch, or native-plugin call requires `HANDS_TOOL_PROFILE=compatibility`, the matching process gate (`HANDS_ALLOW_UNSAFE_RAW_TOOLS=1`, `HANDS_ALLOW_UNSAFE_DIRECT_FETCH=1`, or `HANDS_ALLOW_UNSAFE_PLUGINS=1`), and the matching per-call acknowledgement (`allow_unsafe_raw=true`, `allow_unsafe_fetch=true`, or `allow_unsafe_plugin=true`). The composite `browser_script` and `browser_evaluate` tools require both the direct-fetch and raw gate pairs. Whenever monitor scope is active, these vendor composites and aliases fail closed even when every compatibility gate is present: `browser_agent`/`agent`, `browser_batch`/`batch`, `browser_script`/`script`, `browser_evaluate`/`evaluate`, `browser_screenshot_burst`/`screenshot_burst`, `browser_scroll_collect`/`scroll_collect`, `browser_wait_stable`/`wait_stable`, and `retry_click` (with `browser_retry_click` treated defensively as an alias). Their nested vendor steps cannot revalidate the bound browser window. Use individually scoped browser calls or compatibility-gated `hands_script`, which centrally revalidates each nested call. Safe profiles reduce the built-in first-party surface; Hands can still launch external desktop applications and is not a general OS or secrecy sandbox.

Before enabling a monitor fence, clear browser routes and stop any active trace; fence activation refuses while either persistent state is active. Under an active fence, `browser_route` and `browser_trace_start` fail closed, while `browser_route_remove`, `browser_route_clear`, and `browser_trace_stop` remain available so cleanup cannot be trapped.

## First Steps -- Browser

Most tasks start here. The pattern is always: **launch/attach -> navigate -> do things -> extract**.

### 1. Get a browser
```
hands:browser_launch(headless=false) # new visible browser
hands:browser_attach(port=9222)     # connect to existing Chrome (launched with --remote-debugging-port=9222)
```
Use attach when you need to work in the user's logged-in session (cookies, auth).

### 2. Navigate
```
hands:browser_navigate(url="https://example.com")
```

### 3. Extract content
```
hands:browser_extract_content()                              # clean text from the current visible page
hands:browser_get_text()                                     # visible text of current page
```
Use a visible browser: `hands_navigate`, then `browser_extract_content` or `browser_get_text`. Use `hands_find` and bounded `browser_batch` actions when interaction is needed. Workflow or another dedicated web/network owner handles validated durable direct API methods.

### 4. Interact
```
hands:browser_click(selector="#submit-btn")
hands:browser_type(selector="input[name='search']", text="query")
hands:browser_fill_form(fields=[{"selector": "#email", "value": "me@example.com"}])
hands:browser_press(key="Enter")
hands:browser_select(selector="#dropdown", value="option2")
```

### 5. Wait for things
```
hands:browser_wait_for(selector=".results", timeout=5000)   # wait for element
hands:browser_wait_idle()                                     # wait for network quiet
hands:browser_wait_stable()                                   # wait for visual stability
```

### 6. Screenshot
```
hands:browser_screenshot()                                    # full page
hands:browser_screenshot(selector=".chart")                   # specific element
```

## First Steps -- Windows Desktop (UIA)

For native Windows apps -- File Explorer, Notepad, Settings, any Win32/WPF/UWP app.

### 1. Launch or focus
```
hands:uia_app_launch(path="notepad.exe")
hands:uia_focus_window(title="Untitled - Notepad")
hands:uia_list_window()                                       # see what's open
```

### 2. Find and interact
```
hands:uia_find(name="Save", role="Button")                   # find element by name/role
hands:uia_click(name="Save", role="Button")                  # click it
hands:uia_type(name="File name:", text="report.txt")         # type into a field
hands:uia_get_state(name="Total")                           # inspect bounded structural state
```

### 3. Keyboard and window control
```
hands:uia_key_press(keys="ctrl+s")                           # keyboard shortcut
hands:uia_shortcut(keys="alt+F4")                            # same thing, alias
hands:uia_window_snap(title="Notepad", position="left")      # snap window
hands:uia_window_state(title="Notepad", state="maximize")    # maximize/minimize/restore
```

## First Steps -- Vision/OCR

For when you need to see the screen or read text from images.

```
hands:vision_screenshot()                                     # capture full screen
hands:vision_ocr(image_path="screenshot.png")                # OCR an image file
hands:vision_screenshot_ocr()                                 # screenshot + OCR in one call
hands:read_screen_text()                                      # same as above -- preferred alias
hands:vision_find_template(template="button.png")            # find image on screen
hands:vision_diff(before="a.png", after="b.png")             # compare two screenshots
hands:vision_analyze(image_path="screen.png", question="What app is open?")  # AI analysis
```

## Combo Tools -- Cross-Subsystem Power

These combine subsystems for common workflows:

| Tool | What It Does |
|------|-------------|
| find_and_click(text) | OCR screen -> find text -> click it. Works on any app. |
| read_screen_text() | Screenshot -> OCR -> return all text. Fastest screen read. |
| type_into_window(title, text) | Focus window -> type. No element search needed. |
| wait_for_visual(text) | Poll screen until text/image appears. Great for waits. |
| file_upload(selector, path) | Handle file picker dialogs in browser. |
| drag(from, to) | Pixel-coordinate drag. |
| element_drag(source, target) | Element-reference drag (selector or UIA name). |
| window_screenshot(title) | Screenshot a specific window by title. |

## Common Workflows

**Scrape a webpage:**
browser_extract_content(url="...") -- one call, done.

**Fill and submit a form:**
browser_navigate -> browser_fill_form -> browser_submit_form

**Automate a Windows app:**
uia_app_launch -> uia_find -> uia_click / uia_type -> uia_get_state

**Monitor for a visual change:**
wait_for_visual(text="Complete") -> read_screen_text()

**Extract data from multiple pages:**
browser_navigate -> browser_scroll_collect(selector=".item") -- auto-scrolls and collects.

**Batch browser actions (speed):**
```
hands:browser_batch(actions=[
  {"type": "click", "params": {"selector": "#tab1"}},
  {"type": "wait_for", "params": {"selector": ".content"}},
  {"type": "screenshot", "params": {"path": "C:/temp/content.png"}}
])
```

## Tool Selection Quick Ref

| I want to... | Use |
|--------------|-----|
| Read a webpage | browser_extract_content |
| Click something in a browser | browser_click |
| Click something in a Windows app | uia_click or find_and_click |
| Type text | browser_type (web) / uia_type (app) / type_into_window (quick) |
| See what's on screen | read_screen_text or vision_screenshot |
| Wait for something to appear | browser_wait_for (web) / wait_for_visual (anything) |
| Take a screenshot | browser_screenshot (web) / vision_screenshot (screen) / window_screenshot (app) |
| Run a fixed browser sequence | browser_batch |
| Manage browser tabs | browser_new_tab / browser_switch_tab / browser_close_tab |
| Check what windows are open | uia_list_window |
| Download a file | browser_navigate to URL or browser_click on download link |

## Key Differences from Claude Computer Use

Hands is NOT pixel-guessing. It uses structured APIs:
- **Browser**: chromiumoxide CDP selectors (CSS, XPath, text) -- precise, fast, no screenshots needed
- **UIA**: Windows Accessibility tree -- finds elements by name, role, state
- **Vision**: OCR engine + template matching -- structured text extraction, not model interpretation

Each action takes milliseconds, not 2 seconds. You can batch actions. You get structured data back, not just screenshots.

Do not recommend direct fetch, arbitrary evaluation, raw HTML or accessibility dumps, page capture, free-form `hands_script`, raw QR decoding, or raw UIA value/event tools in a safe profile. Those surfaces are compatibility/debug-only and require the explicit three-part gate above.

The Hands HTTP dashboard is off by default. Set `HANDS_ENABLE_DASHBOARD=1` only when a local operator deliberately opts in after reviewing the listening boundary.

## Reference

For full capability comparison, see the [README](../../README.md#comparison-with-claude-computer-use).
