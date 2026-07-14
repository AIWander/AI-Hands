# AI-Hands tool map

Server: `AI-Hands` · Binary: `hands.exe`  
Qualified names (Grok): `AI-Hands__<tool>` · Live count ~119 · Meta-tier is preferred.

Repo skills (Claude-oriented ladder docs): `skills/hands/` and `skills/hands-getting-started/` at the AI-Hands repo root.

## Meta-tier (`hands_*`) — prefer these

| Tool | Role |
|------|------|
| `hands_read_page` | Fetch readable content; escalate HTTP→Node→Chrome |
| `hands_navigate` | Open URL, auto-launch browser, wait ready |
| `hands_click` | Click any target; reversibility tags; 7-rung |
| `hands_type` | Type into fields with focus verify |
| `hands_find` | Locate element across browser/desktop/screen |
| `hands_capture` | Screenshot + optional OCR verify |
| `hands_verify` | Polling state check + named templates |
| `hands_fill_form` | Multi-field form fill |
| `hands_login_recovery` | Login + 2FA pipeline |
| `hands_scan_qr` | QR / otpauth decode |
| `hands_app_action` | Window open/close/focus/snap/… |
| `hands_script` | Multi-step meta orchestration |
| `hands_health` | Paths + subsystem probe |
| `status` | High-level subsystem status |

## Browser tier (selection)

Lifecycle: `browser_launch`, `browser_attach`, `browser_debug_launch`, `browser_close`, `browser_status`  
Tabs/contexts: `browser_new_tab`, `browser_list_tab`, `browser_switch_tab`, `browser_close_tab`, `browser_context_*`  
Nav: `browser_navigate`, `browser_back`, `browser_forward`, `browser_reload`, `browser_get_url`  
Interact: `browser_click`, `browser_type`, `browser_press`, `browser_hover`, `browser_focus`, `browser_select`, `browser_scroll`, `browser_fill_form`, `browser_submit_form`  
A11y: `browser_a11y_snapshot`, `browser_a11y_find`  
Extract: `browser_get_text`, `browser_get_html`, `browser_extract_content`, `browser_smart_browse`, `browser_http_scrape`, `browser_js_extract`, `browser_bulk_extract`, `browser_scroll_collect`, `browser_iframe_extract`  
Network: `browser_route*`, `browser_get_network_log`, `browser_get_performance_log`, `browser_get_all_network`, `browser_learn_api`  
Wait: `browser_wait_for`, `browser_wait_idle`, `browser_wait_stable`  
Shot: `browser_screenshot`, `browser_screenshot_burst`, `browser_verify_visual`  
Batch/agent: `browser_batch`, `browser_script`, `browser_agent`, `browser_plan`, `browser_evaluate`  
Other: `browser_cookies`, `browser_eval`, `browser_inject_script`, `browser_trace_*`, `file_upload`

## UIA tier

`uia_get_state`, `uia_list_window`, `uia_find`, `uia_click`, `uia_type`, `uia_focus_window`, `uia_key_press`, `uia_hold_key`, `uia_shortcut`, `uia_read_value`, `uia_scroll`, `uia_watch`, `uia_poll_event`, `uia_app_launch`, `uia_batch`, `uia_window_resize`, `uia_window_move`, `uia_window_snap`, `uia_window_state`

## Vision tier

`vision_screenshot`, `vision_screenshot_hidden_window`, `vision_ocr`, `vision_screenshot_ocr`, `vision_check_user_input`, `vision_load_image`, `vision_diff`, `vision_zoom`, `vision_find_template`, `vision_analyze`

## Combo / legacy

Prefer meta-tools. Legacy still registered: `find_and_click`, `type_into_window`, `retry_click`, `read_screen_text`, `wait_for_visual`, `window_screenshot`, `drag`, `element_drag`
