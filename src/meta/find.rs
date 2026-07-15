//! `hands_find` — 6-rung cross-subsystem element finder.
//!
//! Ladder per spec §5.6:
//!   1. A11y cache lookup → text match (ref + name)
//!   2. A11y cache lookup → role+name match
//!   3. Clickables (viewport visible elements by bounding rect)
//!   4. UIA find (for desktop context)
//!   5. OCR (with monitor iteration when scope=screen)
//!   6. Template match (if template bytes provided)
//!
//! return_type="ref" short-circuits after rung 3 (subsequent rungs can't produce refs).
//! Adaptive timeout budget per rung using existing helper in response.rs.

use crate::vision_core;
use serde_json::{json, Value};
use std::time::Instant;

use super::click::{find_best_clickable_coords, find_text_in_ocr_words};
use super::error::MetaError;
use super::instrumentation;
use super::response::{
    adaptive_timeout_multiplier, Confidence, MetaToolResult, Reversibility, RungAttempt,
};
use super::session::SharedSession;
use crate::atomic::{AtomicTool, UiaFindElement};

/// The result payload shape for hands_find.
/// Contains the found element info and how it was located.
fn make_find_result(
    found_type: &str, // "ref", "coords", "text"
    ref_id: Option<&str>,
    selector: Option<&str>,
    coords: Option<(i64, i64)>,
    text: Option<&str>,
    monitor_index: Option<i32>,
) -> Value {
    let mut result = json!({
        "found_type": found_type,
    });
    if let Some(r) = ref_id {
        result["ref_id"] = json!(r);
    }
    if let Some(s) = selector {
        result["selector"] = json!(s);
    }
    if let Some((x, y)) = coords {
        result["x"] = json!(x);
        result["y"] = json!(y);
    }
    if let Some(t) = text {
        result["matched_text"] = json!(t);
    }
    if let Some(m) = monitor_index {
        result["monitor_index"] = json!(m);
    }
    result
}

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

    let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("auto");
    let return_type = args
        .get("return_type")
        .and_then(|v| v.as_str())
        .unwrap_or("any");
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(10_000);
    let monitor = args.get("monitor").and_then(Value::as_u64).unwrap_or(0) as usize;
    let ctx = json!({
        "target": &target,
        "scope": scope,
        "return_type": return_type,
        "monitor": monitor,
    });

    let mut rungs_tried = Vec::new();
    let mut timed_out_rungs: Vec<(String, u64)> = Vec::new();

    let browser_active = super::browser_is_active(browser).await;
    let use_browser = browser_active && (scope == "auto" || scope == "browser");
    let use_desktop = scope == "desktop" || scope == "auto" || scope == "screen";

    // ── RUNG 1: A11y cache → text match ──
    if use_browser {
        let rung_start = Instant::now();
        let rung_timeout = 1000u64; // 1s tight

        let ref_id_opt = super::search_a11y_snapshot(&target);
        let rung_ms = rung_start.elapsed().as_millis() as u64;

        if rung_ms > rung_timeout {
            rungs_tried.push(RungAttempt::timed_out("a11y_text_match", rung_ms));
            timed_out_rungs.push(("a11y_text_match".into(), rung_timeout));
        } else if let Some(ref_id) = ref_id_opt {
            // Try to resolve ref to selector for richer result
            let selector = crate::resolve_a11y_ref(&ref_id, "find", browser).await.ok();
            let attempt = RungAttempt::ok("a11y_text_match", rung_ms);
            instrumentation::log_rung_attempt(
                "hands_find",
                &call_id,
                "a11y_text_match",
                true,
                rung_ms,
                Some(1.0),
                &ctx,
            );
            rungs_tried.push(attempt);

            let elapsed = start.elapsed().as_millis() as u64;
            return make_success(
                "a11y_text_match",
                rungs_tried,
                1.0,
                make_find_result(
                    "ref",
                    Some(&ref_id),
                    selector.as_deref(),
                    None,
                    Some(&target),
                    None,
                ),
                elapsed,
                &call_id,
            )
            .to_value();
        } else {
            rungs_tried.push(RungAttempt::failed(
                "a11y_text_match",
                rung_ms,
                "No text match in a11y cache",
            ));
            instrumentation::log_rung_attempt(
                "hands_find",
                &call_id,
                "a11y_text_match",
                false,
                rung_ms,
                None,
                &ctx,
            );
        }
    }

    // ── RUNG 2: A11y cache → role+name match (refresh snapshot) ──
    if use_browser {
        let rung_start = Instant::now();

        // Refresh a11y snapshot
        let _ = crate::handle_accessibility_snapshot(&json!({}), browser).await;
        let ref_id_opt = super::search_a11y_snapshot(&target);
        let rung_ms = rung_start.elapsed().as_millis() as u64;

        if let Some(ref_id) = ref_id_opt {
            let selector = crate::resolve_a11y_ref(&ref_id, "find", browser).await.ok();
            let attempt = RungAttempt::ok("a11y_role_name", rung_ms);
            instrumentation::log_rung_attempt(
                "hands_find",
                &call_id,
                "a11y_role_name",
                true,
                rung_ms,
                Some(0.8),
                &ctx,
            );
            rungs_tried.push(attempt);

            let elapsed = start.elapsed().as_millis() as u64;
            return make_success(
                "a11y_role_name",
                rungs_tried,
                0.8,
                make_find_result(
                    "ref",
                    Some(&ref_id),
                    selector.as_deref(),
                    None,
                    Some(&target),
                    None,
                ),
                elapsed,
                &call_id,
            )
            .to_value();
        }

        rungs_tried.push(RungAttempt::failed(
            "a11y_role_name",
            rung_ms,
            "No match after refresh",
        ));
        instrumentation::log_rung_attempt(
            "hands_find",
            &call_id,
            "a11y_role_name",
            false,
            rung_ms,
            None,
            &ctx,
        );
    }

    // ── RUNG 3: Clickables (viewport visible elements) ──
    if use_browser {
        let rung_start = Instant::now();

        let clickables_result =
            browser_mcp::tools::handle_tool(browser, "get_clickables", json!({})).await;
        let (ok, val) = super::browser_result_to_value(clickables_result);
        let rung_ms = rung_start.elapsed().as_millis() as u64;

        if ok {
            if let Some((x, y)) = find_best_clickable_coords(&val, &target) {
                // Clickables can return text match — confidence depends on match quality
                let confidence = if val
                    .get("clickables")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| {
                        arr.iter().find(|e| {
                            e.get("text")
                                .and_then(|t| t.as_str())
                                .is_some_and(|t| t.to_lowercase() == target.to_lowercase())
                        })
                    })
                    .is_some()
                {
                    0.9 // exact text match in clickables
                } else {
                    0.6 // fuzzy match
                };

                let attempt = RungAttempt::ok("clickables", rung_ms);
                instrumentation::log_rung_attempt(
                    "hands_find",
                    &call_id,
                    "clickables",
                    true,
                    rung_ms,
                    Some(confidence),
                    &ctx,
                );
                rungs_tried.push(attempt);

                let elapsed = start.elapsed().as_millis() as u64;
                return make_success(
                    "clickables",
                    rungs_tried,
                    confidence,
                    make_find_result("coords", None, None, Some((x, y)), Some(&target), None),
                    elapsed,
                    &call_id,
                )
                .to_value();
            }
        }

        rungs_tried.push(RungAttempt::failed(
            "clickables",
            rung_ms,
            "No matching clickable",
        ));
        instrumentation::log_rung_attempt(
            "hands_find",
            &call_id,
            "clickables",
            false,
            rung_ms,
            None,
            &ctx,
        );
    }

    // ── SHORT-CIRCUIT: return_type="ref" stops here ──
    if return_type == "ref" {
        let elapsed = start.elapsed().as_millis() as u64;
        let error = MetaError::ElementNotFound {
            target: target.clone(),
            scope: "browser (ref-only, rungs 1-3 exhausted)".to_string(),
        };
        instrumentation::log_aggregate(
            "hands_find",
            &call_id,
            false,
            "",
            rungs_tried.len(),
            elapsed,
            None,
            Some("No ref available — OCR/UIA/template can't produce refs"),
        );
        let mut result = MetaToolResult::failure(rungs_tried, error, elapsed);
        result.warnings.push("return_type=ref requested but no ref-capable rung succeeded. Available types: coords, text.".into());
        return result.to_value();
    }

    // ── RUNG 4: UIA find (desktop) ──
    if use_desktop {
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
                    let rung_ms = rung_start.elapsed().as_millis() as u64;
                    // Determine confidence: AutomationId > name
                    let has_automation_id = first
                        .get("automation_id")
                        .and_then(|v| v.as_str())
                        .is_some_and(|id| !id.is_empty());
                    let confidence = if has_automation_id { 1.0 } else { 0.9 };

                    let attempt = RungAttempt::ok("uia_find", rung_ms);
                    instrumentation::log_rung_attempt(
                        "hands_find",
                        &call_id,
                        "uia_find",
                        true,
                        rung_ms,
                        Some(confidence),
                        &ctx,
                    );
                    rungs_tried.push(attempt);

                    let elapsed = start.elapsed().as_millis() as u64;
                    return make_success(
                        "uia_find",
                        rungs_tried,
                        confidence,
                        make_find_result("coords", None, None, Some((cx, cy)), Some(&target), None),
                        elapsed,
                        &call_id,
                    )
                    .to_value();
                }
            }
        }

        let rung_ms = rung_start.elapsed().as_millis() as u64;
        rungs_tried.push(RungAttempt::failed(
            "uia_find",
            rung_ms,
            "UIA element not found",
        ));
        instrumentation::log_rung_attempt(
            "hands_find",
            &call_id,
            "uia_find",
            false,
            rung_ms,
            None,
            &ctx,
        );
    }

    // ── RUNG 5: OCR (with monitor iteration on scope=screen) ──
    if use_desktop || scope == "screen" {
        let rung_start = Instant::now();

        let ocr_result =
            vision_core::execute("vision_screenshot_ocr", &json!({"monitor": monitor})).await;
        let ocr_text = ocr_result
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if ocr_text.to_lowercase().contains(&target.to_lowercase()) {
            // Get word positions for coordinate extraction
            let screenshot_path =
                vision_core::take_screenshot(None, monitor, 80).unwrap_or_default();
            if !screenshot_path.is_empty() {
                if let Ok(words) = vision_core::ocr_image_with_positions(&screenshot_path).await {
                    let _ = std::fs::remove_file(&screenshot_path);
                    if let Some((x, y)) = find_text_in_ocr_words(&words, &target) {
                        if let Ok((global_x, global_y)) =
                            crate::monitor_scope::globalize_local_point(x, y, monitor)
                        {
                            let rung_ms = rung_start.elapsed().as_millis() as u64;
                            // OCR confidence based on text match quality
                            let confidence =
                                if ocr_text.to_lowercase().contains(&target.to_lowercase()) {
                                    0.6
                                } else {
                                    0.5
                                };

                            let attempt = RungAttempt::ok("ocr", rung_ms);
                            instrumentation::log_rung_attempt(
                                "hands_find",
                                &call_id,
                                "ocr",
                                true,
                                rung_ms,
                                Some(confidence),
                                &ctx,
                            );
                            rungs_tried.push(attempt);

                            let elapsed = start.elapsed().as_millis() as u64;
                            return make_success(
                                "ocr",
                                rungs_tried,
                                confidence,
                                make_find_result(
                                    "coords",
                                    None,
                                    None,
                                    Some((global_x, global_y)),
                                    Some(&target),
                                    Some(monitor as i32),
                                ),
                                elapsed,
                                &call_id,
                            )
                            .to_value();
                        }
                    }
                }
            }
        }

        let rung_ms = rung_start.elapsed().as_millis() as u64;
        rungs_tried.push(RungAttempt::failed("ocr", rung_ms, "OCR text not found"));
        instrumentation::log_rung_attempt(
            "hands_find",
            &call_id,
            "ocr",
            false,
            rung_ms,
            None,
            &ctx,
        );
    }

    // ── RUNG 6: Template match (requires template bytes or path) ──
    if let Some(template_val) = args.get("template") {
        let rung_start = Instant::now();
        // template can be a file path string or base64-encoded bytes
        let template_path = template_val.as_str().map(|s| s.to_string());

        if let Some(tpl_path) = template_path {
            let find_args = json!({
                "template_path": tpl_path,
                "threshold": args.get("template_threshold").and_then(|v| v.as_f64()).unwrap_or(0.8),
                "monitor": monitor,
            });
            let template_result = vision_core::execute("vision_find_template", &find_args).await;
            let rung_ms = rung_start.elapsed().as_millis() as u64;

            let found = template_result
                .get("found")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if found {
                let tx = template_result
                    .get("x")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let ty = template_result
                    .get("y")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let confidence = template_result
                    .get("confidence")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.7) as f32;

                if let Ok((global_x, global_y)) =
                    crate::monitor_scope::globalize_local_point(tx, ty, monitor)
                {
                    let attempt = RungAttempt::ok("template_match", rung_ms);
                    instrumentation::log_rung_attempt(
                        "hands_find",
                        &call_id,
                        "template_match",
                        true,
                        rung_ms,
                        Some(confidence),
                        &ctx,
                    );
                    rungs_tried.push(attempt);

                    let elapsed = start.elapsed().as_millis() as u64;
                    return make_success(
                        "template_match",
                        rungs_tried,
                        confidence,
                        make_find_result(
                            "coords",
                            None,
                            None,
                            Some((global_x, global_y)),
                            Some(&target),
                            Some(monitor as i32),
                        ),
                        elapsed,
                        &call_id,
                    )
                    .to_value();
                }
            }

            rungs_tried.push(RungAttempt::failed(
                "template_match",
                rung_ms,
                "Template not found on screen",
            ));
            instrumentation::log_rung_attempt(
                "hands_find",
                &call_id,
                "template_match",
                false,
                rung_ms,
                None,
                &ctx,
            );
        }
    }

    // ── ADAPTIVE RETRY: re-try timed-out rungs with widened timeout ──
    if !timed_out_rungs.is_empty() && start.elapsed().as_millis() < timeout_ms as u128 {
        for (rung_name, initial_timeout) in &timed_out_rungs {
            let multiplier = adaptive_timeout_multiplier(*initial_timeout);
            let widened = initial_timeout * multiplier;
            let retry_name = format!("{}_retry", rung_name);

            // Re-run the specific rung with widened timeout budget
            if rung_name == "a11y_text_match" && use_browser {
                let rung_start = Instant::now();
                // Retry a11y text match with widened budget
                let ref_id_opt = super::search_a11y_snapshot(&target);
                let rung_ms = rung_start.elapsed().as_millis() as u64;

                if rung_ms <= widened {
                    if let Some(ref_id) = ref_id_opt {
                        let selector = crate::resolve_a11y_ref(&ref_id, "find", browser).await.ok();
                        let attempt = RungAttempt::ok(&retry_name, rung_ms);
                        instrumentation::log_rung_attempt(
                            "hands_find",
                            &call_id,
                            &retry_name,
                            true,
                            rung_ms,
                            Some(0.8),
                            &ctx,
                        );
                        rungs_tried.push(attempt);

                        let elapsed = start.elapsed().as_millis() as u64;
                        return make_success(
                            &retry_name,
                            rungs_tried,
                            0.8,
                            make_find_result(
                                "ref",
                                Some(&ref_id),
                                selector.as_deref(),
                                None,
                                Some(&target),
                                None,
                            ),
                            elapsed,
                            &call_id,
                        )
                        .to_value();
                    }
                }

                rungs_tried.push(RungAttempt::failed(&retry_name, rung_ms, "Retry failed"));
                instrumentation::log_rung_attempt(
                    "hands_find",
                    &call_id,
                    &retry_name,
                    false,
                    rung_ms,
                    None,
                    &ctx,
                );
            } else {
                // For other rungs, log the retry attempt (they don't have tight timeouts currently)
                instrumentation::log_rung_attempt(
                    "hands_find",
                    &call_id,
                    &retry_name,
                    false,
                    0,
                    None,
                    &ctx,
                );
            }
        }
    }

    // All rungs failed
    let elapsed = start.elapsed().as_millis() as u64;
    let error = MetaError::not_found(&target, scope);
    instrumentation::log_aggregate(
        "hands_find",
        &call_id,
        false,
        "",
        rungs_tried.len(),
        elapsed,
        None,
        Some(&format!("Element '{}' not found via any strategy", target)),
    );

    MetaToolResult::failure(rungs_tried, error, elapsed).to_value()
}

fn make_success(
    method: &str,
    rungs_tried: Vec<RungAttempt>,
    confidence: f32,
    payload: Value,
    elapsed: u64,
    call_id: &str,
) -> MetaToolResult {
    instrumentation::log_aggregate(
        "hands_find",
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
        .with_reversibility(Reversibility::Reversible) // find is always reversible (read-only)
}
