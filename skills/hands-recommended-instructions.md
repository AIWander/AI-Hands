# Hands MCP Server -- Recommended CLAUDE.md Instructions

Drop the block between the fence markers into your CLAUDE.md (global or per-project)
or Claude chat system prompt to get sane defaults when using the Hands MCP server.

---

```markdown
<!-- ======== BEGIN HANDS INSTRUCTIONS ======== -->

## Hands MCP Server -- Behavioral Defaults

### Safe Visible-Browser Ladder
Use the current visible browser as the normal source of truth:
1. `hands:hands_navigate` -- open the requested page visibly and wait for current state.
2. `hands:browser_extract_content` or `hands:browser_get_text` -- read structured visible content.
3. `hands:hands_find` -- locate a bounded target when interaction is needed.
4. `hands:browser_batch` -- run only fixed, predictable browser actions.
5. Vision (`hands:vision_screenshot_ocr`) -- verify or handle a surface the browser cannot expose.

Do not use raw HTTP/direct fetch, arbitrary evaluation, raw HTML or accessibility dumps, page capture, free-form scripts, raw QR decoding, or UIA value/event surfaces in a safe profile.

### Structured Interaction
Use `hands_find(return_type="ref")` to obtain a bounded current reference, then pass
that reference to an interaction tool. Do not request a raw accessibility-tree dump
in a safe profile.

### Batch When Predictable
Use `browser_batch` or `uia_batch` for predictable multi-step sequences (login flows,
form fills, menu navigation). Use individual calls only when intermediate state
inspection is needed to decide the next step.

### Graduation Boundary
Hands may exercise the visible UI and inspect minimized current-network metadata to
identify endpoint shape. Pattern redaction and output/persistence minimization are
defense in depth, not a secrecy guarantee; use isolated sessions and least privilege.
Workflow or another dedicated web/network owner must validate,
store, credential, and run any durable direct API method. Verify reused results against
current visible state when accuracy matters.

### Profiles and Unsafe Compatibility
`default`, `full`, and `strict` are safe-advertised and expose 105, 107, and 109
entries including `hands_capability_catalog`. `compatibility` exposes 144 entries and
is an unsafe debug escape hatch, not a speed path. A compatibility-only raw,
built-in direct-fetch, or native-plugin call requires all three:
`HANDS_TOOL_PROFILE=compatibility`, the matching process gate
(`HANDS_ALLOW_UNSAFE_RAW_TOOLS=1`, `HANDS_ALLOW_UNSAFE_DIRECT_FETCH=1`, or
`HANDS_ALLOW_UNSAFE_PLUGINS=1`), and the matching per-call acknowledgement
(`allow_unsafe_raw=true`, `allow_unsafe_fetch=true`, or
`allow_unsafe_plugin=true`). The composite `browser_script` and
`browser_evaluate` tools require both the direct-fetch and raw gate pairs. Safe
profiles reduce built-in first-party surface. Whenever monitor scope is active,
these vendor composites and aliases fail closed even when every compatibility
gate is present: `browser_agent`/`agent`, `browser_batch`/`batch`,
`browser_script`/`script`, `browser_evaluate`/`evaluate`,
`browser_screenshot_burst`/`screenshot_burst`,
`browser_scroll_collect`/`scroll_collect`, `browser_wait_stable`/`wait_stable`,
and `retry_click` (with `browser_retry_click` treated defensively as an alias).
Their nested vendor steps cannot revalidate the bound browser window. Use
individually scoped browser calls or compatibility-gated `hands_script`, which
centrally revalidates each nested call.
Hands can still launch external desktop applications and is not a general OS or
secrecy sandbox.

Before enabling a monitor fence, clear browser routes and stop any active trace;
fence activation refuses while either persistent state is active. Under an active
fence, `browser_route` and `browser_trace_start` fail closed, while
`browser_route_remove`, `browser_route_clear`, and `browser_trace_stop` remain
available so cleanup cannot be trapped.

The Hands HTTP dashboard is off by default. Set `HANDS_ENABLE_DASHBOARD=1` only when
a local operator deliberately opts in after reviewing the listening boundary.

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
