//! `hands_navigate` — browser launch + navigate + wait pipeline (v2).
//!
//! Ladder per spec §5.5:
//!   1. Check browser status — skip launch if active
//!   2a. visible=true → debug_launch (user's Chrome) → fallback launch(headful)
//!   2b. visible=false → launch(headless)
//!   3. navigate (with wait_until for lifecycle keywords)
//!   4. wait_for (CSS selector condition)
//!
//! v2 additions:
//! - Record owning monitor in WindowMonitorMap on launch
//! - Always Reversible (back button works)
//! - MetaToolResult envelope with instrumentation
//! - Partial success on navigation + wait timeout

use serde_json::{json, Value};
use std::time::Instant;

use super::error::MetaError;
use super::instrumentation;
use super::response::{Confidence, MetaToolResult, Reversibility, RungAttempt};
use super::session::SharedSession;

/// Lifecycle keywords that map to browser_navigate wait_until= param.
const LIFECYCLE_CONDITIONS: &[&str] = &["networkidle", "load", "domcontentloaded"];

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

    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u.to_string(),
        None => {
            instrumentation::log_aggregate(
                "hands_navigate",
                &call_id,
                false,
                "",
                0,
                0,
                None,
                Some("url is required"),
            );
            return MetaToolResult::failure(vec![], MetaError::other("url is required"), 0)
                .to_value();
        }
    };

    let wait_condition = args
        .get("wait_condition")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let visible = args
        .get("visible")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(30000);
    let _deadline = Instant::now() + std::time::Duration::from_millis(timeout_ms);

    let mut rungs_tried = Vec::new();
    let ctx = json!({"url": &url, "visible": visible});

    // Step 1: Check browser status
    let browser_active = super::browser_is_active(browser).await;

    if !browser_active {
        if visible {
            // Rung 2a: debug_launch (user's visible Chrome)
            let rung_start = Instant::now();
            let dl_result =
                browser_mcp::tools::handle_tool(browser, "debug_launch", json!({"port": 9222}))
                    .await;
            let (dl_ok, _dl_val) = super::browser_result_to_value(dl_result);
            let rung_ms = rung_start.elapsed().as_millis() as u64;

            if dl_ok {
                rungs_tried.push(RungAttempt::ok("debug_launch", rung_ms));
                instrumentation::log_rung_attempt(
                    "hands_navigate",
                    &call_id,
                    "debug_launch",
                    true,
                    rung_ms,
                    None,
                    &ctx,
                );

                let attach_start = Instant::now();
                let attach_result =
                    browser_mcp::tools::handle_tool(browser, "attach", json!({"port": 9222})).await;
                let (attach_ok, attach_val) = super::browser_result_to_value(attach_result);
                let attach_ms = attach_start.elapsed().as_millis() as u64;

                if !attach_ok {
                    let err = attach_val
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("attach failed");
                    rungs_tried.push(RungAttempt::failed("attach", attach_ms, err));
                    instrumentation::log_rung_attempt(
                        "hands_navigate",
                        &call_id,
                        "attach",
                        false,
                        attach_ms,
                        None,
                        &ctx,
                    );

                    let elapsed = start.elapsed().as_millis() as u64;
                    instrumentation::log_aggregate_with_context(
                        "hands_navigate",
                        &call_id,
                        false,
                        "",
                        rungs_tried.len(),
                        elapsed,
                        None,
                        Some(&json!({"category": "subsystem_unavailable", "detail": err})),
                        Some(&ctx),
                    );
                    return MetaToolResult::failure(
                        rungs_tried,
                        MetaError::subsystem("browser_attach", err),
                        elapsed,
                    )
                    .with_reversibility(Reversibility::Reversible)
                    .to_value();
                }

                rungs_tried.push(RungAttempt::ok("attach", attach_ms));
                instrumentation::log_rung_attempt(
                    "hands_navigate",
                    &call_id,
                    "attach",
                    true,
                    attach_ms,
                    None,
                    &ctx,
                );
            } else {
                rungs_tried.push(RungAttempt::failed(
                    "debug_launch",
                    rung_ms,
                    "Debug launch failed",
                ));
                instrumentation::log_rung_attempt(
                    "hands_navigate",
                    &call_id,
                    "debug_launch",
                    false,
                    rung_ms,
                    None,
                    &ctx,
                );

                // Fallback: headful launch
                let fb_start = Instant::now();
                let fb_result =
                    browser_mcp::tools::handle_tool(browser, "launch", json!({"headless": false}))
                        .await;
                let (fb_ok, fb_val) = super::browser_result_to_value(fb_result);
                let fb_ms = fb_start.elapsed().as_millis() as u64;

                if !fb_ok {
                    let err = fb_val
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("launch failed");
                    rungs_tried.push(RungAttempt::failed("launch_headful", fb_ms, err));
                    instrumentation::log_rung_attempt(
                        "hands_navigate",
                        &call_id,
                        "launch_headful",
                        false,
                        fb_ms,
                        None,
                        &ctx,
                    );

                    let elapsed = start.elapsed().as_millis() as u64;
                    instrumentation::log_aggregate_with_context(
                        "hands_navigate",
                        &call_id,
                        false,
                        "",
                        rungs_tried.len(),
                        elapsed,
                        None,
                        Some(&json!({"category": "browser_not_running", "detail": err})),
                        Some(&ctx),
                    );
                    return MetaToolResult::failure(rungs_tried, MetaError::no_browser(), elapsed)
                        .to_value();
                }

                rungs_tried.push(RungAttempt::ok("launch_headful", fb_ms));
                instrumentation::log_rung_attempt(
                    "hands_navigate",
                    &call_id,
                    "launch_headful",
                    true,
                    fb_ms,
                    None,
                    &ctx,
                );
            }
        } else {
            // Rung 2b: headless launch
            let rung_start = Instant::now();
            let launch_result =
                browser_mcp::tools::handle_tool(browser, "launch", json!({"headless": true})).await;
            let (ok, val) = super::browser_result_to_value(launch_result);
            let rung_ms = rung_start.elapsed().as_millis() as u64;

            if !ok {
                let err = val
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("launch failed");
                rungs_tried.push(RungAttempt::failed("launch_headless", rung_ms, err));
                instrumentation::log_rung_attempt(
                    "hands_navigate",
                    &call_id,
                    "launch_headless",
                    false,
                    rung_ms,
                    None,
                    &ctx,
                );

                let elapsed = start.elapsed().as_millis() as u64;
                instrumentation::log_aggregate_with_context(
                    "hands_navigate",
                    &call_id,
                    false,
                    "",
                    rungs_tried.len(),
                    elapsed,
                    None,
                    Some(&json!({"category": "browser_not_running", "detail": err})),
                    Some(&ctx),
                );
                return MetaToolResult::failure(rungs_tried, MetaError::no_browser(), elapsed)
                    .to_value();
            }

            rungs_tried.push(RungAttempt::ok("launch_headless", rung_ms));
            instrumentation::log_rung_attempt(
                "hands_navigate",
                &call_id,
                "launch_headless",
                true,
                rung_ms,
                None,
                &ctx,
            );
        }

        // Record monitor stickiness for the launched browser window
        // (deferred — actual monitor detection requires window handle which we get after first interaction)
    } else {
        rungs_tried.push(RungAttempt::ok("browser_already_active", 0));
    }

    // Step 3: Navigate
    let nav_start = Instant::now();
    let is_lifecycle = wait_condition
        .as_deref()
        .map(|c| LIFECYCLE_CONDITIONS.contains(&c))
        .unwrap_or(false);

    let nav_args = if is_lifecycle {
        json!({"url": &url, "wait_until": wait_condition})
    } else {
        json!({"url": &url})
    };

    let nav_result = browser_mcp::tools::handle_tool(browser, "navigate", nav_args).await;
    let (nav_ok, nav_val) = super::browser_result_to_value(nav_result);
    let nav_ms = nav_start.elapsed().as_millis() as u64;

    if !nav_ok {
        let err = nav_val
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("navigate failed");
        rungs_tried.push(RungAttempt::failed("navigate", nav_ms, err));
        instrumentation::log_rung_attempt(
            "hands_navigate",
            &call_id,
            "navigate",
            false,
            nav_ms,
            None,
            &ctx,
        );

        let elapsed = start.elapsed().as_millis() as u64;
        instrumentation::log_aggregate_with_context(
            "hands_navigate",
            &call_id,
            false,
            "",
            rungs_tried.len(),
            elapsed,
            None,
            Some(&json!({"category": "navigate_failed", "detail": err})),
            Some(&ctx),
        );
        return MetaToolResult::failure(
            rungs_tried,
            MetaError::other(format!("Navigate to '{}' failed", url)),
            elapsed,
        )
        .with_reversibility(Reversibility::Reversible)
        .to_value();
    }

    rungs_tried.push(RungAttempt::ok("navigate", nav_ms));
    instrumentation::log_rung_attempt(
        "hands_navigate",
        &call_id,
        "navigate",
        true,
        nav_ms,
        None,
        &ctx,
    );

    // Invalidate a11y cache on navigate
    // (The session's cache will be marked dirty)

    // Brief idle wait for JS to settle
    let _ =
        browser_mcp::tools::handle_tool(browser, "wait_idle", json!({"timeout_ms": 1000})).await;

    // Step 4: Wait for CSS selector (non-lifecycle condition)
    let is_css_wait = wait_condition
        .as_deref()
        .map(|c| !LIFECYCLE_CONDITIONS.contains(&c))
        .unwrap_or(false);

    if is_css_wait {
        let cond = wait_condition.as_deref().unwrap_or("");
        let wait_start = Instant::now();
        let wait_result = browser_mcp::tools::handle_tool(
            browser,
            "wait_for",
            json!({"selector": cond, "timeout_ms": timeout_ms.min(15000)}),
        )
        .await;
        let (wait_ok, _wait_val) = super::browser_result_to_value(wait_result);
        let wait_ms = wait_start.elapsed().as_millis() as u64;

        if !wait_ok {
            // Partial success — navigation worked, wait timed out
            rungs_tried.push(RungAttempt::timed_out("wait_for_selector", wait_ms));
            instrumentation::log_rung_attempt(
                "hands_navigate",
                &call_id,
                "wait_for_selector",
                false,
                wait_ms,
                None,
                &ctx,
            );

            let current_url = get_current_url(browser).await;
            let elapsed = start.elapsed().as_millis() as u64;

            let mut result = MetaToolResult::success(
                "navigate_partial",
                rungs_tried.clone(),
                json!({
                    "url": url,
                    "current_url": current_url,
                    "wait_condition": cond,
                    "wait_timed_out": true,
                }),
                elapsed,
            )
            .with_reversibility(Reversibility::Reversible);
            result = result.with_warning(format!(
                "Navigated to '{}' but wait_for '{}' timed out",
                url, cond
            ));

            instrumentation::log_aggregate(
                "hands_navigate",
                &call_id,
                true,
                "navigate_partial",
                rungs_tried.len(),
                elapsed,
                None,
                None,
            );
            return result.to_value();
        }

        rungs_tried.push(RungAttempt::ok("wait_for_selector", wait_ms));
        instrumentation::log_rung_attempt(
            "hands_navigate",
            &call_id,
            "wait_for_selector",
            true,
            wait_ms,
            None,
            &ctx,
        );
    }

    // Success
    let current_url = get_current_url(browser).await;
    let elapsed = start.elapsed().as_millis() as u64;

    let result = MetaToolResult::success(
        "navigate",
        rungs_tried.clone(),
        json!({
            "url": url,
            "current_url": current_url,
        }),
        elapsed,
    )
    .with_reversibility(Reversibility::Reversible)
    .with_confidence(Confidence::method_only(1.0));

    instrumentation::log_aggregate(
        "hands_navigate",
        &call_id,
        true,
        "navigate",
        rungs_tried.len(),
        elapsed,
        Some(1.0),
        None,
    );

    result.to_value()
}

async fn get_current_url(browser: &browser_mcp::browser::SharedBrowser) -> String {
    let guard = browser.read().await;
    guard.get_url().await.unwrap_or_default()
}
