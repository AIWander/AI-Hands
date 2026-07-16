//! `hands_fill_form` — automated form filling with per-field tracking.
//!
//! Ladder:
//!   1. A11y pre-scan: build label→input map with field_role for each
//!   2. Per-field: hands_click(label) + appropriate action (type/select/check)
//!   3. Escalate ONLY failed fields (don't re-fill already-filled ones)
//!
//! Features:
//! - Pre-scan extracts field structure plus has_value/value_length, never values
//! - Label matching priority: exact > startsWith > contains > fuzzy
//! - Per-field success tracking with separate filled/failed lists
//! - DOM change detection between fields: re-snapshot if field count changes
//! - Checkbox/radio state-aware: read checked first, click only on mismatch
//! - Autofill detection before marking required fields as missing
//! - Submit with spatial proximity tiebreaker

use serde_json::{json, Value};
use std::time::Instant;

use super::autofill;
use super::consent;
use super::error::MetaError;
use super::field_role::FieldRole;
use super::instrumentation;
use super::label_match;
use super::response::{Confidence, MetaToolResult, Reversibility, RungAttempt};
use super::reversibility as rev;
use super::session::SharedSession;

/// JS to pre-scan all form fields and their labels.
const JS_FORM_PRESCAN: &str = r#"
(function() {
    var fields = [];
    var inputs = document.querySelectorAll('input, textarea, select');
    inputs.forEach(function(el) {
        if (el.type === 'hidden' || el.type === 'submit' || el.type === 'button') return;
        var label = '';
        // 1. Associated <label>
        if (el.id) {
            var lbl = document.querySelector('label[for="' + el.id + '"]');
            if (lbl) label = lbl.textContent.trim();
        }
        // 2. Parent label
        if (!label) {
            var parent = el.closest('label');
            if (parent) label = parent.textContent.trim();
        }
        // 3. aria-label
        if (!label) label = el.getAttribute('aria-label') || '';
        // 4. placeholder
        if (!label) label = el.placeholder || '';
        // 5. name attribute as last resort
        if (!label) label = el.name || '';

        var rect = el.getBoundingClientRect();
        fields.push({
            label: label,
            tag: el.tagName,
            type: el.type || '',
            role: el.getAttribute('role') || '',
            autocomplete: el.autocomplete || el.getAttribute('autocomplete') || '',
            inputmode: el.inputMode || el.getAttribute('inputmode') || '',
            required: el.required || el.getAttribute('aria-required') === 'true',
            has_value: (el.value || '').length > 0,
            value_length: (el.value || '').length,
            checked: el.checked || false,
            disabled: el.disabled || el.getAttribute('aria-disabled') === 'true',
            id: el.id || '',
            name: el.name || '',
            x: Math.round(rect.left + rect.width / 2),
            y: Math.round(rect.top + rect.height / 2)
        });
    });
    return fields;
})()
"#;

/// JS to find submit buttons with their positions.
const JS_FIND_SUBMIT_BUTTONS: &str = r#"
(function() {
    var buttons = [];
    var candidates = document.querySelectorAll(
        'button[type="submit"], input[type="submit"], button:not([type]), [role="button"]'
    );
    candidates.forEach(function(el) {
        var text = el.textContent || el.value || el.getAttribute('aria-label') || '';
        var rect = el.getBoundingClientRect();
        if (rect.width > 0 && rect.height > 0) {
            buttons.push({
                text: text.trim(),
                tag: el.tagName,
                type: el.type || '',
                x: Math.round(rect.left + rect.width / 2),
                y: Math.round(rect.top + rect.height / 2)
            });
        }
    });
    return buttons;
})()
"#;

/// Default submit label patterns to search for.
pub(super) const DEFAULT_SUBMIT_LABELS: &[&str] = &[
    "Submit",
    "Sign Up",
    "Sign In",
    "Continue",
    "Next",
    "Save",
    "Create",
    "Confirm",
    "Apply",
    "Log In",
    "Register",
    "Create Account",
];

/// Per-field fill result with method tracking.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FieldFillResult {
    pub label: String,
    pub success: bool,
    pub method: String, // "typed", "selected", "toggled", "autofill", "skipped", "failed"
    pub reason: Option<String>,
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

    // Parse field values from args
    let fields = match args.get("fields").and_then(|v| v.as_object()) {
        Some(f) => f.clone(),
        None => {
            return MetaToolResult::failure(
                vec![],
                MetaError::other("fields is required (object mapping label→value)"),
                0,
            )
            .to_value();
        }
    };

    let auto_submit = args
        .get("auto_submit")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let submit_label = args
        .get("submit_label")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });

    let ctx = json!({
        "field_count": fields.len(),
        "auto_submit": auto_submit,
    });

    let mut rungs_tried = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // ── Step 1: Pre-scan form fields ──
    let scan_start = Instant::now();
    let browser_active = super::browser_is_active(browser).await;

    if !browser_active {
        let elapsed = start.elapsed().as_millis() as u64;
        return MetaToolResult::failure(vec![], MetaError::no_browser(), elapsed).to_value();
    }

    let scan_result = browser_mcp::tools::handle_tool(
        browser,
        "evaluate",
        json!({"expression": JS_FORM_PRESCAN}),
    )
    .await;
    let (scan_ok, scan_val) = super::browser_result_to_value(scan_result);
    let scan_ms = scan_start.elapsed().as_millis() as u64;

    if !scan_ok {
        rungs_tried.push(RungAttempt::failed(
            "form_prescan",
            scan_ms,
            "Pre-scan failed",
        ));
        let elapsed = start.elapsed().as_millis() as u64;
        return MetaToolResult::failure(
            rungs_tried,
            MetaError::other("Form pre-scan failed"),
            elapsed,
        )
        .to_value();
    }

    // Parse scanned fields
    let form_fields = scan_val
        .as_array()
        .or_else(|| scan_val.get("result").and_then(|v| v.as_array()))
        .cloned()
        .unwrap_or_default();

    let initial_field_count = form_fields.len();
    rungs_tried.push(RungAttempt::ok("form_prescan", scan_ms));
    instrumentation::log_rung_attempt(
        "hands_fill_form",
        &call_id,
        "form_prescan",
        true,
        scan_ms,
        None,
        &ctx,
    );

    // Build label candidates for matching
    let label_candidates: Vec<label_match::LabelCandidate> = form_fields
        .iter()
        .enumerate()
        .map(|(i, f)| label_match::LabelCandidate {
            text: f
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            role: f.get("role").and_then(|v| v.as_str()).map(String::from),
            selector: f
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|id| !id.is_empty())
                .map(|id| format!("#{}", id)),
            index: i,
        })
        .collect();

    // ── Step 2: Fill each field ──
    let mut filled: Vec<String> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    let mut field_results: Vec<FieldFillResult> = Vec::new();
    let mut last_fill_coords: Option<(i64, i64)> = None;

    for (label, value) in &fields {
        let value_str = value.as_str().unwrap_or("");

        // Find matching form field
        let match_result = label_match::find_best_match(label, &label_candidates);

        let matched_field = match match_result {
            Ok(m) => m,
            Err(e) => {
                failed.push(json!({
                    "label": label,
                    "reason": format!("{}", e),
                }));
                field_results.push(FieldFillResult {
                    label: label.clone(),
                    success: false,
                    method: "failed".into(),
                    reason: Some(format!("{}", e)),
                });
                continue;
            }
        };

        let field_data = &form_fields[matched_field.index];
        let field_role = FieldRole::detect(field_data);
        let field_x = field_data.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
        let field_y = field_data.get("y").and_then(|v| v.as_i64()).unwrap_or(0);

        // Check if field is disabled
        let disabled = field_data
            .get("disabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if disabled {
            failed.push(json!({
                "label": label,
                "reason": "Field is disabled",
            }));
            field_results.push(FieldFillResult {
                label: label.clone(),
                success: false,
                method: "skipped".into(),
                reason: Some("Field is disabled".into()),
            });
            continue;
        }

        // ── Autofill detection: check if browser has already filled this field ──
        let field_selector = field_data
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|id| !id.is_empty())
            .map(|id| format!("#{}", id));
        if let Some(ref sel) = field_selector {
            let selector_js = serde_json::to_string(sel).unwrap_or_else(|_| "\"\"".to_string());
            let autofill_js = format!("({})({})", autofill::JS_CHECK_AUTOFILL, selector_js);
            let af_result = browser_mcp::tools::handle_tool(
                browser,
                "evaluate",
                json!({"expression": autofill_js}),
            )
            .await;
            let (af_ok, af_val) = super::browser_result_to_value(af_result);
            if af_ok {
                let af_data = af_val.get("result").unwrap_or(&af_val);
                let af_state = autofill::parse_autofill_result(af_data, field_role);
                if af_state.detected && af_state.expected_shape_match {
                    // Browser autofilled this field with a valid value — skip filling
                    filled.push(label.clone());
                    field_results.push(FieldFillResult {
                        label: label.clone(),
                        success: true,
                        method: "autofill".into(),
                        reason: Some("Browser autofill detected; value withheld".into()),
                    });
                    last_fill_coords = Some((field_x, field_y));
                    continue;
                }
            }
        }

        // Handle by field type
        let (fill_ok, fill_method) = match field_role {
            FieldRole::File => {
                // File inputs can't be filled programmatically
                failed.push(json!({
                    "label": label,
                    "reason": "File input requires user interaction",
                }));
                field_results.push(FieldFillResult {
                    label: label.clone(),
                    success: false,
                    method: "failed".into(),
                    reason: Some("File input requires user interaction".into()),
                });
                (false, "failed")
            }
            FieldRole::Checkbox | FieldRole::Radio => {
                // State-aware: only click if current state doesn't match desired
                let current_checked = field_data
                    .get("checked")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let desired = value_str == "true" || value_str == "1" || value_str == "checked";

                if current_checked != desired {
                    // Click to toggle
                    let click_result = browser_mcp::tools::handle_tool(
                        browser,
                        "click",
                        json!({"x": field_x, "y": field_y}),
                    )
                    .await;
                    let (ok, _) = super::browser_result_to_value(click_result);
                    (ok, "toggled")
                } else {
                    (true, "skipped") // Already in correct state
                }
            }
            FieldRole::Select => {
                // Click to open dropdown, then select option
                let selector = matched_field.selector.as_deref().or_else(|| {
                    field_data
                        .get("id")
                        .and_then(|v| v.as_str())
                        .filter(|id| !id.is_empty())
                });

                if let Some(sel_id) = selector {
                    let select_result = browser_mcp::tools::handle_tool(
                        browser,
                        "select",
                        json!({"selector": format!("#{}", sel_id), "value": value_str}),
                    )
                    .await;
                    let (ok, _) = super::browser_result_to_value(select_result);
                    (ok, "selected")
                } else {
                    // Fallback: click field, then type value
                    let _ = browser_mcp::tools::handle_tool(
                        browser,
                        "click",
                        json!({"x": field_x, "y": field_y}),
                    )
                    .await;
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

                    // Type option text to filter/select
                    let _ = browser_mcp::tools::handle_tool(
                        browser,
                        "type_text",
                        json!({"text": value_str}),
                    )
                    .await;
                    let _ =
                        browser_mcp::tools::handle_tool(browser, "press", json!({"key": "Enter"}))
                            .await;
                    (true, "selected")
                }
            }
            _ => {
                // Text-like field: use hands_type internally
                let type_result = super::type_text::handle(
                    &json!({
                        "target": label,
                        "text": value_str,
                        "clear_first": true,
                        "verify_focus": true,
                        "fast_set": !field_role.is_sensitive(),
                    }),
                    browser,
                    session,
                )
                .await;

                let ok = type_result
                    .get("success")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                (ok, "typed")
            }
        };

        if fill_ok {
            filled.push(label.clone());
            last_fill_coords = Some((field_x, field_y));
            if fill_method != "failed" {
                field_results.push(FieldFillResult {
                    label: label.clone(),
                    success: true,
                    method: fill_method.into(),
                    reason: None,
                });
            }
        } else if field_role != FieldRole::File {
            failed.push(json!({
                "label": label,
                "reason": "Fill action failed",
                "field_role": field_role,
            }));
            field_results.push(FieldFillResult {
                label: label.clone(),
                success: false,
                method: "failed".into(),
                reason: Some("Fill action failed".into()),
            });
        }

        // DOM change detection: re-scan if field count may have changed
        // (dynamic forms that reveal fields after earlier fields are filled)
        if !filled.is_empty() && filled.len().is_multiple_of(3) {
            let rescan = browser_mcp::tools::handle_tool(
                browser,
                "evaluate",
                json!({"expression": JS_FORM_PRESCAN}),
            )
            .await;
            let (_, rescan_val) = super::browser_result_to_value(rescan);
            let new_count = rescan_val
                .as_array()
                .or_else(|| rescan_val.get("result").and_then(|v| v.as_array()))
                .map(|a| a.len())
                .unwrap_or(0);

            if new_count != initial_field_count {
                warnings.push(format!(
                    "Form field count changed from {} to {} — dynamic form detected",
                    initial_field_count, new_count
                ));
            }
        }

        // Small settle between fields
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    }

    // ── Step 3: Submit handling ──
    let mut submit_result = json!(null);
    let mut submit_method = String::new();
    let mut submit_confidence = 0.0f32;

    if auto_submit || args.get("submit_label").is_some() {
        let submit_labels = submit_label
            .as_ref()
            .map(|v| v.iter().map(|s| s.as_str()).collect::<Vec<_>>())
            .unwrap_or_else(|| DEFAULT_SUBMIT_LABELS.to_vec());

        // Find submit buttons
        let btn_result = browser_mcp::tools::handle_tool(
            browser,
            "evaluate",
            json!({"expression": JS_FIND_SUBMIT_BUTTONS}),
        )
        .await;
        let (_, btn_val) = super::browser_result_to_value(btn_result);
        let buttons = btn_val
            .as_array()
            .or_else(|| btn_val.get("result").and_then(|v| v.as_array()))
            .cloned()
            .unwrap_or_default();

        // Find matching button
        let mut best_button: Option<(usize, f64)> = None; // (index, distance_to_last_field)

        for (i, btn) in buttons.iter().enumerate() {
            let btn_text = btn.get("text").and_then(|v| v.as_str()).unwrap_or("");

            let label_match = submit_labels
                .iter()
                .any(|sl| btn_text.to_lowercase().contains(&sl.to_lowercase()));

            if label_match || btn.get("type").and_then(|v| v.as_str()) == Some("submit") {
                // Spatial proximity tiebreaker
                let btn_x = btn.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
                let btn_y = btn.get("y").and_then(|v| v.as_i64()).unwrap_or(0);

                let distance = if let Some((lx, ly)) = last_fill_coords {
                    (((btn_x - lx).pow(2) + (btn_y - ly).pow(2)) as f64).sqrt()
                } else {
                    0.0 // no last field — no preference
                };

                if best_button.is_none() || distance < best_button.unwrap().1 {
                    best_button = Some((i, distance));
                }
            }
        }

        if let Some((idx, _)) = best_button {
            let btn = &buttons[idx];
            let btn_text = btn.get("text").and_then(|v| v.as_str()).unwrap_or("Submit");
            let btn_x = btn.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
            let btn_y = btn.get("y").and_then(|v| v.as_i64()).unwrap_or(0);

            // ── Consent risk check before submit ──
            // Build context from form fields for risk classification
            let form_context = json!({
                "fields": form_fields,
            });
            let consent_classification = consent::classify_consent(
                btn_text,
                &[btn_text],
                None, // URL not easily available here
                Some(&form_context),
            );
            if matches!(consent_classification.risk, consent::RiskLevel::HighRisk) {
                let session_auto_accept = {
                    let s = session.read().unwrap_or_else(|e| e.into_inner());
                    s.auto_accept_low_risk
                };
                if !auto_submit
                    || !consent::should_auto_accept(&consent_classification, session_auto_accept)
                {
                    submit_result = json!({
                        "submitted": false,
                        "reason": format!("Consent classified as {:?}: {}", consent_classification.risk, consent_classification.reasoning),
                        "button_text": btn_text,
                        "consent_risk": format!("{:?}", consent_classification.risk),
                    });

                    let elapsed = start.elapsed().as_millis() as u64;
                    let mut result = MetaToolResult::success(
                        "form_fill",
                        rungs_tried,
                        json!({
                            "filled": filled,
                            "failed": failed,
                            "field_results": field_results,
                            "total_fields": fields.len(),
                            "filled_count": filled.len(),
                            "failed_count": failed.len(),
                            "submitted": false,
                            "submit": submit_result,
                            "consent_blocked": true,
                        }),
                        elapsed,
                    );
                    result = result.with_reversibility(Reversibility::RequiresConfirmation);
                    for w in warnings {
                        result = result.with_warning(w);
                    }
                    return result.to_value();
                }
            }

            // Check reversibility
            let btn_rev = rev::classify_button_action(btn_text, auto_submit);
            if btn_rev == Reversibility::RequiresConfirmation && !auto_submit {
                submit_result = json!({
                    "submitted": false,
                    "reason": "Submit requires confirmation — pass auto_submit=true",
                    "button_text": btn_text,
                });
            } else if btn_rev == Reversibility::Destructive {
                submit_result = json!({
                    "submitted": false,
                    "reason": "Submit button classified as destructive — manual confirmation needed",
                    "button_text": btn_text,
                });
            } else {
                // Click submit
                let click_result = browser_mcp::tools::handle_tool(
                    browser,
                    "click",
                    json!({"x": btn_x, "y": btn_y}),
                )
                .await;
                let (ok, _) = super::browser_result_to_value(click_result);
                submit_method = format!("click:{}", btn_text);
                submit_confidence = 0.9;
                submit_result = json!({
                    "submitted": ok,
                    "button_text": btn_text,
                });
            }
        } else if auto_submit {
            // Fallback: press Enter on last field
            let _ =
                browser_mcp::tools::handle_tool(browser, "press", json!({"key": "Enter"})).await;
            submit_method = "enter_key_fallback".into();
            submit_confidence = 0.5;
            submit_result = json!({
                "submitted": true,
                "method": "enter_key_fallback",
            });
        }
    }

    let elapsed = start.elapsed().as_millis() as u64;
    let total_fields = fields.len();

    let mut result = MetaToolResult::success(
        "form_fill",
        rungs_tried,
        json!({
            "filled": filled,
            "failed": failed,
            "field_results": field_results,
            "total_fields": total_fields,
            "filled_count": filled.len(),
            "failed_count": failed.len(),
            "submitted": submit_result.get("submitted").and_then(|v| v.as_bool()).unwrap_or(false),
            "submit_method": if submit_method.is_empty() { Value::Null } else { json!(submit_method) },
            "submit_button_confidence": if submit_confidence > 0.0 { json!(submit_confidence) } else { Value::Null },
            "submit": submit_result,
        }),
        elapsed,
    );

    // Set confidence based on fill success rate
    let fill_rate = if total_fields > 0 {
        filled.len() as f32 / total_fields as f32
    } else {
        1.0
    };
    result = result.with_confidence(Confidence::method_only(fill_rate));

    // Reversibility depends on whether we submitted
    if submit_result
        .get("submitted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        result = result.with_reversibility(rev::classify_submit_action(auto_submit));
    } else {
        result = result.with_reversibility(Reversibility::Reversible);
    }

    for w in warnings {
        result = result.with_warning(w);
    }

    instrumentation::log_aggregate(
        "hands_fill_form",
        &call_id,
        true,
        "form_fill",
        result.rungs_tried.len(),
        elapsed,
        Some(fill_rate),
        None,
    );

    result.to_value()
}
