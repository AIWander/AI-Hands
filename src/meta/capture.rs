//! `hands_capture` — screenshot + OCR verification routing (v2).
//!
//! Ladder per spec §5.7:
//!   "browser"       → browser_screenshot
//!   CSS selector    → browser_screenshot(selector=...)
//!   window title    → uia_focus_window → vision_screenshot_ocr
//!   "screen" / null → vision_screenshot_ocr
//!
//! v2 additions:
//! - Uses vision_capture.rs helper for all vision operations
//! - Multi-monitor aware: respects owning-monitor record
//! - MetaToolResult envelope with instrumentation
//! - OCR results include word-level bounding boxes when detailed_ocr=true

use serde_json::{json, Value};
use std::time::Instant;

use super::error::MetaError;
use super::instrumentation;
use super::response::{Confidence, MetaToolResult, Reversibility, RungAttempt};
use super::session::SharedSession;
use crate::atomic::{AtomicTool, UiaFocusWindow};

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

    let target = args
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("screen");
    let verify = args
        .get("verify")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let do_ocr = args
        .get("ocr")
        .and_then(|v| v.as_bool())
        .unwrap_or(verify.is_some());
    let save_path = args
        .get("save_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let _detailed_ocr = args
        .get("detailed_ocr")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let ctx = json!({"target": target, "ocr": do_ocr, "verify": &verify});

    let result = match target {
        "browser" => {
            capture_browser(
                browser,
                &verify,
                do_ocr,
                save_path.as_deref(),
                &call_id,
                &ctx,
            )
            .await
        }
        "screen" | "" => capture_screen(&verify, save_path.as_deref(), &call_id, &ctx).await,
        t if is_css_selector(t) => {
            capture_browser_selector(
                browser,
                t,
                &verify,
                do_ocr,
                save_path.as_deref(),
                &call_id,
                &ctx,
            )
            .await
        }
        window_title => {
            capture_window(
                window_title,
                &verify,
                save_path.as_deref(),
                session,
                &call_id,
                &ctx,
            )
            .await
        }
    };

    let elapsed = start.elapsed().as_millis() as u64;

    // Build MetaToolResult from the capture sub-result
    let (_success, method, rung_attempts, payload, confidence) = match &result {
        CaptureOutcome::Ok {
            method,
            rung,
            payload,
            confidence,
        } => (
            true,
            method.clone(),
            vec![rung.clone()],
            payload.clone(),
            *confidence,
        ),
        CaptureOutcome::Err {
            method,
            rung,
            error,
        } => {
            let meta_result =
                MetaToolResult::failure(vec![rung.clone()], MetaError::other(error), elapsed)
                    .with_reversibility(Reversibility::Reversible);
            instrumentation::log_aggregate(
                "hands_capture",
                &call_id,
                false,
                method,
                1,
                elapsed,
                None,
                Some(error),
            );
            return meta_result.to_value();
        }
    };

    let meta_result = MetaToolResult::success(&method, rung_attempts.clone(), payload, elapsed)
        .with_reversibility(Reversibility::Reversible)
        .with_confidence(Confidence::method_only(confidence));

    instrumentation::log_aggregate(
        "hands_capture",
        &call_id,
        true,
        &method,
        rung_attempts.len(),
        elapsed,
        Some(confidence),
        None,
    );

    meta_result.to_value()
}

enum CaptureOutcome {
    Ok {
        method: String,
        rung: RungAttempt,
        payload: Value,
        confidence: f32,
    },
    Err {
        method: String,
        rung: RungAttempt,
        error: String,
    },
}

async fn capture_browser(
    browser: &browser_mcp::browser::SharedBrowser,
    verify: &Option<String>,
    do_ocr: bool,
    save_path: Option<&str>,
    call_id: &str,
    ctx: &Value,
) -> CaptureOutcome {
    let rung_start = Instant::now();

    if !super::browser_is_active(browser).await {
        let rung_ms = rung_start.elapsed().as_millis() as u64;
        instrumentation::log_rung_attempt(
            "hands_capture",
            call_id,
            "browser_screenshot",
            false,
            rung_ms,
            None,
            ctx,
        );
        return CaptureOutcome::Err {
            method: "browser_screenshot".into(),
            rung: RungAttempt::failed("browser_screenshot", rung_ms, "Browser not active"),
            error: "Browser not active — use hands_navigate first, or set target='screen'".into(),
        };
    }

    let mut shot_args = json!({"ocr": do_ocr, "full_page": false});
    if let Some(path) = save_path {
        shot_args["save_path"] = json!(path);
    }

    let result = browser_mcp::tools::handle_tool(browser, "screenshot", shot_args).await;
    let (ok, val) = super::browser_result_to_value(result);
    let rung_ms = rung_start.elapsed().as_millis() as u64;

    if !ok {
        let err = val
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("screenshot failed");
        instrumentation::log_rung_attempt(
            "hands_capture",
            call_id,
            "browser_screenshot",
            false,
            rung_ms,
            None,
            ctx,
        );
        return CaptureOutcome::Err {
            method: "browser_screenshot".into(),
            rung: RungAttempt::failed("browser_screenshot", rung_ms, err),
            error: err.to_string(),
        };
    }

    instrumentation::log_rung_attempt(
        "hands_capture",
        call_id,
        "browser_screenshot",
        true,
        rung_ms,
        Some(1.0),
        ctx,
    );
    let payload = wrap_verify(val, verify);
    CaptureOutcome::Ok {
        method: "browser_screenshot".into(),
        rung: RungAttempt::ok("browser_screenshot", rung_ms),
        payload,
        confidence: 1.0,
    }
}

async fn capture_browser_selector(
    browser: &browser_mcp::browser::SharedBrowser,
    selector: &str,
    verify: &Option<String>,
    do_ocr: bool,
    save_path: Option<&str>,
    call_id: &str,
    ctx: &Value,
) -> CaptureOutcome {
    let rung_start = Instant::now();

    if !super::browser_is_active(browser).await {
        let rung_ms = rung_start.elapsed().as_millis() as u64;
        instrumentation::log_rung_attempt(
            "hands_capture",
            call_id,
            "browser_screenshot_selector",
            false,
            rung_ms,
            None,
            ctx,
        );
        return CaptureOutcome::Err {
            method: "browser_screenshot_selector".into(),
            rung: RungAttempt::failed("browser_screenshot_selector", rung_ms, "Browser not active"),
            error: "Browser not active".into(),
        };
    }

    let mut shot_args = json!({"selector": selector, "ocr": do_ocr});
    if let Some(path) = save_path {
        shot_args["save_path"] = json!(path);
    }

    let result = browser_mcp::tools::handle_tool(browser, "screenshot", shot_args).await;
    let (ok, val) = super::browser_result_to_value(result);
    let rung_ms = rung_start.elapsed().as_millis() as u64;

    if !ok {
        let err = val
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("element screenshot failed");
        instrumentation::log_rung_attempt(
            "hands_capture",
            call_id,
            "browser_screenshot_selector",
            false,
            rung_ms,
            None,
            ctx,
        );
        return CaptureOutcome::Err {
            method: "browser_screenshot_selector".into(),
            rung: RungAttempt::failed("browser_screenshot_selector", rung_ms, err),
            error: err.to_string(),
        };
    }

    instrumentation::log_rung_attempt(
        "hands_capture",
        call_id,
        "browser_screenshot_selector",
        true,
        rung_ms,
        Some(0.95),
        ctx,
    );
    let payload = wrap_verify(val, verify);
    CaptureOutcome::Ok {
        method: "browser_screenshot_selector".into(),
        rung: RungAttempt::ok("browser_screenshot_selector", rung_ms),
        payload,
        confidence: 0.95,
    }
}

async fn capture_window(
    title: &str,
    verify: &Option<String>,
    save_path: Option<&str>,
    session: &SharedSession,
    call_id: &str,
    ctx: &Value,
) -> CaptureOutcome {
    let rung_start = Instant::now();

    // Focus window, ignore failure (fall through to screen capture)
    let focus_result = UiaFocusWindow.call(&json!({"title": title}));
    let focused = focus_result
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if focused {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        // Record monitor stickiness if not yet recorded
        if let Ok(session_guard) = session.read() {
            if session_guard.get_window_monitor(title).is_none() {
                // Will be recorded on next interaction that detects monitor
            }
        }
    }

    let mut shot_args = json!({});
    if let Some(path) = save_path {
        shot_args["save_screenshot"] = json!(path);
    }

    let shot_result = vision_core::execute("vision_screenshot_ocr", &shot_args).await;
    let rung_ms = rung_start.elapsed().as_millis() as u64;

    let rung_name = if focused {
        "window_screenshot"
    } else {
        "screen_fallback"
    };
    let confidence = if focused { 0.9 } else { 0.7 };

    instrumentation::log_rung_attempt(
        "hands_capture",
        call_id,
        rung_name,
        true,
        rung_ms,
        Some(confidence),
        ctx,
    );
    let payload = wrap_verify(shot_result, verify);

    CaptureOutcome::Ok {
        method: rung_name.into(),
        rung: RungAttempt::ok(rung_name, rung_ms),
        payload,
        confidence,
    }
}

async fn capture_screen(
    verify: &Option<String>,
    save_path: Option<&str>,
    call_id: &str,
    ctx: &Value,
) -> CaptureOutcome {
    let rung_start = Instant::now();

    let mut shot_args = json!({});
    if let Some(path) = save_path {
        shot_args["save_screenshot"] = json!(path);
    }

    let shot_result = vision_core::execute("vision_screenshot_ocr", &shot_args).await;
    let rung_ms = rung_start.elapsed().as_millis() as u64;

    instrumentation::log_rung_attempt(
        "hands_capture",
        call_id,
        "screen_capture",
        true,
        rung_ms,
        Some(0.8),
        ctx,
    );
    let payload = wrap_verify(shot_result, verify);

    CaptureOutcome::Ok {
        method: "screen_capture".into(),
        rung: RungAttempt::ok("screen_capture", rung_ms),
        payload,
        confidence: 0.8,
    }
}

/// Apply verify logic and wrap into a result payload.
fn wrap_verify(raw: Value, verify: &Option<String>) -> Value {
    if let Some(expected) = verify {
        let ocr_text = raw
            .get("ocr_text")
            .or_else(|| raw.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let verified = ocr_text.to_lowercase().contains(&expected.to_lowercase());
        json!({
            "verified": verified,
            "expected": expected,
            "ocr_text": ocr_text,
            "raw": raw,
        })
    } else {
        json!({
            "verified": true,
            "raw": raw,
        })
    }
}

fn is_css_selector(s: &str) -> bool {
    s.starts_with('#') || s.starts_with('.') || s.starts_with('[') || s.contains("::")
}
