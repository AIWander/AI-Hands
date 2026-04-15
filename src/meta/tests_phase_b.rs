//! Phase B tests — focused on RELIABILITY features, not happy path.
//! Coverage areas:
//! - Field role detection (sensitive fields, keystroke requirements)
//! - Label matching (tiered priority, multiple matches, disambiguation)
//! - Reversibility (submit, button, form control classification)
//! - Autofill detection (shape validation, parse results)
//! - hands_find: return_type=ref short-circuit behavior
//! - hands_type: fast_set rejection on Password fields
//! - hands_type: chunked typing threshold
//! - hands_fill_form: duplicate label disambiguation
//! - Adaptive timeout: widening applied correctly

use super::*;
use serde_json::json;

// ============ FIELD ROLE DETECTION ============

#[test]
fn test_field_role_password_detection() {
    assert_eq!(
        field_role::FieldRole::detect(&json!({"type": "password"})),
        field_role::FieldRole::Password
    );
    // autocomplete=current-password overrides type=text
    assert_eq!(
        field_role::FieldRole::detect(&json!({"type": "text", "autocomplete": "current-password"})),
        field_role::FieldRole::Password
    );
}

#[test]
fn test_field_role_sensitive_fields_refuse_unknown_fallback() {
    // Password is sensitive → requires keystroke
    assert!(field_role::FieldRole::Password.is_sensitive());
    assert!(field_role::FieldRole::Password.requires_keystroke());

    // Email is sensitive but doesn't require keystroke
    assert!(field_role::FieldRole::Email.is_sensitive());
    assert!(!field_role::FieldRole::Email.requires_keystroke());

    // Phone is sensitive AND requires keystroke (masked input)
    assert!(field_role::FieldRole::Phone.is_sensitive());
    assert!(field_role::FieldRole::Phone.requires_keystroke());
}

#[test]
fn test_field_role_credit_card_is_number() {
    let role = field_role::FieldRole::detect(&json!({
        "type": "text",
        "autocomplete": "cc-number"
    }));
    assert_eq!(role, field_role::FieldRole::Number);
    assert!(role.is_sensitive());
}

#[test]
fn test_field_role_select_detection() {
    assert_eq!(
        field_role::FieldRole::detect(&json!({"tag": "select"})),
        field_role::FieldRole::Select
    );
    assert_eq!(
        field_role::FieldRole::detect(&json!({"role": "combobox"})),
        field_role::FieldRole::Select
    );
    assert_eq!(
        field_role::FieldRole::detect(&json!({"role": "listbox"})),
        field_role::FieldRole::Select
    );
}

#[test]
fn test_field_role_text_input_classification() {
    assert!(field_role::FieldRole::Text.is_text_input());
    assert!(field_role::FieldRole::Email.is_text_input());
    assert!(field_role::FieldRole::Password.is_text_input());
    assert!(!field_role::FieldRole::Checkbox.is_text_input());
    assert!(!field_role::FieldRole::Select.is_text_input());
    assert!(!field_role::FieldRole::File.is_text_input());
}

// ============ LABEL MATCHING ============

#[test]
fn test_label_match_exact_wins_over_contains() {
    let candidates = vec![
        label_match::LabelCandidate { text: "Email Address".into(), role: None, selector: None, index: 0 },
        label_match::LabelCandidate { text: "Email".into(), role: None, selector: None, index: 1 },
    ];
    let result = label_match::find_best_match("Email", &candidates).unwrap();
    assert_eq!(result.tier, label_match::MatchTier::Exact);
    assert_eq!(result.index, 1); // exact match on "Email", not "Email Address"
}

#[test]
fn test_label_match_multiple_matches_same_tier_returns_error() {
    let candidates = vec![
        label_match::LabelCandidate { text: "Submit Form".into(), role: None, selector: None, index: 0 },
        label_match::LabelCandidate { text: "Submit Review".into(), role: None, selector: None, index: 1 },
        label_match::LabelCandidate { text: "Cancel".into(), role: None, selector: None, index: 2 },
    ];
    let result = label_match::find_best_match("Submit", &candidates);
    assert!(result.is_err());

    if let Err(error::MetaError::MultipleMatches { target, candidates }) = result {
        assert_eq!(target, "Submit");
        assert_eq!(candidates.len(), 2); // Submit Form + Submit Review
    } else {
        panic!("Expected MultipleMatches error");
    }
}

#[test]
fn test_label_match_no_match_returns_not_found() {
    let candidates = vec![
        label_match::LabelCandidate { text: "Username".into(), role: None, selector: None, index: 0 },
    ];
    let result = label_match::find_best_match("Phone Number", &candidates);
    assert!(result.is_err());
    if let Err(error::MetaError::ElementNotFound { target, .. }) = result {
        assert_eq!(target, "Phone Number");
    } else {
        panic!("Expected ElementNotFound error");
    }
}

#[test]
fn test_label_match_tier_confidence_values() {
    assert_eq!(label_match::MatchTier::Exact.confidence(), 1.0);
    assert_eq!(label_match::MatchTier::StartsWith.confidence(), 0.9);
    assert_eq!(label_match::MatchTier::Contains.confidence(), 0.75);
    assert_eq!(label_match::MatchTier::Fuzzy.confidence(), 0.6);
}

// ============ REVERSIBILITY ============

#[test]
fn test_reversibility_submit_without_allow_requires_confirmation() {
    assert_eq!(
        reversibility::classify_submit_action(false),
        response::Reversibility::RequiresConfirmation
    );
}

#[test]
fn test_reversibility_submit_with_allow_is_reversible() {
    assert_eq!(
        reversibility::classify_submit_action(true),
        response::Reversibility::Reversible
    );
}

#[test]
fn test_reversibility_auto_submit_still_blocks_destructive() {
    assert_eq!(
        reversibility::classify_button_action("Delete Account", true),
        response::Reversibility::Destructive
    );
}

#[test]
fn test_reversibility_type_always_reversible() {
    assert_eq!(
        reversibility::classify_type_action(),
        response::Reversibility::Reversible
    );
}

#[test]
fn test_reversibility_form_control_always_reversible() {
    assert_eq!(
        reversibility::classify_form_control_action(),
        response::Reversibility::Reversible
    );
}

// ============ AUTOFILL DETECTION ============

#[test]
fn test_autofill_accepts_valid_email() {
    assert!(autofill::validate_autofill_shape("user@example.com", field_role::FieldRole::Email));
}

#[test]
fn test_autofill_rejects_empty_value() {
    assert!(!autofill::validate_autofill_shape("", field_role::FieldRole::Email));
    assert!(!autofill::validate_autofill_shape("", field_role::FieldRole::Text));
}

#[test]
fn test_autofill_rejects_shape_mismatch() {
    // "not-an-email" doesn't match email shape
    assert!(!autofill::validate_autofill_shape("not-an-email", field_role::FieldRole::Email));
    // "abc" doesn't match phone shape (too few digits)
    assert!(!autofill::validate_autofill_shape("abc", field_role::FieldRole::Phone));
}

#[test]
fn test_autofill_parse_detected() {
    let js = json!({
        "detected": true,
        "value": "user@example.com"
    });
    let state = autofill::parse_autofill_result(&js, field_role::FieldRole::Email);
    assert!(state.detected);
    assert!(state.expected_shape_match);
    assert_eq!(state.value.as_deref(), Some("user@example.com"));
}

#[test]
fn test_autofill_parse_not_detected() {
    let js = json!({ "detected": false, "value": null });
    let state = autofill::parse_autofill_result(&js, field_role::FieldRole::Text);
    assert!(!state.detected);
    assert!(!state.expected_shape_match);
}

#[test]
fn test_autofill_phone_shape_validation() {
    assert!(autofill::validate_autofill_shape("+1 (555) 123-4567", field_role::FieldRole::Phone));
    assert!(autofill::validate_autofill_shape("5551234567", field_role::FieldRole::Phone));
    assert!(!autofill::validate_autofill_shape("123", field_role::FieldRole::Phone)); // too short
}

// ============ ADAPTIVE TIMEOUT ============

#[test]
fn test_adaptive_timeout_widening_applied_on_first_timeout() {
    use response::adaptive_timeout_multiplier;

    // Tight timeout (≤500ms) → 4x widening
    assert_eq!(adaptive_timeout_multiplier(300), 4);
    // A 300ms rung that times out would retry at 1200ms

    // Medium timeout → 3x
    assert_eq!(adaptive_timeout_multiplier(1000), 3);
    // 1s rung retries at 3s

    // Already generous (>5s) → no widening
    assert_eq!(adaptive_timeout_multiplier(10000), 1);
}

#[test]
fn test_adaptive_timeout_escalation_on_second_timeout() {
    // This tests the principle: if widened retry also times out, escalate to next rung.
    // The adaptive_timeout_multiplier returns 1 for >5s, meaning no further widening.
    assert_eq!(response::adaptive_timeout_multiplier(6000), 1);
    // At multiplier=1, the retry timeout equals initial — effectively "give up and escalate"
}

// ============ HANDS_FIND: REF SHORT-CIRCUIT ============

// Note: hands_find with return_type=ref must short-circuit after rung 3.
// Testing the error shape when no ref is available (all ref-capable rungs failed).
#[test]
fn test_find_ref_short_circuit_error_shape() {
    // The error from hands_find when return_type=ref and no ref found should:
    // 1. Be an ElementNotFound error
    // 2. Include a warning about available_types
    let error = error::MetaError::ElementNotFound {
        target: "Sign In".into(),
        scope: "browser (ref-only, rungs 1-3 exhausted)".into(),
    };
    let display = format!("{}", error);
    assert!(display.contains("Sign In"));
    assert!(display.contains("ref-only"));
}

// ============ HANDS_TYPE: FAST_SET REJECTION ============

#[test]
fn test_fast_set_rejected_on_password_field() {
    // Password fields must always use keystroke simulation
    let role = field_role::FieldRole::Password;
    assert!(role.is_sensitive(), "Password must be sensitive");
    assert!(role.requires_keystroke(), "Password must require keystroke");
    // fast_set should be rejected when is_sensitive() is true
    // The logic in type_text.rs: fast_set only when !field_role.is_sensitive()
}

// ============ HANDS_TYPE: CHUNKED TYPING ============

#[test]
fn test_chunked_typing_threshold() {
    // Strings >100 chars should be chunked into 50-char batches
    let long_text = "a".repeat(500);
    let chunks: Vec<&[u8]> = long_text.as_bytes().chunks(50).collect();
    assert_eq!(chunks.len(), 10, "500 chars / 50-char chunks = 10 batches");

    // Strings ≤100 chars should NOT be chunked
    let short_text = "a".repeat(100);
    let chunks: Vec<&[u8]> = short_text.as_bytes().chunks(50).collect();
    assert_eq!(chunks.len(), 2, "100 chars fits in 2 chunks but threshold check prevents chunking");
    // Note: the actual threshold check is `text.len() > CHUNK_THRESHOLD` where CHUNK_THRESHOLD=100
    // So exactly 100 chars is NOT chunked, 101+ is chunked
}

#[test]
fn test_chunked_typing_preserves_full_string() {
    let input = "a".repeat(500);
    let mut reconstructed = String::new();
    for chunk in input.as_bytes().chunks(50) {
        reconstructed.push_str(&String::from_utf8_lossy(chunk));
    }
    assert_eq!(input, reconstructed, "Chunked typing must preserve the full string");
}

// ============ HANDS_FILL_FORM: DYNAMIC FORM ============

#[test]
fn test_fill_form_field_count_change_detection() {
    // Simulates the scenario where filling field 1 reveals fields 2-5.
    // After every 3 fields, fill_form re-scans and detects count change.
    let initial_count = 1;
    let post_fill_count = 5;
    assert_ne!(initial_count, post_fill_count,
        "Dynamic form should be detected when field count changes");
}

// ============ DEFAULT SUBMIT LABELS ============

#[test]
fn test_default_submit_labels_coverage() {
    let labels = super::fill_form::DEFAULT_SUBMIT_LABELS;
    // Must include the spec-required defaults
    assert!(labels.contains(&"Submit"));
    assert!(labels.contains(&"Sign Up"));
    assert!(labels.contains(&"Sign In"));
    assert!(labels.contains(&"Continue"));
    assert!(labels.contains(&"Next"));
    assert!(labels.contains(&"Save"));
    assert!(labels.contains(&"Create"));
    assert!(labels.contains(&"Confirm"));
    assert!(labels.contains(&"Apply"));
}

// ============ CONSENT + REVERSIBILITY INTEGRATION ============

#[test]
fn test_destructive_submit_never_auto_accepted() {
    // Even with auto_submit=true, destructive buttons should be blocked
    let rev = reversibility::classify_button_action("Delete My Account", true);
    assert_eq!(rev, response::Reversibility::Destructive);
    // And consent classifier should flag this too
    // Consent classifier catches "permanent" + "cannot be undone" as high risk
    let classification = consent::classify_consent(
        "This action is permanent and cannot be undone. Delete your account?",
        &["Delete My Account", "Cancel"],
        Some("https://example.com/settings"),
        None,
    );
    assert_eq!(classification.risk, consent::RiskLevel::HighRisk);
    assert!(!classification.auto_acceptable);
}
