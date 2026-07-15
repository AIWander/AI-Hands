//! `hands_click` — 7-rung cross-subsystem click ladder (v2).
//!
//! Ladder per spec §5.2:
//!   1. A11y cache search → ref resolution → CSS click
//!   2. Fuzzy text match (match_text)
//!   3. CSS selector (if target looks like CSS)
//!   4. A11y snapshot refresh → retry rung 1
//!   5. get_clickables → best text score → coords click
//!   6. UIA find_element → uia_click at center
//!   7. OCR scan → coords click via uia_click
//!
//! v2 additions:
//! - Reversibility tagging based on target text patterns
//! - Post-click state capture (URL + dialog count)
//! - All targeting adjustments applied via targeting.rs helpers
//! - MetaToolResult envelope with full instrumentation
//! - Confidence model (method-based)

use serde_json::{json, Value};
use std::time::Instant;

use super::consent;
use super::error::MetaError;
use super::instrumentation;
use super::response::{Confidence, MetaToolResult, Reversibility, RungAttempt};
use super::session::SharedSession;
use super::targeting::classify_reversibility;
use crate::atomic::{AtomicTool, UiaClick, UiaFindElement};
use crate::vision_core;

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
            instrumentation::log_aggregate(
                "hands_click",
                &call_id,
                false,
                "",
                0,
                0,
                None,
                Some("target is required"),
            );
            return MetaToolResult::failure(vec![], MetaError::other("target is required"), 0)
                .to_value();
        }
    };

    let page_context = args
        .get("page_context")
        .and_then(|v| v.as_str())
        .unwrap_or("auto");
    let double_click = args
        .get("double_click")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let button = if args
        .get("right_click")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        "right"
    } else {
        "left"
    };
    let strict = args
        .get("strict")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let allow_destructive = args
        .get("allow_destructive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // Pixel-click offsets. When either is non-zero, every rung that resolves an element
    // computes its bounding-box center + (offset_x, offset_y) and performs a coord-based
    // click. With both offsets at zero, ref/selector clicks stay on rungs 1-4 (more
    // reliable, avoids scroll-into-view issues). Subsumes the legacy find_and_click.
    let offset_x = args.get("offset_x").and_then(|v| v.as_i64()).unwrap_or(0);
    let offset_y = args.get("offset_y").and_then(|v| v.as_i64()).unwrap_or(0);
    let offset_active = offset_x != 0 || offset_y != 0;
    let monitor = args.get("monitor").and_then(Value::as_u64).unwrap_or(0) as usize;

    // Pre-click reversibility check
    let reversibility = classify_reversibility(&target);
    let ctx = json!({
        "target": &target,
        "page_context": page_context,
        "button": button,
        "double_click": double_click,
        "strict": strict,
        "allow_destructive": allow_destructive,
        "offset_x": offset_x,
        "offset_y": offset_y,
        "monitor": monitor,
    });

    if reversibility == Reversibility::Destructive && !allow_destructive {
        let error = MetaError::requires_confirmation(
            &target,
            "Target text matches destructive pattern. Pass allow_destructive: true to proceed.",
        );
        let error_value = serde_json::to_value(&error).unwrap_or_else(|_| {
            json!({
                "category": "requires_confirmation",
                "detail": {
                    "action": &target,
                    "reason": "Target text matches destructive pattern. Pass allow_destructive: true to proceed."
                }
            })
        });
        let elapsed = start.elapsed().as_millis() as u64;
        instrumentation::log_aggregate_with_context(
            "hands_click",
            &call_id,
            false,
            "",
            0,
            elapsed,
            None,
            Some(&error_value),
            Some(&ctx),
        );

        return MetaToolResult::failure(vec![], error, elapsed)
            .with_reversibility(Reversibility::Destructive)
            .to_value();
    }

    // ── Consent risk check ──
    // If target text looks like a consent button, classify and gate on risk level.
    if consent::looks_like_consent_button(&target) {
        let _session_auto_accept = {
            let s = session.read().unwrap_or_else(|e| e.into_inner());
            s.auto_accept_low_risk
        };
        let url_hint = args.get("url").and_then(|v| v.as_str());
        let element_ctx = args.get("element_context");
        let classification = consent::classify_consent(
            &target, // use target text as dialog text proxy
            &[target.as_str()],
            url_hint,
            element_ctx,
        );

        match classification.risk {
            consent::RiskLevel::HighRisk | consent::RiskLevel::MediumRisk => {
                if !allow_destructive {
                    let elapsed = start.elapsed().as_millis() as u64;
                    let error = MetaError::requires_confirmation(
                        &target,
                        format!(
                            "Consent classified as {:?}: {}. Pass allow_destructive: true to proceed.",
                            classification.risk, classification.reasoning
                        ),
                    );
                    instrumentation::log_aggregate(
                        "hands_click",
                        &call_id,
                        false,
                        "",
                        0,
                        elapsed,
                        None,
                        Some(&format!("Consent gate: {:?}", classification.risk)),
                    );
                    return MetaToolResult::failure(vec![], error, elapsed)
                        .with_reversibility(Reversibility::RequiresConfirmation)
                        .to_value();
                }
            }
            consent::RiskLevel::LowRisk | consent::RiskLevel::NoRisk => {
                // Auto-accept if session flag allows, otherwise proceed normally
                // (clicking a low-risk consent button is always allowed)
            }
        }
    }

    let mut rungs_tried = Vec::new();

    let browser_active = super::browser_is_active(browser).await;
    let use_browser = browser_active && (page_context == "auto" || page_context == "browser");

    // ── BROWSER RUNGS ──
    if use_browser {
        // Rung 1: A11y cache lookup → ref → CSS click
        {
            let rung_start = Instant::now();
            let ref_id_opt = if looks_like_a11y_ref(&target) {
                Some(target.clone())
            } else {
                super::search_a11y_snapshot(&target)
            };

            if let Some(ref_id) = ref_id_opt {
                match crate::resolve_a11y_ref(&ref_id, "click", browser).await {
                    Ok(selector) => {
                        let (ok, val) = if offset_active {
                            click_browser_with_offset(
                                browser,
                                &selector,
                                offset_x,
                                offset_y,
                                button,
                                double_click,
                            )
                            .await
                        } else {
                            let click_result = browser_mcp::tools::handle_tool(
                                browser, "click",
                                json!({"selector": selector, "button": button, "double_click": double_click}),
                            ).await;
                            super::browser_result_to_value(click_result)
                        };
                        let rung_ms = rung_start.elapsed().as_millis() as u64;

                        if ok {
                            let confidence = if looks_like_a11y_ref(&target) {
                                1.0
                            } else {
                                0.8
                            };
                            let attempt = RungAttempt::ok("a11y_cache", rung_ms);
                            instrumentation::log_rung_attempt(
                                "hands_click",
                                &call_id,
                                "a11y_cache",
                                true,
                                rung_ms,
                                Some(confidence),
                                &ctx,
                            );
                            rungs_tried.push(attempt);

                            let elapsed = start.elapsed().as_millis() as u64;
                            let result = make_success(
                                "a11y_cache",
                                rungs_tried,
                                confidence,
                                reversibility,
                                json!({"ref_id": ref_id, "selector": selector, "target": target, "detail": val}),
                                elapsed,
                                &call_id,
                            );
                            return result.to_value();
                        }
                    }
                    Err(_) => {} // stale or not found
                }
            }

            let rung_ms = rung_start.elapsed().as_millis() as u64;
            rungs_tried.push(RungAttempt::failed(
                "a11y_cache",
                rung_ms,
                "No match or click failed",
            ));
            instrumentation::log_rung_attempt(
                "hands_click",
                &call_id,
                "a11y_cache",
                false,
                rung_ms,
                None,
                &ctx,
            );
        }

        // Rung 2: Fuzzy text match
        {
            let rung_start = Instant::now();
            let (ok, val) = if offset_active {
                // Offset path: find the fuzzy-matched element's bbox via JS, then coord click.
                match match_text_bbox_center(browser, &target).await {
                    Some((cx, cy)) => {
                        let click_x = cx + offset_x;
                        let click_y = cy + offset_y;
                        let click_result = browser_mcp::tools::handle_tool(
                            browser, "click",
                            json!({"x": click_x, "y": click_y, "button": button, "double_click": double_click}),
                        ).await;
                        super::browser_result_to_value(click_result)
                    }
                    None => (false, json!({"error": "no fuzzy text match"})),
                }
            } else {
                let click_result = browser_mcp::tools::handle_tool(
                    browser,
                    "click",
                    json!({"match_text": &target, "button": button, "double_click": double_click}),
                )
                .await;
                super::browser_result_to_value(click_result)
            };
            let rung_ms = rung_start.elapsed().as_millis() as u64;

            if ok {
                let attempt = RungAttempt::ok("match_text", rung_ms);
                instrumentation::log_rung_attempt(
                    "hands_click",
                    &call_id,
                    "match_text",
                    true,
                    rung_ms,
                    Some(0.9),
                    &ctx,
                );
                rungs_tried.push(attempt);

                let elapsed = start.elapsed().as_millis() as u64;
                let result = make_success(
                    "match_text",
                    rungs_tried,
                    0.9,
                    reversibility,
                    json!({"target": target, "detail": val}),
                    elapsed,
                    &call_id,
                );
                return result.to_value();
            }

            rungs_tried.push(RungAttempt::failed("match_text", rung_ms, "No text match"));
            instrumentation::log_rung_attempt(
                "hands_click",
                &call_id,
                "match_text",
                false,
                rung_ms,
                None,
                &ctx,
            );
        }

        // Rung 3: CSS selector (only if target looks like one)
        if looks_like_selector(&target) {
            let rung_start = Instant::now();
            let (ok, val) =
                if offset_active {
                    click_browser_with_offset(
                        browser,
                        &target,
                        offset_x,
                        offset_y,
                        button,
                        double_click,
                    )
                    .await
                } else {
                    let click_result = browser_mcp::tools::handle_tool(
                    browser, "click",
                    json!({"selector": &target, "button": button, "double_click": double_click}),
                ).await;
                    super::browser_result_to_value(click_result)
                };
            let rung_ms = rung_start.elapsed().as_millis() as u64;

            if ok {
                let attempt = RungAttempt::ok("css_selector", rung_ms);
                instrumentation::log_rung_attempt(
                    "hands_click",
                    &call_id,
                    "css_selector",
                    true,
                    rung_ms,
                    Some(0.95),
                    &ctx,
                );
                rungs_tried.push(attempt);

                let elapsed = start.elapsed().as_millis() as u64;
                let result = make_success(
                    "css_selector",
                    rungs_tried,
                    0.95,
                    reversibility,
                    json!({"target": target, "detail": val}),
                    elapsed,
                    &call_id,
                );
                return result.to_value();
            }

            rungs_tried.push(RungAttempt::failed(
                "css_selector",
                rung_ms,
                "Selector not found",
            ));
            instrumentation::log_rung_attempt(
                "hands_click",
                &call_id,
                "css_selector",
                false,
                rung_ms,
                None,
                &ctx,
            );
        }

        // Rung 4: A11y snapshot refresh → retry
        {
            let rung_start = Instant::now();
            let _ = crate::handle_accessibility_snapshot(&json!({}), browser).await;

            let ref_id_refreshed = if looks_like_a11y_ref(&target) {
                Some(target.clone())
            } else {
                super::search_a11y_snapshot(&target)
            };

            if let Some(ref_id) = ref_id_refreshed {
                if let Ok(selector) = crate::resolve_a11y_ref(&ref_id, "click", browser).await {
                    let (ok, val) = if offset_active {
                        click_browser_with_offset(
                            browser,
                            &selector,
                            offset_x,
                            offset_y,
                            button,
                            double_click,
                        )
                        .await
                    } else {
                        let click_result = browser_mcp::tools::handle_tool(
                            browser, "click",
                            json!({"selector": selector, "button": button, "double_click": double_click}),
                        ).await;
                        super::browser_result_to_value(click_result)
                    };
                    let rung_ms = rung_start.elapsed().as_millis() as u64;

                    if ok {
                        let attempt = RungAttempt::ok("a11y_refresh", rung_ms);
                        instrumentation::log_rung_attempt(
                            "hands_click",
                            &call_id,
                            "a11y_refresh",
                            true,
                            rung_ms,
                            Some(0.8),
                            &ctx,
                        );
                        rungs_tried.push(attempt);

                        let elapsed = start.elapsed().as_millis() as u64;
                        let result = make_success(
                            "a11y_refresh",
                            rungs_tried,
                            0.8,
                            reversibility,
                            json!({"ref_id": ref_id, "selector": selector, "target": target, "detail": val}),
                            elapsed,
                            &call_id,
                        );
                        return result.to_value();
                    }
                }
            }

            let rung_ms = rung_start.elapsed().as_millis() as u64;
            rungs_tried.push(RungAttempt::failed(
                "a11y_refresh",
                rung_ms,
                "Refresh didn't help",
            ));
            instrumentation::log_rung_attempt(
                "hands_click",
                &call_id,
                "a11y_refresh",
                false,
                rung_ms,
                None,
                &ctx,
            );
        }

        // Rung 5: get_clickables → best score → coords click
        {
            let rung_start = Instant::now();
            let clickables_result =
                browser_mcp::tools::handle_tool(browser, "get_clickables", json!({})).await;
            let (ok, val) = super::browser_result_to_value(clickables_result);

            if ok {
                if let Some((x, y)) = find_best_clickable_coords(&val, &target) {
                    let click_x = x + offset_x;
                    let click_y = y + offset_y;
                    let click_result = browser_mcp::tools::handle_tool(
                        browser, "click",
                        json!({"x": click_x, "y": click_y, "button": button, "double_click": double_click}),
                    ).await;
                    let (click_ok, click_val) = super::browser_result_to_value(click_result);
                    let rung_ms = rung_start.elapsed().as_millis() as u64;

                    if click_ok {
                        let attempt = RungAttempt::ok("clickables_coords", rung_ms);
                        instrumentation::log_rung_attempt(
                            "hands_click",
                            &call_id,
                            "clickables_coords",
                            true,
                            rung_ms,
                            Some(0.6),
                            &ctx,
                        );
                        rungs_tried.push(attempt);

                        let elapsed = start.elapsed().as_millis() as u64;
                        let result = make_success(
                            "clickables_coords",
                            rungs_tried,
                            0.6,
                            reversibility,
                            json!({"x": click_x, "y": click_y, "target": target, "detail": click_val}),
                            elapsed,
                            &call_id,
                        );
                        return result.to_value();
                    }
                }
            }

            let rung_ms = rung_start.elapsed().as_millis() as u64;
            rungs_tried.push(RungAttempt::failed(
                "clickables_coords",
                rung_ms,
                "No matching clickable",
            ));
            instrumentation::log_rung_attempt(
                "hands_click",
                &call_id,
                "clickables_coords",
                false,
                rung_ms,
                None,
                &ctx,
            );
        }
    }

    // ── DESKTOP RUNGS ──
    if page_context == "desktop" || page_context == "auto" {
        // Rung 6: UIA find → click
        {
            let rung_start = Instant::now();
            let find_result = UiaFindElement.call(&json!({"name": &target, "max_depth": 8}));

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
                        let click_result = UiaClick.call(
                            &json!({"x": click_x, "y": click_y, "button": button, "double_click": double_click}),
                        );
                        let success = click_result
                            .get("success")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let rung_ms = rung_start.elapsed().as_millis() as u64;

                        if success {
                            let confidence = 0.9; // UIA name match
                            let attempt = RungAttempt::ok("uia_find_click", rung_ms);
                            instrumentation::log_rung_attempt(
                                "hands_click",
                                &call_id,
                                "uia_find_click",
                                true,
                                rung_ms,
                                Some(confidence),
                                &ctx,
                            );
                            rungs_tried.push(attempt);

                            let elapsed = start.elapsed().as_millis() as u64;
                            let result = make_success(
                                "uia_find_click",
                                rungs_tried,
                                confidence,
                                reversibility,
                                json!({"x": click_x, "y": click_y, "element": first, "target": target}),
                                elapsed,
                                &call_id,
                            );
                            return result.to_value();
                        }
                    }
                }
            }

            let rung_ms = rung_start.elapsed().as_millis() as u64;
            rungs_tried.push(RungAttempt::failed(
                "uia_find_click",
                rung_ms,
                "UIA element not found",
            ));
            instrumentation::log_rung_attempt(
                "hands_click",
                &call_id,
                "uia_find_click",
                false,
                rung_ms,
                None,
                &ctx,
            );
        }

        // Rung 7: OCR → coords click
        {
            let rung_start = Instant::now();
            let ocr_result =
                vision_core::execute("vision_screenshot_ocr", &json!({"monitor": monitor})).await;
            let ocr_text = ocr_result
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if ocr_text.to_lowercase().contains(&target.to_lowercase()) {
                let screenshot_path =
                    vision_core::take_screenshot(None, monitor, 80).unwrap_or_default();
                if !screenshot_path.is_empty() {
                    if let Ok(words) = vision_core::ocr_image_with_positions(&screenshot_path).await
                    {
                        let _ = std::fs::remove_file(&screenshot_path);
                        if let Some((x, y)) = find_text_in_ocr_words(&words, &target) {
                            let (global_x, global_y) =
                                match crate::monitor_scope::globalize_local_point(x, y, monitor) {
                                    Ok(point) => point,
                                    Err(error) => {
                                        let rung_ms = rung_start.elapsed().as_millis() as u64;
                                        rungs_tried.push(RungAttempt::failed(
                                            "ocr_coords",
                                            rung_ms,
                                            &error,
                                        ));
                                        let elapsed = start.elapsed().as_millis() as u64;
                                        return MetaToolResult::failure(
                                            rungs_tried,
                                            MetaError::other(error),
                                            elapsed,
                                        )
                                        .with_reversibility(reversibility)
                                        .to_value();
                                    }
                                };
                            let click_x = global_x + offset_x;
                            let click_y = global_y + offset_y;
                            let click_result = UiaClick.call(
                                &json!({"x": click_x, "y": click_y, "button": button, "double_click": double_click}),
                            );
                            let success = click_result
                                .get("success")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            let rung_ms = rung_start.elapsed().as_millis() as u64;

                            if success {
                                let confidence = 0.5; // OCR match
                                let attempt = RungAttempt::ok("ocr_coords", rung_ms);
                                instrumentation::log_rung_attempt(
                                    "hands_click",
                                    &call_id,
                                    "ocr_coords",
                                    true,
                                    rung_ms,
                                    Some(confidence),
                                    &ctx,
                                );
                                rungs_tried.push(attempt);

                                let elapsed = start.elapsed().as_millis() as u64;
                                let result = make_success(
                                    "ocr_coords",
                                    rungs_tried,
                                    confidence,
                                    reversibility,
                                    json!({"x": click_x, "y": click_y, "target": target}),
                                    elapsed,
                                    &call_id,
                                );
                                return result.to_value();
                            }
                        }
                    }
                }
            }

            let rung_ms = rung_start.elapsed().as_millis() as u64;
            rungs_tried.push(RungAttempt::failed(
                "ocr_coords",
                rung_ms,
                "OCR text not found or click failed",
            ));
            instrumentation::log_rung_attempt(
                "hands_click",
                &call_id,
                "ocr_coords",
                false,
                rung_ms,
                None,
                &ctx,
            );
        }
    }

    // All rungs failed
    let elapsed = start.elapsed().as_millis() as u64;
    let error_msg = if strict {
        format!(
            "Could not click '{}' via any strategy (strict mode)",
            target
        )
    } else {
        format!("Could not click '{}' via any strategy", target)
    };

    let result = MetaToolResult::failure(
        rungs_tried.clone(),
        MetaError::not_found(&target, page_context),
        elapsed,
    );

    instrumentation::log_aggregate(
        "hands_click",
        &call_id,
        false,
        "",
        rungs_tried.len(),
        elapsed,
        None,
        Some(&error_msg),
    );

    result.to_value()
}

// ── Helpers ──

/// Resolve a CSS selector's center coordinate (viewport pixels) and click at
/// (center.x + offset_x, center.y + offset_y) via browser coord-click. Used by
/// rungs 1, 3, 4 when offset_x/y are set so the click hits a precise sub-pixel
/// within the element instead of its default center.
async fn click_browser_with_offset(
    browser: &browser_mcp::browser::SharedBrowser,
    selector: &str,
    offset_x: i64,
    offset_y: i64,
    button: &str,
    double_click: bool,
) -> (bool, Value) {
    let escaped = selector.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        r#"(() => {{
            const el = document.querySelector("{}");
            if (!el) return JSON.stringify({{error: 'not found'}});
            const r = el.getBoundingClientRect();
            return JSON.stringify({{
                cx: Math.round(r.left + r.width / 2),
                cy: Math.round(r.top + r.height / 2),
                w: Math.round(r.width),
                h: Math.round(r.height)
            }});
        }})()"#,
        escaped,
    );

    let eval_result =
        browser_mcp::tools::handle_tool(browser, "eval", json!({"script": script})).await;
    let (eval_ok, eval_val) = super::browser_result_to_value(eval_result);
    if !eval_ok {
        return (
            false,
            json!({"error": "eval failed for bbox lookup", "detail": eval_val}),
        );
    }

    let parsed = unwrap_eval_json(&eval_val);
    let cx = parsed.get("cx").and_then(|v| v.as_i64());
    let cy = parsed.get("cy").and_then(|v| v.as_i64());
    let (cx, cy) = match (cx, cy) {
        (Some(x), Some(y)) => (x, y),
        _ => {
            return (
                false,
                json!({"error": "could not resolve bbox center", "detail": parsed}),
            )
        }
    };

    let click_x = cx + offset_x;
    let click_y = cy + offset_y;
    let click_result = browser_mcp::tools::handle_tool(
        browser,
        "click",
        json!({"x": click_x, "y": click_y, "button": button, "double_click": double_click}),
    )
    .await;
    let (ok, val) = super::browser_result_to_value(click_result);
    let merged = json!({
        "selector": selector,
        "bbox_center": {"x": cx, "y": cy},
        "click_at": {"x": click_x, "y": click_y},
        "offset": {"x": offset_x, "y": offset_y},
        "detail": val,
    });
    (ok, merged)
}

/// Unwrap eval results from `browser_result_to_value`. The browser `eval` tool
/// stringifies JS return values, so a JS `JSON.stringify({...})` becomes a
/// `Value::String` containing the JSON text. We re-parse it to an object.
/// Also handles the `{"result": "..."}` fallback shape and raw object returns.
fn unwrap_eval_json(eval_val: &Value) -> Value {
    if let Some(s) = eval_val.as_str() {
        // Most common path: eval returned a JS string (JSON.stringify result).
        if let Ok(inner) = serde_json::from_str::<Value>(s) {
            return inner;
        }
    }
    if let Some(s) = eval_val.get("result").and_then(|v| v.as_str()) {
        // Fallback shape from browser_result_to_value when parsing failed.
        if let Ok(inner) = serde_json::from_str::<Value>(s) {
            return inner;
        }
    }
    eval_val.clone()
}

/// Find the best-fuzzy-text-match element on the page via JS, return its
/// bbox center in viewport pixels. Mirrors the scoring of `get_clickables`
/// + `find_best_clickable_coords` but executes in a single JS round-trip
/// and only needs the bbox (not the full clickables list).
async fn match_text_bbox_center(
    browser: &browser_mcp::browser::SharedBrowser,
    target: &str,
) -> Option<(i64, i64)> {
    let target_lower = target.to_lowercase();
    let escaped = target_lower.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        r#"(() => {{
            const target = "{}";
            const candidates = document.querySelectorAll('a, button, input, [role="button"], [role="link"], [onclick], [tabindex]');
            let best = null;
            let bestScore = 0;
            for (const el of candidates) {{
                const text = (el.innerText || el.textContent || '').trim().toLowerCase();
                const aria = (el.getAttribute('aria-label') || '').toLowerCase();
                const title = (el.getAttribute('title') || '').toLowerCase();
                const id = (el.id || '').toLowerCase();
                let score = 0;
                if (text === target || aria === target) score = 100;
                else if (text.startsWith(target) || aria.startsWith(target)) score = 90;
                else if (text.includes(target) || aria.includes(target)) score = 80;
                else if (title.includes(target)) score = 70;
                else if (id.includes(target)) score = 50;
                if (score > bestScore) {{
                    const r = el.getBoundingClientRect();
                    if (r.width > 0 && r.height > 0) {{
                        bestScore = score;
                        best = {{
                            cx: Math.round(r.left + r.width / 2),
                            cy: Math.round(r.top + r.height / 2),
                        }};
                    }}
                }}
            }}
            return JSON.stringify(best || {{}});
        }})()"#,
        escaped,
    );

    let eval_result =
        browser_mcp::tools::handle_tool(browser, "eval", json!({"script": script})).await;
    let (eval_ok, eval_val) = super::browser_result_to_value(eval_result);
    if !eval_ok {
        return None;
    }

    let parsed = unwrap_eval_json(&eval_val);
    let cx = parsed.get("cx").and_then(|v| v.as_i64())?;
    let cy = parsed.get("cy").and_then(|v| v.as_i64())?;
    Some((cx, cy))
}

fn make_success(
    method: &str,
    rungs_tried: Vec<RungAttempt>,
    confidence: f32,
    reversibility: Reversibility,
    payload: Value,
    elapsed: u64,
    call_id: &str,
) -> MetaToolResult {
    instrumentation::log_aggregate(
        "hands_click",
        call_id,
        true,
        method,
        rungs_tried.len(),
        elapsed,
        Some(confidence),
        None,
    );

    MetaToolResult::success(method, rungs_tried, payload, elapsed)
        .with_confidence(Confidence::method_only(confidence))
        .with_reversibility(reversibility)
}

fn looks_like_a11y_ref(s: &str) -> bool {
    if let Some(rest) = s.strip_prefix("ref_") {
        !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
    } else {
        false
    }
}

pub fn looks_like_selector(s: &str) -> bool {
    s.starts_with('#')
        || s.starts_with('.')
        || s.starts_with('[')
        || (s.starts_with("button") && s.contains('['))
        || (s.starts_with("input") && s.contains('['))
        || s.starts_with("a[")
        || s.contains("=\"")
        || s.contains("='")
}

pub fn find_best_clickable_coords(val: &Value, target: &str) -> Option<(i64, i64)> {
    let clickables = val
        .get("clickables")
        .or_else(|| val.get("elements"))
        .and_then(|v| v.as_array())?;

    let target_lower = target.to_lowercase();
    let mut best_score: i64 = 0;
    let mut best_coords: Option<(i64, i64)> = None;

    for elem in clickables {
        let text = elem
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let aria = elem
            .get("aria_label")
            .or_else(|| elem.get("ariaLabel"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let title = elem
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let id = elem
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();

        let score: i64 = if text == target_lower || aria == target_lower {
            100
        } else if text.starts_with(&target_lower) || aria.starts_with(&target_lower) {
            90
        } else if text.contains(&target_lower) || aria.contains(&target_lower) {
            80
        } else if title.contains(&target_lower) {
            70
        } else if id.contains(&target_lower) {
            50
        } else {
            0
        };

        if score > best_score {
            best_score = score;
            let cx = elem.get("x").and_then(|v| v.as_i64()).or_else(|| {
                elem.get("center")
                    .and_then(|c| c.get("x"))
                    .and_then(|v| v.as_i64())
            });
            let cy = elem.get("y").and_then(|v| v.as_i64()).or_else(|| {
                elem.get("center")
                    .and_then(|c| c.get("y"))
                    .and_then(|v| v.as_i64())
            });
            if let (Some(x), Some(y)) = (cx, cy) {
                best_coords = Some((x, y));
            }
        }
    }

    if best_score > 0 {
        best_coords
    } else {
        None
    }
}

pub fn find_text_in_ocr_words(
    words: &[(String, f64, f64, f64, f64)],
    target: &str,
) -> Option<(i64, i64)> {
    let target_lower = target.to_lowercase();
    let target_words: Vec<&str> = target_lower.split_whitespace().collect();
    if target_words.is_empty() {
        return None;
    }

    // Single-word match
    for (word, x, y, w, h) in words {
        if word.to_lowercase().contains(&target_lower) {
            return Some(((x + w / 2.0) as i64, (y + h / 2.0) as i64));
        }
    }

    // Multi-word span match
    if target_words.len() > 1 && words.len() >= target_words.len() {
        for start in 0..=(words.len() - target_words.len()) {
            let matched = target_words
                .iter()
                .enumerate()
                .filter(|(i, tw)| words[start + i].0.to_lowercase().contains(*tw))
                .count();
            if matched == target_words.len() {
                let end = start + target_words.len() - 1;
                let first = &words[start];
                let last = &words[end];
                let cx = ((first.1 + last.1 + last.3) / 2.0) as i64;
                let cy = (first.2 + first.4 / 2.0) as i64;
                return Some((cx, cy));
            }
        }
    }

    None
}
