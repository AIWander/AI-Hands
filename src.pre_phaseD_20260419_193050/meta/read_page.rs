//! `hands_read_page` — escalating web content fetcher (v2).
//!
//! Ladder per spec §5.1:
//!   1. browser_http_scrape  — HTTP GET, no JS (~0.5s)
//!   2. browser_js_extract(linkedom) — Node.js light JS (~2s)
//!   3. browser_smart_browse — internal HTTP→linkedom→jsdom→Chrome (~5s)
//!   4. Full Chrome: launch(headless) + navigate + wait + extract_content (~8s+)
//!
//! v2 additions:
//! - Content quality check with JS-required signal detection
//! - Content hash between rungs to avoid re-extracting identical content
//! - MetaToolResult envelope with rungs_tried, confidence, elapsed_ms
//! - Instrumentation logging per rung attempt

use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Instant;

use super::error::MetaError;
use super::instrumentation;
use super::response::{Confidence, MetaToolResult, RungAttempt};
use super::session::SharedSession;
use super::targeting::{content_is_sufficient, content_needs_js};

/// Minimum content length to consider a rung successful.
const MIN_CONTENT_CHARS: usize = 200;

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
                "hands_read_page",
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

    let wait_for = args
        .get("wait_for")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(15000);
    let deadline = Instant::now() + std::time::Duration::from_millis(timeout_ms);

    let mut rungs_tried = Vec::new();
    let mut last_content_hash: u64 = 0;
    let ctx = json!({"url": &url});

    // If wait_for is set, only Chrome can satisfy — skip to rung 4
    if wait_for.is_some() {
        let result = rung4_chrome(
            &url,
            wait_for.as_deref(),
            browser,
            &mut rungs_tried,
            deadline,
            &call_id,
            &ctx,
        )
        .await;
        let elapsed = start.elapsed().as_millis() as u64;
        log_aggregate(&call_id, &result, &rungs_tried, elapsed);
        return result.to_value();
    }

    // Rung 1: HTTP scrape (no browser, no JS)
    if Instant::now() < deadline {
        let rung_start = Instant::now();
        let result =
            browser_mcp::tools::handle_tool(browser, "http_scrape", json!({"url": &url})).await;
        let rung_ms = rung_start.elapsed().as_millis() as u64;
        let (ok, val) = super::browser_result_to_value(result);

        if ok {
            let content = extract_text(&val);
            let hash = content_hash(&content);
            if content_is_sufficient(&content, MIN_CONTENT_CHARS) {
                let attempt = RungAttempt::ok("http_scrape", rung_ms);
                instrumentation::log_rung_attempt(
                    "hands_read_page",
                    &call_id,
                    "http_scrape",
                    true,
                    rung_ms,
                    Some(1.0),
                    &ctx,
                );
                rungs_tried.push(attempt);

                let elapsed = start.elapsed().as_millis() as u64;
                let result = MetaToolResult::success(
                    "http_scrape", rungs_tried,
                    json!({"content": content, "url": url, "chars": content.len(), "extraction_method": "http_scrape"}),
                    elapsed,
                ).with_confidence(Confidence::method_only(1.0));
                log_aggregate(&call_id, &result, &result.rungs_tried, elapsed);
                return result.to_value();
            }
            last_content_hash = hash;
        }

        let attempt = RungAttempt::failed(
            "http_scrape",
            rung_ms,
            if ok {
                "Content insufficient or JS-required"
            } else {
                "HTTP scrape failed"
            },
        );
        instrumentation::log_rung_attempt(
            "hands_read_page",
            &call_id,
            "http_scrape",
            false,
            rung_ms,
            None,
            &ctx,
        );
        rungs_tried.push(attempt);
    }

    // Rung 2: Node.js linkedom
    if Instant::now() < deadline {
        let rung_start = Instant::now();
        let result = browser_mcp::tools::handle_tool(
            browser,
            "js_extract",
            json!({"url": &url, "engine": "linkedom"}),
        )
        .await;
        let rung_ms = rung_start.elapsed().as_millis() as u64;
        let (ok, val) = super::browser_result_to_value(result);

        if ok {
            let content = extract_text(&val);
            let hash = content_hash(&content);
            // Skip if same content as previous rung (hash dedup)
            if hash != last_content_hash && content_is_sufficient(&content, MIN_CONTENT_CHARS) {
                let attempt = RungAttempt::ok("js_extract_linkedom", rung_ms);
                instrumentation::log_rung_attempt(
                    "hands_read_page",
                    &call_id,
                    "js_extract_linkedom",
                    true,
                    rung_ms,
                    Some(0.9),
                    &ctx,
                );
                rungs_tried.push(attempt);

                let elapsed = start.elapsed().as_millis() as u64;
                let result = MetaToolResult::success(
                    "js_extract_linkedom", rungs_tried,
                    json!({"content": content, "url": url, "chars": content.len(), "extraction_method": "js_extract_linkedom"}),
                    elapsed,
                ).with_confidence(Confidence::method_only(0.9));
                log_aggregate(&call_id, &result, &result.rungs_tried, elapsed);
                return result.to_value();
            }
            last_content_hash = hash;
        }

        let attempt = RungAttempt::failed(
            "js_extract_linkedom",
            rung_ms,
            "Content insufficient or duplicate",
        );
        instrumentation::log_rung_attempt(
            "hands_read_page",
            &call_id,
            "js_extract_linkedom",
            false,
            rung_ms,
            None,
            &ctx,
        );
        rungs_tried.push(attempt);
    }

    // Rung 3: smart_browse
    if Instant::now() < deadline {
        let rung_start = Instant::now();
        let result =
            browser_mcp::tools::handle_tool(browser, "smart_browse", json!({"url": &url})).await;
        let rung_ms = rung_start.elapsed().as_millis() as u64;
        let (ok, val) = super::browser_result_to_value(result);

        if ok {
            let content = extract_text(&val);
            let hash = content_hash(&content);
            if hash != last_content_hash && content_is_sufficient(&content, MIN_CONTENT_CHARS) {
                let attempt = RungAttempt::ok("smart_browse", rung_ms);
                instrumentation::log_rung_attempt(
                    "hands_read_page",
                    &call_id,
                    "smart_browse",
                    true,
                    rung_ms,
                    Some(0.85),
                    &ctx,
                );
                rungs_tried.push(attempt);

                let elapsed = start.elapsed().as_millis() as u64;
                let result = MetaToolResult::success(
                    "smart_browse", rungs_tried,
                    json!({"content": content, "url": url, "chars": content.len(), "extraction_method": "smart_browse"}),
                    elapsed,
                ).with_confidence(Confidence::method_only(0.85));
                log_aggregate(&call_id, &result, &result.rungs_tried, elapsed);
                return result.to_value();
            }
        }

        let attempt =
            RungAttempt::failed("smart_browse", rung_ms, "Content insufficient or duplicate");
        instrumentation::log_rung_attempt(
            "hands_read_page",
            &call_id,
            "smart_browse",
            false,
            rung_ms,
            None,
            &ctx,
        );
        rungs_tried.push(attempt);
    }

    // Rung 4: Full Chrome
    let result = rung4_chrome(
        &url,
        None,
        browser,
        &mut rungs_tried,
        deadline,
        &call_id,
        &ctx,
    )
    .await;
    let elapsed = start.elapsed().as_millis() as u64;
    log_aggregate(&call_id, &result, &rungs_tried, elapsed);
    result.to_value()
}

async fn rung4_chrome(
    url: &str,
    wait_for: Option<&str>,
    browser: &browser_mcp::browser::SharedBrowser,
    rungs_tried: &mut Vec<RungAttempt>,
    deadline: Instant,
    call_id: &str,
    ctx: &Value,
) -> MetaToolResult {
    let rung_start = Instant::now();

    if Instant::now() >= deadline {
        let attempt = RungAttempt::timed_out("chrome_headless", 0);
        rungs_tried.push(attempt);
        return MetaToolResult::failure(
            rungs_tried.clone(),
            MetaError::timeout("chrome_headless", deadline.elapsed().as_millis() as u64),
            deadline.elapsed().as_millis() as u64,
        );
    }

    // Launch headless if browser isn't active
    if !super::browser_is_active(browser).await {
        let launch_result =
            browser_mcp::tools::handle_tool(browser, "launch", json!({"headless": true})).await;
        let (ok, val) = super::browser_result_to_value(launch_result);
        if !ok {
            let err = val
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("launch failed");
            let rung_ms = rung_start.elapsed().as_millis() as u64;
            let attempt = RungAttempt::failed("chrome_headless", rung_ms, err);
            instrumentation::log_rung_attempt(
                "hands_read_page",
                call_id,
                "chrome_headless",
                false,
                rung_ms,
                None,
                ctx,
            );
            rungs_tried.push(attempt);
            return MetaToolResult::failure(rungs_tried.clone(), MetaError::no_browser(), rung_ms);
        }
    }

    // Navigate
    let nav_result = browser_mcp::tools::handle_tool(
        browser,
        "navigate",
        json!({"url": url, "wait_until": "domcontentloaded"}),
    )
    .await;
    let (nav_ok, _nav_val) = super::browser_result_to_value(nav_result);
    if !nav_ok {
        let rung_ms = rung_start.elapsed().as_millis() as u64;
        let attempt = RungAttempt::failed("chrome_headless", rung_ms, "Navigate failed");
        instrumentation::log_rung_attempt(
            "hands_read_page",
            call_id,
            "chrome_headless",
            false,
            rung_ms,
            None,
            ctx,
        );
        rungs_tried.push(attempt);
        return MetaToolResult::failure(
            rungs_tried.clone(),
            MetaError::other(format!("Chrome navigate to '{}' failed", url)),
            rung_ms,
        );
    }

    // Wait for selector if requested
    if let Some(selector) = wait_for {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        if remaining > 0 {
            let _ = browser_mcp::tools::handle_tool(
                browser,
                "wait_for",
                json!({"selector": selector, "timeout_ms": remaining.min(10000)}),
            )
            .await;
        }
    }

    // Extract content
    let extract_result =
        browser_mcp::tools::handle_tool(browser, "extract_content", json!({})).await;
    let (ok, val) = super::browser_result_to_value(extract_result);
    let rung_ms = rung_start.elapsed().as_millis() as u64;

    if ok {
        let content = extract_text(&val);
        let attempt = RungAttempt::ok("chrome_headless", rung_ms);
        instrumentation::log_rung_attempt(
            "hands_read_page",
            call_id,
            "chrome_headless",
            true,
            rung_ms,
            Some(0.8),
            ctx,
        );
        rungs_tried.push(attempt);

        let mut result = MetaToolResult::success(
            "chrome_headless", rungs_tried.clone(),
            json!({"content": content, "url": url, "chars": content.len(), "extraction_method": "chrome_headless"}),
            rung_ms,
        ).with_confidence(Confidence::method_only(0.8));

        // Warn if content still looks JS-dependent
        if content_needs_js(&content) {
            result = result.with_warning("Content may still require client-side JS rendering");
        }
        result
    } else {
        let attempt = RungAttempt::failed("chrome_headless", rung_ms, "Extract failed");
        instrumentation::log_rung_attempt(
            "hands_read_page",
            call_id,
            "chrome_headless",
            false,
            rung_ms,
            None,
            ctx,
        );
        rungs_tried.push(attempt);
        MetaToolResult::failure(
            rungs_tried.clone(),
            MetaError::other(format!("All rungs failed for '{}'", url)),
            rung_ms,
        )
    }
}

/// Pull readable text from a browser tool result value.
fn extract_text(val: &Value) -> String {
    for field in &["content", "text", "result"] {
        if let Some(s) = val.get(field).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    val.to_string()
}

/// Compute content hash for dedup between rungs.
fn content_hash(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

fn log_aggregate(call_id: &str, result: &MetaToolResult, rungs: &[RungAttempt], elapsed: u64) {
    let confidence = result.confidence.as_ref().and_then(|c| c.method);
    let error = result.error.as_ref().map(|e| e.to_string());
    instrumentation::log_aggregate(
        "hands_read_page",
        call_id,
        result.success,
        &result.method,
        rungs.len(),
        elapsed,
        confidence,
        error.as_deref(),
    );
}
