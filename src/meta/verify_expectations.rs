//! `hands_verify_expectations` — post-flow test harness.
//!
//! Runs a list of expectations after a flow completes and reports per-expectation
//! pass/fail with polling, predicates, and tolerance.
//!
//! Workflow owns durable recording and replay. This Hands-side harness checks a
//! replay against current browser, screen, and desktop evidence before trusting it.
//!
//! Supported check_tools (whitelist):
//!   - `browser_get_text` — DOM text content of a CSS selector
//!   - `browser_exists`   — element exists (and is visible) for a CSS selector
//!   - `vision_ocr_text_contains` — region screenshot + OCR + substring check
//!   - `uia_text_exists`  — UIA find by name (desktop)
//!
//! Supported predicates: equals, contains, not_equals, not_contains, regex,
//! gt, lt, in_range. String predicates support a tolerance.string_distance
//! (Levenshtein) for fuzzy matching.

use crate::vision_core;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

use super::session::SharedSession;
use crate::atomic::{AtomicTool, UiaFindElement};

// ── Public entry point ──

pub async fn handle(
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
    _session: &SharedSession,
) -> Value {
    let start = Instant::now();

    let expectations = match args.get("expectations").and_then(|v| v.as_array()) {
        Some(arr) => arr.clone(),
        None => {
            return json!({
                "passed": 0,
                "failed": 0,
                "total": 0,
                "elapsed_ms": start.elapsed().as_millis() as u64,
                "results": [],
                "summary": "0/0 expectations passed",
                "error": "expectations array is required",
            });
        }
    };

    let total = expectations.len();
    let mut results: Vec<Value> = Vec::with_capacity(total);
    let mut passed_count: usize = 0;
    let mut failed_count: usize = 0;

    for (idx, exp) in expectations.iter().enumerate() {
        let result = run_one_expectation(idx, exp, browser).await;
        if result
            .get("passed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            passed_count += 1;
        } else {
            failed_count += 1;
        }
        results.push(result);
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;
    json!({
        "passed": passed_count,
        "failed": failed_count,
        "total": total,
        "elapsed_ms": elapsed_ms,
        "results": results,
        "summary": format!("{}/{} expectations passed", passed_count, total),
    })
}

// ── Per-expectation runner with polling ──

async fn run_one_expectation(
    index: usize,
    exp: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
) -> Value {
    let exp_start = Instant::now();
    let label = exp
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let check_tool = exp
        .get("check_tool")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let check_args = exp.get("args").cloned().unwrap_or_else(|| json!({}));
    let predicate = exp
        .get("expected_predicate")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let tolerance = exp.get("tolerance").cloned().unwrap_or_else(|| json!({}));
    let polls = tolerance
        .get("polls")
        .and_then(|v| v.as_u64())
        .unwrap_or(3)
        .max(1) as u32;
    let interval_ms = tolerance
        .get("interval_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(500);
    let timeout_per_check_ms = tolerance
        .get("timeout_per_check_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(5000);
    let string_distance = tolerance
        .get("string_distance")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    // Reject unsupported check_tools immediately.
    if !is_supported_check_tool(&check_tool) {
        let elapsed_ms = exp_start.elapsed().as_millis() as u64;
        return json!({
            "index": index,
            "label": label,
            "check_tool": check_tool,
            "passed": false,
            "actual": Value::Null,
            "predicate": predicate,
            "polls_used": 0,
            "error": format!("unsupported check_tool: {}", check_tool),
            "elapsed_ms": elapsed_ms,
        });
    }

    let mut last_actual: Value = Value::Null;
    let mut last_error: Option<String> = None;
    let mut polls_used: u32 = 0;
    let mut passed = false;

    for attempt in 1..=polls {
        polls_used = attempt;

        let check_future = run_check_tool(&check_tool, &check_args, browser);
        let check_outcome =
            tokio::time::timeout(Duration::from_millis(timeout_per_check_ms), check_future).await;

        let actual_value = match check_outcome {
            Ok(Ok(v)) => {
                last_error = None;
                v
            }
            Ok(Err(e)) => {
                last_error = Some(e);
                Value::Null
            }
            Err(_) => {
                last_error = Some(format!("check timed out after {}ms", timeout_per_check_ms));
                Value::Null
            }
        };

        last_actual = actual_value.clone();

        if last_error.is_none() && evaluate_predicate(&actual_value, &predicate, string_distance) {
            passed = true;
            break;
        }

        if attempt < polls {
            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        }
    }

    let elapsed_ms = exp_start.elapsed().as_millis() as u64;
    let mut out = json!({
        "index": index,
        "label": label,
        "check_tool": check_tool,
        "passed": passed,
        "actual": last_actual,
        "predicate": predicate,
        "polls_used": polls_used,
        "elapsed_ms": elapsed_ms,
    });

    if !passed {
        let err = last_error
            .unwrap_or_else(|| format!("predicate did not match after {} polls", polls_used));
        out["error"] = json!(err);
    }

    out
}

// ── Check-tool whitelist ──

fn is_supported_check_tool(name: &str) -> bool {
    matches!(
        name,
        "browser_get_text" | "browser_exists" | "vision_ocr_text_contains" | "uia_text_exists"
    )
}

/// Run a single check tool and return the actual value or an error string.
async fn run_check_tool(
    check_tool: &str,
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
) -> Result<Value, String> {
    match check_tool {
        "browser_get_text" => check_browser_get_text(args, browser).await,
        "browser_exists" => check_browser_exists(args, browser).await,
        "vision_ocr_text_contains" => check_vision_ocr_text_contains(args).await,
        "uia_text_exists" => check_uia_text_exists(args),
        other => Err(format!("unsupported check_tool: {}", other)),
    }
}

async fn check_browser_get_text(
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
) -> Result<Value, String> {
    let selector = args
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "browser_get_text requires args.selector".to_string())?;

    let result =
        browser_mcp::tools::handle_tool(browser, "get_text", json!({ "selector": selector })).await;

    let (ok, val) = super::browser_result_to_value(result);
    if !ok {
        return Err(format!(
            "browser_get_text failed: {}",
            val.get("error").and_then(|v| v.as_str()).unwrap_or("?")
        ));
    }

    // Prefer top-level `text` if present; else `result`; else stringify the value.
    let text = val
        .get("text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            val.get("result")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .or_else(|| val.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| val.to_string());

    Ok(Value::String(text))
}

async fn check_browser_exists(
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
) -> Result<Value, String> {
    let selector = args
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "browser_exists requires args.selector".to_string())?;

    let result =
        browser_mcp::tools::handle_tool(browser, "exists", json!({ "selector": selector })).await;

    let (ok, val) = super::browser_result_to_value(result);
    if !ok {
        // Not finding the element is a legitimate `false`, not an error.
        return Ok(Value::Bool(false));
    }

    let exists = val
        .get("exists")
        .and_then(|v| v.as_bool())
        .or_else(|| val.get("found").and_then(|v| v.as_bool()))
        .or_else(|| val.as_bool())
        .unwrap_or(false);
    Ok(Value::Bool(exists))
}

async fn check_vision_ocr_text_contains(args: &Value) -> Result<Value, String> {
    let needle = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "vision_ocr_text_contains requires args.text".to_string())?;

    // Pass through optional crop coordinates so the OCR call can scope itself
    // (vision-core honours x/y/width/height when present and falls back to
    // full-screen otherwise — either way we still substring-match the text).
    let mut shot_args = json!({});
    for key in &["x", "y", "width", "height"] {
        if let Some(v) = args.get(*key) {
            shot_args[*key] = v.clone();
        }
    }

    let ocr_result = vision_core::execute("vision_screenshot_ocr", &shot_args).await;
    let ocr_text = ocr_result
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let found = ocr_text.to_lowercase().contains(&needle.to_lowercase());
    Ok(Value::Bool(found))
}

fn check_uia_text_exists(args: &Value) -> Result<Value, String> {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "uia_text_exists requires args.text".to_string())?;

    let find_result = UiaFindElement.call(&json!({ "name": text, "max_depth": 6 }));
    let exists = find_result
        .get("elements")
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    Ok(Value::Bool(exists))
}

// ── Predicate evaluation ──

fn evaluate_predicate(actual: &Value, predicate: &Value, string_distance: usize) -> bool {
    let p_type = predicate.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let expected = predicate.get("value").cloned().unwrap_or(Value::Null);

    match p_type {
        "equals" => predicate_equals(actual, &expected, string_distance),
        "not_equals" => !predicate_equals(actual, &expected, string_distance),
        "contains" => predicate_contains(actual, &expected, string_distance),
        "not_contains" => !predicate_contains(actual, &expected, string_distance),
        "regex" => predicate_regex(actual, &expected),
        "gt" => predicate_numeric(actual, &expected, |a, e| a > e),
        "lt" => predicate_numeric(actual, &expected, |a, e| a < e),
        "in_range" => predicate_in_range(actual, &expected),
        _ => false,
    }
}

fn predicate_equals(actual: &Value, expected: &Value, string_distance: usize) -> bool {
    // Exact JSON equality first.
    if actual == expected {
        return true;
    }

    // String-bool coercion: "true"/"false" vs true/false.
    if let (Some(a_str), Some(e_bool)) = (actual.as_str(), expected.as_bool()) {
        if let Ok(parsed) = a_str.parse::<bool>() {
            return parsed == e_bool;
        }
    }
    if let (Some(a_bool), Some(e_str)) = (actual.as_bool(), expected.as_str()) {
        if let Ok(parsed) = e_str.parse::<bool>() {
            return a_bool == parsed;
        }
    }

    // Fuzzy string equality (Levenshtein ≤ string_distance).
    if let (Some(a_str), Some(e_str)) = (actual.as_str(), expected.as_str()) {
        if string_distance > 0 {
            return levenshtein(a_str, e_str) <= string_distance;
        }
    }

    false
}

fn predicate_contains(actual: &Value, expected: &Value, string_distance: usize) -> bool {
    let haystack = value_as_string(actual);
    let needle = match expected.as_str() {
        Some(s) => s.to_string(),
        None => value_as_string(expected),
    };

    if haystack.is_empty() && !needle.is_empty() {
        return false;
    }

    if haystack.to_lowercase().contains(&needle.to_lowercase()) {
        return true;
    }

    if string_distance == 0 || needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }

    // Sliding window approximate match.
    let h_chars: Vec<char> = haystack.to_lowercase().chars().collect();
    let n_chars: Vec<char> = needle.to_lowercase().chars().collect();
    let n_len = n_chars.len();
    if h_chars.len() < n_len {
        return false;
    }
    for start in 0..=(h_chars.len() - n_len) {
        let window: String = h_chars[start..start + n_len].iter().collect();
        let needle_str: String = n_chars.iter().collect();
        if levenshtein(&window, &needle_str) <= string_distance {
            return true;
        }
    }
    false
}

fn predicate_regex(actual: &Value, expected: &Value) -> bool {
    let pattern = match expected.as_str() {
        Some(s) => s,
        None => return false,
    };
    let haystack = value_as_string(actual);
    match regex::Regex::new(pattern) {
        Ok(re) => re.is_match(&haystack),
        Err(_) => false,
    }
}

fn predicate_numeric(actual: &Value, expected: &Value, cmp: impl Fn(f64, f64) -> bool) -> bool {
    let a = match value_as_number(actual) {
        Some(n) => n,
        None => return false,
    };
    let e = match value_as_number(expected) {
        Some(n) => n,
        None => return false,
    };
    cmp(a, e)
}

fn predicate_in_range(actual: &Value, expected: &Value) -> bool {
    let a = match value_as_number(actual) {
        Some(n) => n,
        None => return false,
    };
    let min = expected
        .get("min")
        .and_then(value_as_number)
        .unwrap_or(f64::NEG_INFINITY);
    let max = expected
        .get("max")
        .and_then(value_as_number)
        .unwrap_or(f64::INFINITY);
    a >= min && a <= max
}

// ── Coercion helpers ──

fn value_as_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => v.to_string(),
    }
}

fn value_as_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

// ── Levenshtein distance ──

/// Classic DP Levenshtein. O(n*m) time, O(min(n, m)) space.
fn levenshtein(a: &str, b: &str) -> usize {
    if a == b {
        return 0;
    }
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let n = a_chars.len();
    let m = b_chars.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (curr[j - 1] + 1) // insertion
                .min(prev[j] + 1) // deletion
                .min(prev[j - 1] + cost); // substitution
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

// ══════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── Levenshtein ──

    #[test]
    fn levenshtein_distance_zero_for_identical_strings() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("hello", "hello"), 0);
        assert_eq!(levenshtein("ai-hands", "ai-hands"), 0);
    }

    #[test]
    fn levenshtein_distance_correct_for_known_inputs() {
        // Classic Wikipedia example.
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("Saturday", "Sunday"), 3);
        assert_eq!(levenshtein("flaw", "lawn"), 2);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", "xyz"), 3);
        // One-char substitution.
        assert_eq!(levenshtein("cat", "bat"), 1);
    }

    // ── Predicate: equals ──

    #[test]
    fn predicate_equals_passes_exact() {
        assert!(predicate_equals(
            &Value::String("Welcome".into()),
            &Value::String("Welcome".into()),
            0,
        ));
        assert!(predicate_equals(&json!(true), &json!(true), 0));
        assert!(predicate_equals(&json!(42), &json!(42), 0));
        assert!(!predicate_equals(
            &Value::String("Welcome".into()),
            &Value::String("Goodbye".into()),
            0,
        ));
    }

    #[test]
    fn predicate_equals_passes_with_string_distance() {
        // 1 substitution.
        assert!(predicate_equals(
            &Value::String("Welcome".into()),
            &Value::String("Welcame".into()),
            1,
        ));
        // Distance 2 is too tight for 3 edits.
        assert!(!predicate_equals(
            &Value::String("kitten".into()),
            &Value::String("sitting".into()),
            2,
        ));
        assert!(predicate_equals(
            &Value::String("kitten".into()),
            &Value::String("sitting".into()),
            3,
        ));
    }

    // ── Predicate: contains ──

    #[test]
    fn predicate_contains_passes_substring() {
        let actual = Value::String("Welcome to AI-Hands".into());
        let expected = Value::String("Welcome".into());
        assert!(predicate_contains(&actual, &expected, 0));
    }

    #[test]
    fn predicate_contains_case_insensitive() {
        assert!(predicate_contains(
            &Value::String("Welcome to AI-Hands".into()),
            &Value::String("welcome".into()),
            0,
        ));
    }

    #[test]
    fn predicate_contains_passes_with_string_distance() {
        // OCR noise: "Welcome" → "Welcame" — should still match with tolerance.
        let actual = Value::String("Welcame to AI-Hands".into());
        let expected = Value::String("Welcome".into());
        assert!(predicate_contains(&actual, &expected, 1));
        assert!(!predicate_contains(&actual, &expected, 0));
    }

    #[test]
    fn predicate_not_contains_fails_when_present() {
        assert!(!evaluate_predicate(
            &Value::String("Welcome".into()),
            &json!({"type": "not_contains", "value": "Welcome"}),
            0,
        ));
        assert!(evaluate_predicate(
            &Value::String("Welcome".into()),
            &json!({"type": "not_contains", "value": "Error"}),
            0,
        ));
    }

    // ── Predicate: regex ──

    #[test]
    fn predicate_regex_matches() {
        assert!(predicate_regex(
            &Value::String("Order #12345 confirmed".into()),
            &Value::String(r"Order #\d+".into()),
        ));
        assert!(!predicate_regex(
            &Value::String("No numbers here".into()),
            &Value::String(r"Order #\d+".into()),
        ));
        // Invalid regex never matches.
        assert!(!predicate_regex(
            &Value::String("anything".into()),
            &Value::String(r"[".into()),
        ));
    }

    // ── Predicate: numeric ──

    #[test]
    fn predicate_gt_passes_strict() {
        assert!(evaluate_predicate(
            &json!(10),
            &json!({"type": "gt", "value": 5}),
            0,
        ));
        assert!(!evaluate_predicate(
            &json!(5),
            &json!({"type": "gt", "value": 5}),
            0,
        ));
        assert!(!evaluate_predicate(
            &json!(4),
            &json!({"type": "gt", "value": 5}),
            0,
        ));
    }

    #[test]
    fn predicate_lt_passes_strict() {
        assert!(evaluate_predicate(
            &json!(3),
            &json!({"type": "lt", "value": 5}),
            0,
        ));
        assert!(!evaluate_predicate(
            &json!(5),
            &json!({"type": "lt", "value": 5}),
            0,
        ));
    }

    #[test]
    fn predicate_in_range_passes_inclusive() {
        let pred = json!({"type": "in_range", "value": {"min": 0, "max": 100}});
        assert!(evaluate_predicate(&json!(0), &pred, 0));
        assert!(evaluate_predicate(&json!(50), &pred, 0));
        assert!(evaluate_predicate(&json!(100), &pred, 0));
        assert!(!evaluate_predicate(&json!(-1), &pred, 0));
        assert!(!evaluate_predicate(&json!(101), &pred, 0));
    }

    #[test]
    fn predicate_in_range_coerces_strings() {
        let pred = json!({"type": "in_range", "value": {"min": 0, "max": 100}});
        assert!(evaluate_predicate(&Value::String("42".into()), &pred, 0));
    }

    // ── Whitelist + dispatch shape ──

    #[test]
    fn unsupported_check_tool_is_rejected() {
        assert!(!is_supported_check_tool("eval"));
        assert!(!is_supported_check_tool("browser_eval"));
        assert!(is_supported_check_tool("browser_get_text"));
        assert!(is_supported_check_tool("browser_exists"));
        assert!(is_supported_check_tool("vision_ocr_text_contains"));
        assert!(is_supported_check_tool("uia_text_exists"));
    }

    #[test]
    fn evaluate_predicate_unknown_type_fails() {
        assert!(!evaluate_predicate(
            &Value::String("anything".into()),
            &json!({"type": "totally_made_up", "value": "x"}),
            0,
        ));
    }
}
