//! Meta-tools for the hands MCP server (Phase A v2 + Phase B + Phase C).
//!
//! Meta-tools encode best-practice escalation ladders so Claude doesn't need
//! to manually retry failed strategies. Each meta-tool wraps 3–7 primitives,
//! tries them in proven order, and returns on first success.
//!
//! v2: reorganized around shared infrastructure:
//! - Standardized MetaToolResult envelope
//! - Locked MetaError enum (all Phase A/B/C variants)
//! - Hybrid-invalidation A11y cache
//! - Session state (monitor stickiness, subsystem health)
//! - Instrumentation logging
//! - Targeting reliability helpers (31 adjustments)
//! - Vision capture helper (size rules, tiling, caching)
//! - Consent risk classifier (Phase C stub)
//!
//! Phase A tools:
//!   - `hands_read_page`  — escalating web content fetcher
//!   - `hands_click`      — 7-rung cross-subsystem click ladder
//!   - `hands_navigate`   — launch + navigate + wait pipeline
//!   - `hands_capture`    — screenshot + OCR verification routing
//!
//! Phase B tools:
//!   - `hands_find`       — 6-rung cross-subsystem element finder
//!   - `hands_type`       — focus-verified text input with chunked typing
//!   - `hands_fill_form`  — automated form filling with per-field tracking
//!
//! Phase C tools:
//!   - `hands_verify`     — structured verification with polling + stabilization
//!   - `hands_scan_qr`    — QR code scanning and decoding
//!   - `hands_script`     — multi-step meta-tool orchestrator with variable substitution
//!   - `hands_login_recovery` — 5-stage login pipeline (template + script)

// ── Shared infrastructure ──
pub mod cache;
pub mod consent;
pub mod error;
pub mod health;
pub mod instrumentation;
pub mod response;
pub mod session;
pub mod targeting;
pub mod vision_capture;

// ── Phase B shared helpers ──
pub mod autofill;
pub mod field_role;
pub mod label_match;
pub mod reversibility;

// ── Meta-tool implementations (Phase A) ──
pub mod capture;
pub mod click;
pub mod navigate;
pub mod read_page;

// ── Meta-tool implementations (Phase B) ──
pub mod fill_form;
pub mod find;
pub mod type_text;

// ── Phase C shared helpers ──
pub mod nl_parser;
pub mod save_dialog;
pub mod verify_templates;
pub mod window_match;

// ── Meta-tool implementations (Phase C) ──
pub mod app_action;
pub mod qr_scan;
pub mod script;
pub mod templates;
pub mod verify;
pub mod verify_expectations;

// ── Phase D — Self-improvement ──
pub mod self_record;

// ── Tests ──
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_phase_b;
#[cfg(test)]
mod tests_phase_c;

use serde_json::{json, Value};
use session::SharedSession;

// ── Re-exports for convenience ──
pub use session::{new_session, SharedSession as SessionHandle};

// ============ SHARED BROWSER HELPERS ============

/// Extract text content from a browser ToolResult.
pub fn extract_browser_text(result: &browser_mcp::types::ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match c {
            browser_mcp::types::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Convert a browser ToolResult to (success: bool, value: Value).
pub fn browser_result_to_value(result: browser_mcp::types::ToolResult) -> (bool, Value) {
    let text = extract_browser_text(&result);
    if result.is_error {
        (false, json!({"success": false, "error": text}))
    } else {
        let val = serde_json::from_str::<Value>(&text)
            .unwrap_or_else(|_| json!({"result": text.clone(), "raw": text}));
        (true, val)
    }
}

/// Check if the browser has an active page (navigated and connected).
pub async fn browser_is_active(browser: &browser_mcp::browser::SharedBrowser) -> bool {
    let guard = browser.read().await;
    guard.get_url().await.is_ok()
}

// ============ A11Y SNAPSHOT SEARCH ============

/// Search the cached a11y snapshot tree for a node whose accessible name
/// contains `query` (case-insensitive). Returns the ref_id ("ref_N") of the
/// first matching node, if any.
pub fn search_a11y_snapshot(query: &str) -> Option<String> {
    let snapshot = crate::a11y_cache::get_snapshot()?;
    let query_lower = query.to_lowercase();
    search_node(&snapshot, &query_lower)
}

fn search_node(node: &Value, query: &str) -> Option<String> {
    if let Some(name) = node.get("name").and_then(|v| v.as_str()) {
        if name.to_lowercase().contains(query) {
            if let Some(ref_id) = node.get("ref").and_then(|v| v.as_str()) {
                if !ref_id.is_empty() {
                    return Some(ref_id.to_string());
                }
            }
        }
    }
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for child in children {
            if let Some(found) = search_node(child, query) {
                return Some(found);
            }
        }
    }
    None
}

// ============ META-TOOL DISPATCH ============

/// Default meta-tool timeout: 120 seconds.
/// Individual tools may override via their own timeout_ms parameter.
const META_TOOL_TIMEOUT_MS: u64 = 120_000;

/// Handle a meta-tool call by name. Returns None if not a meta-tool.
///
/// Phase C fix1: wrapped in panic boundary + global timeout.
/// A meta-tool failure must NEVER crash hands.exe — it returns a structured error.
pub async fn handle_meta_tool(
    name: &str,
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
    session: &SharedSession,
) -> Option<Value> {
    let fut = dispatch_meta_tool(name, args, browser, session);
    let result_opt = fut.await;
    result_opt
}

/// Inner dispatch — separated so handle_meta_tool can wrap with panic/timeout safety.
async fn dispatch_meta_tool(
    name: &str,
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
    session: &SharedSession,
) -> Option<Value> {
    // Global timeout for any meta-tool call
    let timeout = std::time::Duration::from_millis(
        args.get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(META_TOOL_TIMEOUT_MS),
    );

    let result = match name {
        // Phase A
        "hands_read_page" => {
            Some(tokio::time::timeout(timeout, read_page::handle(args, browser, session)).await)
        }
        "hands_click" => {
            Some(tokio::time::timeout(timeout, click::handle(args, browser, session)).await)
        }
        "hands_navigate" => {
            Some(tokio::time::timeout(timeout, navigate::handle(args, browser, session)).await)
        }
        "hands_capture" => {
            Some(tokio::time::timeout(timeout, capture::handle(args, browser, session)).await)
        }
        // Phase B
        "hands_find" => {
            Some(tokio::time::timeout(timeout, find::handle(args, browser, session)).await)
        }
        "hands_type" => {
            Some(tokio::time::timeout(timeout, type_text::handle(args, browser, session)).await)
        }
        "hands_fill_form" => {
            Some(tokio::time::timeout(timeout, fill_form::handle(args, browser, session)).await)
        }
        // Phase C
        "hands_verify" => {
            Some(tokio::time::timeout(timeout, verify::handle(args, browser, session)).await)
        }
        "hands_verify_expectations" => Some(
            tokio::time::timeout(timeout, verify_expectations::handle(args, browser, session))
                .await,
        ),
        "hands_scan_qr" => {
            Some(tokio::time::timeout(timeout, qr_scan::handle(args, browser, session)).await)
        }
        "hands_app_action" => {
            Some(tokio::time::timeout(timeout, app_action::handle(args, browser, session)).await)
        }
        "hands_script" => Some(
            tokio::time::timeout(timeout, Box::pin(script::handle(args, browser, session))).await,
        ),
        "hands_login_recovery" => {
            let script_payload = templates::login::build_login_script(args);
            Some(
                tokio::time::timeout(
                    timeout,
                    Box::pin(script::handle(&script_payload, browser, session)),
                )
                .await,
            )
        }
        _ => None,
    };

    match result {
        Some(Ok(value)) => Some(value),
        Some(Err(_elapsed)) => {
            eprintln!(
                "[hands] META-TOOL TIMEOUT: '{}' exceeded {}ms",
                name,
                timeout.as_millis()
            );
            Some(json!({
                "success": false,
                "error": format!("Meta-tool '{}' timed out after {}ms", name, timeout.as_millis()),
                "timeout": true,
                "method": name,
            }))
        }
        None => None,
    }
}

// ============ TOOL DEFINITIONS ============

/// Return tool definitions for all meta-tools (Phase A + B).
/// Prepended to tools/list so Claude defaults to these over primitives.
pub fn meta_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "hands_read_page",
            "description": "RECOMMENDED: Fetch readable content from any URL. Auto-escalates HTTP scrape → Node.js rendering → Chrome as needed — no manual browser setup required. Use instead of browser_http_scrape / browser_smart_browse / browser_navigate + browser_extract_content chains.",
            "recommended_for": ["web content extraction", "page reading", "scraping"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "URL to fetch"
                    },
                    "wait_for": {
                        "type": "string",
                        "description": "CSS selector to wait for before extracting (optional). Forces Chrome escalation."
                    },
                    "extract_mode": {
                        "type": "string",
                        "enum": ["text", "html", "markdown"],
                        "default": "text",
                        "description": "Extraction format"
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "default": 15000,
                        "description": "Overall timeout in ms"
                    }
                },
                "required": ["url"]
            }
        }),
        json!({
            "name": "hands_click",
            "description": "RECOMMENDED: Click any visible element across browser and desktop. Auto-escalates: a11y ref → fuzzy text → CSS → coordinates → UIA → OCR. Tags reversibility (Reversible | RequiresConfirmation | Destructive) based on target text. When `offset_x`/`offset_y` are set, every rung resolves the element's bounding box and clicks at `center + offset` via coord-click — subsumes the legacy `find_and_click` combo tool. Use instead of browser_click / uia_click / find_and_click.",
            "recommended_for": ["clicking", "button press", "link navigation", "UI interaction"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "Text label, CSS selector, or a11y ref (e.g. 'Submit', '#btn', 'ref_12')"
                    },
                    "page_context": {
                        "type": "string",
                        "enum": ["auto", "browser", "desktop"],
                        "default": "auto",
                        "description": "Where to look: auto tries browser first if active, then desktop"
                    },
                    "double_click": { "type": "boolean", "default": false },
                    "right_click": { "type": "boolean", "default": false },
                    "strict": {
                        "type": "boolean",
                        "default": false,
                        "description": "Error on ambiguity instead of best-guess"
                    },
                    "allow_destructive": {
                        "type": "boolean",
                        "default": false,
                        "description": "Allow clicking targets classified as destructive (Delete, Pay, etc.)"
                    },
                    "offset_x": {
                        "type": "integer",
                        "default": 0,
                        "description": "Horizontal pixel offset. When non-zero (alone or with offset_y), forces every rung to resolve the discovered element's bounding-box center and click at center+offset via coord-click. With both offsets at zero, rungs 1-4 use the more reliable ref/selector click instead. Applies to all 7 rungs."
                    },
                    "offset_y": {
                        "type": "integer",
                        "default": 0,
                        "description": "Vertical pixel offset. When non-zero (alone or with offset_x), forces every rung to resolve the discovered element's bounding-box center and click at center+offset via coord-click. With both offsets at zero, rungs 1-4 use the more reliable ref/selector click instead. Applies to all 7 rungs."
                    }
                },
                "required": ["target"]
            }
        }),
        json!({
            "name": "hands_navigate",
            "description": "RECOMMENDED: Navigate browser to a URL and wait for interaction-ready state. Auto-launches browser if not running. Records owning monitor for multi-monitor awareness. Always reversible (back button). Use instead of browser_launch + browser_navigate + browser_wait_for chains.",
            "recommended_for": ["navigation", "page load", "browser launch", "URL open"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "URL to navigate to"
                    },
                    "wait_condition": {
                        "type": "string",
                        "description": "CSS selector, 'networkidle', 'load', or 'domcontentloaded'"
                    },
                    "visible": {
                        "type": "boolean",
                        "default": true,
                        "description": "true = visible Chrome (default). false = headless."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "default": 30000,
                        "description": "Overall timeout in ms"
                    }
                },
                "required": ["url"]
            }
        }),
        json!({
            "name": "hands_capture",
            "description": "RECOMMENDED: Screenshot a target with optional OCR text verification. Routes to browser, window, or full-screen capture automatically. Multi-monitor aware. Optional `window_title` focuses a named window via UIA before any capture (orthogonal to `target`) — subsumes the legacy `read_screen_text` combo tool. Use instead of browser_screenshot + vision_ocr chains.",
            "recommended_for": ["screenshot", "screen capture", "OCR", "visual verification"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "'browser', a window title, 'screen' (default), or CSS selector"
                    },
                    "verify": {
                        "type": "string",
                        "description": "Expected text to find via OCR. Returns verified:true/false."
                    },
                    "ocr": {
                        "type": "boolean",
                        "default": true,
                        "description": "Run OCR (default true when verify is set)"
                    },
                    "save_path": {
                        "type": "string",
                        "description": "Save screenshot to file path"
                    },
                    "detailed_ocr": {
                        "type": "boolean",
                        "default": false,
                        "description": "Include word-level bounding boxes in OCR results"
                    },
                    "window_title": {
                        "type": "string",
                        "description": "Optional: focus this window via UIA (~200ms settle) before capture. Orthogonal to `target` — works with target='screen' or target='browser'. For window-only capture, set this AND target=<same title>."
                    }
                }
            }
        }),
        // ── Phase B tools ──
        json!({
            "name": "hands_find",
            "description": "RECOMMENDED: Find any element across browser and desktop. 6-rung ladder: a11y text → a11y role → clickables → UIA → OCR → template. Returns ref, coords, or text match with confidence. Use return_type='ref' for elements you'll interact with next. Use instead of browser_a11y_find / uia_find / vision_ocr search chains.",
            "recommended_for": ["find element", "locate", "search UI", "element lookup"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "Text label, name, or description of the element to find"
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["auto", "browser", "desktop", "screen"],
                        "default": "auto",
                        "description": "Where to search: auto tries browser first, then desktop"
                    },
                    "return_type": {
                        "type": "string",
                        "enum": ["any", "ref"],
                        "default": "any",
                        "description": "'ref' = only return a11y refs (short-circuits after rung 3). 'any' = ref, coords, or text."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "default": 10000,
                        "description": "Overall timeout budget in ms"
                    }
                },
                "required": ["target"]
            }
        }),
        json!({
            "name": "hands_type",
            "description": "RECOMMENDED: Type text into any input field across browser and desktop. Finds element, verifies focus, clears existing content, types with chunked input for long strings. Sensitive fields (password, phone) always use keystroke simulation. Use instead of browser_type / uia_type / type_into_window.",
            "recommended_for": ["typing", "text input", "form field", "enter text"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "Label, name, or selector of the input field"
                    },
                    "text": {
                        "type": "string",
                        "description": "Text to type into the field"
                    },
                    "clear_first": {
                        "type": "boolean",
                        "default": true,
                        "description": "Clear existing field content before typing"
                    },
                    "verify_focus": {
                        "type": "boolean",
                        "default": true,
                        "description": "Verify focus landed on an input element before typing"
                    },
                    "fast_set": {
                        "type": "boolean",
                        "description": "Use JS direct-set for speed (rejected on sensitive fields like password)"
                    },
                    "submit": {
                        "type": "boolean",
                        "default": false,
                        "description": "Press Enter after typing to submit"
                    }
                },
                "required": ["target", "text"]
            }
        }),
        json!({
            "name": "hands_fill_form",
            "description": "RECOMMENDED: Fill an entire form with multiple fields. Pre-scans form structure, matches labels to inputs, fills each field with appropriate strategy (type, select, checkbox toggle). Tracks per-field success/failure. Optional auto-submit with spatial proximity tiebreaker for submit button. Use instead of manual hands_click + hands_type sequences.",
            "recommended_for": ["form filling", "sign up", "login", "registration", "checkout"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "fields": {
                        "type": "object",
                        "description": "Object mapping field labels to values. E.g. {\"Email\": \"user@example.com\", \"Password\": \"secret\"}"
                    },
                    "auto_submit": {
                        "type": "boolean",
                        "default": false,
                        "description": "Automatically click submit button after filling. Tags RequiresConfirmation unless true."
                    },
                    "submit_label": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Custom submit button labels to search for. Defaults to ['Submit', 'Sign Up', 'Sign In', 'Continue', ...]"
                    }
                },
                "required": ["fields"]
            }
        }),
        // ── Phase C tools ──
        json!({
            "name": "hands_verify",
            "description": "RECOMMENDED: Verify page state with polling, stabilization, and multi-rung detection. 5-rung ladder: DOM text → a11y snapshot → element query → OCR → UIA. Supports natural language ('shows Welcome'), structured ({text: '...'}), or named templates (verify_page_loaded). Use instead of manual browser_eval + screenshot + OCR chains for post-action verification.",
            "recommended_for": ["verify", "check", "assert", "confirm state", "page ready", "element visible"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Text to verify is present (or absent if negated=true)"
                    },
                    "regex": {
                        "type": "string",
                        "description": "Regex pattern to match against page content"
                    },
                    "element": {
                        "type": "string",
                        "description": "CSS selector to check for presence/absence"
                    },
                    "natural_text": {
                        "type": "string",
                        "description": "Natural language phrase like 'shows Welcome', 'no error', 'page loaded', 'Submit is visible'"
                    },
                    "template": {
                        "type": "string",
                        "enum": ["verify_page_loaded", "verify_login_success", "verify_form_submitted", "verify_error_displayed", "verify_modal_present", "verify_navigation_completed"],
                        "description": "Named verification template for common patterns"
                    },
                    "template_args": {
                        "type": "object",
                        "description": "Arguments for template (e.g. {from_url: '...', target_url: '...', success_text: '...'})"
                    },
                    "negated": {
                        "type": "boolean",
                        "default": false,
                        "description": "Invert check: verify target is ABSENT rather than present"
                    },
                    "require_visible": {
                        "type": "boolean",
                        "default": false,
                        "description": "Require target is within viewport (not scrolled off or hidden)"
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "default": 5000,
                        "description": "Polling timeout in ms. 0 = single check, no polling."
                    },
                    "must_stabilize_ms": {
                        "type": "integer",
                        "default": 0,
                        "description": "Require N consecutive ms of matching before reporting success"
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["auto", "browser", "desktop"],
                        "default": "auto",
                        "description": "Where to verify: auto tries browser first, then desktop"
                    }
                }
            }
        }),
        json!({
            "name": "hands_scan_qr",
            "description": "Scan a QR code from screen or browser, decode it, and validate otpauth:// URI format. Returns the decoded URI for registration via workflow:totp_register_from_uri. Supports full-screen capture or browser-only capture with optional region cropping.",
            "recommended_for": ["QR code", "2FA setup", "TOTP registration", "authenticator"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "enum": ["screen", "browser"],
                        "default": "screen",
                        "description": "'screen' captures the full display, 'browser' captures the active browser page"
                    },
                    "region": {
                        "type": "object",
                        "description": "Optional crop region for screen capture",
                        "properties": {
                            "x": { "type": "integer" },
                            "y": { "type": "integer" },
                            "width": { "type": "integer" },
                            "height": { "type": "integer" }
                        }
                    }
                }
            }
        }),
        json!({
            "name": "hands_app_action",
            "description": "RECOMMENDED: Window management — open, close, focus, minimize, maximize, restore, snap. Handles save dialogs on close (Auto/Save/Discard/Ask). Multi-monitor aware with monitor stickiness. Post-action verification included. Use instead of manual uia_app_launch + uia_focus_window + uia_window_state + uia_key_press chains.",
            "recommended_for": ["open app", "close window", "focus window", "minimize", "maximize", "snap window", "window management"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["open", "close", "focus", "minimize", "maximize", "restore", "snap_left", "snap_right", "snap_top", "snap_bottom"],
                        "description": "Window action to perform"
                    },
                    "launch_spec": {
                        "type": "string",
                        "description": "For 'open': exe path, Start menu name, or app URI (e.g. 'notepad', 'C:\\\\Windows\\\\notepad.exe', 'ms-settings:')"
                    },
                    "window_match": {
                        "type": "object",
                        "description": "Criteria for targeting an existing window",
                        "properties": {
                            "title": {
                                "type": "string",
                                "description": "Window title substring (case-insensitive contains match)"
                            },
                            "process": {
                                "type": "string",
                                "description": "Process name (case-insensitive exact match, e.g. 'notepad.exe')"
                            },
                            "automation_id": {
                                "type": "string",
                                "description": "UIA automation ID (exact match)"
                            }
                        }
                    },
                    "match_mode": {
                        "type": "string",
                        "enum": ["first", "last_focused", "require_unique", "all"],
                        "default": "last_focused",
                        "description": "How to resolve multiple matching windows"
                    },
                    "on_save_dialog": {
                        "type": "string",
                        "enum": ["auto", "save", "discard", "ask"],
                        "default": "auto",
                        "description": "For 'close': how to handle unsaved-changes dialogs"
                    },
                    "monitor": {
                        "type": ["string", "integer"],
                        "description": "Target monitor: 'current', 'primary', 'owning', or monitor index (0-based)"
                    },
                    "wait_ready": {
                        "type": "boolean",
                        "default": true,
                        "description": "Wait for window to be interaction-ready after action"
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "default": 10000,
                        "description": "Timeout for the operation in ms"
                    },
                    "force_close": {
                        "type": "boolean",
                        "default": false,
                        "description": "For 'close': force-close even if dialog handling fails"
                    }
                },
                "required": ["action"]
            }
        }),
        // ── Phase C Track 4 tools ──
        json!({
            "name": "hands_script",
            "description": "RECOMMENDED: Execute a multi-step automation script. Chains meta-tool calls with {{var}} substitution, output capture, per-step error handling (stop/skip/retry), and per-step + overall timeouts. Use for login flows, form wizards, multi-page workflows. Use instead of manual meta-tool call sequences.",
            "recommended_for": ["automation", "multi-step", "workflow", "scripted sequence", "login flow"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "description": "Ordered list of meta-tool steps to execute",
                        "items": {
                            "type": "object",
                            "properties": {
                                "tool": {
                                    "type": "string",
                                    "description": "Meta-tool name (e.g. 'hands_navigate', 'hands_click', 'hands_verify')"
                                },
                                "args": {
                                    "type": "object",
                                    "description": "Arguments for the meta-tool. Supports {{var}} and {{var.field}} substitution."
                                },
                                "label": {
                                    "type": "string",
                                    "description": "Human-readable step label for logging and error reporting"
                                },
                                "output_var": {
                                    "type": "string",
                                    "description": "Store step result in this variable name for later {{output_var}} references"
                                },
                                "on_error": {
                                    "type": "string",
                                    "enum": ["stop", "skip", "retry"],
                                    "default": "stop",
                                    "description": "Error policy: stop halts script, skip continues, retry retries once then falls back to stop_on_error"
                                },
                                "timeout_ms": {
                                    "type": "integer",
                                    "description": "Per-step timeout override in ms"
                                }
                            },
                            "required": ["tool"]
                        }
                    },
                    "variables": {
                        "type": "object",
                        "description": "Initial variables map. Steps can reference these via {{var}} and add to them via output_var."
                    },
                    "stop_on_error": {
                        "type": "boolean",
                        "default": true,
                        "description": "Default error policy when step has no on_error set"
                    },
                    "verbose": {
                        "type": "boolean",
                        "default": false,
                        "description": "Include full per-step results with rungs_tried in response"
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "default": 60000,
                        "description": "Overall script timeout in ms"
                    }
                },
                "required": ["steps"]
            }
        }),
        json!({
            "name": "hands_login_recovery",
            "description": "RECOMMENDED: 5-stage login recovery pipeline. Navigates to URL, checks if already logged in, fills credentials, handles 2FA, discovers OAuth options, checks Remember Me, and verifies success. Built on hands_script — returns full step-by-step results. Provide credentials or credential_name for auto-fill, totp_name for auto-2FA.",
            "recommended_for": ["login", "sign in", "authentication", "session recovery", "2FA"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Login page URL"
                    },
                    "credential_name": {
                        "type": "string",
                        "description": "Name of stored credential (workflow vault) for auto-lookup"
                    },
                    "username": {
                        "type": "string",
                        "description": "Username or email to fill"
                    },
                    "password": {
                        "type": "string",
                        "description": "Password to fill"
                    },
                    "totp_name": {
                        "type": "string",
                        "description": "Name of TOTP credential for automatic 2FA code generation"
                    },
                    "auto_remember": {
                        "type": "boolean",
                        "default": true,
                        "description": "Check 'Remember me' checkbox if found"
                    },
                    "success_text": {
                        "type": "string",
                        "description": "Text to verify after successful login (e.g. 'Dashboard', 'Welcome')"
                    },
                    "success_url_contains": {
                        "type": "string",
                        "description": "URL substring indicating successful login (e.g. '/dashboard')"
                    }
                },
                "required": ["url"]
            }
        }),
        // ── Phase D — Self-improvement tools ──
        json!({
            "name": "hands_self_record_start",
            "description": "Begin a self-recording session: generate a deterministic flow_name from `task_description`, persist intent metadata to the local index, and return a call plan for workflow:flow_record_start. The caller invokes the returned plan, performs the task, then closes with hands_self_record_stop_and_optimize. Plan-not-action: hands itself does not call workflow:* tools.",
            "recommended_for": ["self-recording", "automation memory", "replay loop", "learn from session"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_description": {
                        "type": "string",
                        "description": "Natural-language description of the task being recorded. Used both as the workflow flow description and as the cosine-similarity index key for future lookups."
                    }
                },
                "required": ["task_description"]
            }
        }),
        json!({
            "name": "hands_self_record_lookup",
            "description": "Search the local self-record index for prior recordings similar to `task_description` (cosine similarity over token-frequency vectors with English-stopword filtering). Returns matching candidates with replay_call_plan stubs. Use BEFORE hands_self_record_start to check if a replay already exists.",
            "recommended_for": ["replay lookup", "automation memory", "find prior recording", "self-record search"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_description": {
                        "type": "string",
                        "description": "Natural-language description of the task to search for."
                    },
                    "threshold": {
                        "type": "number",
                        "default": 0.7,
                        "description": "Minimum cosine similarity (0.0–1.0) required for a record to be returned as a candidate."
                    }
                },
                "required": ["task_description"]
            }
        }),
        json!({
            "name": "hands_self_record_stop_and_optimize",
            "description": "Two-stage orchestrator that closes a self-recording, replays it (dry_run) to get the captured steps, prunes obvious noise (consecutive scrolls, failed-then-success clicks, unreferenced screenshots, instant wait_for), and decides whether to replace a longer existing flow with the shorter pruned one. Stage 1 (no recorded_steps) returns an orchestration_plan to invoke workflow:flow_record_stop + workflow:flow_replay; Stage 2 (recorded_steps populated) returns a replace_plan or 'keep' decision. Plan-not-action: hands itself does not call workflow:* tools.",
            "recommended_for": ["self-recording", "flow optimization", "replay pruning"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Flow name returned earlier by hands_self_record_start."
                    },
                    "recorded_steps": {
                        "description": "OPTIONAL on first call. After running the Stage-1 orchestration_plan, re-invoke this tool with `recorded_steps` set to the `steps` array from the workflow:flow_replay response."
                    }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "hands_verify_expectations",
            "description": "Run a list of post-flow expectations and report per-expectation pass/fail with polling, predicates (equals/contains/not_contains/not_equals/regex/gt/lt/in_range), and tolerance (polls, interval_ms, timeout_per_check_ms, string_distance for fuzzy match via Levenshtein). Pairs with hands_self_record: a saved flow has implicit expectations (final state matches first successful run). Supported check_tools: browser_get_text, browser_exists, vision_ocr_text_contains, uia_text_exists.",
            "recommended_for": ["verify expectations", "post-flow assert", "validate replay", "test harness", "regression check"],
            "inputSchema": {
                "type": "object",
                "properties": {
                    "expectations": {
                        "type": "array",
                        "description": "List of expectation specifications",
                        "items": {
                            "type": "object",
                            "properties": {
                                "label": { "type": "string", "description": "Human-readable name for this expectation (appears in report)" },
                                "check_tool": { "type": "string", "enum": ["browser_get_text", "browser_exists", "vision_ocr_text_contains", "uia_text_exists"] },
                                "args": { "type": "object", "description": "Arguments forwarded to the check_tool" },
                                "expected_predicate": {
                                    "type": "object",
                                    "properties": {
                                        "type": { "type": "string", "enum": ["equals", "contains", "not_contains", "not_equals", "regex", "gt", "lt", "in_range"] },
                                        "value": { "description": "Expected value (or {min, max} for in_range)" }
                                    },
                                    "required": ["type"]
                                },
                                "tolerance": {
                                    "type": "object",
                                    "properties": {
                                        "polls": { "type": "integer", "default": 3 },
                                        "interval_ms": { "type": "integer", "default": 500 },
                                        "timeout_per_check_ms": { "type": "integer", "default": 5000 },
                                        "string_distance": { "type": "integer", "default": 0 }
                                    }
                                }
                            },
                            "required": ["check_tool", "expected_predicate"]
                        }
                    }
                },
                "required": ["expectations"]
            }
        }),
    ]
}
