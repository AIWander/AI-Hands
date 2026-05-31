//! Hands MCP Server — Unified interaction server
//! Combines browser automation, UI automation, and vision into one binary.
//! "The hands Claude uses to interact with everything on screen."
//!
//! Tool categories:
//!   Browser (web): smart_browse, http_scrape, js_extract, bulk_extract, extract_content,
//!                  scroll_collect, iframe_extract, agent, verify_visual, + full CDP browser
//!   UIA (desktop): get_state, list_windows, find_element, click, type_text, focus_window,
//!                  key_press, shortcut, read_value, scroll, watch, poll_events
//!   Vision (screen): screenshot, ocr, screenshot_ocr, diff, find_template, load_image, analyze
//!   Combo (new):  find_and_click, read_screen_text, wait_for_visual, window_screenshot

// Clippy-style lints that are not bug-indicating in this codebase:
// - too_many_arguments: meta-tool handlers intentionally take many params for fine-grained config
// - doc_lazy_continuation: doc comment list formatting is opinion, not correctness
// - needless_range_loop: occasional explicit index loops aid readability
// - single_match: kept for symmetry with adjacent multi-arm matches
#![allow(
    clippy::too_many_arguments,
    clippy::doc_lazy_continuation,
    clippy::needless_range_loop,
    clippy::single_match
)]

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

mod a11y_cache;
mod atomic;
mod dashboard_endpoint;
mod meta;
mod security;
mod stealth;
mod uia;

#[cfg(windows)]
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HWND, LPARAM, RECT, WPARAM},
        Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        },
        UI::{
            Input::KeyboardAndMouse::{
                SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
                KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_LWIN, VK_RETURN,
            },
            Shell::ShellExecuteW,
            WindowsAndMessaging::{
                GetWindowRect, PostMessageW, SetWindowPos, ShowWindow, SET_WINDOW_POS_FLAGS,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_MAXIMIZE,
                SW_MINIMIZE, SW_RESTORE, SW_SHOWNORMAL, WM_CLOSE,
            },
        },
    },
};

// ============ MCP PROTOCOL ============

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: Option<String>,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

// ============ TOOL DEFINITIONS ============

fn get_all_tool_definitions() -> Vec<Value> {
    let mut tools = Vec::new();

    // --- Phase A Meta-tools (prepended — Claude defaults to these) ---
    tools.extend(meta::meta_tool_definitions());

    // --- Browser tools (from browser-mcp lib) ---
    // Tools that support a11y_ref for accessibility-first interaction
    const A11Y_REF_TOOLS: &[&str] = &[
        "click",
        "type",
        "type_text",
        "hover",
        "focus",
        "select",
        "scroll",
    ];

    // Tools that support stealth parameter
    const STEALTH_TOOLS: &[&str] = &["launch", "attach"];

    for t in browser_mcp::tools::list_tools() {
        let mut schema = t.input_schema.clone();
        // Inject a11y_ref parameter into supported tools
        if A11Y_REF_TOOLS.contains(&t.name.as_str()) {
            if let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut()) {
                props.insert("a11y_ref".into(), json!({
                    "type": "string",
                    "description": "Ref ID from the last browser_a11y_snapshot (e.g., 'ref_12'). Preferred over CSS selectors — resolves the element by its accessibility role + name from the cached snapshot."
                }));
            }
        }
        // Inject stealth parameter into launch/attach
        if STEALTH_TOOLS.contains(&t.name.as_str()) {
            if let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut()) {
                props.insert("stealth".into(), json!({
                    "type": "boolean",
                    "default": false,
                    "description": "Enable stealth mode: removes WebDriver indicators, spoofs navigator properties, and applies anti-detection measures to avoid bot detection."
                }));
            }
        }
        tools.push(json!({
            "name": format!("browser_{}", t.name),
            "description": t.description,
            "inputSchema": schema,
        }));
    }

    // --- UIA tools (from uia_lib) ---
    for mut t in uia_lib::get_tool_definitions() {
        uia::augment_tool_definition(&mut t);
        tools.push(t);
    }

    // --- Vision tools (from vision-core) ---
    for t in vision_core::get_all_definitions() {
        tools.push(t);
    }

    // --- Combo tools (new, unique to Hands) ---
    tools.push(json!({
        "name": "find_and_click",
        "description": "\u{26A0} DEPRECATED 2026-05-15 \u{2014} use hands_click (offset_x/offset_y subsume the find_and_click offset behavior; rungs 6+7 cover OCR + UIA fallback). Find text on screen via OCR, then click its location via UIA. Combines vision + UIA in one call. When window_title is provided, focuses that window first. When text isn't found on the visible screen, automatically tries focusing other windows to find it.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Text to find on screen" },
                "button": { "type": "string", "default": "left", "enum": ["left", "right", "middle"] },
                "double_click": { "type": "boolean", "default": false },
                "offset_x": { "type": "integer", "default": 0, "description": "X offset from found text center" },
                "offset_y": { "type": "integer", "default": 0, "description": "Y offset from found text center" },
                "window_title": { "type": "string", "description": "Focus this window first (optional). If text not found on screen, other windows are tried automatically." }
            },
            "required": ["text"]
        }
    }));

    tools.push(json!({
        "name": "read_screen_text",
        "description": "\u{26A0} DEPRECATED 2026-05-15 \u{2014} use hands_capture (accepts optional window_title to focus a named window before OCR) or vision_screenshot_ocr for screen-only OCR. Take a screenshot and return all text via OCR. Optionally target a specific window by title.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "window_title": { "type": "string", "description": "Focus this window first (optional)" },
                "region": {
                    "type": "object",
                    "description": "Crop region {x, y, width, height} (optional)",
                    "properties": {
                        "x": { "type": "integer" },
                        "y": { "type": "integer" },
                        "width": { "type": "integer" },
                        "height": { "type": "integer" }
                    }
                }
            }
        }
    }));

    tools.push(json!({
        "name": "wait_for_visual",
        "description": "Poll the screen until specific text appears (via OCR) or a template image is found. Returns when found or times out.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Text to wait for (OCR)" },
                "template_path": { "type": "string", "description": "Template image path to match (alternative to text)" },
                "timeout_ms": { "type": "integer", "default": 10000 },
                "poll_interval_ms": { "type": "integer", "default": 500 },
                "window_title": { "type": "string", "description": "Focus this window first (optional)" }
            }
        }
    }));

    tools.push(json!({
        "name": "window_screenshot",
        "description": "\u{26A0} PARTIALLY DEPRECATED 2026-05-15 \u{2014} for default (behind=false) capture, prefer `vision_screenshot` (focus the window yourself if needed, then screenshot). For obscured/background windows, prefer the new `vision_screenshot_hidden_window` tool (PrintWindow API, always-on). This combined tool remains registered for backward compatibility. Focus a window by title and take a screenshot of it. Returns OCR text + saves image. With behind=true, captures the window content even if it's obscured by other windows (uses Win32 PrintWindow API).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Window title to match (partial)" },
                "save_path": { "type": "string", "description": "Save screenshot to this path (optional)" },
                "ocr": { "type": "boolean", "default": true, "description": "Run OCR on the screenshot" },
                "behind": { "type": "boolean", "default": false, "description": "If true, capture window content even if obscured by other windows (PrintWindow API). Does NOT bring window to front." }
            },
            "required": ["title"]
        }
    }));

    // New canonical name for the unique PrintWindow path (always behind=true equivalent).
    // The default-mode case (focus + full-screen capture) is redundant with vision_screenshot,
    // so the new tool always uses Win32 PrintWindow and does NOT bring the window to front.
    tools.push(json!({
        "name": "vision_screenshot_hidden_window",
        "description": "Capture a window's content using the Win32 PrintWindow API, even when the window is obscured by other windows or off-screen. Does NOT raise/focus the window. Returns OCR text + saves the image. This is the canonical replacement for `window_screenshot(behind=true)` (renamed 2026-05-15 to surface the unique capability).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Window title to match (case-insensitive substring)" },
                "save_path": { "type": "string", "description": "Save screenshot to this path (optional)" },
                "ocr": { "type": "boolean", "default": true, "description": "Run OCR on the saved image" }
            },
            "required": ["title"]
        }
    }));

    tools.push(json!({
        "name": "type_into_window",
        "description": "\u{26A0} DEPRECATED 2026-05-15 \u{2014} use hands_type (adds focus verification, chunked typing for >100 chars, fast_set, and sensitive-field detection). Focus a window by title, optionally click at coordinates, then type text. Combines focus + click + type in one call.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Window title to focus" },
                "text": { "type": "string", "description": "Text to type" },
                "click_x": { "type": "integer", "description": "Click here first (optional)" },
                "click_y": { "type": "integer", "description": "Click here first (optional)" },
                "delay_ms": { "type": "integer", "default": 100, "description": "Delay after focus before typing" }
            },
            "required": ["title", "text"]
        }
    }));

    tools.push(json!({
        "name": "drag",
        "description": "Mouse drag from one point to another. Useful for moving windows, resizing, selecting text.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "from_x": { "type": "integer" },
                "from_y": { "type": "integer" },
                "to_x": { "type": "integer" },
                "to_y": { "type": "integer" },
                "button": { "type": "string", "default": "left", "enum": ["left", "right"] },
                "duration_ms": { "type": "integer", "default": 300, "description": "How long the drag takes (smooth movement)" }
            },
            "required": ["from_x", "from_y", "to_x", "to_y"]
        }
    }));

    tools.push(json!({
        "name": "uia_window_resize",
        "description": "Resize a window by title substring match.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Window title to match (case-insensitive substring)" },
                "width": { "type": "integer", "description": "New window width in pixels" },
                "height": { "type": "integer", "description": "New window height in pixels" }
            },
            "required": ["title", "width", "height"]
        }
    }));

    tools.push(json!({
        "name": "uia_window_move",
        "description": "Move a window to screen coordinates by title substring match.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Window title to match (case-insensitive substring)" },
                "x": { "type": "integer", "description": "Screen X coordinate for the top-left corner" },
                "y": { "type": "integer", "description": "Screen Y coordinate for the top-left corner" }
            },
            "required": ["title", "x", "y"]
        }
    }));

    tools.push(json!({
        "name": "uia_window_state",
        "description": "Change window state by title match: minimize, maximize, restore, or close.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Window title to match (case-insensitive substring)" },
                "state": {
                    "type": "string",
                    "description": "Window state change to apply",
                    "enum": ["minimize", "maximize", "restore", "close"]
                }
            },
            "required": ["title", "state"]
        }
    }));

    tools.push(json!({
        "name": "uia_window_snap",
        "description": "Snap a window to a screen region by title match.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Window title to match (case-insensitive substring)" },
                "position": {
                    "type": "string",
                    "description": "Target snap region",
                    "enum": ["left", "right", "top-left", "top-right", "center"]
                }
            },
            "required": ["title", "position"]
        }
    }));

    tools.push(json!({
        "name": "uia_app_launch",
        "description": "Launch an app by name using ShellExecuteW with a Start-menu search fallback.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "App name, executable, path, or URI to launch" }
            },
            "required": ["name"]
        }
    }));

    tools.push(json!({
        "name": "browser_a11y_snapshot",
        "description": "Get the current page's accessibility tree - the SEMANTIC structure as seen by assistive technology. Returns roles (button, link, heading, textbox, etc.), accessible names, values, and states (disabled, checked, expanded, required, etc.) in a hierarchical tree format. Each node gets a stable ref ID (e.g., 'ref_0') that can be passed to browser_click, browser_type, browser_hover, browser_focus, browser_select as an a11y_ref parameter for targeted interaction. With incremental=true, returns only what changed since the last snapshot (added, removed, changed nodes). Different from browser_get_html (raw DOM) or browser_get_text (plain text) - this shows what a screen reader sees. Requires browser to be launched/attached first.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "root_selector": {
                    "type": "string",
                    "description": "CSS selector to scope the snapshot to a subtree (optional, default: entire page)"
                },
                "include_ignored": {
                    "type": "boolean",
                    "default": false,
                    "description": "Whether to include nodes ignored by assistive technology (aria-hidden, display:none, etc.)"
                },
                "max_depth": {
                    "type": "integer",
                    "default": 10,
                    "description": "Maximum depth to traverse the tree (default: 10)"
                },
                "incremental": {
                    "type": "boolean",
                    "default": false,
                    "description": "If true, return only the delta (added/removed/changed) since the last snapshot. First call always returns the full tree."
                }
            }
        }
    }));

    tools.push(json!({
        "name": "retry_click",
        "description": "\u{26A0} DEPRECATED 2026-05-15 \u{2014} use browser_click with the new retry / retry_delay_ms options (same behavior, fewer hops). Robust click with automatic retry. If the click fails (element not found, stale, not clickable), waits and retries up to max_attempts times. Supports same args as browser_click (selector, match_text, x, y).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "selector": {"type": "string", "description": "CSS selector"},
                "match_text": {"type": "string", "description": "Fuzzy text match"},
                "x": {"type": "integer"}, "y": {"type": "integer"},
                "max_attempts": {"type": "integer", "description": "Max retry attempts (default: 3)", "default": 3},
                "retry_delay_ms": {"type": "integer", "description": "Delay between retries in ms (default: 500)", "default": 500}
            }
        }
    }));

    tools.push(json!({
        "name": "file_upload",
        "description": "Upload file(s) to an input[type=file] element. Sets files via DataTransfer API and triggers change event. Works in headless mode.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "selector": {"type": "string", "description": "CSS selector for the file input element"},
                "files": {"description": "File path or array of file paths to upload", "oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}]}
            },
            "required": ["selector", "files"]
        }
    }));

    tools.push(json!({
        "name": "status",
        "description": "Get status of all Hands subsystems: browser connection, UIA availability, vision capabilities.",
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    }));

    tools.push(json!({
        "name": "hands_health",
        "description": "Diagnostic health check for the hands server. Returns cpc-paths path resolution status (Volumes, install, backups) plus browser, vision, and UIA subsystem probe results.",
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    }));

    tools.push(json!({
        "name": "hands_summarize_run",
        "description": "Summarize a breadcrumb's run telemetry: tools_used breakdown, total/avg ms, success rate, failure steps, files_changed. Agent uses this to self-optimize ('my last 5 runs averaged 12s, this approach is 4s — adopt it'). Reads from C:/My Drive/Volumes/breadcrumbs/ + hands_meta.jsonl. Pure local-state — no browser/session.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "breadcrumb_id": {
                    "type": "string",
                    "description": "Full id (bc_...), short prefix (bc_TIMESTAMP), or bare timestamp"
                }
            },
            "required": ["breadcrumb_id"]
        }
    }));

    tools.push(json!({
        "name": "vision_cache_stats",
        "description": "Return current vision cache stats (entries, hit_rate, hits, misses, evictions, invalidations, TTL). Optional reset=true zeros all counters and empties the cache. Pure local-state inspection — does not call any vision tool.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "reset": {
                    "type": "boolean",
                    "default": false,
                    "description": "If true, clear all entries and reset stat counters"
                }
            }
        }
    }));

    tools.push(json!({
        "name": "browser_get_performance_log",
        "description": "Get recent network requests from the browser via performance.getEntriesByType('resource'). Returns URL, type, duration, and size for each request. Lightweight — no CDP hooks needed, works with any page. Clears entries after reading so subsequent calls return only new requests.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "max_entries": {
                    "type": "integer",
                    "default": 50,
                    "description": "Maximum number of entries to return (default: 50, most recent first)"
                }
            }
        }
    }));

    tools.push(json!({
        "name": "element_drag",
        "description": "Drag from one CSS-selector element to another, or from a selector to an offset. Resolves element positions via browser get_bounds, then performs a smooth mouse drag. Requires browser to be launched.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "from_selector": {
                    "type": "string",
                    "description": "CSS selector of the element to drag FROM"
                },
                "to_selector": {
                    "type": "string",
                    "description": "CSS selector of the element to drag TO (provide this OR offset_x/offset_y)"
                },
                "offset_x": {
                    "type": "integer",
                    "description": "Horizontal pixel offset from the source element center (alternative to to_selector)"
                },
                "offset_y": {
                    "type": "integer",
                    "description": "Vertical pixel offset from the source element center (alternative to to_selector)"
                },
                "duration_ms": {
                    "type": "integer",
                    "default": 300,
                    "description": "How long the drag takes in ms (smooth movement)"
                }
            },
            "required": ["from_selector"]
        }
    }));

    tools.push(json!({
        "name": "browser_batch",
        "description": "Execute multiple browser actions in sequence. Each action calls the same internal function as the individual tool. Stops on first error unless continue_on_error=true. Returns array of results.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "actions": {
                    "type": "array",
                    "description": "Array of actions to execute sequentially",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": {
                                "type": "string",
                                "enum": ["click", "type_text", "navigate", "wait_idle", "screenshot", "wait_for", "scroll", "a11y_snapshot"],
                                "description": "Action type — maps to the corresponding browser_* tool"
                            },
                            "params": {
                                "type": "object",
                                "description": "Parameters for the action (same as the individual tool)"
                            }
                        },
                        "required": ["type"]
                    }
                },
                "continue_on_error": {
                    "type": "boolean",
                    "default": false,
                    "description": "If true, continue executing remaining actions after an error"
                }
            },
            "required": ["actions"]
        }
    }));

    tools.push(json!({
        "name": "browser_a11y_find",
        "description": "Search the cached accessibility snapshot for elements by role and/or name. Returns matching refs without re-snapshotting. Requires browser_a11y_snapshot to have been called first.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "role": { "type": "string", "description": "ARIA role to find (button, link, textbox, heading, etc.)" },
                "name": { "type": "string", "description": "Accessible name to match (case-insensitive substring)" }
            }
        }
    }));

    tools.push(json!({
        "name": "browser_get_all_network",
        "description": "Get ALL network activity by merging route-based logs (browser_get_network_log) and Performance API logs (browser_get_performance_log). Each entry includes a 'source' field ('route' or 'performance'). Provides the most complete picture of network requests.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "max_entries": {
                    "type": "integer",
                    "default": 100,
                    "description": "Maximum entries per source (default: 100)"
                },
                "clear": {
                    "type": "boolean",
                    "default": true,
                    "description": "Clear route logs after reading (default: true). Performance entries are always cleared."
                }
            }
        }
    }));

    tools.push(json!({
        "name": "browser_learn_api",
        "description": "Analyze captured network traffic to discover API endpoints. Call this AFTER interacting with a page to extract the APIs it uses. Returns structured endpoint patterns (URL, method, headers, body template) that can be stored and replayed via direct HTTP calls.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "filter_pattern": {
                    "type": "string",
                    "description": "Optional regex to filter URLs (e.g., '/api/')"
                },
                "include_static": {
                    "type": "boolean",
                    "default": false,
                    "description": "Include static assets like .js, .css, .png (default: false)"
                },
                "min_response_size": {
                    "type": "integer",
                    "default": 0,
                    "description": "Minimum response body size in bytes (default: 0)"
                }
            }
        }
    }));

    tools.push(json!({
        "name": "uia_batch",
        "description": "Execute multiple UIA (desktop) actions in sequence. Each action calls the same internal function as the individual tool. Stops on first error unless continue_on_error=true. Returns array of results.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "actions": {
                    "type": "array",
                    "description": "Array of actions to execute sequentially",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": {
                                "type": "string",
                                "enum": ["click", "type_text", "key_press", "focus_window", "screenshot", "read_value", "scroll"],
                                "description": "Action type — maps to the corresponding uia_* tool"
                            },
                            "params": {
                                "type": "object",
                                "description": "Parameters for the action (same as the individual tool)"
                            }
                        },
                        "required": ["type"]
                    }
                },
                "continue_on_error": {
                    "type": "boolean",
                    "default": false,
                    "description": "If true, continue executing remaining actions after an error"
                }
            },
            "required": ["actions"]
        }
    }));

    tools
}

// ============ COMBO TOOL HANDLERS ============

async fn handle_find_and_click(args: &Value) -> Value {
    let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let button = args
        .get("button")
        .and_then(|v| v.as_str())
        .unwrap_or("left");
    let double_click = args
        .get("double_click")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let offset_x = args.get("offset_x").and_then(|v| v.as_i64()).unwrap_or(0);
    let offset_y = args.get("offset_y").and_then(|v| v.as_i64()).unwrap_or(0);
    let window_title = args.get("window_title").and_then(|v| v.as_str());

    // If window_title provided, focus that window first
    if let Some(title) = window_title {
        uia_lib::handle_tool_call("uia_focus_window", &json!({"title": title}));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Step 1: Screenshot + OCR
    let ocr_result = meta::vision_cache::cached_execute("vision_screenshot_ocr", &json!({})).await;
    let ocr_text = ocr_result
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let search_lower = text.to_lowercase();
    let mut found_text = !ocr_text.is_empty() && ocr_text.to_lowercase().contains(&search_lower);
    let mut focused_window: Option<String> = window_title.map(|s| s.to_string());

    // If text not found and no explicit window_title was given, try other windows
    if !found_text && window_title.is_none() {
        let win_list = uia_lib::handle_tool_call("uia_list_windows", &json!({}));
        if let Some(windows) = win_list.get("windows").and_then(|v| v.as_array()) {
            // Filter out system/desktop windows
            let skip_classes = [
                "Progman",
                "Shell_TrayWnd",
                "Shell_SecondaryTrayWnd",
                "Windows.UI.Core.CoreWindow",
                "WorkerW",
                "TopLevelWindowForOverflowXamlIsland",
            ];
            for win in windows {
                let class = win.get("class_name").and_then(|v| v.as_str()).unwrap_or("");
                let title_val = win.get("title").and_then(|v| v.as_str()).unwrap_or("");
                if title_val.is_empty() || skip_classes.contains(&class) {
                    continue;
                }
                // Try focusing this window
                uia_lib::handle_tool_call("uia_focus_window", &json!({"title": title_val}));
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                let retry_ocr =
                    meta::vision_cache::cached_execute("vision_screenshot_ocr", &json!({})).await;
                let retry_text = retry_ocr.get("text").and_then(|v| v.as_str()).unwrap_or("");
                if !retry_text.is_empty() && retry_text.to_lowercase().contains(&search_lower) {
                    found_text = true;
                    focused_window = Some(title_val.to_string());
                    break;
                }
            }
        }
    }

    if !found_text {
        return json!({
            "success": false,
            "error": format!("Text '{}' not found on screen (tried all visible windows)", text),
            "ocr_text_snippet": &ocr_text[..ocr_text.len().min(500)]
        });
    }

    // Step 2: Use UIA to find the element and click it
    let find_result = uia_lib::handle_tool_call(
        "uia_find_element",
        &json!({
            "name": text,
            "max_depth": 8
        }),
    );

    if let Some(elements) = find_result.get("elements").and_then(|v| v.as_array()) {
        if let Some(first) = elements.first() {
            if let (Some(cx), Some(cy)) = (
                first
                    .get("center")
                    .and_then(|c| c.get("x"))
                    .and_then(|v| v.as_i64()),
                first
                    .get("center")
                    .and_then(|c| c.get("y"))
                    .and_then(|v| v.as_i64()),
            ) {
                let click_x = cx + offset_x;
                let click_y = cy + offset_y;
                let click_result = uia_lib::handle_tool_call(
                    "uia_click",
                    &json!({
                        "x": click_x,
                        "y": click_y,
                        "button": button,
                        "double_click": double_click
                    }),
                );
                let mut result = json!({
                    "success": true,
                    "found_via": "uia",
                    "method": "uia",
                    "text": text,
                    "clicked": {"x": click_x, "y": click_y},
                    "element": first,
                    "click_result": click_result
                });
                if let Some(ref win) = focused_window {
                    result["focused_window"] = json!(win);
                }
                return result;
            }
        }
    }

    // Fallback: text was in OCR but not in UIA tree — click at OCR coordinates via SendInput
    let screenshot_path = vision_core::take_screenshot(None, 0, 80).unwrap_or_default();
    if !screenshot_path.is_empty() {
        if let Ok(words) = vision_core::ocr_image_with_positions(&screenshot_path).await {
            std::fs::remove_file(&screenshot_path).ok();
            // Find matching word(s) — concatenate consecutive words for multi-word search
            let search_lower = text.to_lowercase();
            let mut match_x = None;
            let mut match_y = None;

            // Try single-word match first
            for (word_text, x, y, w, h) in &words {
                if word_text.to_lowercase().contains(&search_lower)
                    || search_lower.contains(&word_text.to_lowercase())
                {
                    match_x = Some((*x + w / 2.0) as i64 + offset_x);
                    match_y = Some((*y + h / 2.0) as i64 + offset_y);
                    break;
                }
            }

            // If no single word matched, try consecutive word sequences
            if match_x.is_none() {
                for i in 0..words.len() {
                    let mut combined = words[i].0.clone();
                    let start_x = words[i].1;
                    let start_y = words[i].2;
                    let start_h = words[i].4;
                    for j in (i + 1)..words.len().min(i + 6) {
                        combined.push(' ');
                        combined.push_str(&words[j].0);
                        if combined.to_lowercase().contains(&search_lower) {
                            // Click at center of the span
                            let end_x = words[j].1 + words[j].3;
                            match_x = Some(((start_x + end_x) / 2.0) as i64 + offset_x);
                            match_y = Some((start_y + start_h / 2.0) as i64 + offset_y);
                            break;
                        }
                    }
                    if match_x.is_some() {
                        break;
                    }
                }
            }

            if let (Some(cx), Some(cy)) = (match_x, match_y) {
                let click_result = uia_lib::handle_tool_call(
                    "uia_click",
                    &json!({
                        "x": cx,
                        "y": cy,
                        "button": button,
                        "double_click": double_click
                    }),
                );
                let mut result = json!({
                    "success": true,
                    "found_via": "ocr_coordinates",
                    "method": "ocr_coordinates",
                    "text": text,
                    "clicked": {"x": cx, "y": cy},
                    "click_result": click_result
                });
                if let Some(ref win) = focused_window {
                    result["focused_window"] = json!(win);
                }
                return result;
            }
        } else {
            std::fs::remove_file(&screenshot_path).ok();
        }
    }

    // Final fallback: couldn't resolve coordinates either
    let mut result = json!({
        "success": false,
        "error": "Text found via OCR but couldn't resolve click coordinates. Try using coordinates directly.",
        "text_found_in_ocr": true
    });
    if let Some(ref win) = focused_window {
        result["focused_window"] = json!(win);
    }
    result
}

async fn handle_read_screen_text(args: &Value) -> Value {
    // Optionally focus window first
    if let Some(title) = args.get("window_title").and_then(|v| v.as_str()) {
        uia_lib::handle_tool_call("uia_focus_window", &json!({"title": title}));
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    meta::vision_cache::cached_execute("vision_screenshot_ocr", &json!({})).await
}

async fn handle_wait_for_visual(args: &Value) -> Value {
    let text = args.get("text").and_then(|v| v.as_str());
    let template = args.get("template_path").and_then(|v| v.as_str());
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(10000);
    let poll_ms = args
        .get("poll_interval_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(500);

    if text.is_none() && template.is_none() {
        return json!({"success": false, "error": "Provide 'text' or 'template_path'"});
    }

    // Focus window if specified
    if let Some(title) = args.get("window_title").and_then(|v| v.as_str()) {
        uia_lib::handle_tool_call("uia_focus_window", &json!({"title": title}));
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(timeout_ms);
    let poll = std::time::Duration::from_millis(poll_ms);

    loop {
        if start.elapsed() > timeout {
            return json!({
                "success": false,
                "error": "Timeout waiting for visual match",
                "elapsed_ms": start.elapsed().as_millis()
            });
        }

        if let Some(search_text) = text {
            let ocr = meta::vision_cache::cached_execute("vision_screenshot_ocr", &json!({})).await;
            if let Some(ocr_text) = ocr.get("text").and_then(|v| v.as_str()) {
                if ocr_text
                    .to_lowercase()
                    .contains(&search_text.to_lowercase())
                {
                    return json!({
                        "success": true,
                        "found": "text",
                        "text": search_text,
                        "elapsed_ms": start.elapsed().as_millis(),
                        "ocr_text": ocr_text
                    });
                }
            }
        }

        if let Some(tmpl) = template {
            let result = vision_core::execute(
                "vision_find_template",
                &json!({
                    "template_path": tmpl
                }),
            )
            .await;
            if result
                .get("found")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return json!({
                    "success": true,
                    "found": "template",
                    "template": tmpl,
                    "elapsed_ms": start.elapsed().as_millis(),
                    "match_result": result
                });
            }
        }

        tokio::time::sleep(poll).await;
    }
}

async fn handle_window_screenshot(args: &Value) -> Value {
    let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let save_path = args.get("save_path").and_then(|v| v.as_str());
    let do_ocr = args.get("ocr").and_then(|v| v.as_bool()).unwrap_or(true);
    let behind = args
        .get("behind")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if behind {
        // PrintWindow path: capture window content even if obscured
        return handle_window_screenshot_behind(title, save_path, do_ocr).await;
    }

    // Original path: focus window, then screen-capture
    let focus_result = uia_lib::handle_tool_call("uia_focus_window", &json!({"title": title}));
    if !focus_result
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return json!({
            "success": false,
            "error": format!("Could not focus window: {}", title),
            "focus_result": focus_result
        });
    }

    // Brief delay for window to come to front
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    if do_ocr {
        let mut ocr_args = json!({});
        if let Some(path) = save_path {
            ocr_args
                .as_object_mut()
                .unwrap()
                .insert("save_path".to_string(), json!(path));
        }
        let result = meta::vision_cache::cached_execute("vision_screenshot_ocr", &ocr_args).await;
        json!({
            "success": true,
            "window": title,
            "focused": focus_result.get("focused"),
            "ocr_result": result
        })
    } else {
        let mut ss_args = json!({});
        if let Some(path) = save_path {
            ss_args
                .as_object_mut()
                .unwrap()
                .insert("save_path".to_string(), json!(path));
        }
        let result = vision_core::execute("vision_screenshot", &ss_args).await;
        json!({
            "success": true,
            "window": title,
            "focused": focus_result.get("focused"),
            "screenshot_result": result
        })
    }
}

/// Capture a window's content using PrintWindow API — works even if the window is behind others.
/// Does NOT focus/raise the window.
#[cfg(windows)]
async fn handle_window_screenshot_behind(
    title: &str,
    save_path: Option<&str>,
    do_ocr: bool,
) -> Value {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClientRect, GetWindowTextW, IsWindowVisible,
    };

    // PW_RENDERFULLCONTENT = 0x00000002, captures DWM-composed content
    const PW_RENDERFULLCONTENT: u32 = 2;

    // Step 1: Find the HWND by title substring match
    struct SearchCtx {
        filter: String,
        found_hwnd: Option<HWND>,
        found_title: String,
    }

    let mut ctx = SearchCtx {
        filter: title.to_lowercase(),
        found_hwnd: None,
        found_title: String::new(),
    };

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut SearchCtx);
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut buf);
        if len > 0 {
            let win_title = String::from_utf16_lossy(&buf[..len as usize]);
            if win_title.to_lowercase().contains(&ctx.filter) {
                ctx.found_hwnd = Some(hwnd);
                ctx.found_title = win_title;
                return BOOL(0); // stop enumeration
            }
        }
        BOOL(1)
    }

    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut ctx as *mut SearchCtx as isize));
    }

    let hwnd = match ctx.found_hwnd {
        Some(h) => h,
        None => {
            return json!({
                "success": false,
                "error": format!("No window found matching '{}'", title)
            });
        }
    };

    // Step 2: Get window client dimensions
    let mut rect = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;

    if width <= 0 || height <= 0 {
        return json!({
            "success": false,
            "error": format!("Window '{}' has zero size ({}x{})", ctx.found_title, width, height)
        });
    }

    // Step 3: Create compatible DC and bitmap
    let (pixels_bgra, w, h) = unsafe {
        let hdc_window = GetDC(hwnd);
        let hdc_mem = CreateCompatibleDC(hdc_window);
        let hbm = CreateCompatibleBitmap(hdc_window, width, height);
        let old = SelectObject(hdc_mem, hbm);

        // Step 4: PrintWindow into our memory DC
        let pw_result = PrintWindow(hwnd, hdc_mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT));
        if !pw_result.as_bool() {
            // Fallback: try without PW_RENDERFULLCONTENT
            let _ = PrintWindow(hwnd, hdc_mem, PRINT_WINDOW_FLAGS(0));
        }

        // Step 5: Extract pixel data via GetDIBits
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // negative = top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..std::mem::zeroed()
        };

        let mut buf = vec![0u8; (width * height * 4) as usize];
        GetDIBits(
            hdc_mem,
            hbm,
            0,
            height as u32,
            Some(buf.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        // Cleanup GDI objects
        SelectObject(hdc_mem, old);
        let _ = DeleteObject(hbm);
        let _ = DeleteDC(hdc_mem);
        let _ = ReleaseDC(hwnd, hdc_window);

        (buf, width as u32, height as u32)
    };

    // Step 6: Convert BGRA to RGBA
    let mut rgba_pixels = pixels_bgra;
    for chunk in rgba_pixels.chunks_exact_mut(4) {
        chunk.swap(0, 2); // B <-> R
    }

    // Step 7: Create image and save
    let img = match image::RgbaImage::from_raw(w, h, rgba_pixels) {
        Some(img) => img,
        None => {
            return json!({
                "success": false,
                "error": "Failed to create image from pixel data"
            });
        }
    };

    let output_path = match save_path {
        Some(p) => p.to_string(),
        None => {
            let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
            format!("C:\\temp\\window_behind_{}.png", ts)
        }
    };

    // Ensure parent dir exists
    if let Some(parent) = std::path::Path::new(&output_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if let Err(e) = vision_core::save_image(&img, &output_path, 90) {
        return json!({
            "success": false,
            "error": format!("Failed to save image: {}", e)
        });
    }

    // Step 8: Optionally run OCR on the saved image
    if do_ocr {
        let ocr_result =
            meta::vision_cache::cached_execute("vision_ocr", &json!({"image_path": output_path}))
                .await;
        json!({
            "success": true,
            "window": ctx.found_title,
            "behind": true,
            "image_path": output_path,
            "width": w,
            "height": h,
            "ocr_result": ocr_result
        })
    } else {
        json!({
            "success": true,
            "window": ctx.found_title,
            "behind": true,
            "image_path": output_path,
            "width": w,
            "height": h
        })
    }
}

#[cfg(not(windows))]
async fn handle_window_screenshot_behind(
    _title: &str,
    _save_path: Option<&str>,
    _do_ocr: bool,
) -> Value {
    json!({
        "success": false,
        "error": "PrintWindow capture only available on Windows"
    })
}

#[cfg(windows)]
fn get_required_i32(args: &Value, key: &str) -> Result<i32, Value> {
    match args.get(key).and_then(|value| value.as_i64()) {
        Some(value) => i32::try_from(value).map_err(|_| {
            json!({
                "success": false,
                "error": format!("{} must fit in a 32-bit signed integer", key)
            })
        }),
        None => Err(json!({
            "success": false,
            "error": format!("{} parameter required", key)
        })),
    }
}

#[cfg(windows)]
fn get_required_positive_i32(args: &Value, key: &str) -> Result<i32, Value> {
    let value = get_required_i32(args, key)?;
    if value <= 0 {
        return Err(json!({
            "success": false,
            "error": format!("{} must be greater than zero", key)
        }));
    }
    Ok(value)
}

#[cfg(windows)]
fn encode_wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn parse_hwnd_debug_string(hwnd_str: &str) -> Option<HWND> {
    let hex = hwnd_str
        .strip_prefix("HWND(")?
        .strip_suffix(')')?
        .trim_start_matches("0x");
    let value = usize::from_str_radix(hex, 16).ok()?;
    Some(HWND(value as *mut _))
}

#[cfg(windows)]
fn find_window_by_title(title: &str) -> Result<(Value, HWND), Value> {
    let filter = title.trim();
    if filter.is_empty() {
        return Err(json!({
            "success": false,
            "error": "title parameter required"
        }));
    }

    let windows_result = uia_lib::handle_tool_call("uia_list_window", &json!({}));
    let windows = match windows_result
        .get("windows")
        .and_then(|value| value.as_array())
    {
        Some(windows) => windows,
        None => {
            return Err(json!({
                "success": false,
                "error": "UIA window list unavailable",
                "details": windows_result
            }))
        }
    };

    let filter_lower = filter.to_lowercase();
    if let Some(window) = windows.iter().find(|window| {
        window
            .get("title")
            .and_then(|value| value.as_str())
            .map(|value| value.to_lowercase().contains(&filter_lower))
            .unwrap_or(false)
    }) {
        let hwnd_str = window
            .get("hwnd")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        return match parse_hwnd_debug_string(hwnd_str) {
            Some(hwnd) => Ok((window.clone(), hwnd)),
            None => Err(json!({
                "success": false,
                "error": format!("Could not parse window handle for '{}'", filter),
                "window": window
            })),
        };
    }

    Err(json!({
        "success": false,
        "error": format!("No window found matching '{}'", filter),
        "available": windows
            .iter()
            .filter_map(|window| window.get("title").and_then(|value| value.as_str()).map(str::to_owned))
            .collect::<Vec<_>>()
    }))
}

#[cfg(windows)]
fn current_window_rect(hwnd: HWND) -> Result<RECT, Value> {
    let mut rect = RECT::default();
    unsafe {
        GetWindowRect(hwnd, &mut rect).map_err(|error| {
            json!({
                "success": false,
                "error": format!("GetWindowRect failed: {}", error)
            })
        })?;
    }
    Ok(rect)
}

#[cfg(windows)]
fn restore_window(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
}

#[cfg(windows)]
fn set_window_geometry(
    hwnd: HWND,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    flags: SET_WINDOW_POS_FLAGS,
    action: &str,
) -> Result<(), Value> {
    unsafe {
        SetWindowPos(hwnd, HWND(std::ptr::null_mut()), x, y, width, height, flags).map_err(
            |error| {
                json!({
                    "success": false,
                    "error": format!("{} failed: {}", action, error)
                })
            },
        )?;
    }
    Ok(())
}

#[cfg(windows)]
fn monitor_work_area(hwnd: HWND) -> Result<RECT, Value> {
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.0.is_null() {
        return Err(json!({
            "success": false,
            "error": "No monitor found for the target window"
        }));
    }

    let mut monitor_info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut monitor_info) }.as_bool() {
        return Err(json!({
            "success": false,
            "error": "GetMonitorInfoW failed"
        }));
    }
    Ok(monitor_info.rcWork)
}

#[cfg(windows)]
fn send_virtual_key(key: VIRTUAL_KEY) {
    unsafe {
        let inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: key,
                        wScan: 0,
                        dwFlags: KEYBD_EVENT_FLAGS(0),
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: key,
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
        ];
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
fn handle_uia_window_resize(args: &Value) -> Value {
    let title = args
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let width = match get_required_positive_i32(args, "width") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let height = match get_required_positive_i32(args, "height") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let (window, hwnd) = match find_window_by_title(title) {
        Ok(window) => window,
        Err(error) => return error,
    };

    restore_window(hwnd);
    let rect = match current_window_rect(hwnd) {
        Ok(rect) => rect,
        Err(error) => return error,
    };
    if let Err(error) = set_window_geometry(
        hwnd,
        0,
        0,
        width,
        height,
        SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE | SWP_SHOWWINDOW,
        "Window resize",
    ) {
        return error;
    }

    json!({
        "success": true,
        "window": window.get("title").cloned().unwrap_or_else(|| json!(title)),
        "hwnd": window.get("hwnd").cloned().unwrap_or_else(|| json!("")),
        "x": rect.left,
        "y": rect.top,
        "width": width,
        "height": height
    })
}

#[cfg(windows)]
fn handle_uia_window_move(args: &Value) -> Value {
    let title = args
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let x = match get_required_i32(args, "x") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let y = match get_required_i32(args, "y") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let (window, hwnd) = match find_window_by_title(title) {
        Ok(window) => window,
        Err(error) => return error,
    };

    restore_window(hwnd);
    if let Err(error) = set_window_geometry(
        hwnd,
        x,
        y,
        0,
        0,
        SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSIZE | SWP_SHOWWINDOW,
        "Window move",
    ) {
        return error;
    }

    let rect = match current_window_rect(hwnd) {
        Ok(rect) => rect,
        Err(error) => return error,
    };
    json!({
        "success": true,
        "window": window.get("title").cloned().unwrap_or_else(|| json!(title)),
        "hwnd": window.get("hwnd").cloned().unwrap_or_else(|| json!("")),
        "x": rect.left,
        "y": rect.top,
        "width": rect.right - rect.left,
        "height": rect.bottom - rect.top
    })
}

#[cfg(windows)]
fn handle_uia_window_state(args: &Value) -> Value {
    let title = args
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let state = match args.get("state").and_then(|value| value.as_str()) {
        Some(value) => value.trim().to_lowercase(),
        None => {
            return json!({
                "success": false,
                "error": "state parameter required"
            })
        }
    };
    let (window, hwnd) = match find_window_by_title(title) {
        Ok(window) => window,
        Err(error) => return error,
    };

    match state.as_str() {
        "minimize" => unsafe {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
        },
        "maximize" => unsafe {
            let _ = ShowWindow(hwnd, SW_MAXIMIZE);
        },
        "restore" => restore_window(hwnd),
        "close" => {
            if let Err(error) = unsafe { PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)) } {
                return json!({
                    "success": false,
                    "error": format!("Window close failed: {}", error)
                });
            }
        }
        _ => {
            return json!({
                "success": false,
                "error": "state must be one of: minimize, maximize, restore, close"
            })
        }
    }

    json!({
        "success": true,
        "window": window.get("title").cloned().unwrap_or_else(|| json!(title)),
        "hwnd": window.get("hwnd").cloned().unwrap_or_else(|| json!("")),
        "state": state
    })
}

#[cfg(windows)]
fn handle_uia_window_snap(args: &Value) -> Value {
    let title = args
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let position = match args.get("position").and_then(|value| value.as_str()) {
        Some(value) => value.trim().to_lowercase(),
        None => {
            return json!({
                "success": false,
                "error": "position parameter required"
            })
        }
    };
    let (window, hwnd) = match find_window_by_title(title) {
        Ok(window) => window,
        Err(error) => return error,
    };

    restore_window(hwnd);
    let work_area = match monitor_work_area(hwnd) {
        Ok(rect) => rect,
        Err(error) => return error,
    };
    let work_width = (work_area.right - work_area.left).max(1);
    let work_height = (work_area.bottom - work_area.top).max(1);
    let half_width = work_width / 2;
    let half_height = work_height / 2;

    let (x, y, width, height) = match position.as_str() {
        "left" => (work_area.left, work_area.top, half_width, work_height),
        "right" => (
            work_area.left + half_width,
            work_area.top,
            work_width - half_width,
            work_height,
        ),
        "top-left" => (work_area.left, work_area.top, half_width, half_height),
        "top-right" => (
            work_area.left + half_width,
            work_area.top,
            work_width - half_width,
            half_height,
        ),
        "center" => {
            let width = ((work_width * 3) / 5).max(320).min(work_width);
            let height = ((work_height * 3) / 5).max(240).min(work_height);
            (
                work_area.left + (work_width - width) / 2,
                work_area.top + (work_height - height) / 2,
                width,
                height,
            )
        }
        _ => {
            return json!({
                "success": false,
                "error": "position must be one of: left, right, top-left, top-right, center"
            })
        }
    };

    if let Err(error) = set_window_geometry(
        hwnd,
        x,
        y,
        width,
        height,
        SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        "Window snap",
    ) {
        return error;
    }

    json!({
        "success": true,
        "window": window.get("title").cloned().unwrap_or_else(|| json!(title)),
        "hwnd": window.get("hwnd").cloned().unwrap_or_else(|| json!("")),
        "position": position,
        "rect": {
            "x": x,
            "y": y,
            "width": width,
            "height": height
        }
    })
}

#[cfg(windows)]
fn handle_uia_app_launch(args: &Value) -> Value {
    let name = match args.get("name").and_then(|value| value.as_str()) {
        Some(value) if !value.trim().is_empty() => value.trim(),
        _ => {
            return json!({
                "success": false,
                "error": "name parameter required"
            })
        }
    };

    let operation = encode_wide_null("open");
    let target = encode_wide_null(name);
    let shell_result = unsafe {
        ShellExecuteW(
            HWND(std::ptr::null_mut()),
            PCWSTR(operation.as_ptr()),
            PCWSTR(target.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if (shell_result.0 as isize) > 32 {
        return json!({
            "success": true,
            "name": name,
            "method": "ShellExecuteW"
        });
    }

    send_virtual_key(VK_LWIN);
    std::thread::sleep(std::time::Duration::from_millis(250));
    let type_result = uia_lib::handle_tool_call("uia_type_text", &json!({ "text": name }));
    std::thread::sleep(std::time::Duration::from_millis(100));
    send_virtual_key(VK_RETURN);

    json!({
        "success": true,
        "name": name,
        "method": "start_search_fallback",
        "shell_execute_code": shell_result.0 as usize,
        "type_result": type_result
    })
}

#[cfg(not(windows))]
fn handle_uia_window_resize(_args: &Value) -> Value {
    json!({"success": false, "error": "Window resize only available on Windows"})
}

#[cfg(not(windows))]
fn handle_uia_window_move(_args: &Value) -> Value {
    json!({"success": false, "error": "Window move only available on Windows"})
}

#[cfg(not(windows))]
fn handle_uia_window_state(_args: &Value) -> Value {
    json!({"success": false, "error": "Window state changes only available on Windows"})
}

#[cfg(not(windows))]
fn handle_uia_window_snap(_args: &Value) -> Value {
    json!({"success": false, "error": "Window snapping only available on Windows"})
}

#[cfg(not(windows))]
fn handle_uia_app_launch(_args: &Value) -> Value {
    json!({"success": false, "error": "App launch only available on Windows"})
}

fn handle_type_into_window(args: &Value) -> Value {
    let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let delay = args.get("delay_ms").and_then(|v| v.as_u64()).unwrap_or(100);

    // Focus
    let focus_result = uia_lib::handle_tool_call("uia_focus_window", &json!({"title": title}));
    if !focus_result
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return json!({
            "success": false,
            "error": format!("Could not focus window: {}", title),
            "focus_result": focus_result
        });
    }

    std::thread::sleep(std::time::Duration::from_millis(delay));

    // Optional click
    if let (Some(x), Some(y)) = (
        args.get("click_x").and_then(|v| v.as_i64()),
        args.get("click_y").and_then(|v| v.as_i64()),
    ) {
        uia_lib::handle_tool_call("uia_click", &json!({"x": x, "y": y}));
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Type
    let type_result = uia_lib::handle_tool_call("uia_type_text", &json!({"text": text}));

    json!({
        "success": true,
        "window": title,
        "typed": text,
        "type_result": type_result
    })
}

fn handle_drag(args: &Value) -> Value {
    let from_x = args.get("from_x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let from_y = args.get("from_y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let to_x = args.get("to_x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let to_y = args.get("to_y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let duration_ms = args
        .get("duration_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(300);

    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;

        unsafe {
            // Move to start
            let _ = SetCursorPos(from_x, from_y);
            std::thread::sleep(std::time::Duration::from_millis(50));

            // Mouse down
            let down = [windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_LEFTDOWN,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            }];
            SendInput(&down, std::mem::size_of::<INPUT>() as i32);

            // Smooth move
            let steps = (duration_ms / 16).max(1); // ~60fps
            for i in 1..=steps {
                let t = i as f64 / steps as f64;
                let cx = from_x as f64 + (to_x - from_x) as f64 * t;
                let cy = from_y as f64 + (to_y - from_y) as f64 * t;
                let _ = SetCursorPos(cx as i32, cy as i32);
                std::thread::sleep(std::time::Duration::from_millis(16));
            }

            // Mouse up
            let up = [INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_LEFTUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            }];
            SendInput(&up, std::mem::size_of::<INPUT>() as i32);
        }

        json!({
            "success": true,
            "from": {"x": from_x, "y": from_y},
            "to": {"x": to_x, "y": to_y},
            "duration_ms": duration_ms
        })
    }

    #[cfg(not(windows))]
    json!({"success": false, "error": "Drag only available on Windows"})
}

pub(crate) async fn handle_accessibility_snapshot(
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
) -> Value {
    let root_selector = args
        .get("root_selector")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let include_ignored = args
        .get("include_ignored")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let max_depth = args.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(10);
    let incremental = args
        .get("incremental")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // JavaScript that walks the DOM and builds an accessibility tree.
    // Uses HTML semantics + ARIA attributes to determine roles, names, and states.
    let script = format!(
        r#"
    (() => {{
        const MAX_DEPTH = {max_depth};
        const INCLUDE_IGNORED = {include_ignored};
        const ROOT_SELECTOR = "{root_selector_escaped}";

        // Map HTML tags to implicit ARIA roles
        const TAG_ROLE_MAP = {{
            'A': (el) => el.hasAttribute('href') ? 'link' : null,
            'ARTICLE': () => 'article',
            'ASIDE': () => 'complementary',
            'BUTTON': () => 'button',
            'DATALIST': () => 'listbox',
            'DETAILS': () => 'group',
            'DIALOG': () => 'dialog',
            'FIELDSET': () => 'group',
            'FIGURE': () => 'figure',
            'FOOTER': (el) => isLandmark(el) ? 'contentinfo' : null,
            'FORM': () => 'form',
            'H1': () => 'heading',
            'H2': () => 'heading',
            'H3': () => 'heading',
            'H4': () => 'heading',
            'H5': () => 'heading',
            'H6': () => 'heading',
            'HEADER': (el) => isLandmark(el) ? 'banner' : null,
            'HR': () => 'separator',
            'IMG': () => 'img',
            'INPUT': (el) => {{
                const t = (el.type || 'text').toLowerCase();
                const map = {{
                    'button': 'button', 'submit': 'button', 'reset': 'button', 'image': 'button',
                    'checkbox': 'checkbox', 'radio': 'radio', 'range': 'slider',
                    'search': 'searchbox', 'email': 'textbox', 'tel': 'textbox',
                    'text': 'textbox', 'url': 'textbox', 'password': 'textbox',
                    'number': 'spinbutton', 'hidden': null
                }};
                return map[t] !== undefined ? map[t] : 'textbox';
            }},
            'LI': () => 'listitem',
            'MAIN': () => 'main',
            'MATH': () => 'math',
            'MENU': () => 'list',
            'METER': () => 'meter',
            'NAV': () => 'navigation',
            'OL': () => 'list',
            'OPTGROUP': () => 'group',
            'OPTION': () => 'option',
            'OUTPUT': () => 'status',
            'P': () => 'paragraph',
            'PROGRESS': () => 'progressbar',
            'SECTION': (el) => el.hasAttribute('aria-label') || el.hasAttribute('aria-labelledby') ? 'region' : null,
            'SELECT': (el) => el.multiple ? 'listbox' : 'combobox',
            'SUMMARY': () => 'button',
            'TABLE': () => 'table',
            'TBODY': () => 'rowgroup',
            'TD': () => 'cell',
            'TEXTAREA': () => 'textbox',
            'TFOOT': () => 'rowgroup',
            'TH': () => 'columnheader',
            'THEAD': () => 'rowgroup',
            'TR': () => 'row',
            'UL': () => 'list',
        }};

        function isLandmark(el) {{
            // header/footer are landmarks only when not inside article/section/etc.
            let p = el.parentElement;
            while (p) {{
                const t = p.tagName;
                if (t === 'ARTICLE' || t === 'ASIDE' || t === 'MAIN' || t === 'NAV' || t === 'SECTION') return false;
                p = p.parentElement;
            }}
            return true;
        }}

        function getRole(el) {{
            // Explicit ARIA role takes precedence
            const explicit = el.getAttribute('role');
            if (explicit) return explicit.split(' ')[0].trim();
            // Implicit role from tag
            const fn_ = TAG_ROLE_MAP[el.tagName];
            return fn_ ? fn_(el) : null;
        }}

        function getAccessibleName(el) {{
            // aria-labelledby
            const labelledBy = el.getAttribute('aria-labelledby');
            if (labelledBy) {{
                const parts = labelledBy.split(/\s+/).map(id => {{
                    const ref_ = document.getElementById(id);
                    return ref_ ? ref_.textContent.trim() : '';
                }}).filter(Boolean);
                if (parts.length) return parts.join(' ');
            }}
            // aria-label
            const ariaLabel = el.getAttribute('aria-label');
            if (ariaLabel) return ariaLabel;
            // For inputs: associated label
            if (el.tagName === 'INPUT' || el.tagName === 'SELECT' || el.tagName === 'TEXTAREA') {{
                if (el.id) {{
                    const label = document.querySelector('label[for="' + CSS.escape(el.id) + '"]');
                    if (label) return label.textContent.trim();
                }}
                const placeholder = el.getAttribute('placeholder');
                if (placeholder) return placeholder;
                const title = el.getAttribute('title');
                if (title) return title;
            }}
            // For images: alt text
            if (el.tagName === 'IMG') {{
                const alt = el.getAttribute('alt');
                if (alt !== null) return alt;
                const title = el.getAttribute('title');
                if (title) return title;
                return '';
            }}
            // For links and buttons: text content
            if (el.tagName === 'A' || el.tagName === 'BUTTON' || el.tagName === 'SUMMARY') {{
                return el.textContent.trim().slice(0, 200);
            }}
            // For headings
            if (/^H[1-6]$/.test(el.tagName)) {{
                return el.textContent.trim().slice(0, 200);
            }}
            // title attribute as fallback
            const title = el.getAttribute('title');
            if (title) return title;
            return '';
        }}

        function getStates(el) {{
            const states = [];
            if (el.disabled || el.getAttribute('aria-disabled') === 'true') states.push('disabled');
            if (el.checked || el.getAttribute('aria-checked') === 'true') states.push('checked');
            if (el.getAttribute('aria-checked') === 'mixed') states.push('mixed');
            if (el.required || el.getAttribute('aria-required') === 'true') states.push('required');
            if (el.readOnly || el.getAttribute('aria-readonly') === 'true') states.push('readonly');
            if (el.getAttribute('aria-expanded') === 'true') states.push('expanded');
            if (el.getAttribute('aria-expanded') === 'false') states.push('collapsed');
            if (el.getAttribute('aria-selected') === 'true') states.push('selected');
            if (el.getAttribute('aria-pressed') === 'true') states.push('pressed');
            if (el.getAttribute('aria-hidden') === 'true') states.push('hidden');
            if (el.getAttribute('aria-invalid') === 'true') states.push('invalid');
            if (el.getAttribute('aria-busy') === 'true') states.push('busy');
            if (el.getAttribute('aria-modal') === 'true') states.push('modal');
            if (el.getAttribute('aria-current')) states.push('current=' + el.getAttribute('aria-current'));
            return states;
        }}

        function getProperties(el) {{
            const props = {{}};
            // Heading level
            if (/^H([1-6])$/.test(el.tagName)) props.level = parseInt(RegExp.$1);
            const ariaLevel = el.getAttribute('aria-level');
            if (ariaLevel) props.level = parseInt(ariaLevel);
            // Value info
            if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') {{
                const t = (el.type || 'text').toLowerCase();
                if (t === 'password') props.type = 'password';
                if (el.value && t !== 'password') props.value = el.value.slice(0, 100);
            }}
            if (el.tagName === 'SELECT' && el.selectedIndex >= 0) {{
                props.value = el.options[el.selectedIndex]?.text?.slice(0, 100);
            }}
            if (el.tagName === 'METER' || el.tagName === 'PROGRESS') {{
                if (el.value !== undefined) props.value = el.value;
                if (el.max !== undefined) props.max = el.max;
                if (el.min !== undefined) props.min = el.min;
            }}
            if (el.getAttribute('aria-valuemin')) props.valuemin = el.getAttribute('aria-valuemin');
            if (el.getAttribute('aria-valuemax')) props.valuemax = el.getAttribute('aria-valuemax');
            if (el.getAttribute('aria-valuenow')) props.valuenow = el.getAttribute('aria-valuenow');
            if (el.getAttribute('aria-valuetext')) props.valuetext = el.getAttribute('aria-valuetext');
            // Link href
            if (el.tagName === 'A' && el.hasAttribute('href')) {{
                let href = el.getAttribute('href');
                if (href && href.length > 200) href = href.slice(0, 200) + '...';
                props.href = href;
            }}
            // Description
            const describedBy = el.getAttribute('aria-describedby');
            if (describedBy) {{
                const desc = describedBy.split(/\s+/).map(id => {{
                    const ref_ = document.getElementById(id);
                    return ref_ ? ref_.textContent.trim() : '';
                }}).filter(Boolean).join(' ');
                if (desc) props.description = desc.slice(0, 200);
            }}
            // Live region
            const live = el.getAttribute('aria-live');
            if (live && live !== 'off') props.live = live;
            // Autocomplete
            const autocomplete = el.getAttribute('aria-autocomplete') || el.getAttribute('autocomplete');
            if (autocomplete && autocomplete !== 'off') props.autocomplete = autocomplete;
            // Orientation
            const orientation = el.getAttribute('aria-orientation');
            if (orientation) props.orientation = orientation;
            return props;
        }}

        function isIgnored(el) {{
            if (el.nodeType !== 1) return true;
            const style = window.getComputedStyle(el);
            if (style.display === 'none') return true;
            if (style.visibility === 'hidden' && style.position !== 'absolute') return true;
            if (el.getAttribute('aria-hidden') === 'true') return true;
            if (el.tagName === 'SCRIPT' || el.tagName === 'STYLE' || el.tagName === 'NOSCRIPT' ||
                el.tagName === 'TEMPLATE' || el.tagName === 'HEAD' || el.tagName === 'META' ||
                el.tagName === 'LINK' || el.tagName === 'BR') return true;
            return false;
        }}

        function buildNode(el, depth) {{
            if (depth > MAX_DEPTH) return null;
            if (!INCLUDE_IGNORED && isIgnored(el)) return null;

            const role = getRole(el);
            const name = getAccessibleName(el);
            const states = getStates(el);
            const props = getProperties(el);

            // Recurse into children
            const children = [];
            for (const child of el.children) {{
                const childNode = buildNode(child, depth + 1);
                if (childNode) {{
                    if (Array.isArray(childNode)) {{
                        children.push(...childNode);
                    }} else {{
                        children.push(childNode);
                    }}
                }}
            }}

            // If this element has no role and no name, just pass through its children
            if (!role && !name) {{
                if (children.length === 0) return null;
                if (children.length === 1) return children[0];
                return children;
            }}

            // For text-only leaf nodes with a role: include their text content
            let text = null;
            if (children.length === 0 && role) {{
                const tc = el.textContent?.trim();
                if (tc && tc !== name && tc.length <= 500) text = tc;
            }}

            const node = {{}};
            if (role) node.role = role;
            if (name) node.name = name;
            if (text) node.text = text;
            if (states.length) node.states = states;
            if (Object.keys(props).length) node.properties = props;
            if (children.length) node.children = children;
            return node;
        }}

        // Determine root element
        let root;
        if (ROOT_SELECTOR) {{
            root = document.querySelector(ROOT_SELECTOR);
            if (!root) return {{ error: 'Root selector not found: ' + ROOT_SELECTOR }};
        }} else {{
            root = document.body || document.documentElement;
        }}

        const tree = buildNode(root, 0);
        const title = document.title || '';
        const url = location.href;

        return {{
            title: title,
            url: url,
            tree: tree || {{ role: 'document', name: title, children: [] }},
        }};
    }})()
    "#,
        max_depth = max_depth,
        include_ignored = if include_ignored { "true" } else { "false" },
        root_selector_escaped = root_selector.replace('\\', "\\\\").replace('"', "\\\""),
    );

    // Execute the JS via the browser
    let b = browser.read().await;
    let eval_result = b.evaluate(&script).await;
    drop(b);

    match eval_result {
        Ok(tree_value) => {
            let url = tree_value
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mut tree = tree_value.get("tree").cloned().unwrap_or(json!(null));

            // Assign ref IDs to each node and cache the ref→selector mapping
            let refs = a11y_cache::assign_refs(&mut tree);
            let ref_count = refs.len();
            a11y_cache::store_refs(refs, &url);

            // Handle incremental mode
            if incremental {
                let old_tree = a11y_cache::swap_snapshot(&tree);
                if let Some(old) = old_tree {
                    let delta = a11y_cache::diff_trees(&old, &tree);
                    return json!({
                        "success": true,
                        "incremental": true,
                        "title": tree_value.get("title").unwrap_or(&json!("")),
                        "url": &url,
                        "delta": delta,
                        "ref_count": ref_count,
                        "hint": "Use a11y_ref values from the last full snapshot. Refs are refreshed."
                    });
                }
                // No previous snapshot — fall through to full tree
                a11y_cache::store_snapshot(&tree);
            } else {
                a11y_cache::store_snapshot(&tree);
            }

            // Format as indented text representation
            let text_repr = format_ax_tree(&tree, 0);

            json!({
                "success": true,
                "title": tree_value.get("title").unwrap_or(&json!("")),
                "url": &url,
                "tree": tree,
                "formatted": text_repr,
                "ref_count": ref_count,
                "hint": "Each node has a 'ref' field (e.g., 'ref_0'). Pass as a11y_ref to browser_click, browser_type, etc."
            })
        }
        Err(e) => {
            let err_str = format!("{}", e);
            // If browser not launched, provide helpful message
            if err_str.contains("not launched")
                || err_str.contains("No active page")
                || err_str.contains("NoPage")
            {
                json!({
                    "success": false,
                    "error": "Browser not connected. Call browser_launch or browser_attach first, then navigate to a page."
                })
            } else {
                json!({
                    "success": false,
                    "error": format!("Failed to get accessibility snapshot: {}", err_str)
                })
            }
        }
    }
}

/// Format an accessibility tree node into human-readable indented text
fn format_ax_tree(node: &Value, indent: usize) -> String {
    if node.is_null() {
        return String::new();
    }

    // Handle array (pass-through nodes that returned multiple children)
    if let Some(arr) = node.as_array() {
        return arr
            .iter()
            .map(|n| format_ax_tree(n, indent))
            .collect::<Vec<_>>()
            .join("\n");
    }

    let prefix = "  ".repeat(indent);
    let role = node.get("role").and_then(|v| v.as_str()).unwrap_or("node");
    let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("");

    let mut line = if name.is_empty() {
        format!("{}- {}", prefix, role)
    } else {
        format!("{}- {} \"{}\"", prefix, role, name)
    };

    // Add properties inline
    let mut attrs = Vec::new();
    if let Some(props) = node.get("properties").and_then(|v| v.as_object()) {
        for (k, v) in props {
            let val_str = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => v.to_string(),
            };
            attrs.push(format!("{}={}", k, val_str));
        }
    }
    if let Some(states) = node.get("states").and_then(|v| v.as_array()) {
        for s in states {
            if let Some(st) = s.as_str() {
                attrs.push(st.to_string());
            }
        }
    }
    if !attrs.is_empty() {
        line.push_str(&format!(" [{}]", attrs.join(", ")));
    }

    // Add text content if present and different from name
    if let Some(text) = node.get("text").and_then(|v| v.as_str()) {
        if !text.is_empty() {
            line.push_str(&format!(" text=\"{}\"", &text[..text.len().min(100)]));
        }
    }

    let mut result = line;

    // Recurse children
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for child in children {
            let child_text = format_ax_tree(child, indent + 1);
            if !child_text.is_empty() {
                result.push('\n');
                result.push_str(&child_text);
            }
        }
    }

    result
}

async fn handle_retry_click(args: &Value, browser: &browser_mcp::browser::SharedBrowser) -> Value {
    let max_attempts = args
        .get("max_attempts")
        .and_then(|v| v.as_u64())
        .unwrap_or(3) as usize;
    let retry_delay_ms = args
        .get("retry_delay_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(500);

    if let Some(url) = get_browser_url(browser).await {
        if let Err(message) = security::check_browser_write_action(&url, "click") {
            return json!({"success": false, "error": message});
        }
    }

    let click_args = args.clone();
    let mut last_error = String::new();

    for attempt in 1..=max_attempts {
        let result = browser_mcp::tools::handle_tool(browser, "click", click_args.clone()).await;

        let text_parts: Vec<String> = result
            .content
            .iter()
            .filter_map(|c| match c {
                browser_mcp::types::ToolContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        let combined = text_parts.join("\n");

        if !result.is_error {
            return json!({
                "success": true,
                "attempts": attempt,
                "result": serde_json::from_str::<Value>(&combined).unwrap_or_else(|_| json!({"result": combined}))
            });
        }

        last_error = combined;

        if attempt < max_attempts {
            tokio::time::sleep(tokio::time::Duration::from_millis(retry_delay_ms)).await;

            // If using a selector, try wait_for first to let the element appear
            if let Some(selector) = args.get("selector").and_then(|v| v.as_str()) {
                let wait_args = json!({"selector": selector, "timeout_ms": retry_delay_ms * 2});
                let _ = browser_mcp::tools::handle_tool(browser, "wait_for", wait_args).await;
            }
        }
    }

    json!({
        "success": false,
        "attempts": max_attempts,
        "error": last_error,
        "hint": "Element not found or not clickable after all retry attempts"
    })
}

async fn handle_file_upload(args: &Value, browser: &browser_mcp::browser::SharedBrowser) -> Value {
    let selector = match args.get("selector").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return json!({"error": "selector required (CSS selector for input[type=file])"}),
    };

    let file_paths: Vec<String> = match args.get("files") {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => return json!({"error": "files required (string or array of file paths)"}),
    };

    // Verify files exist
    for path in &file_paths {
        if !std::path::Path::new(path).exists() {
            return json!({"error": format!("File not found: {}", path)});
        }
    }

    // Read files and create base64 data
    let mut file_data = Vec::new();
    for path in &file_paths {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => return json!({"error": format!("Cannot read {}: {}", path, e)}),
        };
        let filename = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        let mime = guess_mime(&filename);
        let b64 = {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        };
        file_data.push((filename, mime, b64));
    }

    // Build JS to create File objects and set them on the input
    let files_js: Vec<String> = file_data
        .iter()
        .map(|(name, mime, b64)| {
            format!(
            "new File([Uint8Array.from(atob('{}'), c => c.charCodeAt(0))], '{}', {{type: '{}'}})",
            b64, name, mime
        )
        })
        .collect();

    let script = format!(
        r#"(() => {{
            const input = document.querySelector('{}');
            if (!input) return JSON.stringify({{error: 'Element not found: {}'}});
            const dt = new DataTransfer();
            const files = [{}];
            files.forEach(f => dt.items.add(f));
            input.files = dt.files;
            input.dispatchEvent(new Event('change', {{bubbles: true}}));
            return JSON.stringify({{success: true, files_set: {}}});
        }})()"#,
        selector.replace('\'', "\\'"),
        selector.replace('\'', "\\'"),
        files_js.join(", "),
        file_paths.len()
    );

    let eval_args = json!({"script": script});
    let result = browser_mcp::tools::handle_tool(browser, "eval", eval_args).await;

    let text_parts: Vec<String> = result
        .content
        .iter()
        .filter_map(|c| match c {
            browser_mcp::types::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    let combined = text_parts.join("\n");

    if result.is_error {
        return json!({"error": combined});
    }

    serde_json::from_str::<Value>(&combined)
        .unwrap_or_else(|_| json!({"success": true, "result": combined, "files": file_paths}))
}

fn guess_mime(filename: &str) -> String {
    let lower = filename.to_lowercase();
    if lower.ends_with(".pdf") {
        "application/pdf".into()
    } else if lower.ends_with(".png") {
        "image/png".into()
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg".into()
    } else if lower.ends_with(".gif") {
        "image/gif".into()
    } else if lower.ends_with(".csv") {
        "text/csv".into()
    } else if lower.ends_with(".txt") {
        "text/plain".into()
    } else if lower.ends_with(".doc") || lower.ends_with(".docx") {
        "application/msword".into()
    } else if lower.ends_with(".xlsx") || lower.ends_with(".xls") {
        "application/vnd.ms-excel".into()
    } else {
        "application/octet-stream".into()
    }
}

async fn handle_get_network_log(
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
) -> Value {
    let max_entries = args
        .get("max_entries")
        .and_then(|v| v.as_u64())
        .unwrap_or(50);

    // Use performance API to get resource timing entries, then clear them
    let script = format!(
        r#"(() => {{
        const entries = performance.getEntriesByType('resource');
        const result = entries.slice(-{}).map(e => ({{
            url: e.name,
            type: e.initiatorType || 'unknown',
            duration_ms: Math.round(e.duration * 100) / 100,
            size_bytes: e.transferSize || 0,
            start_ms: Math.round(e.startTime * 100) / 100
        }}));
        performance.clearResourceTimings();
        return JSON.stringify(result);
    }})()"#,
        max_entries
    );

    let eval_args = json!({"script": script});
    let result = browser_mcp::tools::handle_tool(browser, "eval", eval_args).await;
    let text_parts: Vec<String> = result
        .content
        .iter()
        .filter_map(|c| match c {
            browser_mcp::types::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    let combined = text_parts.join("\n");

    // If JS eval fails (e.g. CDP timeout, no page loaded), return empty rather than error
    if result.is_error {
        return json!({"entries": [], "count": 0, "note": format!("JS eval failed (returning empty): {}", combined)});
    }

    match serde_json::from_str::<Value>(&combined) {
        Ok(val) => {
            // The evaluate tool may return the JSON as a string within JSON
            if let Some(s) = val.as_str() {
                serde_json::from_str::<Value>(s).unwrap_or_else(|_| json!({"entries": s}))
            } else if val.is_array() {
                json!({"entries": val, "count": val.as_array().map(|a| a.len()).unwrap_or(0)})
            } else {
                json!({"entries": val})
            }
        }
        Err(_) => json!({"entries": [], "note": combined}),
    }
}

fn handle_a11y_find(args: &Value) -> Value {
    let role_filter = args
        .get("role")
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase());
    let name_filter = args
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase());

    if role_filter.is_none() && name_filter.is_none() {
        return json!({"error": "Provide at least one of 'role' or 'name' to search for."});
    }

    let snapshot = match a11y_cache::get_snapshot() {
        Some(s) => s,
        None => {
            return json!({"error": "No cached accessibility snapshot. Call browser_a11y_snapshot first."})
        }
    };

    // Search the flattened tree
    fn search_nodes(
        node: &Value,
        role_filter: &Option<String>,
        name_filter: &Option<String>,
        results: &mut Vec<Value>,
    ) {
        if node.is_null() {
            return;
        }
        if let Some(arr) = node.as_array() {
            for child in arr {
                search_nodes(child, role_filter, name_filter, results);
            }
            return;
        }
        if let Some(obj) = node.as_object() {
            let role = obj.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let ref_id = obj.get("ref").and_then(|v| v.as_str()).unwrap_or("");

            let role_match = role_filter
                .as_ref()
                .is_none_or(|f| role.to_lowercase() == *f);
            let name_match = name_filter
                .as_ref()
                .is_none_or(|f| name.to_lowercase().contains(f.as_str()));

            if role_match && name_match && !ref_id.is_empty() {
                results.push(serde_json::json!({
                    "ref": ref_id,
                    "role": role,
                    "name": name,
                }));
            }

            if let Some(children) = obj.get("children").and_then(|v| v.as_array()) {
                for child in children {
                    search_nodes(child, role_filter, name_filter, results);
                }
            }
        }
    }

    let mut results = Vec::new();
    search_nodes(&snapshot, &role_filter, &name_filter, &mut results);

    json!({
        "matches": results,
        "count": results.len(),
    })
}

async fn handle_get_all_network(
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
) -> Value {
    let max_entries = args
        .get("max_entries")
        .and_then(|v| v.as_u64())
        .unwrap_or(100);
    let clear = args.get("clear").and_then(|v| v.as_bool()).unwrap_or(true);

    // Source 1: Route-based network log (via browser-mcp)
    let route_result =
        browser_mcp::tools::handle_tool(browser, "get_network_log", json!({"clear": clear})).await;
    let route_text: String = route_result
        .content
        .iter()
        .filter_map(|c| match c {
            browser_mcp::types::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut route_entries: Vec<Value> = if !route_result.is_error {
        serde_json::from_str::<Value>(&route_text)
            .ok()
            .and_then(|v| {
                // The response may have entries at top level or nested
                if let Some(arr) = v.as_array() {
                    Some(arr.clone())
                } else {
                    v.get("entries").and_then(|e| e.as_array()).cloned()
                }
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    // Tag each route entry with source
    for entry in &mut route_entries {
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("source".into(), json!("route"));
        }
    }
    route_entries.truncate(max_entries as usize);

    // Source 2: Performance API log
    let perf_result = handle_get_network_log(&json!({"max_entries": max_entries}), browser).await;
    let mut perf_entries: Vec<Value> = perf_result
        .get("entries")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Tag each perf entry with source
    for entry in &mut perf_entries {
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("source".into(), json!("performance"));
        }
    }

    // Merge
    let mut all_entries = route_entries;
    all_entries.extend(perf_entries);

    json!({
        "entries": all_entries,
        "count": all_entries.len(),
        "sources": ["route", "performance"],
    })
}

async fn handle_element_drag(args: &Value, browser: &browser_mcp::browser::SharedBrowser) -> Value {
    let from_selector = match args.get("from_selector").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return json!({"error": "Missing 'from_selector' parameter"}),
    };
    let to_selector = args.get("to_selector").and_then(|v| v.as_str());
    let offset_x = args.get("offset_x").and_then(|v| v.as_i64());
    let offset_y = args.get("offset_y").and_then(|v| v.as_i64());
    let duration_ms = args
        .get("duration_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(300);

    if to_selector.is_none() && offset_x.is_none() && offset_y.is_none() {
        return json!({"error": "Must provide either 'to_selector' or 'offset_x'/'offset_y'"});
    }

    // Get bounds of from_selector
    let from_bounds_result =
        browser_mcp::tools::handle_tool(browser, "get_bounds", json!({"selector": from_selector}))
            .await;
    let from_text: String = from_bounds_result
        .content
        .iter()
        .filter_map(|c| match c {
            browser_mcp::types::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if from_bounds_result.is_error {
        return json!({"error": format!("Cannot get bounds for from_selector: {}", from_text)});
    }
    let from_bounds: Value = match serde_json::from_str(&from_text) {
        Ok(v) => v,
        Err(_) => {
            return json!({"error": format!("Invalid bounds response for from_selector: {}", from_text)})
        }
    };

    let from_x = from_bounds.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0)
        + from_bounds
            .get("width")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            / 2.0;
    let from_y = from_bounds.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0)
        + from_bounds
            .get("height")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            / 2.0;

    let (to_x, to_y) = if let Some(to_sel) = to_selector {
        // Resolve to_selector to coordinates
        let to_bounds_result =
            browser_mcp::tools::handle_tool(browser, "get_bounds", json!({"selector": to_sel}))
                .await;
        let to_text: String = to_bounds_result
            .content
            .iter()
            .filter_map(|c| match c {
                browser_mcp::types::ToolContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if to_bounds_result.is_error {
            return json!({"error": format!("Cannot get bounds for to_selector: {}", to_text)});
        }
        let to_bounds: Value = match serde_json::from_str(&to_text) {
            Ok(v) => v,
            Err(_) => {
                return json!({"error": format!("Invalid bounds response for to_selector: {}", to_text)})
            }
        };
        let tx = to_bounds.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0)
            + to_bounds
                .get("width")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                / 2.0;
        let ty = to_bounds.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0)
            + to_bounds
                .get("height")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                / 2.0;
        (tx, ty)
    } else {
        // Use offsets from from_selector center
        (
            from_x + offset_x.unwrap_or(0) as f64,
            from_y + offset_y.unwrap_or(0) as f64,
        )
    };

    // Call the existing drag handler with resolved coordinates
    let drag_args = json!({
        "from_x": from_x as i64,
        "from_y": from_y as i64,
        "to_x": to_x as i64,
        "to_y": to_y as i64,
        "duration_ms": duration_ms,
    });
    handle_drag(&drag_args)
}

fn handle_hands_status() -> Value {
    let uia_ok = {
        let result = uia_lib::handle_tool_call("uia_list_windows", &json!({}));
        result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || result.get("windows").is_some()
    };

    json!({
        "server": "hands",
        "version": "0.1.0",
        "subsystems": {
            "browser": "available (call browser_launch or browser_attach first)",
            "uia": if uia_ok { "active" } else { "unavailable (Windows only)" },
            "vision": "available (screenshot + OCR + template matching)"
        },
        "tool_count": get_all_tool_definitions().len(),
        "categories": {
            "browser": "Token-saving web: smart_browse, http_scrape, bulk_extract, etc.",
            "uia": "Desktop automation: click, type, focus, shortcuts, element inspection",
            "vision": "Screen reading: screenshot, OCR, diff, template matching",
            "combo": "Multi-system: find_and_click, read_screen_text, wait_for_visual, etc."
        }
    })
}

async fn get_browser_url(browser: &browser_mcp::browser::SharedBrowser) -> Option<String> {
    let guard = browser.read().await;
    guard.get_url().await.ok()
}

// ============ A11Y REF RESOLUTION ============

/// Resolve an a11y ref to a CSS selector by executing JS in the browser.
/// The JS finds the element by role + accessible name, then generates a unique selector.
pub(crate) async fn resolve_a11y_ref(
    ref_id: &str,
    _tool: &str,
    browser: &browser_mcp::browser::SharedBrowser,
) -> Result<String, String> {
    let resolve_js = a11y_cache::ref_resolution_js(ref_id)?;

    // Wrap the resolution JS to return a unique selector for the found element
    let script = format!(
        r#"(() => {{
            const el = {};
            if (!el) return JSON.stringify({{error: 'Element not found for ref: {}'}});

            // Generate a unique CSS selector for this element
            function uniqueSelector(element) {{
                if (element.id) return '#' + CSS.escape(element.id);

                // Try aria-label
                const label = element.getAttribute('aria-label');
                if (label) {{
                    const sel = element.tagName.toLowerCase() + '[aria-label="' + label.replace(/"/g, '\\"') + '"]';
                    if (document.querySelectorAll(sel).length === 1) return sel;
                }}

                // Build path from parent
                let path = [];
                let current = element;
                while (current && current !== document.body && current !== document.documentElement) {{
                    let sel = current.tagName.toLowerCase();
                    if (current.id) {{
                        path.unshift('#' + CSS.escape(current.id));
                        break;
                    }}
                    // Add nth-of-type if needed
                    const parent = current.parentElement;
                    if (parent) {{
                        const siblings = Array.from(parent.children).filter(c => c.tagName === current.tagName);
                        if (siblings.length > 1) {{
                            const idx = siblings.indexOf(current) + 1;
                            sel += ':nth-of-type(' + idx + ')';
                        }}
                    }}
                    path.unshift(sel);
                    current = current.parentElement;
                }}
                return path.join(' > ');
            }}

            return JSON.stringify({{selector: uniqueSelector(el)}});
        }})()"#,
        resolve_js,
        ref_id.replace('"', "\\\""),
    );

    let eval_args = json!({"script": script});
    let result = browser_mcp::tools::handle_tool(browser, "eval", eval_args).await;

    let text_parts: Vec<String> = result
        .content
        .iter()
        .filter_map(|c| match c {
            browser_mcp::types::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    let combined = text_parts.join("\n");

    if result.is_error {
        return Err(format!(
            "Failed to resolve a11y ref '{}': {}",
            ref_id, combined
        ));
    }

    // Parse the JSON result — eval returns string-within-string (e.g. "{\"selector\":\"...\"}")
    // serde parses that as Value::String, so we must unwrap the inner string
    let val: Value = match serde_json::from_str::<Value>(&combined) {
        Ok(Value::String(inner)) => {
            // eval wraps JS string results in quotes — parse the inner JSON
            serde_json::from_str(&inner)
                .map_err(|e| format!("Failed to parse inner ref result: {}", e))?
        }
        Ok(v) => v,
        Err(_) => combined
            .trim_matches('"')
            .replace("\\\"", "\"")
            .parse::<Value>()
            .map_err(|e| format!("Parse error: {}", e))?,
    };

    if let Some(err) = val.get("error").and_then(|v| v.as_str()) {
        // Auto-refresh: re-take a11y snapshot once and retry before failing
        eprintln!(
            "[hands] a11y ref '{}' not found, auto-refreshing snapshot...",
            ref_id
        );
        let refresh_result = handle_accessibility_snapshot(&json!({}), browser).await;
        if refresh_result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            // Retry resolution with fresh cache
            if let Ok(resolve_js2) = a11y_cache::ref_resolution_js(ref_id) {
                let script2 = format!(
                    r#"(() => {{
                        const el = {};
                        if (!el) return JSON.stringify({{error: 'Element not found for ref: {}'}});
                        function uniqueSelector(element) {{
                            if (element.id) return '#' + CSS.escape(element.id);
                            const label = element.getAttribute('aria-label');
                            if (label) {{
                                const sel = element.tagName.toLowerCase() + '[aria-label="' + label.replace(/"/g, '\\"') + '"]';
                                if (document.querySelectorAll(sel).length === 1) return sel;
                            }}
                            let path = [];
                            let current = element;
                            while (current && current !== document.body && current !== document.documentElement) {{
                                let sel = current.tagName.toLowerCase();
                                if (current.id) {{ path.unshift('#' + CSS.escape(current.id)); break; }}
                                const parent = current.parentElement;
                                if (parent) {{
                                    const siblings = Array.from(parent.children).filter(c => c.tagName === current.tagName);
                                    if (siblings.length > 1) {{ const idx = siblings.indexOf(current) + 1; sel += ':nth-of-type(' + idx + ')'; }}
                                }}
                                path.unshift(sel);
                                current = current.parentElement;
                            }}
                            return path.join(' > ');
                        }}
                        return JSON.stringify({{selector: uniqueSelector(el)}});
                    }})()"#,
                    resolve_js2,
                    ref_id.replace('"', "\\\""),
                );
                let retry_result =
                    browser_mcp::tools::handle_tool(browser, "eval", json!({"script": script2}))
                        .await;
                let retry_text: Vec<String> = retry_result
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        browser_mcp::types::ToolContent::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect();
                let retry_combined = retry_text.join("\n");
                if !retry_result.is_error {
                    if let Ok(retry_val) = serde_json::from_str::<Value>(&retry_combined) {
                        let inner = match retry_val {
                            Value::String(s) => serde_json::from_str(&s).unwrap_or(json!({})),
                            v => v,
                        };
                        if let Some(sel) = inner.get("selector").and_then(|v| v.as_str()) {
                            return Ok(sel.to_string());
                        }
                    }
                }
            }
        }
        return Err(format!(
            "Ref '{}' resolution failed after auto-refresh: {}.",
            ref_id, err
        ));
    }

    val.get("selector")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("Ref '{}' resolved but no selector returned", ref_id))
}

// ============ LEARN API HANDLER ============

async fn handle_learn_api(args: &Value, browser: &browser_mcp::browser::SharedBrowser) -> Value {
    let filter_pattern = args.get("filter_pattern").and_then(|v| v.as_str());
    let include_static = args
        .get("include_static")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let min_response_size = args
        .get("min_response_size")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Get all network data (merges route + performance)
    let network =
        handle_get_all_network(&json!({"max_entries": 500, "clear": false}), browser).await;
    let entries = match network.get("entries").and_then(|v| v.as_array()) {
        Some(e) => e.clone(),
        None => {
            return json!({"success": false, "error": "No network data captured. Navigate and interact with a page first."})
        }
    };

    let static_exts = [
        ".js", ".css", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".woff", ".woff2", ".ttf",
        ".eot", ".map", ".webp", ".avif",
    ];

    let filter_re = filter_pattern.and_then(|p| regex::Regex::new(p).ok());

    // Hoisted out of the loop below to avoid `regex_creation_in_loops` (compiled once, reused).
    let uuid_re =
        regex::Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap();
    let id_re = regex::Regex::new(r"/\d{2,}/").unwrap();

    let mut api_entries: Vec<Value> = Vec::new();
    let mut static_filtered = 0u64;
    let total = entries.len();

    for entry in &entries {
        let url = entry
            .get("url")
            .and_then(|v| v.as_str())
            .or_else(|| entry.get("name").and_then(|v| v.as_str()))
            .unwrap_or("");
        if url.is_empty() {
            continue;
        }

        // Filter static assets
        if !include_static {
            let url_lower = url.to_lowercase();
            if static_exts.iter().any(|ext| url_lower.contains(ext))
                || url_lower.contains("/static/")
                || url_lower.contains("/assets/")
                || url_lower.contains("fonts.")
                || url_lower.contains("data:")
            {
                static_filtered += 1;
                continue;
            }
        }

        // Filter by regex pattern if provided
        if let Some(ref re) = filter_re {
            if !re.is_match(url) {
                continue;
            }
        }

        // Filter by min response size
        let resp_size = entry
            .get("transferSize")
            .and_then(|v| v.as_u64())
            .or_else(|| entry.get("response_size").and_then(|v| v.as_u64()))
            .unwrap_or(0);
        if resp_size < min_response_size && min_response_size > 0 {
            continue;
        }

        let method = entry
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET")
            .to_string();
        let content_type = entry
            .get("content_type")
            .and_then(|v| v.as_str())
            .or_else(|| entry.get("contentType").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();

        // Build URL pattern: replace UUIDs and numeric IDs with {id}
        let url_pattern = uuid_re.replace_all(url, "{uuid}").to_string();
        let url_pattern = id_re.replace_all(&url_pattern, "/{id}/").to_string();
        // Remove query params for pattern matching
        let url_pattern_base = url_pattern
            .split('?')
            .next()
            .unwrap_or(&url_pattern)
            .to_string();

        // Extract headers (from route entries)
        let headers = entry
            .get("request_headers")
            .cloned()
            .or_else(|| entry.get("headers").cloned())
            .unwrap_or(json!({}));

        // Detect auth type from headers
        let auth_type = if let Some(obj) = headers.as_object() {
            if obj.keys().any(|k| k.to_lowercase() == "authorization") {
                let auth_val = obj
                    .iter()
                    .find(|(k, _)| k.to_lowercase() == "authorization")
                    .map(|(_, v)| v.as_str().unwrap_or(""))
                    .unwrap_or("");
                if auth_val.starts_with("Bearer ") {
                    "bearer"
                } else if auth_val.starts_with("Basic ") {
                    "basic"
                } else {
                    "custom"
                }
            } else if obj.keys().any(|k| {
                k.to_lowercase().contains("api-key") || k.to_lowercase().contains("apikey")
            }) {
                "api_key"
            } else if obj.keys().any(|k| k.to_lowercase() == "cookie") {
                "cookie"
            } else {
                "none"
            }
        } else {
            "none"
        };

        // Extract body template (from route entries with POST/PUT/PATCH)
        let body_template = if ["POST", "PUT", "PATCH"].contains(&method.as_str()) {
            entry
                .get("request_body")
                .cloned()
                .or_else(|| entry.get("body").cloned())
        } else {
            None
        };

        // Extract response shape (first-level JSON keys)
        let response_shape: Option<Vec<String>> = entry.get("response_body").and_then(|v| {
            if let Some(obj) = v.as_object() {
                Some(obj.keys().cloned().collect())
            } else if let Some(s) = v.as_str() {
                serde_json::from_str::<Value>(s)
                    .ok()
                    .and_then(|parsed| parsed.as_object().map(|o| o.keys().cloned().collect()))
            } else {
                None
            }
        });

        api_entries.push(json!({
            "url_pattern": url_pattern_base,
            "url_actual": url,
            "method": method,
            "headers": headers,
            "auth_type": auth_type,
            "body_template": body_template,
            "response_shape": response_shape,
            "content_type": content_type,
            "response_size": resp_size,
        }));
    }

    // Deduplicate by method + url_pattern, keep one representative and count occurrences
    let mut deduped: std::collections::HashMap<String, (Value, u64)> =
        std::collections::HashMap::new();
    for entry in &api_entries {
        let key = format!(
            "{}:{}",
            entry.get("method").and_then(|v| v.as_str()).unwrap_or(""),
            entry
                .get("url_pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("")
        );
        let counter = deduped.entry(key).or_insert_with(|| (entry.clone(), 0));
        counter.1 += 1;
    }

    let mut discovered: Vec<Value> = deduped
        .into_values()
        .map(|(mut entry, count)| {
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("calls_observed".into(), json!(count));
                obj.remove("url_actual");
            }
            entry
        })
        .collect();

    // Sort by frequency (most-called first)
    discovered.sort_by(|a, b| {
        let ca = a
            .get("calls_observed")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let cb = b
            .get("calls_observed")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        cb.cmp(&ca)
    });

    json!({
        "discovered_apis": discovered,
        "total_requests_captured": total,
        "api_requests_found": discovered.len(),
        "static_filtered": static_filtered,
    })
}

// ============ BATCH HANDLERS ============

/// Resolve $step[N].result and $prev.result references in a JSON value
fn resolve_step_refs(value: &Value, results: &[Value]) -> Value {
    match value {
        Value::String(s) => {
            let mut out = s.clone();
            // Replace $step[N].result references
            let step_re = regex::Regex::new(r"\$step\[(\d+)\]\.result").unwrap();
            for cap in step_re.captures_iter(s) {
                let idx: usize = cap[1].parse().unwrap_or(usize::MAX);
                if let Some(prev) = results.get(idx) {
                    let replacement = prev
                        .get("result")
                        .map(|v| match v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .unwrap_or_default();
                    out = out.replace(&cap[0], &replacement);
                }
            }
            // Replace $prev.result with last result
            if out.contains("$prev.result") {
                if let Some(last) = results.last() {
                    let replacement = last
                        .get("result")
                        .map(|v| match v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .unwrap_or_default();
                    out = out.replace("$prev.result", &replacement);
                }
            }
            Value::String(out)
        }
        Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                new_map.insert(k.clone(), resolve_step_refs(v, results));
            }
            Value::Object(new_map)
        }
        Value::Array(arr) => {
            Value::Array(arr.iter().map(|v| resolve_step_refs(v, results)).collect())
        }
        other => other.clone(),
    }
}

async fn handle_browser_batch(
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
) -> Value {
    let actions = match args.get("actions").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return json!({"success": false, "error": "actions array required"}),
    };
    let continue_on_error = args
        .get("continue_on_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut results: Vec<Value> = Vec::with_capacity(actions.len());

    for (i, action) in actions.iter().enumerate() {
        let action_type = match action.get("type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                let err =
                    json!({"index": i, "success": false, "error": "action missing 'type' field"});
                if continue_on_error {
                    results.push(err);
                    continue;
                } else {
                    results.push(err);
                    break;
                }
            }
        };
        let raw_params = action.get("params").cloned().unwrap_or(json!({}));
        let params = resolve_step_refs(&raw_params, &results);

        // Map action type to browser tool name
        let tool_name = match action_type {
            "click" => "click",
            "type_text" | "type" => "type_text",
            "navigate" => "navigate",
            "wait_idle" => "wait_idle",
            "screenshot" => "screenshot",
            "wait_for" => "wait_for",
            "scroll" => "scroll",
            "a11y_snapshot" => {
                // Special: a11y_snapshot is handled by hands, not browser-mcp directly
                let result = handle_accessibility_snapshot(&params, browser).await;
                results.push(json!({"index": i, "type": action_type, "result": result}));
                continue;
            }
            other => {
                let err = json!({"index": i, "success": false, "error": format!("unknown action type: {}", other)});
                if continue_on_error {
                    results.push(err);
                    continue;
                } else {
                    results.push(err);
                    break;
                }
            }
        };

        let result = browser_mcp::tools::handle_tool(browser, tool_name, params).await;
        let text_parts: Vec<String> = result
            .content
            .iter()
            .filter_map(|c| match c {
                browser_mcp::types::ToolContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        let combined = text_parts.join("\n");
        let val = if result.is_error {
            json!({"index": i, "type": action_type, "success": false, "error": combined})
        } else {
            let parsed = serde_json::from_str::<Value>(&combined)
                .unwrap_or_else(|_| json!({"result": combined}));
            json!({"index": i, "type": action_type, "success": true, "result": parsed})
        };

        let failed = result.is_error;
        results.push(val);
        if failed && !continue_on_error {
            break;
        }
    }

    json!({"success": true, "results": results, "total": actions.len(), "executed": results.len()})
}

fn handle_uia_batch(args: &Value) -> Value {
    let actions = match args.get("actions").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return json!({"success": false, "error": "actions array required"}),
    };
    let continue_on_error = args
        .get("continue_on_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut results: Vec<Value> = Vec::with_capacity(actions.len());

    for (i, action) in actions.iter().enumerate() {
        let action_type = match action.get("type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                let err =
                    json!({"index": i, "success": false, "error": "action missing 'type' field"});
                if continue_on_error {
                    results.push(err);
                    continue;
                } else {
                    results.push(err);
                    break;
                }
            }
        };
        let params = action.get("params").cloned().unwrap_or(json!({}));

        // Map action type to UIA tool name
        let tool_name = match action_type {
            "click" => "uia_click",
            "type_text" | "type" => "uia_type_text",
            "key_press" => "uia_key_press",
            "focus_window" => "uia_focus_window",
            "screenshot" => "vision_screenshot",
            "read_value" => "uia_read_value",
            "scroll" => "uia_scroll",
            other => {
                let err = json!({"index": i, "success": false, "error": format!("unknown action type: {}", other)});
                if continue_on_error {
                    results.push(err);
                    continue;
                } else {
                    results.push(err);
                    break;
                }
            }
        };

        // UIA tools are sync, vision is async but we handle screenshot specially
        let result = if tool_name == "vision_screenshot" {
            // Can't call async from sync fn — use a simple screenshot via UIA approach
            // Actually we just call uia_lib for UIA tools, vision tools need special handling
            // For screenshot in UIA batch, we use uia_lib's approach if available,
            // otherwise report that screenshot needs the async vision tool
            json!({"success": false, "error": "screenshot in uia_batch not supported — use browser_batch or vision_screenshot directly"})
        } else {
            dispatch_uia_tool(tool_name, &params)
        };

        let success = result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
            && result.get("error").is_none();

        let val = json!({"index": i, "type": action_type, "result": result});

        let failed = !success;
        results.push(val);
        if failed && !continue_on_error {
            break;
        }
    }

    json!({"success": true, "results": results, "total": actions.len(), "executed": results.len()})
}

fn dispatch_uia_tool(name: &str, args: &Value) -> Value {
    match name {
        "uia_get_state" => uia::handle_get_state(args),
        "uia_click" => uia::handle_click(args),
        "uia_type" | "uia_type_text" => uia::handle_type(args),
        _ => uia_lib::handle_tool_call(name, args),
    }
}

// ============ TOOL DISPATCH ============

pub(crate) async fn handle_tool_call(
    name: &str,
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
    session: &meta::SessionHandle,
) -> Value {
    let _dispatch_start = std::time::Instant::now();
    let _result = handle_tool_call_inner(name, args, browser, session).await;
    dashboard_endpoint::record_action(name, args, _dispatch_start.elapsed().as_millis() as u64);
    _result
}

async fn handle_tool_call_inner(
    name: &str,
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
    session: &meta::SessionHandle,
) -> Value {
    // Phase A v2 Meta-tools (checked first — highest priority)
    // Phase C fix1: catch_unwind boundary — meta-tool panics must never crash hands.exe
    let meta_result =
        std::panic::AssertUnwindSafe(meta::handle_meta_tool(name, args, browser, session));
    match futures::FutureExt::catch_unwind(meta_result).await {
        Ok(Some(result)) => {
            // Update browser snapshot after any meta-tool runs
            if let Ok(guard) = browser.try_read() {
                dashboard_endpoint::update_browser_snapshot(guard.status());
            }
            return result;
        }
        Ok(None) => {} // Not a meta-tool, continue to other dispatch paths
        Err(panic_info) => {
            let panic_msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = panic_info.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            eprintln!(
                "[hands] PANIC CAUGHT in meta-tool '{}': {}",
                name, panic_msg
            );
            return json!({
                "success": false,
                "error": format!("Internal error in '{}': {}", name, panic_msg),
                "panic_caught": true,
            });
        }
    }

    // Update browser snapshot for any browser_ prefixed tool calls
    if name.starts_with("browser_") {
        if let Ok(guard) = browser.try_read() {
            dashboard_endpoint::update_browser_snapshot(guard.status());
        }
    }

    // Combo tools (unique to Hands)
    match name {
        "find_and_click" => return handle_find_and_click(args).await,
        "read_screen_text" => return handle_read_screen_text(args).await,
        "wait_for_visual" => return handle_wait_for_visual(args).await,
        "window_screenshot" => return handle_window_screenshot(args).await,
        "vision_screenshot_hidden_window" => {
            // Canonical PrintWindow capture. Always uses the behind=true code path.
            let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let save_path = args.get("save_path").and_then(|v| v.as_str());
            let do_ocr = args.get("ocr").and_then(|v| v.as_bool()).unwrap_or(true);
            return handle_window_screenshot_behind(title, save_path, do_ocr).await;
        }
        "type_into_window" => return handle_type_into_window(args),
        "drag" => return handle_drag(args),
        "uia_window_resize" => return handle_uia_window_resize(args),
        "uia_window_move" => return handle_uia_window_move(args),
        "uia_window_state" => return handle_uia_window_state(args),
        "uia_window_snap" => return handle_uia_window_snap(args),
        "uia_app_launch" => return handle_uia_app_launch(args),
        "retry_click" => return handle_retry_click(args, browser).await,
        "file_upload" => return handle_file_upload(args, browser).await,
        "status" | "hands_status" => return handle_hands_status(),
        "hands_health" => return meta::health::hands_health(),
        "hands_summarize_run" => return meta::summarize_run::handle(args),
        "vision_cache_stats" => return meta::vision_cache::handle_stats(args),
        // Phase D — self-record / replay loop (plan-not-action: returns workflow:* call plans)
        "hands_self_record_start" => {
            return meta::self_record::handle_self_record_start(args).await
        }
        "hands_self_record_lookup" => {
            return meta::self_record::handle_self_record_lookup(args).await
        }
        "hands_self_record_stop_and_optimize" => {
            return meta::self_record::handle_self_record_stop_and_optimize(args).await
        }
        "browser_a11y_snapshot" | "browser_accessibility_snapshot" => {
            return handle_accessibility_snapshot(args, browser).await
        }
        "browser_get_performance_log" => return handle_get_network_log(args, browser).await,
        "element_drag" => return handle_element_drag(args, browser).await,
        "browser_batch" => return handle_browser_batch(args, browser).await,
        "browser_a11y_find" => return handle_a11y_find(args),
        "browser_get_all_network" => return handle_get_all_network(args, browser).await,
        "browser_learn_api" => return handle_learn_api(args, browser).await,
        "uia_batch" => return handle_uia_batch(args),
        _ => {}
    }

    // Browser tools (prefixed with browser_)
    if let Some(browser_tool) = name.strip_prefix("browser_") {
        if let Some(url) = get_browser_url(browser).await {
            if let Err(message) = security::check_browser_write_action(&url, browser_tool) {
                return json!({"success": false, "error": message});
            }
        }

        // A11y ref resolution: if a11y_ref is provided, resolve it to a DOM element via JS
        let mut resolved_args = args.clone();
        if let Some(a11y_ref) = args.get("a11y_ref").and_then(|v| v.as_str()) {
            match resolve_a11y_ref(a11y_ref, browser_tool, browser).await {
                Ok(selector) => {
                    if let Some(obj) = resolved_args.as_object_mut() {
                        obj.insert("selector".into(), json!(selector));
                        obj.remove("a11y_ref");
                    }
                }
                Err(e) => return json!({"success": false, "error": e}),
            }
        }

        // Check if stealth mode requested (for launch/attach)
        let stealth_explicit = resolved_args.get("stealth").and_then(|v| v.as_bool());
        let mut wants_stealth = stealth_explicit.unwrap_or(false);

        // Default stealth=true when headless=true and stealth wasn't explicitly set
        if browser_tool == "launch"
            && stealth_explicit.is_none()
            && resolved_args
                .get("headless")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        {
            wants_stealth = true;
        }

        // Strip stealth param before passing to browser-mcp (it doesn't know about it)
        if let Some(obj) = resolved_args.as_object_mut() {
            obj.remove("stealth");
        }

        let saved_args = resolved_args.clone();
        let result = browser_mcp::tools::handle_tool(browser, browser_tool, resolved_args).await;

        // Auto-retry with temp profile on Chrome profile lock (exit code 21)
        if result.is_error && browser_tool == "launch" {
            let err_text: String = result
                .content
                .iter()
                .filter_map(|c| match c {
                    browser_mcp::types::ToolContent::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if err_text.contains("ExitStatus(21)")
                || err_text.contains("before websocket URL")
                || err_text.contains("exit code: 21")
            {
                // Auto-retry: create temp profile and retry launch once
                let temp_profile = std::env::temp_dir().join("hands_chrome_profile");
                let _ = std::fs::create_dir_all(&temp_profile);
                let temp_profile_str = temp_profile.to_string_lossy().to_string();
                eprintln!(
                    "[hands] Chrome profile locked, retrying with temp profile: {}",
                    temp_profile_str
                );

                let mut retry_args = saved_args.clone();
                if let Some(obj) = retry_args.as_object_mut() {
                    obj.insert("profile_path".into(), json!(temp_profile_str));
                }
                let retry_result =
                    browser_mcp::tools::handle_tool(browser, browser_tool, retry_args).await;

                if !retry_result.is_error {
                    // Retry succeeded — inject stealth if needed, then return with auto_profile flag
                    if wants_stealth {
                        let stealth_result = browser_mcp::tools::handle_tool(
                            browser,
                            "evaluate",
                            json!({"expression": stealth::STEALTH_JS}),
                        )
                        .await;
                        if stealth_result.is_error {
                            eprintln!(
                                "[hands] Stealth JS injection warning (auto-retry): {:?}",
                                stealth_result.content
                            );
                        }
                    }
                    let retry_text: Vec<String> = retry_result
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            browser_mcp::types::ToolContent::Text { text } => Some(text.clone()),
                            _ => None,
                        })
                        .collect();
                    let combined_retry = retry_text.join("\n");
                    let mut val = serde_json::from_str::<Value>(&combined_retry)
                        .unwrap_or_else(|_| json!({"result": combined_retry}));
                    if let Some(obj) = val.as_object_mut() {
                        obj.insert("auto_profile".into(), json!(temp_profile_str));
                        if wants_stealth {
                            obj.insert("stealth".into(), json!(true));
                        }
                    }
                    return val;
                }

                // Retry also failed — return original error with helpful message
                return json!({
                    "success": false,
                    "error": "Chrome profile is locked by another instance. Auto-retry with temp profile also failed. Either close other Chrome instances or provide a different profile_path.",
                    "auto_retry_profile": temp_profile_str,
                    "raw_error": err_text
                });
            }
        }

        // After successful navigate, auto-take a11y snapshot so refs are immediately available
        if !result.is_error && browser_tool == "navigate" {
            let snap = handle_accessibility_snapshot(&json!({}), browser).await;
            let ref_count = snap.get("ref_count").and_then(|v| v.as_u64()).unwrap_or(0);
            eprintln!(
                "[hands] Auto a11y snapshot after navigate: {} refs cached",
                ref_count
            );
            // Inject a11y hint into the navigate response
            let text_parts: Vec<String> = result
                .content
                .iter()
                .filter_map(|c| match c {
                    browser_mcp::types::ToolContent::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect();
            let combined = text_parts.join("\n");
            let mut val = serde_json::from_str::<Value>(&combined)
                .unwrap_or_else(|_| json!({"result": combined}));
            if let Some(obj) = val.as_object_mut() {
                obj.insert("a11y_refs_available".into(), json!(true));
                obj.insert("a11y_ref_count".into(), json!(ref_count));
                obj.insert("a11y_hint".into(), json!("Use a11y_ref from browser_a11y_snapshot for element interaction. Refs are pre-cached."));
            }
            return val;
        }

        // After successful launch/attach with stealth, inject anti-detection JS
        if wants_stealth
            && !result.is_error
            && (browser_tool == "launch" || browser_tool == "attach")
        {
            let stealth_result = browser_mcp::tools::handle_tool(
                browser,
                "evaluate",
                json!({"expression": stealth::STEALTH_JS}),
            )
            .await;
            if stealth_result.is_error {
                eprintln!(
                    "[hands] Stealth JS injection warning: {:?}",
                    stealth_result.content
                );
            }
        }

        // Extract text content from ToolResult
        let text_parts: Vec<String> = result
            .content
            .iter()
            .filter_map(|c| match c {
                browser_mcp::types::ToolContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        let combined = text_parts.join("\n");
        return if result.is_error {
            json!({"success": false, "error": combined})
        } else {
            let mut val = serde_json::from_str::<Value>(&combined)
                .unwrap_or_else(|_| json!({"result": combined}));
            if wants_stealth {
                if let Some(obj) = val.as_object_mut() {
                    obj.insert("stealth".into(), json!(true));
                }
            }
            val
        };
    }

    // UIA tools (uia_ prefix)
    if name.starts_with("uia_") {
        return dispatch_uia_tool(name, args);
    }

    // Vision tools (vision_ prefix)
    if name.starts_with("vision_") {
        return vision_core::execute(name, args).await;
    }

    json!({"error": format!("Unknown tool: {}", name)})
}

// ============ MCP STDIO SERVER ============

fn handle_request(
    request: &JsonRpcRequest,
    browser: &browser_mcp::browser::SharedBrowser,
    session: &meta::SessionHandle,
    rt: &tokio::runtime::Handle,
) -> Option<JsonRpcResponse> {
    let id = request.id.clone().unwrap_or(Value::Null);
    let method = request.method.as_deref().unwrap_or("");

    // Notifications (method starts with "notifications/") get no response
    if method.starts_with("notifications/") {
        return None;
    }

    let response = match method {
        "initialize" => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "hands",
                    "version": "0.1.0",
                    "description": "Unified interaction: browser + UIA + vision"
                }
            })),
            error: None,
        },

        "tools/list" => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(json!({ "tools": get_all_tool_definitions() })),
            error: None,
        },

        "tools/call" => {
            let params = request.params.as_ref();
            let tool_name = params
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let tool_args = params
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or(json!({}));

            let result = rt.block_on(handle_tool_call(tool_name, &tool_args, browser, session));

            JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id,
                result: Some(json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&result)
                            .unwrap_or_else(|_| result.to_string())
                    }]
                })),
                error: None,
            }
        }

        _ => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(json!({
                "code": -32601,
                "message": format!("Method not found: {}", method)
            })),
        },
    };

    Some(response)
}

fn main() {
    // Log startup
    let _ = std::fs::write(
        std::env::temp_dir().join("hands_mcp_started.txt"),
        format!(
            "Hands MCP started at {:?}\nPID: {}\n",
            std::time::SystemTime::now(),
            std::process::id()
        ),
    );

    // Spawn HTTP dashboard endpoint (127.0.0.1:9102 by default)
    dashboard_endpoint::spawn();

    // Create tokio runtime for async browser/vision operations
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    let browser = browser_mcp::browser::create_shared();
    let session = meta::new_session();

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(_) => continue,
        };

        if let Some(response) = handle_request(&request, &browser, &session, rt.handle()) {
            let response_str = serde_json::to_string(&response).unwrap_or_default();
            let mut out = stdout.lock();
            let _ = writeln!(out, "{}", response_str);
            let _ = out.flush();
        }
    }
}
