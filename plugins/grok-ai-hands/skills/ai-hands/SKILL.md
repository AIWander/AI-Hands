---
name: ai-hands
description: >
  Drive AI-Hands (hands.exe) for browser, Windows desktop (UIA), vision/OCR, and
  multi-step automation. Prefer hands_* meta-tools over raw browser_*/uia_*/vision_*
  primitives. Use when automating Chrome/web, clicking/typing on screen, controlling
  Windows apps, screenshots/OCR, form fill, login recovery, or when the user says
  /ai-hands, hands, AI-Hands, browser automation, or desktop control.
metadata:
  short-description: "AI-Hands meta-tools + routing"
---

# AI-Hands

MCP server **AI-Hands** (`hands.exe`). ~119 tools: browser (chromiumoxide CDP), UIA, vision, and **hands_*** meta-tools that escalate across tiers.

Also load **`ai-hands-safety`** for any real-click / login / form work.

## Tool calling (Grok)

1. `search_tool` with a keyword (e.g. `hands_click`) to get the **input schema**.
2. `use_tool` with `tool_name` = qualified name `AI-Hands__<tool>` and matching `tool_input`.
3. Never invent parameters — schema is source of truth.

Other harnesses may use `hands:<tool>` or server-qualified names; always follow that host's MCP naming.

## Default: meta-tools first

| Intent | Tool | Notes |
|--------|------|-------|
| Read/scrape a URL | `hands_read_page` | HTTP → Node → Chrome; no manual launch |
| Go somewhere (then interact) | `hands_navigate` | Auto-launches visible Chrome by default |
| Click anything | `hands_click` | 7-rung ladder; tags Reversible / RequiresConfirmation / Destructive |
| Type into a field | `hands_type` | Focus verify, clear, chunked; sensitive fields = keystrokes |
| Find element | `hands_find` | `return_type=ref` before interact |
| Screenshot / OCR | `hands_capture` | browser / window title / screen + optional verify |
| Did it work? | `hands_verify` | Poll + templates (login, form, modal, …) |
| Fill multi-field form | `hands_fill_form` | Label→input match; careful with `auto_submit` |
| Login + 2FA | `hands_login_recovery` | Prefer `credential_name` / `totp_name` vault refs |
| Multi-step flow | `hands_script` | `{{var}}`, `output_var`, on_error stop/skip/retry |
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
- Prefer **a11y refs** (`ref_N` from snapshot/find) over CSS when calling primitives.
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

### Multi-step
```
hands_script(steps=[{tool, args, label, output_var, on_error}, ...], variables={...})
```
Step `tool` values are meta names: `hands_navigate`, `hands_click`, `hands_verify`, etc.

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
| Trace timing | `browser_trace_*` |
| Exact UIA tree for a native app | `uia_get_state` → `uia_click` with `element_ref` (after focus) |
| Template match by image | `vision_find_template` |
| Hidden/obscured window pixels | `vision_screenshot_hidden_window` |
| Named app shortcuts | `uia_shortcut` |
| List windows (rare) | `uia_list_window` once + cache; prefer title focus first |

Full map: [references/tool-map.md](references/tool-map.md). Recipes: invoke **`ai-hands-workflows`**.

## Emit before irreversible UI actions

One line: `Action check: [reversible | RequiresConfirmation: <target> — asking | origin: <domain>]`
