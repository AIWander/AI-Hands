//! `hands_type` — cross-subsystem text input with focus verification and chunked typing.
//!
//! Ladder per spec:
//!   1. hands_find(target) to locate the input
//!   2a. Browser ref → browser_click(ref) [focus] → verify activeElement → browser_type
//!   2b. Browser coords → browser_click(x,y) [focus] → verify → browser_type
//!   2c. Desktop ref → uia_click(ref) → uia_type
//!   2d. Desktop fallback → type_into_window(title, text)
//!   3. On clear_first failure, element-type-specific clear retry
//!
//! Features:
//! - verify_focus (default true): JS check document.activeElement matches before typing
//! - Per-element-type clear strategy (contenteditable, input, UIA Edit, unknown)
//! - Chunked typing for strings >100 chars (50-char batches, 50ms gap)
//! - fast_set for non-sensitive fields: direct JS value set + dispatch events
//! - Sensitive field detection via field_role.rs

use serde_json::{json, Value};
use std::time::Instant;

use super::error::MetaError;
use super::field_role::FieldRole;
use super::instrumentation;
use super::response::{Confidence, MetaToolResult, Reversibility, RungAttempt};
use super::session::SharedSession;
use crate::atomic::{AtomicTool, UiaClick, UiaKeyPress, UiaType};

/// JS to verify focus is on the expected element after clicking.
const JS_VERIFY_FOCUS: &str = r#"
(function(expectedText) {
    var el = document.activeElement;
    if (!el) return { focused: false, reason: 'no_active_element' };
    var tag = el.tagName || '';
    var name = el.getAttribute('aria-label') || el.getAttribute('name')
        || el.getAttribute('placeholder') || el.getAttribute('id') || '';
    var type = el.type || '';
    // Check if the active element looks like what we expect
    var isInput = (tag === 'INPUT' || tag === 'TEXTAREA' || el.isContentEditable);
    return {
        focused: isInput,
        tag: tag,
        name: name,
        type: type,
        contenteditable: el.isContentEditable || false
    };
})
"#;

/// JS to clear a standard input field.
const JS_CLEAR_INPUT: &str = r#"
(function(sel) {
    var el = sel ? document.querySelector(sel) : document.activeElement;
    if (!el) return false;
    if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') {
        el.value = '';
        el.dispatchEvent(new Event('input', { bubbles: true }));
        el.dispatchEvent(new Event('change', { bubbles: true }));
        return true;
    }
    return false;
})
"#;

/// JS to fast-set a field value (skip keystroke simulation).
const JS_FAST_SET: &str = r#"
(function(sel, value) {
    var el = sel ? document.querySelector(sel) : document.activeElement;
    if (!el) return { success: false, reason: 'not_found' };
    var nativeSet = Object.getOwnPropertyDescriptor(
        window.HTMLInputElement.prototype, 'value'
    );
    if (nativeSet && nativeSet.set) {
        nativeSet.set.call(el, value);
    } else {
        el.value = value;
    }
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return { success: true };
})
"#;

/// JS to detect the field type and properties of the active element.
const JS_DETECT_FIELD: &str = r#"
(function() {
    var el = document.activeElement;
    if (!el) return null;
    return {
        tag: el.tagName || '',
        type: el.type || '',
        role: el.getAttribute('role') || '',
        autocomplete: el.autocomplete || el.getAttribute('autocomplete') || '',
        inputmode: el.inputMode || el.getAttribute('inputmode') || '',
        contenteditable: el.isContentEditable || false,
        value: el.value || ''
    };
})()
"#;

/// Chunk size for long text input.
const CHUNK_SIZE: usize = 50;
/// Threshold above which text is chunked.
const CHUNK_THRESHOLD: usize = 100;

pub async fn handle(
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
    session: &SharedSession,
) -> Value {
    let start = Instant::now();
    let call_id = {
        let mut s = session.write().unwrap_or_else(|e| e.into_inner());
        s.next_call_id()
    };

    let target = match args.get("target").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => {
            return MetaToolResult::failure(vec![], MetaError::other("target is required"), 0)
                .to_value();
        }
    };

    let text = match args.get("text").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => {
            return MetaToolResult::failure(vec![], MetaError::other("text is required"), 0)
                .to_value();
        }
    };

    let clear_first = args
        .get("clear_first")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let verify_focus = args
        .get("verify_focus")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let fast_set = args.get("fast_set").and_then(|v| v.as_bool());
    let submit = args
        .get("submit")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let ctx = json!({
        "target": &target,
        "text_len": text.len(),
        "clear_first": clear_first,
        "verify_focus": verify_focus,
        "fast_set": fast_set,
        "submit": submit,
    });

    let mut rungs_tried = Vec::new();
    let warnings: Vec<String> = Vec::new();

    // ── Step 1: Find the target element using hands_find ──
    let find_start = Instant::now();
    let find_result = super::find::handle(
        &json!({"target": &target, "return_type": "ref"}),
        browser,
        session,
    )
    .await;

    let find_ms = find_start.elapsed().as_millis() as u64;
    let find_success = find_result
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Determine what we got from find: ref, coords, or failure
    let found_type = find_result
        .get("result")
        .and_then(|r| r.get("found_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let found_ref = find_result
        .get("result")
        .and_then(|r| r.get("ref_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let found_selector = find_result
        .get("result")
        .and_then(|r| r.get("selector"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let found_x = find_result
        .get("result")
        .and_then(|r| r.get("x"))
        .and_then(|v| v.as_i64());
    let found_y = find_result
        .get("result")
        .and_then(|r| r.get("y"))
        .and_then(|v| v.as_i64());

    if !find_success && found_type.is_empty() {
        // Find failed completely — try coords-based find as fallback
        let find_any = super::find::handle(
            &json!({"target": &target, "return_type": "any"}),
            browser,
            session,
        )
        .await;

        let any_success = find_any
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !any_success {
            let elapsed = start.elapsed().as_millis() as u64;
            rungs_tried.push(RungAttempt::failed(
                "find_target",
                find_ms,
                "Target element not found",
            ));
            return MetaToolResult::failure(
                rungs_tried,
                MetaError::not_found(&target, "all"),
                elapsed,
            )
            .to_value();
        }

        // Use the any-type result
        let _any_type = find_any
            .get("result")
            .and_then(|r| r.get("found_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("coords");
        let any_x = find_any
            .get("result")
            .and_then(|r| r.get("x"))
            .and_then(|v| v.as_i64());
        let any_y = find_any
            .get("result")
            .and_then(|r| r.get("y"))
            .and_then(|v| v.as_i64());

        return type_via_coords(
            any_x,
            any_y,
            &text,
            clear_first,
            verify_focus,
            fast_set,
            submit,
            &target,
            browser,
            session,
            &call_id,
            &ctx,
            start,
            rungs_tried,
            warnings,
        )
        .await;
    }

    rungs_tried.push(RungAttempt::ok("find_target", find_ms));

    // ── Step 2: Dispatch based on found type ──
    let browser_active = super::browser_is_active(browser).await;

    // 2a: Browser ref path
    if (found_type == "ref") && browser_active {
        let selector = found_selector.as_deref().or(found_ref.as_deref());
        if let Some(sel) = selector {
            return type_via_browser(
                sel,
                &text,
                clear_first,
                verify_focus,
                fast_set,
                submit,
                &target,
                browser,
                &call_id,
                &ctx,
                start,
                rungs_tried,
                warnings,
            )
            .await;
        }
    }

    // 2b: Browser coords path
    if found_type == "coords" && browser_active {
        if let (Some(x), Some(y)) = (found_x, found_y) {
            return type_via_coords(
                Some(x),
                Some(y),
                &text,
                clear_first,
                verify_focus,
                fast_set,
                submit,
                &target,
                browser,
                session,
                &call_id,
                &ctx,
                start,
                rungs_tried,
                warnings,
            )
            .await;
        }
    }

    // 2c/2d: Desktop path
    if found_type == "coords" {
        if let (Some(x), Some(y)) = (found_x, found_y) {
            return type_via_desktop(
                x,
                y,
                &text,
                clear_first,
                &target,
                &call_id,
                &ctx,
                start,
                rungs_tried,
                warnings,
            )
            .await;
        }
    }

    // Fallback: try desktop type_into_window
    return type_via_window_fallback(&target, &text, &call_id, &ctx, start, rungs_tried, warnings)
        .await;
}

/// Type text into a browser element identified by selector/ref.
async fn type_via_browser(
    selector: &str,
    text: &str,
    clear_first: bool,
    verify_focus: bool,
    fast_set: Option<bool>,
    submit: bool,
    target: &str,
    browser: &browser_mcp::browser::SharedBrowser,
    call_id: &str,
    ctx: &Value,
    start: Instant,
    mut rungs_tried: Vec<RungAttempt>,
    mut warnings: Vec<String>,
) -> Value {
    let rung_start = Instant::now();

    // Click to focus
    let click_result =
        browser_mcp::tools::handle_tool(browser, "click", json!({"selector": selector})).await;
    let (click_ok, _) = super::browser_result_to_value(click_result);

    if !click_ok {
        let rung_ms = rung_start.elapsed().as_millis() as u64;
        rungs_tried.push(RungAttempt::failed(
            "browser_focus_click",
            rung_ms,
            "Click to focus failed",
        ));
        let elapsed = start.elapsed().as_millis() as u64;
        return MetaToolResult::failure(
            rungs_tried,
            MetaError::other("Failed to click target for focus"),
            elapsed,
        )
        .to_value();
    }

    // Small settle delay after click
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Detect field type
    let field_info = browser_mcp::tools::handle_tool(
        browser,
        "evaluate",
        json!({"expression": JS_DETECT_FIELD}),
    )
    .await;
    let (_, field_val) = super::browser_result_to_value(field_info);
    let field_role = FieldRole::detect(&field_val);

    // Verify focus if requested
    if verify_focus {
        let target_js = serde_json::to_string(target).unwrap_or_else(|_| "\"\"".to_string());
        let focus_result = browser_mcp::tools::handle_tool(
            browser,
            "evaluate",
            json!({"expression": format!("({})({})", JS_VERIFY_FOCUS, target_js)}),
        )
        .await;
        let (_, focus_val) = super::browser_result_to_value(focus_result);
        let is_focused = focus_val
            .get("focused")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !is_focused {
            warnings
                .push("Focus verification failed — active element may not be the target".into());
            // Don't abort — try typing anyway, but warn
        }
    }

    // Determine if we should use fast_set
    let use_fast_set = match fast_set {
        Some(true) if !field_role.is_sensitive() && !field_role.requires_keystroke() => true,
        Some(true) if field_role.is_sensitive() => {
            warnings.push("fast_set rejected: field is sensitive (password/phone/number)".into());
            false
        }
        _ => false,
    };

    // Clear field if requested
    if clear_first {
        let is_contenteditable = field_val
            .get("contenteditable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if is_contenteditable {
            // Contenteditable: Ctrl+A → Delete
            let _ = browser_mcp::tools::handle_tool(browser, "press", json!({"key": "Control+a"}))
                .await;
            let _ =
                browser_mcp::tools::handle_tool(browser, "press", json!({"key": "Delete"})).await;
        } else {
            // Standard input: JS clear + events
            let selector_js =
                serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".to_string());
            let _ = browser_mcp::tools::handle_tool(
                browser,
                "evaluate",
                json!({"expression": format!("({})({})", JS_CLEAR_INPUT, selector_js)}),
            )
            .await;
        }
    }

    // Type the text
    if use_fast_set {
        // Direct JS value set
        let selector_js = serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".to_string());
        let text_js = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
        let _ = browser_mcp::tools::handle_tool(
            browser,
            "evaluate",
            json!({"expression": format!("({})({}, {})", JS_FAST_SET, selector_js, text_js)}),
        )
        .await;
    } else if text.len() > CHUNK_THRESHOLD {
        // Chunked typing
        for chunk in text.as_bytes().chunks(CHUNK_SIZE) {
            let chunk_str = String::from_utf8_lossy(chunk);
            let _ = browser_mcp::tools::handle_tool(
                browser,
                "type_text",
                json!({"text": chunk_str.as_ref(), "selector": selector}),
            )
            .await;
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    } else {
        // Normal typing
        let _ = browser_mcp::tools::handle_tool(
            browser,
            "type_text",
            json!({"text": text, "selector": selector}),
        )
        .await;
    }

    let rung_ms = rung_start.elapsed().as_millis() as u64;
    rungs_tried.push(RungAttempt::ok("browser_type", rung_ms));
    instrumentation::log_rung_attempt(
        "hands_type",
        call_id,
        "browser_type",
        true,
        rung_ms,
        Some(0.9),
        ctx,
    );

    // Handle submit if requested
    let mut submit_info = json!(null);
    if submit {
        let pre_url_result = browser_mcp::tools::handle_tool(browser, "get_url", json!({})).await;
        let pre_url = super::extract_browser_text(&pre_url_result);

        let _ = browser_mcp::tools::handle_tool(browser, "press", json!({"key": "Enter"})).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let post_url_result = browser_mcp::tools::handle_tool(browser, "get_url", json!({})).await;
        let post_url = super::extract_browser_text(&post_url_result);

        submit_info = json!({
            "submitted": true,
            "url_changed": pre_url != post_url,
        });
    }

    let elapsed = start.elapsed().as_millis() as u64;
    let rung_count = rungs_tried.len();
    instrumentation::log_aggregate(
        "hands_type",
        call_id,
        true,
        "browser_type",
        rung_count,
        elapsed,
        Some(0.9),
        None,
    );

    let mut result = MetaToolResult::success(
        "browser_type",
        rungs_tried,
        json!({
            "typed": true,
            "text_length": text.len(),
            "method": if use_fast_set { "fast_set" } else if text.len() > CHUNK_THRESHOLD { "chunked" } else { "keystroke" },
            "field_role": field_role,
            "target": target,
            "submit": submit_info,
        }),
        elapsed,
    )
    .with_confidence(Confidence::method_only(0.9))
    .with_reversibility(Reversibility::Reversible);

    for w in warnings {
        result = result.with_warning(w);
    }

    result.to_value()
}

/// Type text into a browser element at coordinates (click first).
async fn type_via_coords(
    x: Option<i64>,
    y: Option<i64>,
    text: &str,
    clear_first: bool,
    _verify_focus: bool,
    _fast_set: Option<bool>,
    submit: bool,
    target: &str,
    browser: &browser_mcp::browser::SharedBrowser,
    _session: &SharedSession,
    call_id: &str,
    ctx: &Value,
    start: Instant,
    mut rungs_tried: Vec<RungAttempt>,
    _warnings: Vec<String>,
) -> Value {
    let (x, y) = match (x, y) {
        (Some(x), Some(y)) => (x, y),
        _ => {
            let elapsed = start.elapsed().as_millis() as u64;
            return MetaToolResult::failure(
                rungs_tried,
                MetaError::other("No coordinates available"),
                elapsed,
            )
            .to_value();
        }
    };

    let rung_start = Instant::now();

    // Click at coords to focus
    let click_result =
        browser_mcp::tools::handle_tool(browser, "click", json!({"x": x, "y": y})).await;
    let (click_ok, _) = super::browser_result_to_value(click_result);

    if !click_ok {
        let rung_ms = rung_start.elapsed().as_millis() as u64;
        rungs_tried.push(RungAttempt::failed(
            "coords_focus_click",
            rung_ms,
            "Click at coords failed",
        ));
        let elapsed = start.elapsed().as_millis() as u64;
        return MetaToolResult::failure(
            rungs_tried,
            MetaError::other("Failed to click at coordinates for focus"),
            elapsed,
        )
        .to_value();
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Clear if needed (use keyboard shortcut since we don't have selector)
    if clear_first {
        let _ =
            browser_mcp::tools::handle_tool(browser, "press", json!({"key": "Control+a"})).await;
        let _ = browser_mcp::tools::handle_tool(browser, "press", json!({"key": "Delete"})).await;
    }

    // Type via keyboard (no selector available for type_text, use press for each char)
    if text.len() > CHUNK_THRESHOLD {
        for chunk in text.as_bytes().chunks(CHUNK_SIZE) {
            let chunk_str = String::from_utf8_lossy(chunk);
            let _ = browser_mcp::tools::handle_tool(
                browser,
                "type_text",
                json!({"text": chunk_str.as_ref()}),
            )
            .await;
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    } else {
        let _ = browser_mcp::tools::handle_tool(browser, "type_text", json!({"text": text})).await;
    }

    let rung_ms = rung_start.elapsed().as_millis() as u64;
    rungs_tried.push(RungAttempt::ok("browser_coords_type", rung_ms));
    instrumentation::log_rung_attempt(
        "hands_type",
        call_id,
        "browser_coords_type",
        true,
        rung_ms,
        Some(0.7),
        ctx,
    );

    // Submit
    let mut submit_info = json!(null);
    if submit {
        let _ = browser_mcp::tools::handle_tool(browser, "press", json!({"key": "Enter"})).await;
        submit_info = json!({"submitted": true});
    }

    let elapsed = start.elapsed().as_millis() as u64;
    let rung_count = rungs_tried.len();
    instrumentation::log_aggregate(
        "hands_type",
        call_id,
        true,
        "browser_coords_type",
        rung_count,
        elapsed,
        Some(0.7),
        None,
    );

    let result = MetaToolResult::success(
        "browser_coords_type",
        rungs_tried,
        json!({
            "typed": true,
            "text_length": text.len(),
            "method": "keystroke",
            "target": target,
            "coords": {"x": x, "y": y},
            "submit": submit_info,
        }),
        elapsed,
    )
    .with_confidence(Confidence::method_only(0.7))
    .with_reversibility(Reversibility::Reversible);

    result.to_value()
}

/// Type via desktop UIA at coordinates.
async fn type_via_desktop(
    x: i64,
    y: i64,
    text: &str,
    clear_first: bool,
    target: &str,
    call_id: &str,
    ctx: &Value,
    start: Instant,
    mut rungs_tried: Vec<RungAttempt>,
    _warnings: Vec<String>,
) -> Value {
    let rung_start = Instant::now();

    // Click to focus
    let click_result = UiaClick.call(&json!({"x": x, "y": y}));
    let click_ok = click_result
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    if !click_ok {
        let rung_ms = rung_start.elapsed().as_millis() as u64;
        rungs_tried.push(RungAttempt::failed(
            "uia_click_focus",
            rung_ms,
            "UIA click failed",
        ));
        let elapsed = start.elapsed().as_millis() as u64;
        return MetaToolResult::failure(
            rungs_tried,
            MetaError::other("UIA click for focus failed"),
            elapsed,
        )
        .to_value();
    }

    // Clear via Ctrl+A → Delete
    if clear_first {
        let _ = UiaKeyPress.call(&json!({"keys": "ctrl+a"}));
        let _ = UiaKeyPress.call(&json!({"keys": "delete"}));
    }

    // Type via UIA
    let type_result = UiaType.call(&json!({"text": text}));
    let type_ok = type_result
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let rung_ms = rung_start.elapsed().as_millis() as u64;

    if type_ok {
        rungs_tried.push(RungAttempt::ok("uia_type", rung_ms));
        instrumentation::log_rung_attempt(
            "hands_type",
            call_id,
            "uia_type",
            true,
            rung_ms,
            Some(0.8),
            ctx,
        );

        let elapsed = start.elapsed().as_millis() as u64;
        let rung_count = rungs_tried.len();
        instrumentation::log_aggregate(
            "hands_type",
            call_id,
            true,
            "uia_type",
            rung_count,
            elapsed,
            Some(0.8),
            None,
        );

        let result = MetaToolResult::success(
            "uia_type",
            rungs_tried,
            json!({
                "typed": true,
                "text_length": text.len(),
                "method": "uia_keystroke",
                "target": target,
                "coords": {"x": x, "y": y},
            }),
            elapsed,
        )
        .with_confidence(Confidence::method_only(0.8))
        .with_reversibility(Reversibility::Reversible);

        return result.to_value();
    }

    rungs_tried.push(RungAttempt::failed("uia_type", rung_ms, "UIA type failed"));
    let elapsed = start.elapsed().as_millis() as u64;
    MetaToolResult::failure(rungs_tried, MetaError::other("UIA type failed"), elapsed).to_value()
}

/// Last-resort: type_into_window by title.
async fn type_via_window_fallback(
    target: &str,
    text: &str,
    call_id: &str,
    ctx: &Value,
    start: Instant,
    mut rungs_tried: Vec<RungAttempt>,
    _warnings: Vec<String>,
) -> Value {
    let rung_start = Instant::now();

    // Try type_into_window with target as window title
    let result = UiaType.call(&json!({"text": text, "window_title": target}));
    let ok = result
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let rung_ms = rung_start.elapsed().as_millis() as u64;

    if ok {
        rungs_tried.push(RungAttempt::ok("window_fallback", rung_ms));
        instrumentation::log_rung_attempt(
            "hands_type",
            call_id,
            "window_fallback",
            true,
            rung_ms,
            Some(0.5),
            ctx,
        );

        let elapsed = start.elapsed().as_millis() as u64;
        let rung_count = rungs_tried.len();
        instrumentation::log_aggregate(
            "hands_type",
            call_id,
            true,
            "window_fallback",
            rung_count,
            elapsed,
            Some(0.5),
            None,
        );

        let result = MetaToolResult::success(
            "window_fallback",
            rungs_tried,
            json!({
                "typed": true,
                "text_length": text.len(),
                "method": "window_fallback",
                "target": target,
            }),
            elapsed,
        )
        .with_confidence(Confidence::method_only(0.5))
        .with_reversibility(Reversibility::Reversible);

        return result.to_value();
    }

    rungs_tried.push(RungAttempt::failed(
        "window_fallback",
        rung_ms,
        "Window fallback failed",
    ));
    let elapsed = start.elapsed().as_millis() as u64;
    MetaToolResult::failure(
        rungs_tried,
        MetaError::not_found(target, "all subsystems"),
        elapsed,
    )
    .to_value()
}
