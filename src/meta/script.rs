//! `hands_script` — multi-step meta-tool orchestrator.
//!
//! Executes a sequence of hands meta-tool calls with variable substitution,
//! output capture, and per-step error handling.
//!
//! Features:
//!   - {{var}} and {{var.field.subfield}} variable substitution (up to 3 levels)
//!   - Per-step output capture into variables map
//!   - on_error: stop | skip | retry
//!   - Per-step and overall timeout_ms
//!   - Verbose mode with full MetaToolResult per step

use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;

use super::error::MetaError;
use super::instrumentation;
use super::response::{MetaToolResult, RungAttempt};
use super::session::SharedSession;

/// Per-step error handling policy.
#[derive(Debug, Clone, PartialEq)]
enum OnError {
    Stop,
    Skip,
    Retry,
}

impl OnError {
    fn from_str(s: &str) -> Self {
        match s {
            "skip" => Self::Skip,
            "retry" => Self::Retry,
            _ => Self::Stop,
        }
    }
}

/// Parsed step definition.
struct StepDef {
    tool: String,
    args: Value,
    label: Option<String>,
    output_var: Option<String>,
    on_error: OnError,
    timeout_ms: Option<u64>,
}

/// Per-step execution result for the response.
struct StepResult {
    index: usize,
    label: Option<String>,
    tool: String,
    success: bool,
    elapsed_ms: u64,
    result: Value,
    error: Option<String>,
    retried: bool,
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

    // ── Parse steps array ──
    let steps_raw = match args.get("steps").and_then(|v| v.as_array()) {
        Some(arr) => arr.clone(),
        None => {
            instrumentation::log_aggregate(
                "hands_script",
                &call_id,
                false,
                "",
                0,
                0,
                None,
                Some("steps array is required"),
            );
            return MetaToolResult::failure(vec![], MetaError::other("steps array is required"), 0)
                .to_value();
        }
    };

    if steps_raw.is_empty() {
        return MetaToolResult::failure(
            vec![],
            MetaError::other("steps array must not be empty"),
            0,
        )
        .to_value();
    }

    let stop_on_error = args
        .get("stop_on_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let verbose = args
        .get("verbose")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let overall_timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(60_000);

    // ── Initialize variables map ──
    let mut variables: HashMap<String, Value> = HashMap::new();
    if let Some(init_vars) = args.get("variables").and_then(|v| v.as_object()) {
        for (k, v) in init_vars {
            variables.insert(k.clone(), v.clone());
        }
    }

    // ── Parse step definitions ──
    let steps: Vec<StepDef> = steps_raw
        .iter()
        .map(|s| StepDef {
            tool: s
                .get("tool")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            args: s.get("args").cloned().unwrap_or(json!({})),
            label: s
                .get("label")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            output_var: s
                .get("output_var")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            on_error: OnError::from_str(
                s.get("on_error").and_then(|v| v.as_str()).unwrap_or("stop"),
            ),
            timeout_ms: s.get("timeout_ms").and_then(|v| v.as_u64()),
        })
        .collect();

    // ── Execute steps ──
    let mut step_results: Vec<StepResult> = Vec::new();
    let mut steps_succeeded: usize = 0;
    let mut steps_failed: usize = 0;
    let mut failed_step: Option<Value> = None;
    let mut rungs_tried: Vec<RungAttempt> = Vec::new();

    for (idx, step) in steps.iter().enumerate() {
        // Check overall timeout
        let elapsed_total = start.elapsed().as_millis() as u64;
        if elapsed_total > overall_timeout_ms {
            let timeout_err = MetaError::timeout("hands_script overall", elapsed_total);
            let result = build_script_result(
                idx,
                steps_succeeded,
                steps_failed,
                &failed_step,
                &variables,
                &step_results,
                verbose,
                elapsed_total,
                &rungs_tried,
            );
            // Add timeout warning
            if let Some(obj) = result.as_object() {
                let mut r = obj.clone();
                r.insert("timeout".to_string(), json!(true));
                r.insert(
                    "timeout_error".to_string(),
                    serde_json::to_value(&timeout_err).unwrap_or(json!("timeout")),
                );
                return json!(r);
            }
            return result;
        }

        // Validate step has a tool name
        if step.tool.is_empty() {
            let err_msg = format!("Step {} has no tool name", idx);
            step_results.push(StepResult {
                index: idx,
                label: step.label.clone(),
                tool: String::new(),
                success: false,
                elapsed_ms: 0,
                result: Value::Null,
                error: Some(err_msg.clone()),
                retried: false,
            });
            steps_failed += 1;
            failed_step = Some(json!({
                "index": idx,
                "label": step.label,
                "error": err_msg,
            }));
            if stop_on_error {
                break;
            }
            continue;
        }

        // Execute (with optional retry)
        let max_attempts = if step.on_error == OnError::Retry {
            2
        } else {
            1
        };
        let mut attempt_result: Option<StepResult> = None;

        for attempt in 0..max_attempts {
            let retried = attempt > 0;
            let step_start = Instant::now();

            // Substitute variables into step args
            let resolved_args = substitute_variables(&step.args, &variables);

            // Execute the meta-tool
            let step_result = execute_step(
                &step.tool,
                &resolved_args,
                browser,
                session,
                step.timeout_ms,
            )
            .await;

            let step_elapsed = step_start.elapsed().as_millis() as u64;

            match step_result {
                StepOutcome::Success(result_value) => {
                    // Capture output variable
                    if let Some(ref var_name) = step.output_var {
                        variables.insert(var_name.clone(), result_value.clone());
                    }

                    let rung = RungAttempt::ok(format!("step_{}/{}", idx, step.tool), step_elapsed);
                    rungs_tried.push(rung);

                    attempt_result = Some(StepResult {
                        index: idx,
                        label: step.label.clone(),
                        tool: step.tool.clone(),
                        success: true,
                        elapsed_ms: step_elapsed,
                        result: result_value,
                        error: None,
                        retried,
                    });
                    break; // success, no more attempts
                }
                StepOutcome::Failed(err_msg, result_value) => {
                    // Store partial result in output_var even on failure
                    if let Some(ref var_name) = step.output_var {
                        variables.insert(var_name.clone(), result_value.clone());
                    }

                    let rung = RungAttempt::failed(
                        format!("step_{}/{}", idx, step.tool),
                        step_elapsed,
                        &err_msg,
                    );
                    rungs_tried.push(rung);

                    attempt_result = Some(StepResult {
                        index: idx,
                        label: step.label.clone(),
                        tool: step.tool.clone(),
                        success: false,
                        elapsed_ms: step_elapsed,
                        result: result_value,
                        error: Some(err_msg),
                        retried,
                    });
                    // If retry policy and first attempt, loop again
                    if step.on_error == OnError::Retry && attempt == 0 {
                        // Small delay before retry
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        continue;
                    }
                    break;
                }
                StepOutcome::NotAMetaTool => {
                    let err_msg = format!("'{}' is not a recognized meta-tool", step.tool);
                    let rung =
                        RungAttempt::failed(format!("step_{}/{}", idx, step.tool), 0, &err_msg);
                    rungs_tried.push(rung);

                    attempt_result = Some(StepResult {
                        index: idx,
                        label: step.label.clone(),
                        tool: step.tool.clone(),
                        success: false,
                        elapsed_ms: 0,
                        result: Value::Null,
                        error: Some(err_msg),
                        retried,
                    });
                    break;
                }
            }
        }

        if let Some(sr) = attempt_result {
            let is_success = sr.success;
            if is_success {
                steps_succeeded += 1;
            } else {
                steps_failed += 1;
                if failed_step.is_none() {
                    failed_step = Some(json!({
                        "index": sr.index,
                        "label": sr.label,
                        "error": sr.error,
                    }));
                }
            }
            step_results.push(sr);

            // Stop on error if configured
            if !is_success && (stop_on_error || step.on_error == OnError::Stop) {
                break;
            }
        }
    }

    let elapsed = start.elapsed().as_millis() as u64;

    instrumentation::log_aggregate(
        "hands_script",
        &call_id,
        steps_failed == 0,
        &format!("{}/{} steps", steps_succeeded, step_results.len()),
        rungs_tried.len(),
        elapsed,
        None,
        if steps_failed > 0 {
            Some("script had failures")
        } else {
            None
        },
    );

    build_script_result(
        step_results.len(),
        steps_succeeded,
        steps_failed,
        &failed_step,
        &variables,
        &step_results,
        verbose,
        elapsed,
        &rungs_tried,
    )
}

// ── Step execution ──

enum StepOutcome {
    Success(Value),
    Failed(String, Value),
    NotAMetaTool,
}

async fn execute_step(
    tool: &str,
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
    session: &SharedSession,
    timeout_ms: Option<u64>,
) -> StepOutcome {
    let effective_timeout = timeout_ms.unwrap_or(30_000);
    let timeout_dur = std::time::Duration::from_millis(effective_timeout);

    // Phase C fix3: call meta-tool handlers DIRECTLY instead of going through
    // handle_meta_tool → dispatch_meta_tool. This avoids:
    //   1. Re-entrant dispatcher overhead (double timeout wrappers)
    //   2. Potential deadlock when meta-tools call other meta-tools
    //   3. The generic timeout error that masks the real result
    // The script's own per-step timeout is the single timeout layer.
    let direct_result: Option<Result<Value, _>> = match tool {
        "hands_read_page" => Some(
            tokio::time::timeout(
                timeout_dur,
                super::read_page::handle(args, browser, session),
            )
            .await,
        ),
        "hands_click" => Some(
            tokio::time::timeout(timeout_dur, super::click::handle(args, browser, session)).await,
        ),
        "hands_navigate" => Some(
            tokio::time::timeout(timeout_dur, super::navigate::handle(args, browser, session))
                .await,
        ),
        "hands_capture" => Some(
            tokio::time::timeout(timeout_dur, super::capture::handle(args, browser, session)).await,
        ),
        "hands_find" => Some(
            tokio::time::timeout(timeout_dur, super::find::handle(args, browser, session)).await,
        ),
        "hands_type" => Some(
            tokio::time::timeout(
                timeout_dur,
                super::type_text::handle(args, browser, session),
            )
            .await,
        ),
        "hands_fill_form" => Some(
            tokio::time::timeout(
                timeout_dur,
                super::fill_form::handle(args, browser, session),
            )
            .await,
        ),
        "hands_verify" => Some(
            tokio::time::timeout(timeout_dur, super::verify::handle(args, browser, session)).await,
        ),
        "hands_scan_qr" => Some(
            tokio::time::timeout(timeout_dur, super::qr_scan::handle(args, browser, session)).await,
        ),
        "hands_app_action" => Some(
            tokio::time::timeout(
                timeout_dur,
                super::app_action::handle(args, browser, session),
            )
            .await,
        ),
        "hands_script" => {
            Some(tokio::time::timeout(timeout_dur, Box::pin(handle(args, browser, session))).await)
        }
        "hands_login_recovery" => {
            let script_payload = super::templates::login::build_login_script(args);
            Some(
                tokio::time::timeout(
                    timeout_dur,
                    Box::pin(handle(&script_payload, browser, session)),
                )
                .await,
            )
        }
        _ => None,
    };

    // If we got a direct result, classify it
    if let Some(timeout_result) = direct_result {
        return match timeout_result {
            Ok(value) => classify_step_result(value),
            Err(_) => StepOutcome::Failed(
                format!("Step timed out after {}ms", effective_timeout),
                json!({"timeout": true, "timeout_ms": effective_timeout}),
            ),
        };
    }

    // Not a known meta-tool — try underlying tool layers as fallthrough
    eprintln!(
        "[hands_script] '{}' not a meta-tool, trying underlying dispatch",
        tool
    );
    let fallthrough = Box::pin(dispatch_underlying_tool(tool, args, browser));
    match tokio::time::timeout(timeout_dur, fallthrough).await {
        Ok(Some(value)) => {
            eprintln!(
                "[hands_script] fallthrough resolved '{}' via underlying layer",
                tool
            );
            classify_step_result(value)
        }
        Ok(None) => {
            // Try name variants before giving up
            let variants = tool_name_variants(tool);
            for variant in &variants {
                let under_fut = Box::pin(dispatch_underlying_tool(&variant, args, browser));
                if let Ok(Some(value)) = tokio::time::timeout(timeout_dur, under_fut).await {
                    eprintln!(
                        "[hands_script] name resolver: '{}' resolved as underlying '{}'",
                        tool, variant
                    );
                    return classify_step_result(value);
                }
            }
            StepOutcome::NotAMetaTool
        }
        Err(_) => StepOutcome::Failed(
            format!("Step timed out after {}ms", effective_timeout),
            json!({"timeout": true, "timeout_ms": effective_timeout}),
        ),
    }
}

/// Classify a tool result value into Success or Failed.
fn classify_step_result(value: Value) -> StepOutcome {
    let success = value
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if success {
        StepOutcome::Success(value)
    } else {
        let err = value
            .get("error")
            .map(|e| {
                if let Some(msg) = e.as_str() {
                    msg.to_string()
                } else {
                    serde_json::to_string(e).unwrap_or_else(|_| "unknown error".into())
                }
            })
            .unwrap_or_else(|| "meta-tool returned success=false".into());
        StepOutcome::Failed(err, value)
    }
}

/// Try dispatching a tool name to underlying subsystems (uia_lib, vision_core, browser_mcp).
/// Returns Some(Value) if the tool was found and executed, None if unrecognized.
async fn dispatch_underlying_tool(
    tool: &str,
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
) -> Option<Value> {
    if tool.starts_with("uia_") {
        let result = uia_lib::handle_tool_call(tool, args);
        // Check if uia_lib recognized the tool (vs returning "Unknown tool")
        if result
            .get("error")
            .and_then(|e| e.as_str())
            .map(|e| e.starts_with("Unknown tool"))
            .unwrap_or(false)
        {
            return None;
        }
        return Some(result);
    }
    if tool.starts_with("vision_") {
        let result = vision_core::execute(tool, args).await;
        return Some(result);
    }
    if tool.starts_with("browser_") {
        if let Some(browser_tool) = tool.strip_prefix("browser_") {
            let result = browser_mcp::tools::handle_tool(browser, browser_tool, args.clone()).await;
            let (_, val) = super::browser_result_to_value(result);
            return Some(val);
        }
    }
    None
}

/// Generate name variants to try when a tool name isn't found directly.
/// Tries with/without common prefixes (hands_, uia_, browser_).
fn tool_name_variants(name: &str) -> Vec<String> {
    let mut variants = Vec::new();
    // Try adding hands_ prefix
    if !name.starts_with("hands_") {
        variants.push(format!("hands_{}", name));
    }
    // Try removing hands_ prefix
    if let Some(stripped) = name.strip_prefix("hands_") {
        variants.push(stripped.to_string());
    }
    // Try adding/removing uia_ prefix
    if !name.starts_with("uia_") {
        variants.push(format!("uia_{}", name));
    }
    if let Some(stripped) = name.strip_prefix("uia_") {
        variants.push(stripped.to_string());
    }
    // Try adding/removing browser_ prefix
    if !name.starts_with("browser_") {
        variants.push(format!("browser_{}", name));
    }
    if let Some(stripped) = name.strip_prefix("browser_") {
        variants.push(stripped.to_string());
    }
    variants
}

// ── Response builder ──

fn build_script_result(
    steps_attempted: usize,
    steps_succeeded: usize,
    steps_failed: usize,
    failed_step: &Option<Value>,
    variables: &HashMap<String, Value>,
    step_results: &[StepResult],
    verbose: bool,
    elapsed_ms: u64,
    rungs_tried: &[RungAttempt],
) -> Value {
    let mut response = json!({
        "success": steps_failed == 0,
        "steps_attempted": steps_attempted,
        "steps_succeeded": steps_succeeded,
        "steps_failed": steps_failed,
        "elapsed_ms": elapsed_ms,
        "reversibility": "reversible",
    });

    if let Some(ref fs) = failed_step {
        response["failed_step"] = fs.clone();
    }

    // Convert variables to JSON object
    let vars_obj: serde_json::Map<String, Value> = variables
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    response["variables_final"] = Value::Object(vars_obj);

    if verbose {
        let per_step: Vec<Value> = step_results
            .iter()
            .map(|sr| {
                json!({
                    "index": sr.index,
                    "label": sr.label,
                    "tool": sr.tool,
                    "success": sr.success,
                    "elapsed_ms": sr.elapsed_ms,
                    "result": sr.result,
                    "error": sr.error,
                    "retried": sr.retried,
                })
            })
            .collect();
        response["per_step"] = json!(per_step);
    }

    // Attach rungs_tried for MetaToolResult compatibility
    if verbose {
        let rungs: Vec<Value> = rungs_tried
            .iter()
            .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
            .collect();
        response["rungs_tried"] = json!(rungs);
    }

    response
}

// ── Variable substitution ──

/// Substitute {{var}} and {{var.field.subfield}} in a JSON value.
/// Recursively walks the value tree, replacing template patterns in strings.
pub(crate) fn substitute_variables(value: &Value, vars: &HashMap<String, Value>) -> Value {
    match value {
        Value::String(s) => substitute_in_string(s, vars),
        Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                new_map.insert(k.clone(), substitute_variables(v, vars));
            }
            Value::Object(new_map)
        }
        Value::Array(arr) => {
            Value::Array(arr.iter().map(|v| substitute_variables(v, vars)).collect())
        }
        other => other.clone(),
    }
}

/// Substitute template variables in a string value.
///
/// If the entire string is a single `{{var}}` reference, the raw Value is returned
/// (preserving types like numbers, objects, arrays). If mixed with other text,
/// the value is stringified and interpolated.
///
/// Supports dot-notation up to 3 levels: `{{var.field.subfield}}`.
fn substitute_in_string(s: &str, vars: &HashMap<String, Value>) -> Value {
    let trimmed = s.trim();

    // Fast path: entire string is a single {{var}} or {{var.field}} reference
    if trimmed.starts_with("{{") && trimmed.ends_with("}}") && trimmed.matches("{{").count() == 1 {
        let key = &trimmed[2..trimmed.len() - 2];
        if let Some(val) = resolve_dotted_key(key, vars) {
            return val.clone();
        }
    }

    // Slow path: mixed text with embedded {{var}} references
    let mut result = s.to_string();
    let mut search_from = 0;

    loop {
        let open = match result[search_from..].find("{{") {
            Some(pos) => search_from + pos,
            None => break,
        };
        let close = match result[open..].find("}}") {
            Some(pos) => open + pos,
            None => break,
        };

        let key = &result[open + 2..close];
        if let Some(val) = resolve_dotted_key(key, vars) {
            let replacement = value_to_string(val);
            result = format!("{}{}{}", &result[..open], replacement, &result[close + 2..]);
            search_from = open + replacement.len();
        } else {
            // Variable not found — leave the placeholder intact
            search_from = close + 2;
        }
    }

    Value::String(result)
}

/// Resolve a dotted key path like "result.data.id" against the variables map.
/// Supports up to 3 levels of nesting.
fn resolve_dotted_key<'a>(key: &str, vars: &'a HashMap<String, Value>) -> Option<&'a Value> {
    let parts: Vec<&str> = key.splitn(4, '.').collect();
    if parts.is_empty() {
        return None;
    }

    // Look up the root variable
    let root = vars.get(parts[0])?;

    // Navigate dot-path (up to 3 levels total = root + 2 sub-fields)
    let mut current = root;
    for &part in &parts[1..] {
        match current {
            Value::Object(map) => {
                current = map.get(part)?;
            }
            Value::Array(arr) => {
                // Support numeric index access: {{var.0}}, {{var.1}}
                let idx: usize = part.parse().ok()?;
                current = arr.get(idx)?;
            }
            _ => return None,
        }
    }

    Some(current)
}

/// Convert a Value to a string for interpolation into mixed-text templates.
fn value_to_string(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        // Objects and arrays get JSON-serialized
        other => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_substitute_simple_var() {
        let mut vars = HashMap::new();
        vars.insert("url".to_string(), json!("https://example.com"));
        vars.insert("count".to_string(), json!(42));

        let input = json!({"target": "{{url}}", "limit": "{{count}}"});
        let result = substitute_variables(&input, &vars);

        assert_eq!(result["target"], "https://example.com");
        // Single {{count}} returns raw number
        assert_eq!(result["limit"], 42);
    }

    #[test]
    fn test_substitute_dotted_path() {
        let mut vars = HashMap::new();
        vars.insert(
            "result".to_string(),
            json!({
                "data": {
                    "id": 123,
                    "name": "test"
                }
            }),
        );

        let input = json!({"id": "{{result.data.id}}", "name": "{{result.data.name}}"});
        let result = substitute_variables(&input, &vars);

        assert_eq!(result["id"], 123);
        assert_eq!(result["name"], "test");
    }

    #[test]
    fn test_substitute_mixed_text() {
        let mut vars = HashMap::new();
        vars.insert("host".to_string(), json!("example.com"));
        vars.insert("port".to_string(), json!(8080));

        let input = json!("https://{{host}}:{{port}}/api");
        let result = substitute_variables(&input, &vars);

        assert_eq!(result, "https://example.com:8080/api");
    }

    #[test]
    fn test_substitute_missing_var_preserved() {
        let vars = HashMap::new();
        let input = json!("Hello {{unknown}}");
        let result = substitute_variables(&input, &vars);
        assert_eq!(result, "Hello {{unknown}}");
    }

    #[test]
    fn test_substitute_array_index() {
        let mut vars = HashMap::new();
        vars.insert("items".to_string(), json!(["a", "b", "c"]));

        let input = json!("{{items.1}}");
        let result = substitute_variables(&input, &vars);
        assert_eq!(result, "b");
    }

    #[test]
    fn test_substitute_preserves_object() {
        let mut vars = HashMap::new();
        vars.insert(
            "config".to_string(),
            json!({"key": "value", "nested": true}),
        );

        // Entire string is a single reference — raw value preserved
        let input = json!("{{config}}");
        let result = substitute_variables(&input, &vars);
        assert!(result.is_object());
        assert_eq!(result["key"], "value");
    }

    #[test]
    fn test_substitute_in_nested_structure() {
        let mut vars = HashMap::new();
        vars.insert("email".to_string(), json!("user@example.com"));
        vars.insert("pass".to_string(), json!("secret"));

        let input = json!({
            "fields": {
                "Email": "{{email}}",
                "Password": "{{pass}}"
            },
            "tags": ["{{email}}", "extra"]
        });
        let result = substitute_variables(&input, &vars);

        assert_eq!(result["fields"]["Email"], "user@example.com");
        assert_eq!(result["fields"]["Password"], "secret");
        assert_eq!(result["tags"][0], "user@example.com");
        assert_eq!(result["tags"][1], "extra");
    }
}
