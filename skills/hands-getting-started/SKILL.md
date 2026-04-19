---
name: hands-getting-started
description: 'Getting started with Hands -- the 116-tool automation server for browser,

  Windows desktop, and vision/OCR tasks. Use when: first time using hands,

  unsure which hands tool to pick, need a workflow example, or want to

  understand what hands can do vs other approaches.'
---

## What Hands Is

A single MCP server (hands.exe) with 87 tools across 4 subsystems. It replaces pixel-guessing with structured, fast automation.

| Subsystem | Prefix | Tools | What It Does |
|-----------|--------|-------|-------------|
| Browser | browser_* | 51 | Playwright-based. DOM, JS, network, forms, tabs. |
| UIA | uia_* | 16 | Windows UI Automation. Native app control. |
| Vision | vision_* | 9 | Screenshots, OCR, template match, image diff. |
| Combo | (mixed) | 11 | Cross-subsystem: drag, file_upload, find_and_click, read_screen_text, etc. |

## First Steps -- Browser

Most tasks start here. The pattern is always: **launch/attach -> navigate -> do things -> extract**.

### 1. Get a browser
```
hands:browser_launch()              # new headless browser
hands:browser_attach(port=9222)     # connect to existing Chrome (launched with --remote-debugging-port=9222)
```
Use attach when you need to work in the user's logged-in session (cookies, auth).

### 2. Navigate
```
hands:browser_navigate(url="https://example.com")
```

### 3. Extract content
```
hands:browser_extract_content(url="https://example.com")   # clean text, no junk -- best for reading pages
hands:browser_get_text()                                     # raw visible text of current page
hands:browser_get_html(selector="main")                      # HTML of a specific element
hands:browser_js_extract(script="document.title")            # run JS and return result
```
browser_extract_content is the go-to for "read this webpage." Use instead of web_fetch.

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
hands:uia_read_value(name="Total")                           # read a value
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
uia_app_launch -> uia_find -> uia_click / uia_type -> uia_read_value

**Monitor for a visual change:**
wait_for_visual(text="Complete") -> read_screen_text()

**Extract data from multiple pages:**
browser_navigate -> browser_scroll_collect(selector=".item") -- auto-scrolls and collects.

**Batch browser actions (speed):**
```
hands:browser_batch(actions=[
  {"action": "click", "selector": "#tab1"},
  {"action": "wait_for", "selector": ".content"},
  {"action": "get_text", "selector": ".content"}
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
| Run JavaScript | browser_eval |
| Manage browser tabs | browser_new_tab / browser_switch_tab / browser_close_tab |
| Check what windows are open | uia_list_window |
| Download a file | browser_navigate to URL or browser_click on download link |

## Key Differences from Claude Computer Use

Hands is NOT pixel-guessing. It uses structured APIs:
- **Browser**: Playwright selectors (CSS, XPath, text) -- precise, fast, no screenshots needed
- **UIA**: Windows Accessibility tree -- finds elements by name, role, state
- **Vision**: OCR engine + template matching -- structured text extraction, not model interpretation

Each action takes milliseconds, not 2 seconds. You can batch actions. You get structured data back, not just screenshots.

## Reference

For full capability comparison, see the [README](../../README.md#comparison-with-claude-computer-use).
