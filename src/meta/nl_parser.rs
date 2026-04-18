//! NL pattern parser for hands_verify — maps natural language phrasings
//! to structured VerifyExpectation. Three-layer: structured primary, NL secondary, templates tertiary.
//!
//! Pure pattern matching — no LLM calls. Case-insensitive with quote stripping.

use serde::{Deserialize, Serialize};

/// Structured expectation that the verify ladder checks against.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyExpectation {
    pub check_type: CheckType,
    pub target: String,
    pub negated: bool,
    pub require_visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CheckType {
    TextPresent,
    TextAbsent,
    ElementPresent,
    ElementAbsent,
    RegexMatch,
    PageReady,
}

/// Strip leading/trailing quotes (single or double) from a string.
fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\''))) {
            return &s[1..s.len() - 1];
        }
    s
}

/// Parse a natural language verification phrase into a structured expectation.
/// Returns Ok(expectation) on success, Err with suggestion on failure.
pub fn parse_nl(input: &str) -> Result<VerifyExpectation, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(
            "Empty verification phrase. Use structured form: {\"text\": \"...\", \"require_visible\": true}".into()
        );
    }

    let lower = trimmed.to_lowercase();

    // ── Page ready patterns ──
    if lower == "page loaded"
        || lower == "page is ready"
        || lower == "page ready"
        || lower == "page is loaded"
    {
        return Ok(VerifyExpectation {
            check_type: CheckType::PageReady,
            target: String::new(),
            negated: false,
            require_visible: false,
        });
    }

    // ── Negation patterns (check these before positive) ──

    // "does not contain X" / "doesn't contain X"
    if let Some(rest) = strip_negation_prefix(&lower, "does not contain ")
        .or_else(|| strip_negation_prefix(&lower, "doesn't contain "))
        .or_else(|| strip_negation_prefix(&lower, "does not show "))
        .or_else(|| strip_negation_prefix(&lower, "doesn't show "))
        .or_else(|| strip_negation_prefix(&lower, "does not display "))
        .or_else(|| strip_negation_prefix(&lower, "doesn't display "))
    {
        let target = extract_target(trimmed, rest);
        return Ok(VerifyExpectation {
            check_type: CheckType::TextAbsent,
            target,
            negated: true,
            require_visible: false,
        });
    }

    // "no X"
    if lower.starts_with("no ") && lower.len() > 3 {
        let target = strip_quotes(trimmed[3..].trim()).to_string();
        return Ok(VerifyExpectation {
            check_type: CheckType::TextAbsent,
            target,
            negated: true,
            require_visible: false,
        });
    }

    // "not X"
    if lower.starts_with("not ") && lower.len() > 4 {
        let target = strip_quotes(trimmed[4..].trim()).to_string();
        return Ok(VerifyExpectation {
            check_type: CheckType::TextAbsent,
            target,
            negated: true,
            require_visible: false,
        });
    }

    // "X is gone" / "X is absent" / "X is not visible" / "X disappeared"
    if let Some(x) = strip_suffix_pattern(&lower, " is gone")
        .or_else(|| strip_suffix_pattern(&lower, " is absent"))
        .or_else(|| strip_suffix_pattern(&lower, " disappeared"))
        .or_else(|| strip_suffix_pattern(&lower, " is not visible"))
        .or_else(|| strip_suffix_pattern(&lower, " is hidden"))
    {
        let target = extract_target_from_start(trimmed, x);
        return Ok(VerifyExpectation {
            check_type: CheckType::TextAbsent,
            target,
            negated: true,
            require_visible: false,
        });
    }

    // ── Positive prefix patterns ──

    // "shows X" / "displays X" / "contains X"
    if let Some(rest) = strip_positive_prefix(&lower, "shows ")
        .or_else(|| strip_positive_prefix(&lower, "displays "))
        .or_else(|| strip_positive_prefix(&lower, "contains "))
    {
        let target = extract_target(trimmed, rest);
        return Ok(VerifyExpectation {
            check_type: CheckType::TextPresent,
            target,
            negated: false,
            require_visible: false,
        });
    }

    // ── Positive suffix patterns (with visibility) ──

    // "X appears" / "X is visible" / "X is displayed" / "X is shown"
    if let Some(x) = strip_suffix_pattern(&lower, " appears")
        .or_else(|| strip_suffix_pattern(&lower, " is visible"))
        .or_else(|| strip_suffix_pattern(&lower, " is displayed"))
        .or_else(|| strip_suffix_pattern(&lower, " is shown"))
    {
        let target = extract_target_from_start(trimmed, x);
        return Ok(VerifyExpectation {
            check_type: CheckType::TextPresent,
            target,
            negated: false,
            require_visible: true,
        });
    }

    // "X is present"
    if let Some(x) = strip_suffix_pattern(&lower, " is present") {
        let target = extract_target_from_start(trimmed, x);
        return Ok(VerifyExpectation {
            check_type: CheckType::TextPresent,
            target,
            negated: false,
            require_visible: false,
        });
    }

    Err(format!(
        "Could not parse NL phrase: \"{}\". Use structured form: {{\"text\": \"...\", \"require_visible\": true}}",
        trimmed
    ))
}

/// Try to strip a negation prefix, returning the remainder length for target extraction.
fn strip_negation_prefix(lower: &str, prefix: &str) -> Option<usize> {
    if lower.starts_with(prefix) {
        Some(lower.len() - prefix.len())
    } else {
        None
    }
}

/// Try to strip a positive prefix, returning the remainder length for target extraction.
fn strip_positive_prefix(lower: &str, prefix: &str) -> Option<usize> {
    if lower.starts_with(prefix) {
        Some(lower.len() - prefix.len())
    } else {
        None
    }
}

/// Try to strip a suffix pattern, returning the prefix portion length.
fn strip_suffix_pattern(lower: &str, suffix: &str) -> Option<usize> {
    if lower.ends_with(suffix) {
        let prefix_len = lower.len() - suffix.len();
        if prefix_len > 0 {
            Some(prefix_len)
        } else {
            None
        }
    } else {
        None
    }
}

/// Extract target text from the end of the original (case-preserving) input,
/// given the remaining character count after a prefix was stripped.
fn extract_target(original: &str, remaining_chars: usize) -> String {
    let start = original.len() - remaining_chars;
    strip_quotes(original[start..].trim()).to_string()
}

/// Extract target text from the start of the original (case-preserving) input,
/// given the prefix character count before a suffix was stripped.
fn extract_target_from_start(original: &str, prefix_chars: usize) -> String {
    strip_quotes(original[..prefix_chars].trim()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shows_text() {
        let r = parse_nl("shows Welcome Back").unwrap();
        assert_eq!(r.check_type, CheckType::TextPresent);
        assert_eq!(r.target, "Welcome Back");
        assert!(!r.negated);
        assert!(!r.require_visible);
    }

    #[test]
    fn test_displays_text() {
        let r = parse_nl("displays Success").unwrap();
        assert_eq!(r.check_type, CheckType::TextPresent);
        assert_eq!(r.target, "Success");
        assert!(!r.negated);
    }

    #[test]
    fn test_contains_text() {
        let r = parse_nl("contains \"order confirmed\"").unwrap();
        assert_eq!(r.check_type, CheckType::TextPresent);
        assert_eq!(r.target, "order confirmed");
        assert!(!r.negated);
    }

    #[test]
    fn test_appears_visible() {
        let r = parse_nl("Dashboard appears").unwrap();
        assert_eq!(r.check_type, CheckType::TextPresent);
        assert_eq!(r.target, "Dashboard");
        assert!(!r.negated);
        assert!(r.require_visible);
    }

    #[test]
    fn test_is_visible() {
        let r = parse_nl("Submit button is visible").unwrap();
        assert_eq!(r.check_type, CheckType::TextPresent);
        assert_eq!(r.target, "Submit button");
        assert!(!r.negated);
        assert!(r.require_visible);
    }

    #[test]
    fn test_is_displayed() {
        let r = parse_nl("Error message is displayed").unwrap();
        assert_eq!(r.check_type, CheckType::TextPresent);
        assert_eq!(r.target, "Error message");
        assert!(r.require_visible);
    }

    #[test]
    fn test_negation_no() {
        let r = parse_nl("no error").unwrap();
        assert_eq!(r.check_type, CheckType::TextAbsent);
        assert_eq!(r.target, "error");
        assert!(r.negated);
    }

    #[test]
    fn test_negation_doesnt_show() {
        let r = parse_nl("doesn't show Loading").unwrap();
        assert_eq!(r.check_type, CheckType::TextAbsent);
        assert_eq!(r.target, "Loading");
        assert!(r.negated);
    }

    #[test]
    fn test_negation_is_gone() {
        let r = parse_nl("Spinner is gone").unwrap();
        assert_eq!(r.check_type, CheckType::TextAbsent);
        assert_eq!(r.target, "Spinner");
        assert!(r.negated);
    }

    #[test]
    fn test_negation_disappeared() {
        let r = parse_nl("Loading indicator disappeared").unwrap();
        assert_eq!(r.check_type, CheckType::TextAbsent);
        assert_eq!(r.target, "Loading indicator");
        assert!(r.negated);
    }

    #[test]
    fn test_page_ready() {
        let r = parse_nl("page loaded").unwrap();
        assert_eq!(r.check_type, CheckType::PageReady);
        assert_eq!(r.target, "");
        assert!(!r.negated);
    }

    #[test]
    fn test_page_is_ready() {
        let r = parse_nl("page is ready").unwrap();
        assert_eq!(r.check_type, CheckType::PageReady);
    }

    #[test]
    fn test_case_insensitive() {
        let r = parse_nl("SHOWS Hello World").unwrap();
        assert_eq!(r.check_type, CheckType::TextPresent);
        assert_eq!(r.target, "Hello World");
    }

    #[test]
    fn test_quoted_target() {
        let r = parse_nl("contains 'Sign In'").unwrap();
        assert_eq!(r.target, "Sign In");
    }

    #[test]
    fn test_does_not_contain() {
        let r = parse_nl("does not contain Error").unwrap();
        assert_eq!(r.check_type, CheckType::TextAbsent);
        assert_eq!(r.target, "Error");
        assert!(r.negated);
    }

    #[test]
    fn test_not_prefix() {
        let r = parse_nl("not visible").unwrap();
        assert_eq!(r.check_type, CheckType::TextAbsent);
        assert_eq!(r.target, "visible");
        assert!(r.negated);
    }

    #[test]
    fn test_unparseable_returns_error() {
        let r = parse_nl("foobar something random");
        assert!(r.is_err());
        let msg = r.unwrap_err();
        assert!(msg.contains("Could not parse NL phrase"));
        assert!(msg.contains("structured form"));
    }

    #[test]
    fn test_empty_returns_error() {
        let r = parse_nl("");
        assert!(r.is_err());
    }

    #[test]
    fn test_is_shown_visible() {
        let r = parse_nl("Welcome is shown").unwrap();
        assert_eq!(r.check_type, CheckType::TextPresent);
        assert_eq!(r.target, "Welcome");
        assert!(r.require_visible);
    }

    #[test]
    fn test_is_not_visible() {
        let r = parse_nl("Modal is not visible").unwrap();
        assert_eq!(r.check_type, CheckType::TextAbsent);
        assert_eq!(r.target, "Modal");
        assert!(r.negated);
    }
}
