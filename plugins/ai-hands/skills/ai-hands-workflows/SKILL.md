---
name: ai-hands-workflows
description: Use short verified AI-Hands recipes for multi-step browser, desktop, UIA, form, login, monitor-scoped, and visual automation. Trigger when a task needs several Hands calls, a safe playbook, recovery after a stale or failed UI action, or a decision between batching and individual calls.
---

# AI-Hands Workflows

Load `ai-hands` for tool selection and `ai-hands-safety` at consequential action boundaries. Observe, act in a short bounded burst, and re-observe. Never let remembered site behavior replace current evidence.

## Visible page read

1. Navigate with `hands_navigate`.
2. Read structured visible content with `browser_extract_content` or `browser_get_text`.
3. Use `hands_find` only when a bounded target must be located.

## Navigate and interact

1. `hands_navigate` to the intended origin.
2. `hands_find` the target from the current page.
3. Use `hands_type` or `hands_click` for one bounded mutation.
4. Verify with `hands_verify`, structured text, or a fresh capture.

## Forms and login

- Fill with `hands_fill_form(auto_submit=false)`.
- Keep submit outside a batch when it sends, buys, deletes, changes an account, or grants access.
- Verify the domain before credential use.
- Prefer vault-backed names with `hands_login_recovery`; never place raw secrets in chat.

## Desktop applications

1. Open or focus through `hands_app_action`.
2. Re-read the unique window title and current bounds.
3. Type or click once.
4. Capture or query state to verify.
5. Handle save, close, and permission dialogs as separate consequential steps.

## Batch only fixed steps

Use `browser_batch` or `uia_batch` only when every action is known in advance and no intermediate result changes the next step. Keep consequential submits outside the batch. Under an active monitor fence, use individually scoped calls because vendor composites that cannot revalidate nested display scope fail closed.

## Failure recovery

- Element missing: re-observe, broaden `hands_find`, and find a fresh reference.
- Stale reference: never reuse it after navigation; locate the element again.
- Wrong focus: inspect windows, focus the exact title, and retry once.
- OCR noise: prefer DOM or UIA text, then zoom or crop only the scoped region.
- Repeated failure: stop thrashing, capture current evidence, change one assumption, and verify.

Durable recording, replay, adaptation, schedules, credentials, and direct API methods belong to Workflow or another dedicated owner. Hands supplies current-state evidence and action.
