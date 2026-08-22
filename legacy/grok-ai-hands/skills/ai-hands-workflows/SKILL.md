---
name: ai-hands-workflows
description: >
  Ready-made safe-profile AI-Hands recipes: visible page reads, fixed batches, forms,
  login recovery, desktop app control, and visual verify loops. Use when the
  user wants a hands playbook, automation recipe, or says /ai-hands-workflows.
metadata:
  short-description: "AI-Hands multi-step recipes"
---

# AI-Hands Workflows

Load **`ai-hands`** + **`ai-hands-safety`** first. Tool names below are logical; call via Grok as `AI-Hands__<name>` after `search_tool`.

## 1. Read a page (no interaction)

```
hands_navigate(url="https://example.com")
browser_extract_content()  # or browser_get_text()
```

Keep the visible browser as the source of truth. Use `hands_find` if a bounded target must be located.

## 2. Navigate then interact

```
hands_navigate(url="https://example.com/app", wait_condition="networkidle")
hands_find(target="Search", return_type="ref")
hands_type(target="Search", text="query", submit=true)
hands_verify(natural_text="shows results", timeout_ms=10000)
```

## 3. Fill a form

```
hands_navigate(url="...")
hands_fill_form(
  fields={"Email": "user@example.com", "Name": "Joe"},
  auto_submit=false
)
# After user OK if submit is irreversible:
hands_click(target="Submit")
hands_verify(template="verify_form_submitted")
```

Never set `auto_submit=true` on money/account-critical forms without explicit user OK.

## 4. Login recovery (vault-backed)

```
hands_login_recovery(
  url="https://example.com/login",
  credential_name="example",
  totp_name="example",
  success_text="Dashboard",
  success_url_contains="/app"
)
```

Verify domain first. Prefer vault names over raw username/password in chat.

## 5. Desktop app (Notepad-style)

```
hands_app_action(action="open", launch_spec="notepad")
hands_type(target="Document", text="hello from hands")
hands_capture(target="Untitled", verify="hello from hands")
hands_app_action(action="close", window_match={"title": "Notepad"}, on_save_dialog="discard")
```

## 6. Fixed browser batch

Use `browser_batch` only when every action is fixed and predictable, such as click,
wait, type, and screenshot in a known current page. Keep login submission or another
irreversible step outside the batch so the user can confirm it. Use individual calls
when an intermediate result determines the next action.

## 7. Attach to existing Chrome (keep cookies)

```
browser_debug_launch(port=9222, url="https://…")  # or user already running Chrome with debug port
browser_attach(port=9222)
# then hands_navigate / hands_click as usual on that session
```

## 8. Visual wait / change detect

```
hands_capture(target="screen", save_path="C:/temp/before.png")
# … trigger action …
hands_verify(text="Complete", timeout_ms=30000, must_stabilize_ms=500)
# or vision_diff(image_a=before, image_b="screen")
```

## 9. Minimized network evidence to durable owner

```
# exercise the visible UI with hands_*
browser_get_network_log / browser_learn_api  # minimized, pattern-redacted output
# hand endpoint shape to Workflow or another web/network owner for validation
```

Hands does not own durable direct API methods. Pattern redaction is defense in depth, not a secrecy guarantee; use isolated sessions and least privilege. Compatibility-only built-in direct fetch is an unsafe debug escape hatch, not the speed path.

## 10. Compatibility gate

Raw/direct-fetch/value/trace/QR/event/native-plugin capabilities require all three: `HANDS_TOOL_PROFILE=compatibility`, `HANDS_ALLOW_UNSAFE_RAW_TOOLS=1`, `HANDS_ALLOW_UNSAFE_DIRECT_FETCH=1`, or `HANDS_ALLOW_UNSAFE_PLUGINS=1` as appropriate, and the matching per-call `allow_unsafe_raw=true`, `allow_unsafe_fetch=true`, or `allow_unsafe_plugin=true`. The composite `browser_script` and `browser_evaluate` tools require both the direct-fetch and raw gate pairs. Whenever monitor scope is active, these vendor composites and aliases fail closed even when every compatibility gate is present: `browser_agent`/`agent`, `browser_batch`/`batch`, `browser_script`/`script`, `browser_evaluate`/`evaluate`, `browser_screenshot_burst`/`screenshot_burst`, `browser_scroll_collect`/`scroll_collect`, `browser_wait_stable`/`wait_stable`, and `retry_click` (with `browser_retry_click` treated defensively as an alias). Their nested vendor steps cannot revalidate the bound browser window. Use individually scoped browser calls or compatibility-gated `hands_script`, which centrally revalidates each nested call. Safe profiles reduce built-in first-party surface; Hands can still launch external desktop applications and is not a general OS or secrecy sandbox. The Hands HTTP dashboard is off unless `HANDS_ENABLE_DASHBOARD=1` is deliberately set.

Before enabling a monitor fence, clear browser routes and stop any active trace; fence activation refuses while either persistent state is active. Under an active fence, `browser_route` and `browser_trace_start` fail closed, while `browser_route_remove`, `browser_route_clear`, and `browser_trace_stop` remain available so cleanup cannot be trapped.

## 11. Failure playbook

| Symptom | Fix |
|---------|-----|
| Element not found | `hands_find` broader text; re-nav; `hands_capture` |
| Stale ref after navigation | Find again; don't reuse old `ref_N` |
| Chrome profile locked | Close orphan Chrome; new context |
| Click hit wrong control | `strict=true` on `hands_click`; use offset only when intentional |
| OCR noise | Prefer DOM/`hands_verify` text rungs; `vision_zoom` for tiny text |
| Popup stole focus | `hands_app_action(focus)` or `uia_focus_window` then retry |

## Post-action always

Prefer `hands_verify` or `hands_capture(verify=…)` over assuming success from a click return alone.
