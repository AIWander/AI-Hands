//! Browser autofill detection — check if browser has pre-filled form fields.
//! Used by hands_fill_form to avoid overwriting autofilled values.

use super::field_role::FieldRole;
use serde::{Deserialize, Serialize};

/// Result of autofill detection for a single field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutofillState {
    /// Whether autofill was detected on this field.
    pub detected: bool,
    /// The autofilled value (if any).
    pub value: Option<String>,
    /// Whether the autofilled value matches the expected shape for the field role.
    pub expected_shape_match: bool,
}

/// JS to check autofill state of a field by selector.
/// Checks :-webkit-autofill pseudo-class and reads current value.
pub const JS_CHECK_AUTOFILL: &str = r#"
(function(sel) {
    var el = document.querySelector(sel);
    if (!el) return { detected: false, value: null, reason: 'not_found' };

    var value = el.value || '';

    // Check :-webkit-autofill (works in Chrome/Edge)
    var isAutofilled = false;
    try {
        isAutofilled = el.matches(':-webkit-autofill');
    } catch(e) {
        // Pseudo-class not supported
    }

    // Also check if field has a value but no user input events were fired
    // (heuristic: autofilled fields often have values before any interaction)
    if (!isAutofilled && value.length > 0) {
        // Check data attribute that some browsers set
        if (el.hasAttribute('data-autofilled') ||
            el.getAttribute('autocomplete') !== 'off') {
            // Could be autofill — trust the value if it exists
            isAutofilled = true;
        }
    }

    return {
        detected: isAutofilled,
        value: value || null,
        type: el.type || 'text',
        autocomplete: el.autocomplete || ''
    };
})
"#;

/// Validate that an autofilled value matches the expected shape for a field role.
pub fn validate_autofill_shape(value: &str, role: FieldRole) -> bool {
    if value.is_empty() {
        return false;
    }

    match role {
        FieldRole::Email => {
            // Basic email shape: contains @ and at least one dot after @
            value.contains('@') && value.split('@').nth(1).map_or(false, |d| d.contains('.'))
        }
        FieldRole::Phone => {
            // Phone: mostly digits, possibly with +, -, (, ), spaces
            let digit_count = value.chars().filter(|c| c.is_ascii_digit()).count();
            digit_count >= 7 && digit_count <= 15
        }
        FieldRole::Number => {
            // Numeric: parseable as number (possibly with formatting)
            let cleaned: String = value
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            !cleaned.is_empty() && cleaned.parse::<f64>().is_ok()
        }
        FieldRole::Url => {
            value.starts_with("http://") || value.starts_with("https://") || value.contains("://")
        }
        FieldRole::Date => {
            // Date: contains digits and separators
            let has_digits = value.chars().any(|c| c.is_ascii_digit());
            let has_separator = value.contains('-') || value.contains('/') || value.contains('.');
            has_digits && has_separator
        }
        // Text, Search, Password — any non-empty value is valid shape
        _ => true,
    }
}

/// Build an AutofillState from raw JS check result.
pub fn parse_autofill_result(js_result: &serde_json::Value, role: FieldRole) -> AutofillState {
    let detected = js_result
        .get("detected")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let value = js_result
        .get("value")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let shape_match = value
        .as_deref()
        .map(|v| validate_autofill_shape(v, role))
        .unwrap_or(false);

    AutofillState {
        detected,
        value,
        expected_shape_match: shape_match,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_shape_valid() {
        assert!(validate_autofill_shape(
            "user@example.com",
            FieldRole::Email
        ));
        assert!(validate_autofill_shape("a@b.co", FieldRole::Email));
    }

    #[test]
    fn test_email_shape_invalid() {
        assert!(!validate_autofill_shape("not-email", FieldRole::Email));
        assert!(!validate_autofill_shape("@", FieldRole::Email));
        assert!(!validate_autofill_shape("", FieldRole::Email));
    }

    #[test]
    fn test_phone_shape_valid() {
        assert!(validate_autofill_shape("555-123-4567", FieldRole::Phone));
        assert!(validate_autofill_shape(
            "+1 (555) 123-4567",
            FieldRole::Phone
        ));
        assert!(validate_autofill_shape("5551234567", FieldRole::Phone));
    }

    #[test]
    fn test_phone_shape_invalid() {
        assert!(!validate_autofill_shape("abc", FieldRole::Phone));
        assert!(!validate_autofill_shape("123", FieldRole::Phone)); // too short
    }

    #[test]
    fn test_number_shape() {
        assert!(validate_autofill_shape("42", FieldRole::Number));
        assert!(validate_autofill_shape("3.14", FieldRole::Number));
        assert!(!validate_autofill_shape("abc", FieldRole::Number));
    }

    #[test]
    fn test_text_any_value() {
        assert!(validate_autofill_shape("anything", FieldRole::Text));
        assert!(!validate_autofill_shape("", FieldRole::Text));
    }

    #[test]
    fn test_parse_autofill_detected() {
        let js = serde_json::json!({
            "detected": true,
            "value": "user@example.com",
            "type": "email"
        });
        let state = parse_autofill_result(&js, FieldRole::Email);
        assert!(state.detected);
        assert_eq!(state.value.as_deref(), Some("user@example.com"));
        assert!(state.expected_shape_match);
    }

    #[test]
    fn test_parse_autofill_not_detected() {
        let js = serde_json::json!({
            "detected": false,
            "value": null
        });
        let state = parse_autofill_result(&js, FieldRole::Text);
        assert!(!state.detected);
        assert!(!state.expected_shape_match);
    }

    #[test]
    fn test_parse_autofill_shape_mismatch() {
        let js = serde_json::json!({
            "detected": true,
            "value": "not-an-email"
        });
        let state = parse_autofill_result(&js, FieldRole::Email);
        assert!(state.detected);
        assert!(!state.expected_shape_match);
    }
}
