# Hands MCP Server -- Recommended CLAUDE.md Instructions

Drop the block between the fence markers into your CLAUDE.md (global or per-project)
or Claude chat system prompt to get sane defaults when using the Hands MCP server.

---

```markdown
<!-- ======== BEGIN HANDS INSTRUCTIONS ======== -->

## Hands MCP Server -- Behavioral Defaults

### Escalation Ladder (always start cheap)
When fetching web content, follow this order -- stop at the first rung that works:
1. `hands:browser_http_scrape` -- raw HTTP, no browser. Use for static pages.
2. `hands:browser_smart_browse` -- JS-capable fetch, still no Chrome.
3. `hands:browser_extract_content` -- headless Chrome, returns clean text.
4. `hands:browser_launch` + interactive tools -- only when you need to click/fill/scroll.
5. Vision (`hands:vision_screenshot_ocr`) -- last resort, or for verification.

Do NOT launch Chrome just to read a page. Try http_scrape first.

### Accessibility-First Interaction
After `browser_navigate`, an a11y snapshot is auto-cached. Use `a11y_ref` params
(e.g., `browser_click(a11y_ref: "ref_5")`) instead of CSS selectors. Re-snapshot
only after navigation or dynamic content changes, not between clicks on static pages.

### Batch When Predictable
Use `browser_batch` or `uia_batch` for predictable multi-step sequences (login flows,
form fills, menu navigation). Use individual calls only when intermediate state
inspection is needed to decide the next step.

### Graduation Pipeline
When you automate a browser flow that will be repeated:
1. Complete the flow with browser tools.
2. Call `browser_get_all_network` to capture HTTP traffic.
3. Call `browser_learn_api` to extract API patterns from the traffic.
4. Future runs: use `browser_http_scrape` with the discovered API endpoints -- no Chrome needed.

### Vision = Verification, Not Perception
Use vision tools (screenshot, OCR, template match) to **confirm** states, not to
**drive** decisions. Never OCR a web page when `browser_get_text` or
`browser_extract_content` can give you structured text directly.

### Desktop Apps = UIA, Not Vision
For native Windows apps, use `uia_*` tools (find, click, type, shortcut). Only fall
back to vision when the app doesn't expose UIA elements (`uia_list_window` returns
nothing useful for that app).

### Cleanup
Always call `browser_close` when you're done with a browser session. Leaked Chrome
processes waste memory and can block future launches.

### Browser Compatibility
Use browser compatibility adjustments only for authorized automation testing in
environments the user controls or has permission to test. Don't use them by
default -- they can add startup latency.

<!-- ======== END HANDS INSTRUCTIONS ======== -->
```

---

## Notes for integration

- The block above is self-contained -- copy the fenced section as-is.
- Works in `~/.claude/CLAUDE.md` (global), project `.claude/CLAUDE.md`, or Claude
  chat/Cowork system preferences.
- Assumes the Hands server is registered as `hands` in your MCP config. If you used
  a different name, find-replace `hands:` with your prefix.
- These are behavioral guidelines, not tool definitions. The MCP client discovers
  tools automatically from the server.
