//! Phase A v2 tests — focused on RELIABILITY features, not happy path.
//! Coverage areas:
//! - Cache invalidation scenarios (event, mutation observer, hash mismatch)
//! - Error taxonomy (all categories, Display impl)
//! - Reversibility classification edge cases
//! - Targeting helpers (fuzzy match, priority order)
//! - Adaptive timeout widening
//! - Instrumentation redaction (OTP scrubbing)
//! - Vision capture (downscale, crop, tile)
//! - Consent classifier (risk levels)

use super::*;
use serde_json::json;

// ============ CACHE INVALIDATION ============

#[test]
fn test_cache_event_invalidation_navigate() {
    let mut cache = cache::A11yMetaCache::new();
    cache.store(json!({"role": "document"}), "https://example.com");
    assert!(cache.get().is_some(), "Cache should be valid after store");

    cache.on_event(cache::InvalidationEvent::Navigate);
    assert!(
        cache.get().is_none(),
        "Cache should be invalid after navigate"
    );
}

#[test]
fn test_cache_event_invalidation_click() {
    let mut cache = cache::A11yMetaCache::new();
    cache.store(json!({"role": "button"}), "https://example.com");

    cache.on_event(cache::InvalidationEvent::Click);
    assert!(cache.get().is_none(), "Cache should be invalid after click");
}

#[test]
fn test_cache_event_invalidation_type() {
    let mut cache = cache::A11yMetaCache::new();
    cache.store(json!({"role": "textbox"}), "https://example.com");

    cache.on_event(cache::InvalidationEvent::Type);
    assert!(cache.get().is_none(), "Cache should be invalid after type");
}

#[test]
fn test_cache_mutation_invalidation() {
    let mut cache = cache::A11yMetaCache::new();
    cache.store(
        json!({"role": "document", "children": []}),
        "https://example.com",
    );
    assert!(cache.get().is_some());

    cache.on_mutation();
    assert!(
        cache.get().is_none(),
        "Mutation observer dirty flag should invalidate cache"
    );
}

#[test]
fn test_cache_hash_mismatch_invalidation() {
    let mut cache = cache::A11yMetaCache::new();
    cache.store(
        json!({"role": "document", "name": "Page 1"}),
        "https://example.com",
    );

    // Same content — hash matches
    assert!(cache.verify_hash(&json!({"role": "document", "name": "Page 1"})));

    // Different content — hash mismatch
    assert!(!cache.verify_hash(&json!({"role": "document", "name": "Page 2"})));
}

#[test]
fn test_cache_stays_valid_without_events() {
    let mut cache = cache::A11yMetaCache::new();
    cache.store(json!({"role": "document"}), "https://example.com");

    // No events, no mutations, same hash — cache stays valid
    assert!(cache.get().is_some());
    assert!(cache.get().is_some()); // Second read still valid
}

#[test]
fn test_cache_clear_resets_everything() {
    let mut cache = cache::A11yMetaCache::new();
    cache.store(json!({"role": "document"}), "https://example.com");
    cache.on_mutation(); // dirty
    cache.clear();

    assert!(!cache.has_data());
    assert!(cache.get().is_none());
    assert_eq!(cache.url(), "");
}

// ============ ERROR TAXONOMY ============

#[test]
fn test_error_categories() {
    assert_eq!(
        error::MetaError::not_found("btn", "browser").category(),
        "targeting"
    );
    assert_eq!(error::MetaError::no_browser().category(), "infrastructure");
    assert_eq!(error::MetaError::no_page().category(), "infrastructure");
    assert_eq!(
        error::MetaError::timeout("click", 5000).category(),
        "infrastructure"
    );
    assert_eq!(
        error::MetaError::subsystem("uia", "COM error").category(),
        "infrastructure"
    );
    assert_eq!(error::MetaError::DynamicContentChanged.category(), "state");
    assert_eq!(
        error::MetaError::requires_confirmation("Delete", "irreversible").category(),
        "content"
    );
    assert_eq!(error::MetaError::other("unknown").category(), "other");

    // Phase B/C variants
    let script_err = error::MetaError::ScriptStepFailed {
        step_index: 2,
        step_label: Some("login".into()),
        inner: Box::new(error::MetaError::no_page()),
    };
    assert_eq!(script_err.category(), "script");

    let input_err = error::MetaError::RequiresUserInput {
        needs: "credentials".into(),
        field_label: None,
        field_type: None,
        reason: "No saved credentials".into(),
    };
    assert_eq!(input_err.category(), "input");
}

#[test]
fn test_error_display() {
    let err = error::MetaError::not_found("Sign In", "browser");
    assert!(err.to_string().contains("Sign In"));
    assert!(err.to_string().contains("browser"));

    let err = error::MetaError::timeout("click", 5000);
    assert!(err.to_string().contains("5000"));
}

#[test]
fn test_error_multiple_matches() {
    let err = error::MetaError::MultipleMatches {
        target: "Submit".into(),
        candidates: vec![
            error::MatchCandidate {
                text: "Submit Form".into(),
                role: Some("button".into()),
                selector: None,
                confidence: 0.9,
            },
            error::MatchCandidate {
                text: "Submit Review".into(),
                role: Some("button".into()),
                selector: None,
                confidence: 0.85,
            },
        ],
    };
    assert!(err.to_string().contains("2 candidates"));
}

// ============ REVERSIBILITY CLASSIFICATION ============

#[test]
fn test_reversibility_destructive_patterns() {
    use response::Reversibility;
    use targeting::classify_reversibility;

    assert_eq!(
        classify_reversibility("Delete Account"),
        Reversibility::Destructive
    );
    assert_eq!(
        classify_reversibility("Remove Item"),
        Reversibility::Destructive
    );
    assert_eq!(
        classify_reversibility("Confirm Payment"),
        Reversibility::Destructive
    );
    assert_eq!(
        classify_reversibility("Pay Now"),
        Reversibility::Destructive
    );
    assert_eq!(
        classify_reversibility("Publish Article"),
        Reversibility::Destructive
    );
}

#[test]
fn test_reversibility_confirmation_patterns() {
    use response::Reversibility;
    use targeting::classify_reversibility;

    assert_eq!(
        classify_reversibility("Submit Form"),
        Reversibility::RequiresConfirmation
    );
    assert_eq!(
        classify_reversibility("Sign Up"),
        Reversibility::RequiresConfirmation
    );
    assert_eq!(
        classify_reversibility("Create Account"),
        Reversibility::RequiresConfirmation
    );
    assert_eq!(
        classify_reversibility("Save Changes"),
        Reversibility::RequiresConfirmation
    );
}

#[test]
fn test_reversibility_safe_patterns() {
    use response::Reversibility;
    use targeting::classify_reversibility;

    assert_eq!(classify_reversibility("Next"), Reversibility::Reversible);
    assert_eq!(
        classify_reversibility("Read More"),
        Reversibility::Reversible
    );
    assert_eq!(
        classify_reversibility("Open Settings"),
        Reversibility::Reversible
    );
    assert_eq!(
        classify_reversibility("View Details"),
        Reversibility::Reversible
    );
    assert_eq!(classify_reversibility("Go Back"), Reversibility::Reversible);
}

#[test]
fn test_reversibility_case_insensitive() {
    use response::Reversibility;
    use targeting::classify_reversibility;

    assert_eq!(classify_reversibility("DELETE"), Reversibility::Destructive);
    assert_eq!(
        classify_reversibility("delete all"),
        Reversibility::Destructive
    );
    assert_eq!(
        classify_reversibility("SUBMIT"),
        Reversibility::RequiresConfirmation
    );
}

// ============ TARGETING HELPERS ============

#[test]
fn test_fuzzy_match_exact() {
    assert_eq!(targeting::fuzzy_match_score("Submit", "Submit"), 1.0);
    assert_eq!(targeting::fuzzy_match_score("submit", "SUBMIT"), 1.0); // case insensitive
}

#[test]
fn test_fuzzy_match_containment() {
    let score = targeting::fuzzy_match_score("Sign", "Sign In");
    assert!(score > 0.5, "Containment should score >0.5, got {}", score);
}

#[test]
fn test_fuzzy_match_no_match() {
    let score = targeting::fuzzy_match_score("xyz", "abc");
    assert!(score < 0.3, "No match should score low, got {}", score);
}

#[test]
fn test_match_priority_order() {
    let elem = json!({
        "name": "Submit Form",
        "aria-label": "Send",
        "placeholder": "Enter text",
        "title": "Click to submit"
    });
    let matches = targeting::match_priority_text(&elem);
    assert!(!matches.is_empty());
    // First match should be name (highest priority 1.0)
    assert_eq!(matches[0].0, "Submit Form");
    assert_eq!(matches[0].1, 1.0);
}

#[test]
fn test_locale_insensitive_eq() {
    assert!(targeting::locale_insensitive_eq("Hello", "hello"));
    assert!(targeting::locale_insensitive_eq("SUBMIT", "submit"));
    assert!(!targeting::locale_insensitive_eq("Submit", "Cancel"));
}

#[test]
fn test_content_needs_js() {
    assert!(targeting::content_needs_js(
        "Please enable JavaScript to continue"
    ));
    assert!(targeting::content_needs_js("This site requires javascript"));
    assert!(!targeting::content_needs_js("Welcome to our website"));
}

#[test]
fn test_content_sufficient() {
    assert!(!targeting::content_is_sufficient("short", 200));
    assert!(targeting::content_is_sufficient(&"a".repeat(201), 200));
    assert!(!targeting::content_is_sufficient(
        "Please enable JavaScript to continue. This site requires JavaScript.",
        50, // Short threshold but JS-required
    ));
}

// ============ ADAPTIVE TIMEOUT ============

#[test]
fn test_adaptive_timeout_widening() {
    use response::adaptive_timeout_multiplier;

    // Tight timeout → 4x widening
    assert_eq!(adaptive_timeout_multiplier(200), 4);
    assert_eq!(adaptive_timeout_multiplier(500), 4);

    // Medium → 3x
    assert_eq!(adaptive_timeout_multiplier(1000), 3);
    assert_eq!(adaptive_timeout_multiplier(2000), 3);

    // Wide → 2x
    assert_eq!(adaptive_timeout_multiplier(3000), 2);
    assert_eq!(adaptive_timeout_multiplier(5000), 2);

    // Already generous → no widening
    assert_eq!(adaptive_timeout_multiplier(10000), 1);
}

// ============ RESPONSE ENVELOPE ============

#[test]
fn test_meta_tool_result_success() {
    let result = response::MetaToolResult::success(
        "a11y_cache",
        vec![response::RungAttempt::ok("a11y_cache", 150)],
        json!({"clicked": true}),
        150,
    );
    assert!(result.success);
    assert_eq!(result.method, "a11y_cache");
    assert_eq!(result.rungs_tried.len(), 1);
    assert!(result.error.is_none());
}

#[test]
fn test_meta_tool_result_failure() {
    let result = response::MetaToolResult::failure(
        vec![
            response::RungAttempt::failed("rung1", 100, "not found"),
            response::RungAttempt::timed_out("rung2", 500),
        ],
        error::MetaError::not_found("btn", "browser"),
        600,
    );
    assert!(!result.success);
    assert_eq!(result.rungs_tried.len(), 2);
    assert!(result.rungs_tried[1].timed_out);
    assert!(result.error.is_some());
}

#[test]
fn test_meta_tool_result_serialization() {
    let result = response::MetaToolResult::success("test", vec![], json!({}), 0)
        .with_confidence(response::Confidence::dual(0.9, 0.7))
        .with_reversibility(response::Reversibility::Destructive)
        .with_warning("Test warning");

    let val = result.to_value();
    assert_eq!(val["success"], true);
    assert_eq!(val["reversibility"], "destructive");
    let method_conf = val["confidence"]["method"].as_f64().unwrap();
    assert!(
        (method_conf - 0.9).abs() < 0.01,
        "method confidence: {}",
        method_conf
    );
    let loc_conf = val["confidence"]["location"].as_f64().unwrap();
    assert!(
        (loc_conf - 0.7).abs() < 0.01,
        "location confidence: {}",
        loc_conf
    );
    assert_eq!(val["warnings"][0], "Test warning");
}

// ============ CLICK HELPERS ============

#[test]
fn test_clickable_scoring_exact_match() {
    let clickables = json!({
        "clickables": [
            {"text": "Submit", "x": 100, "y": 200},
            {"text": "Cancel", "x": 300, "y": 200},
        ]
    });
    let coords = click::find_best_clickable_coords(&clickables, "Submit");
    assert_eq!(coords, Some((100, 200)));
}

#[test]
fn test_clickable_scoring_partial_match() {
    let clickables = json!({
        "clickables": [
            {"text": "Submit Application Form", "x": 100, "y": 200},
            {"text": "Cancel", "x": 300, "y": 200},
        ]
    });
    let coords = click::find_best_clickable_coords(&clickables, "Submit");
    assert!(coords.is_some());
}

#[test]
fn test_clickable_scoring_no_match() {
    let clickables = json!({
        "clickables": [
            {"text": "OK", "x": 100, "y": 200},
        ]
    });
    let coords = click::find_best_clickable_coords(&clickables, "Sign In");
    assert!(coords.is_none());
}

#[test]
fn test_ocr_word_single_match() {
    let words = vec![
        ("Submit".into(), 100.0, 200.0, 60.0, 20.0),
        ("Form".into(), 170.0, 200.0, 40.0, 20.0),
    ];
    let coords = click::find_text_in_ocr_words(&words, "Submit");
    assert!(coords.is_some());
    let (x, y) = coords.unwrap();
    assert_eq!(x, 130); // center of "Submit" box
    assert_eq!(y, 210); // center of "Submit" box
}

#[test]
fn test_ocr_word_multiword_span() {
    let words = vec![
        ("Sign".into(), 100.0, 200.0, 40.0, 20.0),
        ("In".into(), 145.0, 200.0, 20.0, 20.0),
        ("Here".into(), 175.0, 200.0, 40.0, 20.0),
    ];
    let coords = click::find_text_in_ocr_words(&words, "Sign In");
    assert!(coords.is_some());
}

// ============ VISION CAPTURE ============

#[test]
fn test_downscale_preserves_aspect() {
    let (w, h) = vision_capture::downscale_dimensions(3840, 2160);
    let orig_ratio = 3840.0 / 2160.0;
    let new_ratio = w as f64 / h as f64;
    assert!((orig_ratio - new_ratio).abs() < 0.05);
    assert!(w <= vision_capture::TARGET_LONG_EDGE);
}

#[test]
fn test_tile_coverage() {
    let tiles = vision_capture::compute_tile_grid(1920, 1080, 2, 2);
    let total: u32 = tiles.iter().map(|(_, _, w, h)| w * h).sum();
    assert_eq!(total, 1920 * 1080);
}

#[test]
fn test_crop_padding() {
    let (x, y, w, h) = vision_capture::compute_crop_region(500, 300, 100, 50, 1920, 1080);
    assert!(x < 500);
    assert!(y < 300);
    assert!(w > 100);
    assert!(h > 50);
}

// ============ CONSENT CLASSIFIER ============

#[test]
fn test_consent_cookie_banner_no_risk() {
    let result = consent::classify_consent(
        "We use cookies to improve your experience",
        &["Accept", "Decline"],
        Some("https://example.com"),
        None,
    );
    assert!(matches!(
        result.risk,
        consent::RiskLevel::NoRisk | consent::RiskLevel::LowRisk
    ));
    assert!(result.auto_acceptable);
}

#[test]
fn test_consent_payment_high_risk() {
    let result = consent::classify_consent(
        "You will be charged $99.99 per month",
        &["Pay Now", "Cancel"],
        Some("https://example.com/checkout"),
        None,
    );
    assert_eq!(result.risk, consent::RiskLevel::HighRisk);
    assert!(!result.auto_acceptable);
}

#[test]
fn test_consent_delete_high_risk() {
    let result = consent::classify_consent(
        "This action cannot be undone. Your account will be permanently deleted.",
        &["Delete My Account", "Keep Account"],
        Some("https://example.com/settings"),
        None,
    );
    assert_eq!(result.risk, consent::RiskLevel::HighRisk);
    assert!(!result.auto_acceptable);
}

#[test]
fn test_consent_auto_accept_respects_flag() {
    let classification = consent::ConsentClassification {
        risk: consent::RiskLevel::LowRisk,
        reasoning: "test".into(),
        signals: vec![],
        auto_acceptable: true,
    };

    assert!(!consent::should_auto_accept(&classification, false));
    assert!(consent::should_auto_accept(&classification, true));
}

// ============ SESSION STATE ============

#[test]
fn test_session_subsystem_check() {
    let mut state = session::SessionState::default();
    state.subsystem_health.browser = session::SubsystemStatus::Available;
    state.subsystem_health.uia = session::SubsystemStatus::Unavailable {
        reason: "COM init failed".into(),
    };

    assert!(state.subsystem_available("browser").is_ok());
    assert!(state.subsystem_available("uia").is_err());
    assert!(state.subsystem_available("unknown").is_ok()); // unknown = assume OK
}

#[test]
fn test_session_a11y_dirty_flag() {
    let mut state = session::SessionState::default();
    assert!(!state.check_and_clear_a11y_dirty());

    state.mark_a11y_dirty();
    assert!(state.check_and_clear_a11y_dirty());
    assert!(!state.check_and_clear_a11y_dirty()); // cleared
}

#[test]
fn test_session_call_id_generation() {
    let mut state = session::SessionState::default();
    let id1 = state.next_call_id();
    let id2 = state.next_call_id();
    assert_ne!(id1, id2);
    assert!(id1.starts_with("call_"));
}

#[test]
fn test_session_monitor_tracking() {
    let mut state = session::SessionState::default();
    state.record_window_monitor("chrome", 0, 1.0);

    let monitor = state.get_window_monitor("chrome");
    assert!(monitor.is_some());
    assert_eq!(monitor.unwrap().monitor_index, 0);

    assert!(state.get_window_monitor("notepad").is_none());
}
