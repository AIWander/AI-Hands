//! `hands_verify` — structured verification with polling, stabilization, and location confidence.
//!
//! Verification ladder (5 rungs):
//!   1. DOM text search via browser_eval JS
//!   2. A11y snapshot search
//!   3. Element query via browser_eval (querySelector)
//!   4. OCR fallback via vision_screenshot_ocr (throttled to 2s gap)
//!   5. UIA text search (for desktop scope)
//!
//! Features:
//!   - Polling with cheap rungs at 500ms, OCR at 2s minimum gap
//!   - Navigation guard: URL change resets stabilization timer
//!   - must_stabilize_ms: require N consecutive matches
//!   - require_visible: viewport boundary check
//!   - Location-based confidence: heading=1.0, body=0.7, footer/nav=0.4, hidden=0.2
//!   - Three input modes: structured (text|regex|element), natural_text, template

use serde_json::{json, Value};
use std::time::Instant;

use super::error::MetaError;
use super::instrumentation;
use super::nl_parser;
use super::response::{Confidence, MetaToolResult, Reversibility, RungAttempt};
use super::session::SharedSession;
use super::verify_templates;
#[cfg(feature = "desktop")]
use crate::atomic::{AtomicTool, UiaFindElement};

/// Main entry point for hands_verify.
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

    let ctx = args.clone();

    // ── Parse input: exactly one of structured / natural_text / template ──
    let parsed = parse_verify_input(args);
    let verify_config = match parsed {
        Ok(c) => c,
        Err(e) => {
            instrumentation::log_aggregate(
                "hands_verify",
                &call_id,
                false,
                "",
                0,
                0,
                None,
                Some(&e),
            );
            return MetaToolResult::failure(vec![], MetaError::other(&e), 0).to_value();
        }
    };

    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(5000);
    let must_stabilize_ms = args
        .get("must_stabilize_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let require_visible = args
        .get("require_visible")
        .and_then(|v| v.as_bool())
        .unwrap_or(verify_config.require_visible);
    let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("auto");

    let browser_active = super::browser_is_active(browser).await;
    let use_browser = browser_active && (scope == "auto" || scope == "browser");
    let use_desktop = scope == "desktop" || scope == "auto";

    let mut rungs_tried = Vec::new();
    let mut checks_made: u32 = 0;
    let mut last_ocr_time: Option<Instant> = None;
    let mut first_match_time: Option<Instant> = None;
    let mut match_method = String::new();
    let mut match_evidence = String::new();
    let mut match_confidence: f32 = 0.0;
    let mut initial_url = String::new();

    // Capture initial URL for navigation guard
    if use_browser {
        initial_url = get_browser_url(browser).await;
    }

    // ── Polling loop ──
    // timeout_ms == 0 means single check (no polling)
    let single_shot = timeout_ms == 0;
    let effective_timeout = if single_shot { 30000 } else { timeout_ms };

    loop {
        let elapsed = start.elapsed().as_millis() as u64;
        if elapsed > effective_timeout {
            break;
        }

        checks_made += 1;

        // Navigation guard: check URL hasn't changed (resets stabilization)
        if use_browser && !initial_url.is_empty() {
            let current_url = get_browser_url(browser).await;
            if current_url != initial_url {
                // URL changed — reset stabilization timer
                first_match_time = None;
                initial_url = current_url;
            }
        }

        // ── Try each rung ──
        let check_result = run_verification_ladder(
            &verify_config,
            browser,
            use_browser,
            use_desktop,
            require_visible,
            &mut rungs_tried,
            &mut last_ocr_time,
            &call_id,
            &ctx,
        )
        .await;

        if let Some((method, evidence, confidence)) = check_result {
            // Match found
            if first_match_time.is_none() {
                first_match_time = Some(Instant::now());
                match_method = method;
                match_evidence = evidence;
                match_confidence = confidence;
            }

            // Check stabilization
            if must_stabilize_ms > 0 {
                if let Some(first) = first_match_time {
                    let stabilized_for = first.elapsed().as_millis() as u64;
                    if stabilized_for >= must_stabilize_ms {
                        // Stabilized — success
                        let total_elapsed = start.elapsed().as_millis() as u64;
                        return make_verify_success(
                            &match_method,
                            &match_evidence,
                            match_confidence,
                            checks_made,
                            total_elapsed,
                            Some(stabilized_for),
                            rungs_tried,
                            &call_id,
                        )
                        .to_value();
                    }
                }
            } else {
                // No stabilization required — immediate success
                let total_elapsed = start.elapsed().as_millis() as u64;
                return make_verify_success(
                    &match_method,
                    &match_evidence,
                    match_confidence,
                    checks_made,
                    total_elapsed,
                    None,
                    rungs_tried,
                    &call_id,
                )
                .to_value();
            }
        } else {
            // No match — reset stabilization
            first_match_time = None;
        }

        if single_shot {
            break;
        }

        // Sleep before next poll (500ms for cheap rungs)
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    // ── Verification failed ──
    let total_elapsed = start.elapsed().as_millis() as u64;
    let error = if total_elapsed >= effective_timeout && !single_shot {
        MetaError::timeout("hands_verify", total_elapsed)
    } else {
        MetaError::VerificationFailed {
            evidence: format!(
                "Target '{}' not found after {} checks",
                verify_config.target, checks_made
            ),
            confidence: 0.0,
        }
    };

    instrumentation::log_aggregate(
        "hands_verify",
        &call_id,
        false,
        "",
        rungs_tried.len(),
        total_elapsed,
        None,
        Some(&error.to_string()),
    );

    let mut fail_result = MetaToolResult::failure(rungs_tried, error, total_elapsed);
    fail_result.reversibility = Reversibility::Reversible;
    fail_result.result = json!({
        "verified": false,
        "method": "",
        "evidence": format!("Not found after {} checks in {}ms", checks_made, total_elapsed),
        "confidence": 0.0,
        "checks_made": checks_made,
        "elapsed_ms": total_elapsed,
        "stabilized_for_ms": null,
    });
    fail_result.to_value()
}

// ── Verification config parsed from args ──

struct VerifyConfig {
    target: String,
    check_mode: VerifyMode,
    negated: bool,
    require_visible: bool,
}

enum VerifyMode {
    Text,
    Regex,
    Element,
    PageReady,
}

fn parse_verify_input(args: &Value) -> Result<VerifyConfig, String> {
    let has_text = args.get("text").and_then(|v| v.as_str()).is_some();
    let has_regex = args.get("regex").and_then(|v| v.as_str()).is_some();
    let has_element = args.get("element").and_then(|v| v.as_str()).is_some();
    let has_nl = args.get("natural_text").and_then(|v| v.as_str()).is_some();
    let has_template = args.get("template").and_then(|v| v.as_str()).is_some();

    let input_count = [has_text, has_regex, has_element, has_nl, has_template]
        .iter()
        .filter(|&&x| x)
        .count();

    if input_count == 0 {
        return Err("One of text, regex, element, natural_text, or template is required".into());
    }
    if input_count > 1 {
        let mut conflicting = Vec::new();
        if has_text {
            conflicting.push("text");
        }
        if has_regex {
            conflicting.push("regex");
        }
        if has_element {
            conflicting.push("element");
        }
        if has_nl {
            conflicting.push("natural_text");
        }
        if has_template {
            conflicting.push("template");
        }
        return Err(format!(
            "Only one of text, regex, element, natural_text, or template may be set (got: {})",
            conflicting.join(", ")
        ));
    }

    let negated = args
        .get("negated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let require_visible = args
        .get("require_visible")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Structured: text
    if let Some(text) = args.get("text").and_then(|v| v.as_str()) {
        return Ok(VerifyConfig {
            target: text.to_string(),
            check_mode: VerifyMode::Text,
            negated,
            require_visible,
        });
    }

    // Structured: regex
    if let Some(regex) = args.get("regex").and_then(|v| v.as_str()) {
        return Ok(VerifyConfig {
            target: regex.to_string(),
            check_mode: VerifyMode::Regex,
            negated,
            require_visible,
        });
    }

    // Structured: element (CSS selector)
    if let Some(element) = args.get("element").and_then(|v| v.as_str()) {
        return Ok(VerifyConfig {
            target: element.to_string(),
            check_mode: VerifyMode::Element,
            negated,
            require_visible,
        });
    }

    // NL: natural_text
    if let Some(nl) = args.get("natural_text").and_then(|v| v.as_str()) {
        let expectation = nl_parser::parse_nl(nl)?;
        let (mode, negated_from_nl) = match expectation.check_type {
            nl_parser::CheckType::TextPresent | nl_parser::CheckType::TextAbsent => {
                (VerifyMode::Text, expectation.negated)
            }
            nl_parser::CheckType::ElementPresent | nl_parser::CheckType::ElementAbsent => {
                (VerifyMode::Element, expectation.negated)
            }
            nl_parser::CheckType::RegexMatch => (VerifyMode::Regex, expectation.negated),
            nl_parser::CheckType::PageReady => (VerifyMode::PageReady, false),
        };
        return Ok(VerifyConfig {
            target: expectation.target,
            check_mode: mode,
            negated: negated_from_nl,
            require_visible: expectation.require_visible || require_visible,
        });
    }

    // Template
    if let Some(template_name) = args.get("template").and_then(|v| v.as_str()) {
        let template_args = args
            .get("template_args")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let expansion = verify_templates::resolve_template(template_name, &template_args)?;
        // For templates, use the first required text_present check as the target,
        // or fall back to the template description
        let target = expansion
            .checks
            .iter()
            .find(|c| c.check_type == "text_present" && c.target.is_some())
            .and_then(|c| c.target.clone())
            .unwrap_or_default();
        return Ok(VerifyConfig {
            target,
            check_mode: VerifyMode::Text,
            negated: false,
            require_visible: false,
        });
    }

    Err("Invalid verify input".into())
}

// ── Rung execution ──

/// Run through the verification ladder. Returns Some((method, evidence, confidence)) on match.
async fn run_verification_ladder(
    config: &VerifyConfig,
    browser: &browser_mcp::browser::SharedBrowser,
    use_browser: bool,
    use_desktop: bool,
    require_visible: bool,
    rungs_tried: &mut Vec<RungAttempt>,
    last_ocr_time: &mut Option<Instant>,
    call_id: &str,
    ctx: &Value,
) -> Option<(String, String, f32)> {
    // ── PageReady mode: special handling ──
    if matches!(config.check_mode, VerifyMode::PageReady) {
        if use_browser {
            return run_page_ready_check(browser, rungs_tried, call_id, ctx).await;
        }
        return None;
    }

    // ── Browser rungs ──
    if use_browser {
        // Rung 1: DOM text search via browser_eval
        if matches!(config.check_mode, VerifyMode::Text | VerifyMode::Regex)
            && !config.target.is_empty()
        {
            let rung_start = Instant::now();
            let result = run_dom_text_search(
                browser,
                &config.target,
                &config.check_mode,
                config.negated,
                require_visible,
            )
            .await;
            let rung_ms = rung_start.elapsed().as_millis() as u64;

            match result {
                RungResult::Found {
                    evidence,
                    confidence,
                } => {
                    let attempt = RungAttempt::ok("dom_text_search", rung_ms);
                    instrumentation::log_rung_attempt(
                        "hands_verify",
                        call_id,
                        "dom_text_search",
                        true,
                        rung_ms,
                        Some(confidence),
                        ctx,
                    );
                    rungs_tried.push(attempt);
                    return Some(("dom_text_search".into(), evidence, confidence));
                }
                RungResult::NotFound => {
                    // For negated checks, not finding the text means success
                    if config.negated {
                        let attempt = RungAttempt::ok("dom_text_search", rung_ms);
                        instrumentation::log_rung_attempt(
                            "hands_verify",
                            call_id,
                            "dom_text_search",
                            true,
                            rung_ms,
                            Some(0.8),
                            ctx,
                        );
                        rungs_tried.push(attempt);
                        return Some((
                            "dom_text_search".into(),
                            format!("'{}' not found (negated check passed)", config.target),
                            0.8,
                        ));
                    }
                    rungs_tried.push(RungAttempt::failed(
                        "dom_text_search",
                        rung_ms,
                        "Text not found in DOM",
                    ));
                    instrumentation::log_rung_attempt(
                        "hands_verify",
                        call_id,
                        "dom_text_search",
                        false,
                        rung_ms,
                        None,
                        ctx,
                    );
                }
                RungResult::Error(e) => {
                    rungs_tried.push(RungAttempt::failed("dom_text_search", rung_ms, &e));
                    instrumentation::log_rung_attempt(
                        "hands_verify",
                        call_id,
                        "dom_text_search",
                        false,
                        rung_ms,
                        None,
                        ctx,
                    );
                }
            }
        }

        // Rung 2: A11y snapshot search
        if matches!(config.check_mode, VerifyMode::Text) && !config.target.is_empty() {
            let rung_start = Instant::now();
            let ref_id = super::search_a11y_snapshot(&config.target);
            let rung_ms = rung_start.elapsed().as_millis() as u64;

            let found = ref_id.is_some();
            if found && !config.negated {
                let attempt = RungAttempt::ok("a11y_search", rung_ms);
                instrumentation::log_rung_attempt(
                    "hands_verify",
                    call_id,
                    "a11y_search",
                    true,
                    rung_ms,
                    Some(0.85),
                    ctx,
                );
                rungs_tried.push(attempt);
                return Some((
                    "a11y_search".into(),
                    format!(
                        "Found '{}' in a11y tree (ref: {})",
                        config.target,
                        ref_id.unwrap()
                    ),
                    0.85,
                ));
            } else if !found && config.negated {
                let attempt = RungAttempt::ok("a11y_search", rung_ms);
                instrumentation::log_rung_attempt(
                    "hands_verify",
                    call_id,
                    "a11y_search",
                    true,
                    rung_ms,
                    Some(0.8),
                    ctx,
                );
                rungs_tried.push(attempt);
                return Some((
                    "a11y_search".into(),
                    format!(
                        "'{}' absent from a11y tree (negated check passed)",
                        config.target
                    ),
                    0.8,
                ));
            } else {
                rungs_tried.push(RungAttempt::failed(
                    "a11y_search",
                    rung_ms,
                    "Not found in a11y snapshot",
                ));
                instrumentation::log_rung_attempt(
                    "hands_verify",
                    call_id,
                    "a11y_search",
                    false,
                    rung_ms,
                    None,
                    ctx,
                );
            }
        }

        // Rung 3: Element query via browser_eval (querySelector)
        if matches!(config.check_mode, VerifyMode::Element) && !config.target.is_empty() {
            let rung_start = Instant::now();
            let result =
                run_element_query(browser, &config.target, config.negated, require_visible).await;
            let rung_ms = rung_start.elapsed().as_millis() as u64;

            match result {
                RungResult::Found {
                    evidence,
                    confidence,
                } => {
                    let attempt = RungAttempt::ok("element_query", rung_ms);
                    instrumentation::log_rung_attempt(
                        "hands_verify",
                        call_id,
                        "element_query",
                        true,
                        rung_ms,
                        Some(confidence),
                        ctx,
                    );
                    rungs_tried.push(attempt);
                    return Some(("element_query".into(), evidence, confidence));
                }
                RungResult::NotFound => {
                    if config.negated {
                        let attempt = RungAttempt::ok("element_query", rung_ms);
                        instrumentation::log_rung_attempt(
                            "hands_verify",
                            call_id,
                            "element_query",
                            true,
                            rung_ms,
                            Some(0.9),
                            ctx,
                        );
                        rungs_tried.push(attempt);
                        return Some((
                            "element_query".into(),
                            format!("Element '{}' absent (negated check passed)", config.target),
                            0.9,
                        ));
                    }
                    rungs_tried.push(RungAttempt::failed(
                        "element_query",
                        rung_ms,
                        "Element not found",
                    ));
                    instrumentation::log_rung_attempt(
                        "hands_verify",
                        call_id,
                        "element_query",
                        false,
                        rung_ms,
                        None,
                        ctx,
                    );
                }
                RungResult::Error(e) => {
                    rungs_tried.push(RungAttempt::failed("element_query", rung_ms, &e));
                    instrumentation::log_rung_attempt(
                        "hands_verify",
                        call_id,
                        "element_query",
                        false,
                        rung_ms,
                        None,
                        ctx,
                    );
                }
            }
        }
    }

    // Rung 4: OCR fallback (gated behind desktop feature)
    #[cfg(feature = "desktop")]
    if matches!(config.check_mode, VerifyMode::Text) && !config.target.is_empty() {
        let should_run_ocr = match last_ocr_time {
            Some(t) => t.elapsed().as_millis() >= 2000,
            None => true,
        };

        if should_run_ocr {
            let rung_start = Instant::now();
            *last_ocr_time = Some(Instant::now());
            let result = run_ocr_search(&config.target, config.negated).await;
            let rung_ms = rung_start.elapsed().as_millis() as u64;

            match result {
                RungResult::Found {
                    evidence,
                    confidence,
                } => {
                    let attempt = RungAttempt::ok("ocr_search", rung_ms);
                    instrumentation::log_rung_attempt(
                        "hands_verify",
                        call_id,
                        "ocr_search",
                        true,
                        rung_ms,
                        Some(confidence),
                        ctx,
                    );
                    rungs_tried.push(attempt);
                    return Some(("ocr_search".into(), evidence, confidence));
                }
                RungResult::NotFound => {
                    if config.negated {
                        let attempt = RungAttempt::ok("ocr_search", rung_ms);
                        instrumentation::log_rung_attempt(
                            "hands_verify",
                            call_id,
                            "ocr_search",
                            true,
                            rung_ms,
                            Some(0.6),
                            ctx,
                        );
                        rungs_tried.push(attempt);
                        return Some((
                            "ocr_search".into(),
                            format!(
                                "'{}' not found via OCR (negated check passed)",
                                config.target
                            ),
                            0.6,
                        ));
                    }
                    rungs_tried.push(RungAttempt::failed(
                        "ocr_search",
                        rung_ms,
                        "Text not found via OCR",
                    ));
                    instrumentation::log_rung_attempt(
                        "hands_verify",
                        call_id,
                        "ocr_search",
                        false,
                        rung_ms,
                        None,
                        ctx,
                    );
                }
                RungResult::Error(e) => {
                    rungs_tried.push(RungAttempt::failed("ocr_search", rung_ms, &e));
                    instrumentation::log_rung_attempt(
                        "hands_verify",
                        call_id,
                        "ocr_search",
                        false,
                        rung_ms,
                        None,
                        ctx,
                    );
                }
            }
        }
    }

    // Rung 5: UIA text search (gated behind desktop feature)
    #[cfg(feature = "desktop")]
    if use_desktop && matches!(config.check_mode, VerifyMode::Text) && !config.target.is_empty() {
        let rung_start = Instant::now();
        let result = run_uia_text_search(&config.target, config.negated);
        let rung_ms = rung_start.elapsed().as_millis() as u64;

        match result {
            RungResult::Found {
                evidence,
                confidence,
            } => {
                let attempt = RungAttempt::ok("uia_text_search", rung_ms);
                instrumentation::log_rung_attempt(
                    "hands_verify",
                    call_id,
                    "uia_text_search",
                    true,
                    rung_ms,
                    Some(confidence),
                    ctx,
                );
                rungs_tried.push(attempt);
                return Some(("uia_text_search".into(), evidence, confidence));
            }
            RungResult::NotFound => {
                if config.negated {
                    let attempt = RungAttempt::ok("uia_text_search", rung_ms);
                    instrumentation::log_rung_attempt(
                        "hands_verify",
                        call_id,
                        "uia_text_search",
                        true,
                        rung_ms,
                        Some(0.7),
                        ctx,
                    );
                    rungs_tried.push(attempt);
                    return Some((
                        "uia_text_search".into(),
                        format!(
                            "'{}' not found via UIA (negated check passed)",
                            config.target
                        ),
                        0.7,
                    ));
                }
                rungs_tried.push(RungAttempt::failed(
                    "uia_text_search",
                    rung_ms,
                    "Text not found via UIA",
                ));
                instrumentation::log_rung_attempt(
                    "hands_verify",
                    call_id,
                    "uia_text_search",
                    false,
                    rung_ms,
                    None,
                    ctx,
                );
            }
            RungResult::Error(e) => {
                rungs_tried.push(RungAttempt::failed("uia_text_search", rung_ms, &e));
                instrumentation::log_rung_attempt(
                    "hands_verify",
                    call_id,
                    "uia_text_search",
                    false,
                    rung_ms,
                    None,
                    ctx,
                );
            }
        }
    }

    None
}

// ── Rung result type ──

enum RungResult {
    Found { evidence: String, confidence: f32 },
    NotFound,
    Error(String),
}

// ── Individual rung implementations ──

/// Rung 1: DOM text search via browser_eval.
/// Also computes location-based confidence (heading=1.0, body=0.7, footer/nav=0.4).
async fn run_dom_text_search(
    browser: &browser_mcp::browser::SharedBrowser,
    target: &str,
    mode: &VerifyMode,
    negated: bool,
    require_visible: bool,
) -> RungResult {
    let escaped_target = target
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n");

    let script = match mode {
        VerifyMode::Regex => {
            format!(
                r#"(() => {{
                    const re = new RegExp('{}', 'i');
                    const body = document.body ? document.body.innerText : '';
                    const match = re.test(body);
                    if (!match) return JSON.stringify({{found: false}});
                    const m = body.match(re);
                    const snippet = m ? m[0].substring(0, 100) : '';
                    return JSON.stringify({{found: true, snippet: snippet, location: 'body'}});
                }})()"#,
                escaped_target
            )
        }
        _ => {
            // Text search with location detection and optional visibility check
            let visibility_check = if require_visible {
                r#"
                    // Find the element containing the text
                    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
                    let textNode = null;
                    while (walker.nextNode()) {
                        if (walker.currentNode.textContent.toLowerCase().includes(target.toLowerCase())) {
                            textNode = walker.currentNode;
                            break;
                        }
                    }
                    if (textNode) {
                        const el = textNode.parentElement;
                        if (el) {
                            const rect = el.getBoundingClientRect();
                            const vw = window.innerWidth || document.documentElement.clientWidth;
                            const vh = window.innerHeight || document.documentElement.clientHeight;
                            if (rect.bottom < 0 || rect.top > vh || rect.right < 0 || rect.left > vw) {
                                return JSON.stringify({found: false, reason: 'not_in_viewport'});
                            }
                            if (rect.width === 0 || rect.height === 0) {
                                return JSON.stringify({found: false, reason: 'zero_size'});
                            }
                        }
                    }
                "#
            } else {
                ""
            };

            format!(
                r#"(() => {{
                    const target = '{}';
                    const body = document.body ? document.body.innerText : '';
                    const found = body.toLowerCase().includes(target.toLowerCase());
                    if (!found) return JSON.stringify({{found: false}});
                    {}
                    // Detect location for confidence scoring
                    let location = 'body';
                    const headings = document.querySelectorAll('h1, h2, h3');
                    for (const h of headings) {{
                        if (h.textContent.toLowerCase().includes(target.toLowerCase())) {{
                            location = 'heading';
                            break;
                        }}
                    }}
                    if (location === 'body') {{
                        const nav = document.querySelector('nav, [role="navigation"]');
                        if (nav && nav.textContent.toLowerCase().includes(target.toLowerCase())) {{
                            location = 'nav';
                        }}
                        const footer = document.querySelector('footer, [role="contentinfo"]');
                        if (footer && footer.textContent.toLowerCase().includes(target.toLowerCase())) {{
                            location = 'footer';
                        }}
                    }}
                    const idx = body.toLowerCase().indexOf(target.toLowerCase());
                    const snippet = body.substring(Math.max(0, idx - 20), idx + target.length + 20);
                    return JSON.stringify({{found: true, snippet: snippet, location: location}});
                }})()"#,
                escaped_target, visibility_check
            )
        }
    };

    let eval_result =
        browser_mcp::tools::handle_tool(browser, "eval", json!({"script": script})).await;

    let (ok, val) = super::browser_result_to_value(eval_result);
    if !ok {
        return RungResult::Error(format!("browser_eval failed: {}", val));
    }

    // Parse the JSON result from the script
    let result_str = val
        .get("result")
        .and_then(|v| v.as_str())
        .or_else(|| val.as_str())
        .unwrap_or("");
    let parsed: Value = serde_json::from_str(result_str).unwrap_or_else(|_| val.clone());

    let found = parsed
        .get("found")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if found && !negated {
        let location = parsed
            .get("location")
            .and_then(|v| v.as_str())
            .unwrap_or("body");
        let snippet = parsed.get("snippet").and_then(|v| v.as_str()).unwrap_or("");
        let location_confidence = location_to_confidence(location);
        let method_confidence: f32 = 0.95; // DOM search is very reliable
        let combined = method_confidence.min(location_confidence + 0.1); // slight boost from location

        RungResult::Found {
            evidence: format!("Found in DOM ({}): \"{}\"", location, snippet),
            confidence: combined,
        }
    } else if !found && !negated {
        RungResult::NotFound
    } else if found && negated {
        // Found but negated — this is a failure (target should NOT be present)
        RungResult::NotFound
    } else {
        // !found && negated — handled by caller
        RungResult::NotFound
    }
}

/// Rung 3: Element query via querySelector.
async fn run_element_query(
    browser: &browser_mcp::browser::SharedBrowser,
    selector: &str,
    negated: bool,
    require_visible: bool,
) -> RungResult {
    let escaped = selector.replace('\\', "\\\\").replace('\'', "\\'");

    let visibility_part = if require_visible {
        r#"
            const rect = el.getBoundingClientRect();
            const vw = window.innerWidth || document.documentElement.clientWidth;
            const vh = window.innerHeight || document.documentElement.clientHeight;
            const inViewport = rect.bottom > 0 && rect.top < vh && rect.right > 0 && rect.left < vw && rect.width > 0 && rect.height > 0;
            if (!inViewport) return JSON.stringify({found: false, reason: 'not_in_viewport'});
        "#
    } else {
        ""
    };

    let script = format!(
        r#"(() => {{
            const el = document.querySelector('{}');
            if (!el) return JSON.stringify({{found: false}});
            {}
            const tag = el.tagName.toLowerCase();
            const text = (el.textContent || '').substring(0, 100);
            const role = el.getAttribute('role') || '';
            return JSON.stringify({{found: true, tag: tag, text: text, role: role}});
        }})()"#,
        escaped, visibility_part
    );

    let eval_result =
        browser_mcp::tools::handle_tool(browser, "eval", json!({"script": script})).await;

    let (ok, val) = super::browser_result_to_value(eval_result);
    if !ok {
        return RungResult::Error(format!("browser_eval failed: {}", val));
    }

    let result_str = val
        .get("result")
        .and_then(|v| v.as_str())
        .or_else(|| val.as_str())
        .unwrap_or("");
    let parsed: Value = serde_json::from_str(result_str).unwrap_or_else(|_| val.clone());

    let found = parsed
        .get("found")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if found && !negated {
        let tag = parsed.get("tag").and_then(|v| v.as_str()).unwrap_or("?");
        let text = parsed.get("text").and_then(|v| v.as_str()).unwrap_or("");
        RungResult::Found {
            evidence: format!(
                "Element <{}> found: \"{}\"",
                tag,
                &text[..text.len().min(80)]
            ),
            confidence: 0.9,
        }
    } else if !found && negated {
        RungResult::Found {
            evidence: format!("Element '{}' absent (negated check passed)", selector),
            confidence: 0.9,
        }
    } else {
        RungResult::NotFound
    }
}

/// Rung 4: OCR text search.
#[cfg(feature = "desktop")]
async fn run_ocr_search(target: &str, negated: bool) -> RungResult {
    let ocr_result = vision_core::execute("vision_screenshot_ocr", &json!({})).await;
    let ocr_text = ocr_result
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let found = ocr_text.to_lowercase().contains(&target.to_lowercase());

    if found && !negated {
        // Extract context snippet
        let lower = ocr_text.to_lowercase();
        let target_lower = target.to_lowercase();
        let idx = lower.find(&target_lower).unwrap_or(0);
        let start = idx.saturating_sub(30);
        let end = (idx + target.len() + 30).min(ocr_text.len());
        let snippet = &ocr_text[start..end];

        RungResult::Found {
            evidence: format!("Found via OCR: \"{}\"", snippet),
            confidence: 0.6,
        }
    } else if !found && negated {
        RungResult::Found {
            evidence: format!("'{}' not found via OCR (negated check passed)", target),
            confidence: 0.6,
        }
    } else {
        RungResult::NotFound
    }
}

/// Rung 5: UIA text search for desktop elements.
#[cfg(feature = "desktop")]
fn run_uia_text_search(target: &str, negated: bool) -> RungResult {
    let find_result = UiaFindElement.call(&json!({"name": target, "max_depth": 6}));

    let found = find_result
        .get("elements")
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);

    if found && !negated {
        let elements = find_result
            .get("elements")
            .and_then(|v| v.as_array())
            .unwrap();
        let first = &elements[0];
        let name = first.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let control_type = first
            .get("control_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        RungResult::Found {
            evidence: format!("Found via UIA: '{}' ({})", name, control_type),
            confidence: 0.75,
        }
    } else if !found && negated {
        RungResult::Found {
            evidence: format!("'{}' not found via UIA (negated check passed)", target),
            confidence: 0.7,
        }
    } else {
        RungResult::NotFound
    }
}

/// PageReady check: body non-trivial + no loading indicators.
async fn run_page_ready_check(
    browser: &browser_mcp::browser::SharedBrowser,
    rungs_tried: &mut Vec<RungAttempt>,
    call_id: &str,
    ctx: &Value,
) -> Option<(String, String, f32)> {
    let rung_start = Instant::now();

    let script = r#"(() => {
        const body = document.body ? document.body.innerText : '';
        const charCount = body.length;
        const loadingIndicators = document.querySelectorAll(
            '[class*="loading"], [class*="spinner"], [class*="skeleton"], [aria-busy="true"]'
        );
        const hasLoading = loadingIndicators.length > 0;
        const readyState = document.readyState;
        return JSON.stringify({
            charCount: charCount,
            hasLoading: hasLoading,
            readyState: readyState,
            ready: charCount > 50 && !hasLoading && readyState === 'complete'
        });
    })()"#;

    let eval_result =
        browser_mcp::tools::handle_tool(browser, "eval", json!({"script": script})).await;

    let (ok, val) = super::browser_result_to_value(eval_result);
    let rung_ms = rung_start.elapsed().as_millis() as u64;

    if !ok {
        rungs_tried.push(RungAttempt::failed(
            "page_ready",
            rung_ms,
            "browser_eval failed",
        ));
        instrumentation::log_rung_attempt(
            "hands_verify",
            call_id,
            "page_ready",
            false,
            rung_ms,
            None,
            ctx,
        );
        return None;
    }

    let result_str = val
        .get("result")
        .and_then(|v| v.as_str())
        .or_else(|| val.as_str())
        .unwrap_or("");
    let parsed: Value = serde_json::from_str(result_str).unwrap_or_else(|_| val.clone());

    let ready = parsed
        .get("ready")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let char_count = parsed
        .get("charCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    if ready {
        let attempt = RungAttempt::ok("page_ready", rung_ms);
        instrumentation::log_rung_attempt(
            "hands_verify",
            call_id,
            "page_ready",
            true,
            rung_ms,
            Some(0.9),
            ctx,
        );
        rungs_tried.push(attempt);
        Some((
            "page_ready".into(),
            format!(
                "Page ready: {} chars, readyState=complete, no loading indicators",
                char_count
            ),
            0.9,
        ))
    } else {
        rungs_tried.push(RungAttempt::failed(
            "page_ready",
            rung_ms,
            format!("Not ready: {} chars", char_count),
        ));
        instrumentation::log_rung_attempt(
            "hands_verify",
            call_id,
            "page_ready",
            false,
            rung_ms,
            None,
            ctx,
        );
        None
    }
}

// ── Helpers ──

/// Get current browser URL.
async fn get_browser_url(browser: &browser_mcp::browser::SharedBrowser) -> String {
    let guard = browser.read().await;
    guard.get_url().await.unwrap_or_default()
}

/// Location string to confidence score.
fn location_to_confidence(location: &str) -> f32 {
    match location {
        "heading" => 1.0,
        "body" => 0.7,
        "footer" | "nav" => 0.4,
        "hidden" => 0.2,
        _ => 0.7,
    }
}

/// Build a successful verification MetaToolResult.
fn make_verify_success(
    method: &str,
    evidence: &str,
    confidence: f32,
    checks_made: u32,
    elapsed_ms: u64,
    stabilized_for_ms: Option<u64>,
    rungs_tried: Vec<RungAttempt>,
    call_id: &str,
) -> MetaToolResult {
    let location_confidence = if evidence.contains("heading") {
        1.0
    } else if evidence.contains("footer") || evidence.contains("nav") {
        0.4
    } else {
        0.7
    };

    instrumentation::log_aggregate(
        "hands_verify",
        call_id,
        true,
        method,
        rungs_tried.len(),
        elapsed_ms,
        Some(confidence),
        None,
    );

    let result = json!({
        "verified": true,
        "method": method,
        "evidence": evidence,
        "confidence": confidence,
        "checks_made": checks_made,
        "elapsed_ms": elapsed_ms,
        "stabilized_for_ms": stabilized_for_ms,
    });

    MetaToolResult::success(method, rungs_tried, result, elapsed_ms)
        .with_confidence(Confidence::dual(confidence, location_confidence))
        .with_reversibility(Reversibility::Reversible)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_structured_text() {
        let args = json!({"text": "Hello"});
        let config = parse_verify_input(&args).unwrap();
        assert_eq!(config.target, "Hello");
        assert!(!config.negated);
        assert!(matches!(config.check_mode, VerifyMode::Text));
    }

    #[test]
    fn test_parse_structured_element() {
        let args = json!({"element": "#submit-btn"});
        let config = parse_verify_input(&args).unwrap();
        assert_eq!(config.target, "#submit-btn");
        assert!(matches!(config.check_mode, VerifyMode::Element));
    }

    #[test]
    fn test_parse_structured_regex() {
        let args = json!({"regex": "Order #\\d+"});
        let config = parse_verify_input(&args).unwrap();
        assert_eq!(config.target, "Order #\\d+");
        assert!(matches!(config.check_mode, VerifyMode::Regex));
    }

    #[test]
    fn test_parse_nl_shows() {
        let args = json!({"natural_text": "shows Welcome"});
        let config = parse_verify_input(&args).unwrap();
        assert_eq!(config.target, "Welcome");
        assert!(!config.negated);
    }

    #[test]
    fn test_parse_nl_negated() {
        let args = json!({"natural_text": "no error"});
        let config = parse_verify_input(&args).unwrap();
        assert_eq!(config.target, "error");
        assert!(config.negated);
    }

    #[test]
    fn test_parse_nl_page_ready() {
        let args = json!({"natural_text": "page loaded"});
        let config = parse_verify_input(&args).unwrap();
        assert!(matches!(config.check_mode, VerifyMode::PageReady));
    }

    #[test]
    fn test_parse_template() {
        let args = json!({"template": "verify_page_loaded"});
        let config = parse_verify_input(&args).unwrap();
        assert!(matches!(config.check_mode, VerifyMode::Text));
    }

    #[test]
    fn test_parse_no_input_errors() {
        let args = json!({});
        assert!(parse_verify_input(&args).is_err());
    }

    #[test]
    fn test_parse_multiple_inputs_errors() {
        let args = json!({"text": "hello", "element": "#foo"});
        assert!(parse_verify_input(&args).is_err());
    }

    #[test]
    fn test_parse_negated_flag() {
        let args = json!({"text": "Error", "negated": true});
        let config = parse_verify_input(&args).unwrap();
        assert!(config.negated);
    }

    #[test]
    fn test_location_confidence_scores() {
        assert_eq!(location_to_confidence("heading"), 1.0);
        assert_eq!(location_to_confidence("body"), 0.7);
        assert_eq!(location_to_confidence("footer"), 0.4);
        assert_eq!(location_to_confidence("nav"), 0.4);
        assert_eq!(location_to_confidence("hidden"), 0.2);
        assert_eq!(location_to_confidence("unknown"), 0.7);
    }

    #[test]
    fn test_uia_text_search_negated_not_found() {
        // UIA search with a target that almost certainly doesn't exist
        let result = run_uia_text_search("___nonexistent_zzzz___", true);
        match result {
            RungResult::Found { confidence, .. } => {
                assert!(confidence > 0.0);
            }
            _ => {
                // UIA might not be available in test env — that's ok
            }
        }
    }
}
