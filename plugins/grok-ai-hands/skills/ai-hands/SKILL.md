---
name: ai-hands
description: >
  Drive AI-Hands (hands.exe) for visible browser, Windows desktop (UIA), vision/OCR,
  and bounded verified automation. Prefer safe hands_* meta-tools over browser_*/uia_*/vision_*
  primitives. Use when automating Chrome/web, clicking/typing on screen, controlling
  Windows apps, screenshots/OCR, form fill, login recovery, or when the user says
  /ai-hands, hands, AI-Hands, browser automation, or desktop control.
metadata:
  short-description: "AI-Hands meta-tools + routing"
---

# AI-Hands

MCP server **AI-Hands** (`hands.exe`). The recommended safe `default` profile advertises 104 operational tools plus the catalog (105 entries); safe `full` has 106 plus the catalog (107), and safe `strict` has 108 plus the catalog (109). `compatibility` has all 143 plus the catalog (144) and is an unsafe debug escape hatch.

Also load **`ai-hands-safety`** for any real-click / login / form work.

## Tool calling (Grok)

1. `search_tool` with a keyword (e.g. `hands_click`) to get the **input schema**.
2. `use_tool` with `tool_name` = qualified name `AI-Hands__<tool>` and matching `tool_input`.
3. Never invent parameters — schema is source of truth.

Other harnesses may use `hands:<tool>` or server-qualified names; always follow that host's MCP naming.

## Default: meta-tools first

| Intent | Tool | Notes |
|--------|------|-------|
| Read a URL visibly | `hands_navigate` then `browser_extract_content` / `browser_get_text` | Current browser state is authoritative |
| Go somewhere (then interact) | `hands_navigate` | Auto-launches visible Chrome by default |
| Click anything | `hands_click` | 7-rung ladder; tags Reversible / RequiresConfirmation / Destructive |
| Type into a field | `hands_type` | Focus verify, clear, chunked; sensitive fields = keystrokes |
| Find element | `hands_find` | `return_type=ref` before interact |
| Screenshot / OCR | `hands_capture` | browser / window title / screen + optional verify |
| Did it work? | `hands_verify` | Poll + templates (login, form, modal, …) |
| Fill multi-field form | `hands_fill_form` | Label→input match; careful with `auto_submit` |
| Login + 2FA | `hands_login_recovery` | Prefer `credential_name` / `totp_name` vault refs |
| Fixed browser sequence | `browser_batch` | Use only fixed, predictable actions |
| Window open/close/snap | `hands_app_action` | Desktop window management |
| Focus known app/window | `uia_focus_window` / `hands_app_action(focus)` | **By title — never full list first** |
| Health / paths | `hands_health` / `status` | Subsystem probe |

**Do not start with** `browser_click`, `uia_click`, `find_and_click`, `type_into_window` unless a meta-tool failed or you need a known-good low-level escape hatch.

## Windows discovery (list is OK — use it well)

`uia_list_window` can hang on stuck HWNDs in older builds; hang-safe builds skip hung windows and return partial lists. Still prefer title focus when you already know the app.

**When list is helpful**
- First discovery: "what Solitaire/Chrome/etc. windows exist?"
- Multi-instance pick (two Chromes, two Notepads)
- After open/launch when title is unknown/unstable
- Debugging "is the app up?"

**How to mitigate stuck without ignoring list**
1. Prefer `uia_focus_window(title=…)` / `hands_app_action(focus|open)` when title is known.
2. If listing: call with defaults (or `timeout_ms=2000`); read `skipped_hung` / `truncated` / `note` in the response.
3. **Cache** the list for the phase — don't re-list every step (plugin hook soft rate-limit ~45s still applies to reduce thrash).
4. Use list results to pick a title, then **focus by title** for the rest of the work.
5. Escape for rapid re-list: `AI_HANDS_ALLOW_UIA_LIST=1`.

## Deprecated combo tools (avoid)

| Deprecated | Use instead |
|------------|-------------|
| `find_and_click` | `hands_click` |
| `type_into_window` | `hands_type` |
| `retry_click` | `browser_click` retries or `hands_click` |
| `read_screen_text` | `hands_capture` |
| `window_screenshot` (default) | `hands_capture` / `vision_screenshot` |

## Engine facts (don't re-learn wrong)

- **Not Playwright.** CDP via chromiumoxide; attaches to installed Chrome.
- Logged-in session: `browser_debug_launch` then `browser_attach`, or attach to existing `--remote-debugging-port=9222`.
- Browser content is **not** reliable via UIA — use browser/a11y or meta-tools with `page_context=browser`.
- Prefer a current structured ref returned by `hands_find(return_type="ref")`; do not request a raw accessibility dump in a safe profile.
- Stealth / bot-compat only if the user explicitly asks.

## Standard loops

### Interact then confirm
```
hands_navigate(url) → hands_click/type/fill → hands_verify(natural_text|template)
```

### Desktop app
```
hands_app_action(action=open, launch_spec=...) → hands_find/click/type → hands_capture(verify=...)
```

### Fixed multi-step
```
hands_navigate(url) → hands_find(return_type=ref) → browser_batch(fixed actions) → hands_verify(...)
```
Use individual calls when an intermediate result determines the next action.

### Failure recovery
1. Re-`hands_find` / re-snapshot (stale refs after nav).
2. Widen target text or set `page_context` / `scope` explicitly.
3. `hands_capture` to see state; only then drop to primitives.
4. Profile lock → close stuck Chrome or `browser_context_create`.

## When to use primitives

| Situation | Primitive |
|-----------|-----------|
| Network mock/block/log | `browser_route*` + `browser_get_network_log` |
| Multi-tab / multi-context accounts | `browser_*_tab`, `browser_context_*` |
| Learn API from traffic | `browser_learn_api` |
| Trace timing | Compatibility/debug only; prefer screenshots plus explicit state verification |
| Exact UIA tree for a native app | `uia_get_state` → `uia_click` with `element_ref` (after focus) |
| Template match by image | `vision_find_template` |
| Hidden/obscured window pixels | `vision_screenshot_hidden_window` |
| Named app shortcuts | `uia_shortcut` |
| List windows (rare) | `uia_list_window` once + cache; prefer title focus first |

Full map: [references/tool-map.md](references/tool-map.md). Recipes: invoke **`ai-hands-workflows`**.

## Unsafe compatibility boundary

Safe profiles hide built-in first-party raw/direct-fetch/native-plugin front doors. Do not recommend arbitrary evaluation, raw HTML or accessibility dumps, page capture, free-form `hands_script`, raw QR decoding, or UIA value/event surfaces in a safe profile. Compatibility is not the speed path. Each such call requires:

1. `HANDS_TOOL_PROFILE=compatibility`
2. `HANDS_ALLOW_UNSAFE_RAW_TOOLS=1`, `HANDS_ALLOW_UNSAFE_DIRECT_FETCH=1`, or `HANDS_ALLOW_UNSAFE_PLUGINS=1`
3. `allow_unsafe_raw=true`, `allow_unsafe_fetch=true`, or `allow_unsafe_plugin=true` on that exact call

`browser_script` and `browser_evaluate` are composite tools: each requires both the direct-fetch and raw process gates and both per-call acknowledgements. Whenever monitor scope is active, these vendor composites and aliases fail closed even when every compatibility gate is present: `browser_agent`/`agent`, `browser_batch`/`batch`, `browser_script`/`script`, `browser_evaluate`/`evaluate`, `browser_screenshot_burst`/`screenshot_burst`, `browser_scroll_collect`/`scroll_collect`, `browser_wait_stable`/`wait_stable`, and `retry_click` (with `browser_retry_click` treated defensively as an alias). Their nested vendor steps cannot revalidate the bound browser window. Use individually scoped browser calls or compatibility-gated `hands_script`, which centrally revalidates each nested call.

Before enabling a monitor fence, clear browser routes and stop any active trace; fence activation refuses while either persistent state is active. Under an active fence, `browser_route` and `browser_trace_start` fail closed, while `browser_route_remove`, `browser_route_clear`, and `browser_trace_stop` remain available so cleanup cannot be trapped.

Workflow or another dedicated web/network owner handles validated durable/direct API methods. Hands can still launch and act through external desktop applications and is not a general OS or secrecy sandbox. The Hands HTTP dashboard is off by default; opt in with `HANDS_ENABLE_DASHBOARD=1` only after reviewing the listening boundary.

## Emit before irreversible UI actions

One line: `Action check: [reversible | RequiresConfirmation: <target> — asking | origin: <domain>]`
