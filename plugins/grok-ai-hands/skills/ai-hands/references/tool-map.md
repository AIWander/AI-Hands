# AI-Hands tool map

Server: `AI-Hands` · Binary: `hands.exe`  
Qualified names (Grok): `AI-Hands__<tool>`. Safe advertised counts including the catalog are `default` 105, `full` 107, and `strict` 109. Unsafe `compatibility` advertises 144.

Repo skills (Claude-oriented ladder docs): `skills/hands/` and `skills/hands-getting-started/` at the AI-Hands repo root.

## Meta-tier (`hands_*`) — prefer these

| Tool | Role |
|------|------|
| `hands_navigate` | Open URL, auto-launch browser, wait ready |
| `hands_click` | Click any target; reversibility tags; 7-rung |
| `hands_type` | Type into fields with focus verify |
| `hands_find` | Locate element across browser/desktop/screen |
| `hands_capture` | Screenshot + optional OCR verify |
| `hands_verify` | Polling state check + named templates |
| `hands_fill_form` | Multi-field form fill |
| `hands_login_recovery` | Login + 2FA pipeline |
| `hands_app_action` | Window open/close/focus/snap/… |
| `hands_health` | Paths + subsystem probe |
| `status` | High-level subsystem status |

## Browser tier (selection)

Lifecycle: `browser_launch`, `browser_attach`, `browser_debug_launch`, `browser_close`, `browser_status`  
Tabs/contexts: `browser_new_tab`, `browser_list_tab`, `browser_switch_tab`, `browser_close_tab`, `browser_context_*`  
Nav: `browser_navigate`, `browser_back`, `browser_forward`, `browser_reload`, `browser_get_url`  
Interact: `browser_click`, `browser_type`, `browser_press`, `browser_hover`, `browser_focus`, `browser_select`, `browser_scroll`, `browser_fill_form`, `browser_submit_form`  
Find: `hands_find`, `browser_exists`, `browser_get_bounds`

Extract: `browser_get_text`, `browser_extract_content`, `browser_scroll_collect`, `browser_iframe_extract`

Network: `browser_route*`, `browser_get_network_log`, `browser_get_performance_log`, `browser_get_all_network`, `browser_learn_api`

Wait: `browser_wait_for`, `browser_wait_idle`, `browser_wait_stable`

Shot: `browser_screenshot`, `browser_screenshot_burst`, `browser_verify_visual`

Batch/agent: `browser_batch`, `browser_agent`, `browser_plan`
Other: `browser_cookies`, `file_upload`

## UIA tier

`uia_get_state`, `uia_list_window`, `uia_find`, `uia_click`, `uia_type`, `uia_focus_window`, `uia_key_press`, `uia_hold_key`, `uia_shortcut`, `uia_scroll`, `uia_app_launch`, `uia_batch`, `uia_window_resize`, `uia_window_move`, `uia_window_snap`, `uia_window_state`

## Vision tier

`vision_screenshot`, `vision_screenshot_hidden_window`, `vision_ocr`, `vision_screenshot_ocr`, `vision_load_image`, `vision_diff`, `vision_zoom`, `vision_find_template`, `vision_analyze`

## Combo / legacy

Prefer meta-tools. Legacy still registered: `find_and_click`, `type_into_window`, `retry_click`, `read_screen_text`, `wait_for_visual`, `window_screenshot`, `drag`, `element_drag`

## Unsafe compatibility/debug only

Do not recommend these in `default`, `full`, or `strict`:

- Direct fetch: `browser_http_scrape`, `browser_crawl`, `browser_map`, `browser_smart_browse`, `browser_bulk_extract`, `browser_js_extract`, `browser_script`, `browser_evaluate`, `hands_read_page`.
- Raw/debug: `browser_a11y_find`, `browser_a11y_snapshot`, `browser_eval`, `browser_get_clickables`, `browser_get_forms`, `browser_get_html`, `browser_page_capture`, `browser_page_dump`, `browser_trace_start`, `browser_trace_stop`, `browser_trace_save`, `hands_scan_qr`, `hands_script`, `uia_poll_event`, `uia_read_value`, `uia_watch`, `vision_check_user_input`, and `browser_inject_script`.
- Native plugin execution: `hands_plugin_load`, `hands_plugin_call`, `hands_plugin_unload`.

Each call requires `HANDS_TOOL_PROFILE=compatibility`, the matching process gate (`HANDS_ALLOW_UNSAFE_RAW_TOOLS=1`, `HANDS_ALLOW_UNSAFE_DIRECT_FETCH=1`, or `HANDS_ALLOW_UNSAFE_PLUGINS=1`), and the matching per-call acknowledgement (`allow_unsafe_raw=true`, `allow_unsafe_fetch=true`, or `allow_unsafe_plugin=true`). The composite `browser_script` and `browser_evaluate` tools require both the direct-fetch and raw gate pairs. Whenever monitor scope is active, these vendor composites and aliases fail closed even when every compatibility gate is present: `browser_agent`/`agent`, `browser_batch`/`batch`, `browser_script`/`script`, `browser_evaluate`/`evaluate`, `browser_screenshot_burst`/`screenshot_burst`, `browser_scroll_collect`/`scroll_collect`, `browser_wait_stable`/`wait_stable`, and `retry_click` (with `browser_retry_click` treated defensively as an alias). Their nested vendor steps cannot revalidate the bound browser window. Use individually scoped browser calls or compatibility-gated `hands_script`, which centrally revalidates each nested call. Compatibility is a deliberate debug escape hatch, not a speed path.

Before enabling a monitor fence, clear browser routes and stop any active trace; fence activation refuses while either persistent state is active. Under an active fence, `browser_route` and `browser_trace_start` fail closed, while `browser_route_remove`, `browser_route_clear`, and `browser_trace_stop` remain available so cleanup cannot be trapped.

Use visible browser observation (`hands_navigate`, `hands_find`, `browser_extract_content`, `browser_get_text`, fixed `browser_batch`) in safe profiles. Workflow or another dedicated web/network owner handles validated durable/direct API methods. Safe profiles reduce built-in first-party surface; Hands can still launch external desktop applications and is not a general OS or secrecy sandbox. The Hands HTTP dashboard is off by default and requires `HANDS_ENABLE_DASHBOARD=1` to opt in.
