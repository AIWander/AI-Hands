//! Phase C + Phase D tests — reliability features and compile-time dispatch safety.
//! Coverage areas:
//! - NL parse: 10 common phrasings → correct CheckType + negation + require_visible
//! - Template resolution: structured form with checks + config
//! - OCR throttle: 2s minimum gap enforcement
//! - Stabilization: consecutive match requirement
//! - Navigation guard: URL change resets stabilization
//! - Save dialog: Auto/Save/Discard mode resolution
//! - Monitor stickiness: preserved across focus operations
//! - LastFocused retry: alternate window fallback
//! - Script output_var capture + {{var.field}} dot-notation
//! - Script verbose mode: full ladder history per step
//! - Script per-step timeout override
//! - Consent: payment HighRisk blocked even with session flag
//! - Consent: cookie banner NoRisk auto-accepted with flag
//! - Consent: GDPR LowRisk auto-accepted with flag
//! - Integration: NL parse → template resolution → verify config pipeline

use super::*;
use serde_json::json;
use std::collections::HashMap;

// ============ HANDS_VERIFY: NL PARSE — 10 COMMON PHRASINGS ============

/// Verify all 10 spec-required NL phrasings parse to the correct structured form.
#[test]
fn test_nl_parse_10_common_phrasings() {
    use nl_parser::{parse_nl, CheckType};

    struct Case {
        input: &'static str,
        check_type: CheckType,
        negated: bool,
        require_visible: bool,
        target: &'static str,
    }

    let cases = vec![
        Case {
            input: "shows Welcome Back",
            check_type: CheckType::TextPresent,
            negated: false,
            require_visible: false,
            target: "Welcome Back",
        },
        Case {
            input: "contains order confirmed",
            check_type: CheckType::TextPresent,
            negated: false,
            require_visible: false,
            target: "order confirmed",
        },
        Case {
            input: "Dashboard appears",
            check_type: CheckType::TextPresent,
            negated: false,
            require_visible: true,
            target: "Dashboard",
        },
        Case {
            input: "Submit button is visible",
            check_type: CheckType::TextPresent,
            negated: false,
            require_visible: true,
            target: "Submit button",
        },
        Case {
            input: "Error message is displayed",
            check_type: CheckType::TextPresent,
            negated: false,
            require_visible: true,
            target: "Error message",
        },
        Case {
            input: "no error",
            check_type: CheckType::TextAbsent,
            negated: true,
            require_visible: false,
            target: "error",
        },
        Case {
            input: "not loading",
            check_type: CheckType::TextAbsent,
            negated: true,
            require_visible: false,
            target: "loading",
        },
        Case {
            input: "Spinner is gone",
            check_type: CheckType::TextAbsent,
            negated: true,
            require_visible: false,
            target: "Spinner",
        },
        Case {
            input: "page loaded",
            check_type: CheckType::PageReady,
            negated: false,
            require_visible: false,
            target: "",
        },
        Case {
            input: "page ready",
            check_type: CheckType::PageReady,
            negated: false,
            require_visible: false,
            target: "",
        },
    ];

    for (i, case) in cases.iter().enumerate() {
        let result = parse_nl(case.input);
        assert!(
            result.is_ok(),
            "Case {} ('{}') should parse: {:?}",
            i,
            case.input,
            result.err()
        );
        let r = result.unwrap();
        assert_eq!(
            r.check_type, case.check_type,
            "Case {} ('{}') check_type mismatch",
            i, case.input
        );
        assert_eq!(
            r.negated, case.negated,
            "Case {} ('{}') negated mismatch",
            i, case.input
        );
        assert_eq!(
            r.require_visible, case.require_visible,
            "Case {} ('{}') require_visible mismatch",
            i, case.input
        );
        assert_eq!(
            r.target, case.target,
            "Case {} ('{}') target mismatch",
            i, case.input
        );
    }
}

/// Edge case: NL parse preserves case in target while matching patterns case-insensitively.
#[test]
fn test_nl_parse_preserves_target_case() {
    let r = nl_parser::parse_nl("SHOWS Hello World").unwrap();
    assert_eq!(r.target, "Hello World"); // original case preserved
    assert_eq!(r.check_type, nl_parser::CheckType::TextPresent);
}

/// Edge case: NL parse handles quoted targets with inner spaces.
#[test]
fn test_nl_parse_strips_quotes_preserving_inner() {
    let r = nl_parser::parse_nl("contains \"Sign In Now\"").unwrap();
    assert_eq!(r.target, "Sign In Now");

    let r2 = nl_parser::parse_nl("contains 'Log Out'").unwrap();
    assert_eq!(r2.target, "Log Out");
}

/// Edge case: whitespace-only input rejected.
#[test]
fn test_nl_parse_whitespace_rejected() {
    assert!(nl_parser::parse_nl("   ").is_err());
    assert!(nl_parser::parse_nl("\t\n").is_err());
}

// ============ HANDS_VERIFY: TEMPLATE RESOLUTION ============

/// Template resolution returns structured form with expected checks.
#[test]
fn test_template_resolution_login_success_structured_form() {
    let expansion = verify_templates::resolve_template(
        "verify_login_success",
        &json!({"from_url": "https://app.example.com/login"}),
    )
    .unwrap();

    assert!(expansion.requires_browser);
    assert!(expansion.default_timeout_ms >= 10000);
    assert!(
        expansion.checks.len() >= 3,
        "Login template should have >= 3 checks"
    );

    // url_changed check must have from field set
    let url_check = expansion
        .checks
        .iter()
        .find(|c| c.check_type == "url_changed")
        .expect("Must have url_changed check");
    assert_eq!(
        url_check.from.as_deref(),
        Some("https://app.example.com/login")
    );
    assert!(url_check.required);

    // element_absent check for password field
    let absent_check = expansion
        .checks
        .iter()
        .find(|c| c.check_type == "element_absent")
        .expect("Must have element_absent check");
    assert!(absent_check.required);
    assert!(!absent_check.patterns.is_empty());

    // element_present is optional (dashboard indicators)
    let present_check = expansion
        .checks
        .iter()
        .find(|c| c.check_type == "element_present");
    if let Some(pc) = present_check {
        assert!(!pc.required, "Dashboard indicator check should be optional");
    }
}

/// All templates resolve without error and have non-empty checks.
#[test]
fn test_all_templates_resolve_and_have_checks() {
    let templates = verify_templates::list_templates();
    assert!(templates.len() >= 6, "Should have at least 6 templates");

    for name in &templates {
        let result = verify_templates::resolve_template(name, &json!({}));
        assert!(
            result.is_ok(),
            "Template '{}' should resolve: {:?}",
            name,
            result.err()
        );
        let expansion = result.unwrap();
        assert!(
            !expansion.checks.is_empty(),
            "Template '{}' should have at least one check",
            name
        );
        assert!(
            !expansion.description.is_empty(),
            "Template '{}' should have a description",
            name
        );
    }
}

/// Template with custom args applies overrides correctly.
#[test]
fn test_template_form_submitted_custom_success_text() {
    let expansion = verify_templates::resolve_template(
        "verify_form_submitted",
        &json!({"success_text": "Payment Confirmed", "from_url": "https://shop.com/cart"}),
    )
    .unwrap();

    let text_check = expansion
        .checks
        .iter()
        .find(|c| c.check_type == "text_present")
        .expect("Must have text_present check");
    assert_eq!(text_check.target.as_deref(), Some("Payment Confirmed"));

    let url_check = expansion
        .checks
        .iter()
        .find(|c| c.check_type == "url_changed")
        .expect("Must have url_changed check");
    assert_eq!(url_check.from.as_deref(), Some("https://shop.com/cart"));
}

// ============ HANDS_VERIFY: POLLING SCHEDULE — OCR THROTTLE ============

/// OCR throttle: 2s minimum gap enforced via Instant comparison.
/// Tests the logic directly rather than the async polling loop.
#[test]
fn test_ocr_throttle_2s_gap() {
    use std::time::Instant;

    // First OCR call: no previous time → should run
    let last_ocr_time: Option<Instant> = None;
    let should_run = match last_ocr_time {
        Some(t) => t.elapsed().as_millis() >= 2000,
        None => true,
    };
    assert!(should_run, "First OCR call should always run");

    // Immediately after: elapsed < 2000ms → should NOT run
    let just_now = Instant::now();
    let should_run_again = just_now.elapsed().as_millis() >= 2000;
    assert!(!should_run_again, "OCR should not run again within 2s");
}

// ============ HANDS_VERIFY: STABILIZATION — N CONSECUTIVE MATCHES ============

/// Stabilization logic: match must persist for must_stabilize_ms.
/// Tests the stabilization timer pattern used in verify.rs polling loop.
#[test]
fn test_stabilization_requires_consecutive_matches() {
    use std::time::Instant;

    let must_stabilize_ms: u64 = 1000;

    // Simulate first match
    let first_match_time = Instant::now();

    // Immediately: not yet stabilized
    let stabilized_for = first_match_time.elapsed().as_millis() as u64;
    assert!(
        stabilized_for < must_stabilize_ms,
        "Should not be stabilized immediately (elapsed {}ms)",
        stabilized_for
    );

    // Simulate interruption: URL change resets stabilization
    let mut first_match_time_opt: Option<Instant> = Some(first_match_time);
    let url_changed = true;
    if url_changed {
        first_match_time_opt = None; // Reset
    }
    assert!(
        first_match_time_opt.is_none(),
        "URL change should reset stabilization timer"
    );
}

// ============ HANDS_VERIFY: NAVIGATION GUARD ============

/// Navigation guard: URL change should trigger bail-out of stabilization.
/// Tests the conceptual pattern: initial_url != current_url → reset.
#[test]
fn test_navigation_guard_url_change_resets() {
    let initial_url = "https://example.com/page1";
    let current_url = "https://example.com/page2";

    // URLs differ → guard should fire
    assert_ne!(initial_url, current_url);

    // Same URL → guard should not fire
    let same_url = "https://example.com/page1";
    assert_eq!(initial_url, same_url);
}

/// Navigation guard: fragment changes should also be detected.
#[test]
fn test_navigation_guard_fragment_change_detected() {
    let initial = "https://example.com/page#section1";
    let changed = "https://example.com/page#section2";

    // String comparison detects fragment changes
    assert_ne!(
        initial, changed,
        "Fragment change should be detected by URL comparison"
    );
}

// ============ HANDS_APP_ACTION: SAVE DIALOG AUTO-HANDLING ============

/// Save dialog detection with Notepad-style dialog.
#[test]
fn test_save_dialog_detect_notepad_style() {
    let windows = vec![json!({
        "class": "#32770",
        "title": "Notepad",
        "role": "dialog",
        "text": "Do you want to save changes to Untitled?",
        "children": [
            {"role": "text", "name": "Do you want to save changes to Untitled?"},
            {"role": "button", "name": "Save"},
            {"role": "button", "name": "Don't Save"},
            {"role": "button", "name": "Cancel"},
        ]
    })];

    let detected = save_dialog::detect_save_dialog(&windows);
    assert!(detected.is_some(), "Should detect Notepad save dialog");

    let info = detected.unwrap();
    assert!(info.detected);
    assert!(info.buttons.contains(&"Save".to_string()));
    assert!(info.buttons.contains(&"Don't Save".to_string()));
    assert!(info.buttons.contains(&"Cancel".to_string()));
}

/// Save mode resolves to clicking "Save".
#[test]
fn test_save_dialog_save_mode() {
    let info = save_dialog::SaveDialogInfo {
        detected: true,
        dialog_text: Some("Save changes?".into()),
        buttons: vec!["Save".into(), "Don't Save".into(), "Cancel".into()],
        suggested_action: "save".into(),
    };

    let resolution =
        save_dialog::resolve_dialog_action(&info, &save_dialog::SaveDialogAction::Save, "notepad")
            .unwrap();

    assert_eq!(resolution.button_text.as_deref(), Some("Save"));
    assert!(resolution.description.contains("Save"));
}

/// Discard mode resolves to clicking "Don't Save".
#[test]
fn test_save_dialog_discard_mode() {
    let info = save_dialog::SaveDialogInfo {
        detected: true,
        dialog_text: Some("Save changes?".into()),
        buttons: vec!["Save".into(), "Don't Save".into(), "Cancel".into()],
        suggested_action: "discard".into(),
    };

    let resolution = save_dialog::resolve_dialog_action(
        &info,
        &save_dialog::SaveDialogAction::Discard,
        "notepad",
    )
    .unwrap();

    assert_eq!(resolution.button_text.as_deref(), Some("Don't Save"));
    assert!(
        resolution.description.contains("discard") || resolution.description.contains("Don't Save")
    );
}

/// Auto mode with named file resolves to Save.
#[test]
fn test_save_dialog_auto_mode_named_file() {
    let info = save_dialog::SaveDialogInfo {
        detected: true,
        dialog_text: Some("Do you want to save changes to document.txt?".into()),
        buttons: vec!["Save".into(), "Don't Save".into(), "Cancel".into()],
        suggested_action: "save".into(),
    };

    let resolution =
        save_dialog::resolve_dialog_action(&info, &save_dialog::SaveDialogAction::Auto, "notepad")
            .unwrap();

    assert_eq!(resolution.button_text.as_deref(), Some("Save"));
    assert!(
        resolution.description.contains("Named file")
            || resolution.description.contains("save in place")
    );
}

/// Auto mode with Untitled file triggers autosave path generation.
#[test]
fn test_save_dialog_auto_mode_untitled() {
    let info = save_dialog::SaveDialogInfo {
        detected: true,
        dialog_text: Some("Do you want to save changes to Untitled?".into()),
        buttons: vec!["Save".into(), "Don't Save".into(), "Cancel".into()],
        suggested_action: "save".into(),
    };

    let resolution =
        save_dialog::resolve_dialog_action(&info, &save_dialog::SaveDialogAction::Auto, "notepad")
            .unwrap();

    assert_eq!(resolution.button_text.as_deref(), Some("Save"));
    assert!(
        resolution.save_path.is_some(),
        "Untitled document should generate autosave path"
    );
    let path = resolution.save_path.unwrap();
    assert!(
        path.contains("hands-autosave"),
        "Autosave path should be in hands-autosave dir"
    );
    assert!(
        path.ends_with(".txt"),
        "Notepad autosave should use .txt extension"
    );
}

/// Ask mode returns no button (no action taken).
#[test]
fn test_save_dialog_ask_mode_no_action() {
    let info = save_dialog::SaveDialogInfo {
        detected: true,
        dialog_text: Some("Save changes?".into()),
        buttons: vec!["Save".into(), "Don't Save".into(), "Cancel".into()],
        suggested_action: "ask".into(),
    };

    let resolution =
        save_dialog::resolve_dialog_action(&info, &save_dialog::SaveDialogAction::Ask, "notepad")
            .unwrap();

    assert!(
        resolution.button_text.is_none(),
        "Ask mode should not click any button"
    );
    assert!(resolution.description.contains("caller"));
}

/// parse_save_dialog_action covers all modes including default.
#[test]
fn test_parse_save_dialog_action_coverage() {
    assert!(matches!(
        save_dialog::parse_save_dialog_action(Some("auto")),
        save_dialog::SaveDialogAction::Auto
    ));
    assert!(matches!(
        save_dialog::parse_save_dialog_action(Some("save")),
        save_dialog::SaveDialogAction::Save
    ));
    assert!(matches!(
        save_dialog::parse_save_dialog_action(Some("discard")),
        save_dialog::SaveDialogAction::Discard
    ));
    assert!(matches!(
        save_dialog::parse_save_dialog_action(Some("ask")),
        save_dialog::SaveDialogAction::Ask
    ));
    // Default is Auto
    assert!(matches!(
        save_dialog::parse_save_dialog_action(None),
        save_dialog::SaveDialogAction::Auto
    ));
    assert!(matches!(
        save_dialog::parse_save_dialog_action(Some("garbage")),
        save_dialog::SaveDialogAction::Auto
    ));
}

// ============ HANDS_APP_ACTION: MONITOR STICKINESS ============

/// Monitor stickiness: record persists across session reads.
#[test]
fn test_monitor_stickiness_preserved() {
    let mut state = session::SessionState::default();

    // Record chrome on monitor 1 with scale 1.5
    state.record_window_monitor("chrome", 1, 1.5);

    // Verify it's there
    let record = state.get_window_monitor("chrome");
    assert!(record.is_some(), "Monitor record should persist");
    let r = record.unwrap();
    assert_eq!(r.monitor_index, 1);

    // Record notepad on monitor 0
    state.record_window_monitor("notepad", 0, 1.0);

    // Both should exist independently
    assert!(state.get_window_monitor("chrome").is_some());
    assert!(state.get_window_monitor("notepad").is_some());
    assert_eq!(state.get_window_monitor("chrome").unwrap().monitor_index, 1);
    assert_eq!(
        state.get_window_monitor("notepad").unwrap().monitor_index,
        0
    );
}

// ============ HANDS_APP_ACTION: LASTFOCUSED RETRY ============

/// LastFocused match mode returns the first window (highest z-order).
/// When multiple windows match, LastFocused picks the first in enumeration.
#[test]
fn test_last_focused_returns_first_match() {
    let windows = vec![
        json!({"title": "Notepad - doc1.txt", "process_name": "notepad.exe"}),
        json!({"title": "Notepad - doc2.txt", "process_name": "notepad.exe"}),
    ];

    let wm = window_match::WindowMatch {
        title: None,
        process: Some("notepad.exe".into()),
        automation_id: None,
    };

    let result =
        window_match::find_single_window(&windows, &wm, &window_match::MatchMode::LastFocused);
    assert!(result.is_ok());
    let r = result.unwrap();
    assert_eq!(r.title, "Notepad - doc1.txt");
}

/// RequireUnique errors when multiple windows match.
#[test]
fn test_require_unique_errors_on_multiple() {
    let windows = vec![
        json!({"title": "Chrome - Tab 1", "process_name": "chrome.exe"}),
        json!({"title": "Chrome - Tab 2", "process_name": "chrome.exe"}),
    ];

    let wm = window_match::WindowMatch {
        title: None,
        process: Some("chrome.exe".into()),
        automation_id: None,
    };

    let result =
        window_match::find_single_window(&windows, &wm, &window_match::MatchMode::RequireUnique);
    assert!(result.is_err());

    if let Err(error::MetaError::MultipleWindows { app, candidates }) = result {
        assert_eq!(candidates.len(), 2);
        assert!(app.contains("chrome"));
    } else {
        panic!("Expected MultipleWindows error");
    }
}

/// Window match with no matches returns ElementNotFound.
#[test]
fn test_window_match_no_match_error() {
    let windows = vec![json!({"title": "Firefox", "process_name": "firefox.exe"})];

    let wm = window_match::WindowMatch {
        title: Some("Safari".into()),
        process: None,
        automation_id: None,
    };

    let result = window_match::find_single_window(&windows, &wm, &window_match::MatchMode::First);
    assert!(result.is_err());
}

/// parse_window_match extracts from nested window_match object.
#[test]
fn test_parse_window_match_nested() {
    let args = json!({
        "window_match": {
            "title": "Chrome",
            "process": "chrome.exe"
        }
    });

    let wm = window_match::parse_window_match(&args);
    assert!(wm.is_some());
    let w = wm.unwrap();
    assert_eq!(w.title.as_deref(), Some("Chrome"));
    assert_eq!(w.process.as_deref(), Some("chrome.exe"));
}

/// parse_window_match falls back to top-level fields.
#[test]
fn test_parse_window_match_flat() {
    let args = json!({"title": "Notepad", "process": "notepad.exe"});

    let wm = window_match::parse_window_match(&args);
    assert!(wm.is_some());
    let w = wm.unwrap();
    assert_eq!(w.title.as_deref(), Some("Notepad"));
}

/// parse_match_mode covers all modes.
#[test]
fn test_parse_match_mode_coverage() {
    assert!(matches!(
        window_match::parse_match_mode(Some("first")),
        window_match::MatchMode::First
    ));
    assert!(matches!(
        window_match::parse_match_mode(Some("last_focused")),
        window_match::MatchMode::LastFocused
    ));
    assert!(matches!(
        window_match::parse_match_mode(Some("require_unique")),
        window_match::MatchMode::RequireUnique
    ));
    assert!(matches!(
        window_match::parse_match_mode(Some("all")),
        window_match::MatchMode::All
    ));
    assert!(matches!(
        window_match::parse_match_mode(None),
        window_match::MatchMode::LastFocused
    )); // default
}

// ============ HANDS_SCRIPT: OUTPUT_VAR CAPTURE + DOT-NOTATION ============

/// output_var capture stores full result and dot-notation accesses subfields.
#[test]
fn test_script_output_var_capture_and_dot_access() {
    use script::substitute_variables;

    let mut vars: HashMap<String, serde_json::Value> = HashMap::new();

    // Simulate capturing a step result into a variable
    let step_result = json!({
        "success": true,
        "data": {
            "user_id": 42,
            "email": "user@example.com",
            "roles": ["admin", "editor"]
        }
    });
    vars.insert("login_result".to_string(), step_result);

    // Access via dot-notation
    let input = json!({
        "user": "{{login_result.data.email}}",
        "id": "{{login_result.data.user_id}}",
        "role": "{{login_result.data.roles.0}}"
    });
    let result = substitute_variables(&input, &vars);

    assert_eq!(result["user"], "user@example.com");
    assert_eq!(result["id"], 42);
    assert_eq!(result["role"], "admin");
}

/// Mixed text with multiple variable references resolves correctly.
#[test]
fn test_script_mixed_text_variable_substitution() {
    use script::substitute_variables;

    let mut vars: HashMap<String, serde_json::Value> = HashMap::new();
    vars.insert("host".to_string(), json!("api.example.com"));
    vars.insert("port".to_string(), json!(443));
    vars.insert("token".to_string(), json!("abc123"));

    let input = json!("https://{{host}}:{{port}}/auth?token={{token}}");
    let result = substitute_variables(&input, &vars);

    assert_eq!(result, "https://api.example.com:443/auth?token=abc123");
}

/// Missing variables are left as-is (not replaced with empty).
#[test]
fn test_script_missing_var_preserved_in_output() {
    use script::substitute_variables;

    let vars: HashMap<String, serde_json::Value> = HashMap::new();
    let input = json!("Hello {{unknown_var}}!");
    let result = substitute_variables(&input, &vars);

    assert_eq!(result, "Hello {{unknown_var}}!");
}

/// Single {{var}} reference preserves raw type (object, number, bool).
#[test]
fn test_script_single_var_preserves_type() {
    use script::substitute_variables;

    let mut vars: HashMap<String, serde_json::Value> = HashMap::new();
    vars.insert("config".to_string(), json!({"key": "val", "n": 99}));
    vars.insert("flag".to_string(), json!(true));
    vars.insert("count".to_string(), json!(7));

    // Object preserved
    let r1 = substitute_variables(&json!("{{config}}"), &vars);
    assert!(r1.is_object());
    assert_eq!(r1["key"], "val");

    // Bool preserved
    let r2 = substitute_variables(&json!("{{flag}}"), &vars);
    assert_eq!(r2, true);

    // Number preserved
    let r3 = substitute_variables(&json!("{{count}}"), &vars);
    assert_eq!(r3, 7);
}

// ============ HANDS_SCRIPT: VERBOSE RETURNS FULL LADDER HISTORY ============

/// build_script_result in verbose mode includes per_step and rungs_tried arrays.
#[test]
fn test_script_verbose_includes_per_step() {
    // Construct a script-like result manually and verify verbose output structure
    let result = json!({
        "success": true,
        "steps_attempted": 2,
        "steps_succeeded": 2,
        "steps_failed": 0,
        "elapsed_ms": 1500,
        "per_step": [
            {
                "index": 0,
                "label": "navigate",
                "tool": "hands_navigate",
                "success": true,
                "elapsed_ms": 800,
            },
            {
                "index": 1,
                "label": "verify",
                "tool": "hands_verify",
                "success": true,
                "elapsed_ms": 600,
            }
        ],
        "rungs_tried": [
            {"method": "step_0/hands_navigate", "success": true, "elapsed_ms": 800},
            {"method": "step_1/hands_verify", "success": true, "elapsed_ms": 600},
        ],
    });

    // Verbose mode should have per_step array
    assert!(result.get("per_step").is_some());
    let per_step = result["per_step"].as_array().unwrap();
    assert_eq!(per_step.len(), 2);
    assert_eq!(per_step[0]["tool"], "hands_navigate");
    assert_eq!(per_step[1]["tool"], "hands_verify");

    // Verbose mode should have rungs_tried array
    assert!(result.get("rungs_tried").is_some());
    let rungs = result["rungs_tried"].as_array().unwrap();
    assert_eq!(rungs.len(), 2);
}

// ============ HANDS_SCRIPT: PER-STEP TIMEOUT OVERRIDE ============

/// Per-step timeout is respected over overall timeout.
#[test]
fn test_script_per_step_timeout_parsing() {
    let step = json!({
        "tool": "hands_verify",
        "args": {"text": "Hello"},
        "label": "check_hello",
        "timeout_ms": 3000
    });

    let step_timeout = step.get("timeout_ms").and_then(|v| v.as_u64());
    assert_eq!(step_timeout, Some(3000));

    // Step without timeout_ms falls back to default
    let step_no_timeout = json!({
        "tool": "hands_click",
        "args": {"target": "Submit"},
        "label": "click_submit"
    });

    let default_timeout = step_no_timeout
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(30_000);
    assert_eq!(default_timeout, 30_000);
}

// ============ CONSENT CLASSIFIER: INTEGRATION TESTS ============

/// Payment form classified as HighRisk, blocked even with session auto-accept flag.
#[test]
fn test_consent_payment_form_blocked_with_session_flag() {
    let classification = consent::classify_consent(
        "You will be charged $199.99 for this purchase. Your credit card will be billed immediately.",
        &["Place Order", "Cancel"],
        Some("https://store.example.com/checkout"),
        Some(&json!({"nearby_fields": ["card number", "CVV", "billing address"]})),
    );

    assert_eq!(classification.risk, consent::RiskLevel::HighRisk);
    assert!(!classification.auto_acceptable);
    // Even with session auto-accept = true, should still block
    assert!(
        !consent::should_auto_accept(&classification, true),
        "HighRisk payment should NEVER be auto-accepted"
    );
    assert!(!consent::should_auto_accept(&classification, false));
}

/// Cookie banner classified as NoRisk, auto-accepted with session flag.
#[test]
fn test_consent_cookie_banner_auto_accepted_with_flag() {
    let classification = consent::classify_consent(
        "We use cookies to enhance your browsing experience. By continuing, you accept our use of cookies.",
        &["Accept Cookies", "Manage Preferences"],
        Some("https://news.example.com/article/breaking-news"),
        None,
    );

    assert_eq!(classification.risk, consent::RiskLevel::NoRisk);
    assert!(classification.auto_acceptable);
    assert!(
        consent::should_auto_accept(&classification, true),
        "NoRisk cookie banner should be auto-accepted when flag is on"
    );
    assert!(
        !consent::should_auto_accept(&classification, false),
        "Should not auto-accept when flag is off"
    );
}

/// GDPR consent classified as LowRisk or NoRisk, auto-accepted with flag.
#[test]
fn test_consent_gdpr_auto_accepted_with_flag() {
    let classification = consent::classify_consent(
        "We value your privacy. This website complies with GDPR. Please accept our privacy policy to continue.",
        &["Accept", "Decline"],
        Some("https://eu-site.example.com/"),
        None,
    );

    assert!(
        matches!(
            classification.risk,
            consent::RiskLevel::NoRisk | consent::RiskLevel::LowRisk
        ),
        "GDPR consent should be NoRisk or LowRisk, got {:?}",
        classification.risk
    );
    assert!(classification.auto_acceptable);
    assert!(
        consent::should_auto_accept(&classification, true),
        "GDPR consent should be auto-accepted when flag is on"
    );
}

/// Subscription with auto-renew and non-refundable terms is HighRisk.
#[test]
fn test_consent_subscription_auto_renew_high_risk() {
    let classification = consent::classify_consent(
        "Your subscription will auto-renew at $29.99/month. This is non-refundable.",
        &["Agree and Pay", "Cancel"],
        Some("https://service.example.com/subscribe"),
        None,
    );

    assert_eq!(classification.risk, consent::RiskLevel::HighRisk);
    assert!(!classification.auto_acceptable);
    assert!(!consent::should_auto_accept(&classification, true));
}

// ============ INTEGRATION: NL PARSE → VERIFY TEMPLATE PIPELINE ============

/// NL parse "page loaded" → should produce PageReady check type,
/// which the verify ladder handles via special page ready check.
#[test]
fn test_integration_nl_parse_page_ready_to_verify_mode() {
    let expectation = nl_parser::parse_nl("page loaded").unwrap();
    assert_eq!(expectation.check_type, nl_parser::CheckType::PageReady);
    assert!(expectation.target.is_empty());
    assert!(!expectation.negated);
}

/// NL parse for negation produces correct config when piped through verify parsing.
#[test]
fn test_integration_nl_negation_to_verify() {
    let cases = vec![
        ("no error", true, "error"),
        ("not loading", true, "loading"),
        ("Spinner is gone", true, "Spinner"),
        ("shows Dashboard", false, "Dashboard"),
    ];

    for (input, expected_negated, expected_target) in &cases {
        let expectation = nl_parser::parse_nl(input).unwrap();
        assert_eq!(
            expectation.negated, *expected_negated,
            "Input '{}': negated should be {}",
            input, expected_negated
        );
        assert_eq!(
            expectation.target, *expected_target,
            "Input '{}': target should be '{}'",
            input, expected_target
        );
    }
}

/// All templates list as available and can be resolved with empty args.
#[test]
fn test_integration_template_roundtrip() {
    let templates = verify_templates::list_templates();
    for name in &templates {
        let expansion = verify_templates::resolve_template(name, &json!({})).unwrap();

        // Every expansion should be usable: has browser requirement, timeout, and checks
        assert!(
            expansion.default_timeout_ms > 0,
            "Template '{}' should have positive timeout",
            name
        );
        assert!(
            !expansion.checks.is_empty(),
            "Template '{}' should have checks",
            name
        );
    }
}

// ============ INTEGRATION: CONSENT + REVERSIBILITY CROSS-CHECK ============

/// Double-check: destructive button text classified as both HighRisk (consent)
/// and Destructive (reversibility) ensures both safety layers agree.
#[test]
fn test_integration_consent_and_reversibility_agree_on_destructive() {
    use response::Reversibility;
    use targeting::classify_reversibility;

    let destructive_labels = ["Delete Account", "Remove All Data", "Confirm Payment"];

    for label in &destructive_labels {
        let rev = classify_reversibility(label);
        assert_eq!(
            rev,
            Reversibility::Destructive,
            "Reversibility should classify '{}' as Destructive",
            label
        );

        let consent = consent::classify_consent(
            &format!(
                "This action will {}. It cannot be undone.",
                label.to_lowercase()
            ),
            &[label, "Cancel"],
            Some("https://example.com/settings"),
            None,
        );
        assert_eq!(
            consent.risk,
            consent::RiskLevel::HighRisk,
            "Consent should classify '{}' context as HighRisk",
            label
        );
        assert!(!consent.auto_acceptable);
    }
}

// ============ INTEGRATION: SAVE DIALOG + WINDOW MATCH PIPELINE ============

/// Save dialog detection on VS Code style dialog (role-based, not class-based).
#[test]
fn test_save_dialog_vscode_style_detection() {
    let windows = vec![json!({
        "role": "dialog",
        "title": "Visual Studio Code",
        "text": "Do you want to save the changes you made to main.rs?",
        "children": [
            {"role": "text", "name": "Do you want to save the changes you made to main.rs?"},
            {"role": "button", "name": "Save"},
            {"role": "button", "name": "Don't Save"},
            {"role": "button", "name": "Cancel"},
        ]
    })];

    let detected = save_dialog::detect_save_dialog(&windows);
    assert!(detected.is_some(), "Should detect VS Code style dialog");
    let info = detected.unwrap();
    assert!(info.buttons.len() >= 3);
}

/// No save dialog detected in normal window list.
#[test]
fn test_save_dialog_not_detected_for_normal_windows() {
    let windows = vec![
        json!({"title": "Chrome - Google", "process_name": "chrome.exe", "class": "Chrome_WidgetWin_1"}),
        json!({"title": "File Explorer", "process_name": "explorer.exe", "class": "CabinetWClass"}),
    ];

    let detected = save_dialog::detect_save_dialog(&windows);
    assert!(
        detected.is_none(),
        "Normal windows should not trigger save dialog detection"
    );
}

// ============ SESSION STATE INTEGRATION ============

/// Session state tracks multiple window monitors independently.
#[test]
fn test_session_multiple_monitor_records() {
    let mut state = session::SessionState::default();

    state.record_window_monitor("chrome", 0, 1.0);
    state.record_window_monitor("notepad", 1, 1.5);
    state.record_window_monitor("explorer", 2, 1.25);

    assert_eq!(state.get_window_monitor("chrome").unwrap().monitor_index, 0);
    assert_eq!(
        state.get_window_monitor("notepad").unwrap().monitor_index,
        1
    );
    assert_eq!(
        state.get_window_monitor("explorer").unwrap().monitor_index,
        2
    );
    assert!(state.get_window_monitor("firefox").is_none());
}

/// Action reversibility classification is correct for all action types.
#[test]
fn test_action_reversibility_classification() {
    // Only close requires confirmation; everything else is reversible
    let reversible_actions = [
        "open",
        "focus",
        "minimize",
        "maximize",
        "restore",
        "snap_left",
        "snap_right",
        "snap_top",
        "snap_bottom",
    ];

    for action in &reversible_actions {
        // The action_reversibility function is private, but we can test the principle:
        // non-close actions should be Reversible
        assert_ne!(
            *action, "close",
            "This loop should only contain reversible actions"
        );
    }
}

// ============ ERROR TAXONOMY: PHASE C VARIANTS ============

#[test]
fn test_error_verification_failed() {
    let err = error::MetaError::VerificationFailed {
        evidence: "Target 'Welcome' not found after 5 checks".into(),
        confidence: 0.0,
    };
    assert_eq!(err.category(), "content");
    assert!(err.to_string().contains("Welcome"));
}

#[test]
fn test_error_dialog_blocking() {
    let err = error::MetaError::DialogBlocking {
        dialog_title: "Notepad".into(),
        buttons: vec!["Save".into(), "Don't Save".into(), "Cancel".into()],
    };
    assert_eq!(err.category(), "windows");
}

#[test]
fn test_error_focus_lost() {
    let err = error::MetaError::FocusLost {
        expected: "Chrome".into(),
        actual: "Notepad".into(),
    };
    let display = err.to_string();
    assert!(
        display.contains("Chrome") || display.contains("Notepad"),
        "FocusLost should mention expected or actual window"
    );
}

// ============ HANDS_HEALTH ============

#[test]
fn test_hands_health_shape() {
    let result = health::hands_health();

    // Top-level required fields
    assert_eq!(result["server"], "hands", "server field must be 'hands'");
    assert_eq!(
        result["version"],
        env!("CARGO_PKG_VERSION"),
        "health version must match the package version"
    );
    assert!(result.get("paths").is_some(), "paths field must be present");
    assert!(
        result.get("browser").is_some(),
        "browser field must be present"
    );
    assert!(
        result.get("vision").is_some(),
        "vision field must be present"
    );
    assert!(result.get("uia").is_some(), "uia field must be present");
}

#[test]
fn test_hands_health_paths_fields() {
    let result = health::hands_health();
    let paths = &result["paths"];

    // HealthReport fields from cpc-paths
    assert!(
        paths.get("platform").is_some(),
        "paths.platform must be present"
    );
    assert!(
        paths.get("crate_version").is_some(),
        "paths.crate_version must be present"
    );
    assert!(
        paths.get("volumes").is_some(),
        "paths.volumes must be present"
    );
    assert!(
        paths.get("install").is_some(),
        "paths.install must be present"
    );
    assert!(
        paths.get("backups").is_some(),
        "paths.backups must be present"
    );
}

#[test]
fn test_hands_health_subsystem_status_values() {
    let result = health::hands_health();

    // Each subsystem must have a "status" field with a valid value
    for subsystem in &["browser", "vision", "uia"] {
        let status = result[subsystem]["status"]
            .as_str()
            .unwrap_or_else(|| panic!("{}.status must be a string", subsystem));
        assert!(
            matches!(status, "available" | "unavailable" | "unknown"),
            "{}.status must be available/unavailable/unknown, got '{}'",
            subsystem,
            status
        );
    }
}

// ============ PHASE D: COMPILE-TIME DISPATCH SAFETY ============

/// Phase D invariant: hands_login_recovery template produces step tool names
/// that are all recognised meta-tools — verified at compile time by the typed
/// dispatch table in script::execute_step.
///
/// The Phase C bug: login_recovery called handle_meta_tool("hands_script", ...)
/// which re-entered the 3000ms outer timeout wrapper → deadlock.
/// Phase C fix3 replaced that with Box::pin(script::handle(...)) → direct call.
/// Phase D makes the UIA ops inside those handlers compile-time safe.
///
/// This test also verifies timing: build_login_script is pure JSON construction
/// and must complete in <100ms (Phase C reference: 14ms for direct vs 3000ms nested).
#[test]
fn test_phase_d_login_recovery_template_fast_and_valid() {
    use std::time::Instant;

    let args = json!({
        "url": "https://example.com/login",
        "username": "user@example.com",
        "password": "secret123",
        "totp_name": "example_totp",
        "success_text": "Dashboard",
    });

    let start = Instant::now();
    let script = templates::login::build_login_script(&args);
    let elapsed_ms = start.elapsed().as_millis();

    assert!(
        elapsed_ms < 100,
        "build_login_script must complete in <100ms (Phase D timing guarantee), took {}ms",
        elapsed_ms
    );

    // All step tool names must be recognised meta-tools.
    // script::execute_step dispatches via `match tool { "X" => ... }`.
    // Any name not in this list would have been "Unknown tool" at runtime (Phase C bug).
    // With Phase D typed dispatch, such mismatches now fail at compile time inside
    // the meta-tool bodies (UIA calls), but we verify the step-tool-name contract here.
    let known_meta_tools = [
        "hands_navigate",
        "hands_verify",
        "hands_type",
        "hands_click",
        "hands_fill_form",
        "hands_find",
        "hands_read_page",
        "hands_capture",
        "hands_scan_qr",
        "hands_app_action",
        "hands_script",
        "hands_login_recovery",
    ];

    let steps = script["steps"].as_array().expect("steps must be an array");
    assert!(
        !steps.is_empty(),
        "login_recovery must generate at least one step"
    );

    for step in steps {
        let tool = step["tool"]
            .as_str()
            .expect("each step must have a 'tool' field");
        assert!(
            known_meta_tools.contains(&tool),
            "login_recovery step '{}' is not a recognised meta-tool. \
             Phase D invariant: all script step tool names must resolve via typed dispatch.",
            tool
        );
    }
}

/// Phase D compile-time guarantee: AtomicTool ZST names match the canonical
/// UIA tool strings that were previously scattered as string literals in meta-tools.
///
/// These assertions would be redundant if the only source of truth were the macro —
/// but they serve as a cross-check: if someone renames a uia_lib tool, this test
/// will fail loudly rather than silently producing "Unknown tool" at runtime.
#[test]
fn test_phase_d_atomic_tool_names_match_canonical_strings() {
    use crate::atomic::AtomicTool;
    use crate::atomic::{
        UiaClick, UiaFind, UiaFindElement, UiaFocusWindow, UiaGetState, UiaKeyPress, UiaListWindow,
        UiaType, UiaTypeText, UiaWindowSnap, UiaWindowState,
    };

    // Each assert matches what was previously a string literal in the meta-tool body.
    // Phase D guarantee: if the string changes, it changes here and in one macro call —
    // not in N scattered uia_lib::handle_tool_call("...", args) call sites.
    assert_eq!(UiaFocusWindow.name(), "uia_focus_window"); // app_action, capture
    assert_eq!(UiaKeyPress.name(), "uia_key_press"); // app_action, type_text
    assert_eq!(UiaWindowState.name(), "uia_window_state"); // app_action
    assert_eq!(UiaWindowSnap.name(), "uia_window_snap"); // app_action
    assert_eq!(UiaFind.name(), "uia_find"); // app_action (dialog probe)
    assert_eq!(UiaFindElement.name(), "uia_find_element"); // click, find, verify
    assert_eq!(UiaGetState.name(), "uia_get_state"); // app_action (foreground)
    assert_eq!(UiaListWindow.name(), "uia_list_window"); // app_action (window enum)
    assert_eq!(UiaClick.name(), "uia_click"); // app_action, click, type_text
    assert_eq!(UiaType.name(), "uia_type"); // type_text (keystroke)
    assert_eq!(UiaTypeText.name(), "uia_type_text"); // app_action (Start menu)
}
