---
name: ai-hands-workflows
description: >
  Ready-made multi-step AI-Hands recipes: scrape pages, fill forms, login recovery,
  desktop app control, visual verify loops, hands_script orchestration. Use when the
  user wants a hands playbook, automation recipe, or says /ai-hands-workflows.
metadata:
  short-description: "AI-Hands multi-step recipes"
---

# AI-Hands Workflows

Load **`ai-hands`** + **`ai-hands-safety`** first. Tool names below are logical; call via Grok as `AI-Hands__<name>` after `search_tool`.

## 1. Read a page (no interaction)

```
hands_read_page(url="https://example.com", extract_mode="text")
```

Optional: `wait_for` CSS selector (forces Chrome). Prefer this over `web_fetch` when JS or anti-bot is likely.

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

## 6. hands_script multi-step

```json
{
  "variables": { "base": "https://example.com" },
  "timeout_ms": 120000,
  "steps": [
    {
      "tool": "hands_navigate",
      "label": "open site",
      "args": { "url": "{{base}}/login" }
    },
    {
      "tool": "hands_fill_form",
      "label": "credentials",
      "args": { "fields": { "Email": "a@b.com", "Password": "…" } },
      "on_error": "stop"
    },
    {
      "tool": "hands_click",
      "label": "sign in",
      "args": { "target": "Sign in" }
    },
    {
      "tool": "hands_verify",
      "label": "landed",
      "args": { "template": "verify_login_success", "template_args": { "success_text": "Welcome" } },
      "output_var": "verify_result"
    }
  ]
}
```

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

## 9. Network → API graduation

```
browser_trace_start / browser_route(pattern="**/api/**", action="log")
# exercise UI with hands_*
browser_get_network_log / browser_learn_api
# store patterns in workflow vault for later pure-HTTP calls
```

## 10. Failure playbook

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
