//! Consent risk classifier — full implementation for Phase C auto-accept.
//! Classifies dialog/prompt as NoRisk | LowRisk | MediumRisk | HighRisk.
//! Uses text patterns, button analysis, URL context, and element context
//! to produce a composite risk score with weighted signals.

use serde::{Deserialize, Serialize};

/// Risk classification for dialog auto-accept decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// Cookie banners, GDPR consent, privacy policy ack, newsletter dismiss
    NoRisk,
    /// Site ToS for normal browsing, content preferences, location sharing
    LowRisk,
    /// ToS for new account creation, subscription signups, data sharing
    MediumRisk,
    /// Payment terms, binding arbitration, account deletion, financial
    HighRisk,
}

/// Classification result with reasoning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsentClassification {
    pub risk: RiskLevel,
    pub reasoning: String,
    pub signals: Vec<ConsentSignal>,
    /// Whether this should be auto-accepted given session flag.
    pub auto_acceptable: bool,
}

/// Individual signal that contributed to the risk classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsentSignal {
    pub source: SignalSource,
    pub text: String,
    pub weight: f32,
}

/// Where the signal came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalSource {
    DialogText,
    ButtonText,
    ElementContext,
    UrlContext,
}

/// Classify a dialog or prompt's risk level.
///
/// Analyzes dialog text, button labels, URL, and nearby element context
/// to produce a weighted composite risk score.
pub fn classify_consent(
    dialog_text: &str,
    button_texts: &[&str],
    url: Option<&str>,
    element_context: Option<&serde_json::Value>,
) -> ConsentClassification {
    let lower_text = dialog_text.to_lowercase();
    let lower_buttons: Vec<String> = button_texts.iter().map(|b| b.to_lowercase()).collect();
    let lower_url = url.map(|u| u.to_lowercase()).unwrap_or_default();

    let mut signals = Vec::new();

    // ── High risk text patterns ──
    let high_risk_text = [
        "payment", "charge", "subscription", "billing", "arbitration",
        "delete my account", "permanent", "cannot be undone", "irreversible",
        "financial", "credit card", "debit card",
        "total due", "order total", "amount", "bank account", "wire transfer",
        "non-refundable", "auto-renew", "recurring charge", "final sale",
    ];
    for pattern in &high_risk_text {
        if lower_text.contains(pattern) {
            signals.push(ConsentSignal {
                source: SignalSource::DialogText,
                text: format!("High-risk keyword: {}", pattern),
                weight: 1.0,
            });
        }
    }

    // ── Medium risk text patterns ──
    let medium_risk_text = [
        "create account", "sign up", "register",
        "data sharing", "third party", "subscribe",
        "marketing emails", "share data with partners", "opt-in",
        "mailing list", "free trial",
    ];
    for pattern in &medium_risk_text {
        if lower_text.contains(pattern) {
            signals.push(ConsentSignal {
                source: SignalSource::DialogText,
                text: format!("Medium-risk keyword: {}", pattern),
                weight: 0.5,
            });
        }
    }

    // ── Low risk text patterns ──
    let low_risk_text = [
        "terms of service", "privacy policy", "content preferences",
        "notification settings", "location sharing",
    ];
    for pattern in &low_risk_text {
        if lower_text.contains(pattern) {
            signals.push(ConsentSignal {
                source: SignalSource::DialogText,
                text: format!("Low-risk keyword: {}", pattern),
                weight: -0.3,
            });
        }
    }

    // ── No risk text patterns (cookie banners, GDPR) ──
    let no_risk_text = [
        "we value your privacy", "this site uses cookies",
        "accept all cookies", "only necessary cookies", "manage preferences",
        "cookie", "gdpr", "we use cookies",
        "continue to site", "age verification", "newsletter", "dismiss",
    ];
    for pattern in &no_risk_text {
        if lower_text.contains(pattern) {
            signals.push(ConsentSignal {
                source: SignalSource::DialogText,
                text: format!("No-risk keyword: {}", pattern),
                weight: -0.5,
            });
        }
    }

    // ── High risk button patterns ──
    let high_risk_buttons = [
        "pay now", "delete", "confirm payment", "purchase",
        "place order", "confirm purchase", "submit payment", "agree and pay",
    ];
    for btn in &lower_buttons {
        for pattern in &high_risk_buttons {
            if btn.contains(pattern) {
                signals.push(ConsentSignal {
                    source: SignalSource::ButtonText,
                    text: format!("High-risk button: {}", btn),
                    weight: 1.0,
                });
            }
        }
    }

    // ── Medium risk button patterns ──
    let medium_risk_buttons = [
        "create account", "sign up", "register", "start trial",
    ];
    for btn in &lower_buttons {
        for pattern in &medium_risk_buttons {
            if btn.contains(pattern) {
                signals.push(ConsentSignal {
                    source: SignalSource::ButtonText,
                    text: format!("Medium-risk button: {}", btn),
                    weight: 0.5,
                });
            }
        }
    }

    // ── Low risk button patterns ──
    let low_risk_buttons = ["i agree", "accept terms", "continue"];
    for btn in &lower_buttons {
        for pattern in &low_risk_buttons {
            if btn.contains(pattern) {
                signals.push(ConsentSignal {
                    source: SignalSource::ButtonText,
                    text: format!("Low-risk button: {}", btn),
                    weight: -0.3,
                });
            }
        }
    }

    // ── No risk button patterns ──
    let no_risk_buttons = [
        "accept cookies", "got it", "ok", "dismiss", "close", "allow all",
    ];
    for btn in &lower_buttons {
        for pattern in &no_risk_buttons {
            // For short patterns like "ok", require exact match or surrounded by non-alpha
            if pattern.len() <= 2 {
                if btn == pattern {
                    signals.push(ConsentSignal {
                        source: SignalSource::ButtonText,
                        text: format!("No-risk button: {}", btn),
                        weight: -0.5,
                    });
                }
            } else if btn.contains(pattern) {
                signals.push(ConsentSignal {
                    source: SignalSource::ButtonText,
                    text: format!("No-risk button: {}", btn),
                    weight: -0.5,
                });
            }
        }
    }

    // ── URL context ──
    let high_risk_urls = ["/checkout", "/payment", "/billing", "/purchase", "/subscribe"];
    for pattern in &high_risk_urls {
        if lower_url.contains(pattern) {
            signals.push(ConsentSignal {
                source: SignalSource::UrlContext,
                text: format!("High-risk URL: {}", pattern),
                weight: 1.0,
            });
        }
    }

    let medium_risk_urls = ["/register", "/signup", "/create-account", "/settings/privacy"];
    for pattern in &medium_risk_urls {
        if lower_url.contains(pattern) {
            signals.push(ConsentSignal {
                source: SignalSource::UrlContext,
                text: format!("Medium-risk URL: {}", pattern),
                weight: 0.5,
            });
        }
    }

    // ── Element context analysis ──
    if let Some(ctx) = element_context {
        let ctx_str = ctx.to_string().to_lowercase();

        // Payment fields nearby → bump to HighRisk
        let payment_indicators = ["credit card", "cvv", "expiration", "card number",
            "cvc", "billing address", "payment method"];
        let has_payment = payment_indicators.iter().any(|p| ctx_str.contains(p));
        if has_payment {
            signals.push(ConsentSignal {
                source: SignalSource::ElementContext,
                text: "Payment fields detected in nearby elements".into(),
                weight: 1.0,
            });
        }

        // Account creation fields nearby → bump to MediumRisk
        let account_indicators = ["password", "confirm password", "create account",
            "username"];
        let has_account = account_indicators.iter().any(|p| ctx_str.contains(p));
        if has_account && !has_payment {
            signals.push(ConsentSignal {
                source: SignalSource::ElementContext,
                text: "Account creation fields detected in nearby elements".into(),
                weight: 0.5,
            });
        }
    }

    // ── Composite scoring ──
    let total_weight: f32 = signals.iter().map(|s| s.weight).sum();
    let risk = if total_weight >= 1.0 {
        RiskLevel::HighRisk
    } else if total_weight >= 0.3 {
        RiskLevel::MediumRisk
    } else if total_weight >= -0.3 {
        RiskLevel::LowRisk
    } else {
        RiskLevel::NoRisk
    };

    let auto_acceptable = matches!(risk, RiskLevel::NoRisk | RiskLevel::LowRisk);

    let reasoning = match risk {
        RiskLevel::NoRisk => "Standard cookie/privacy consent — no risk to user".into(),
        RiskLevel::LowRisk => "Standard browsing terms — low risk".into(),
        RiskLevel::MediumRisk => "Account creation or data sharing — requires user review".into(),
        RiskLevel::HighRisk => "Financial commitment or irreversible action — always surface to user".into(),
    };

    ConsentClassification {
        risk,
        reasoning,
        signals,
        auto_acceptable,
    }
}

/// Check if the session should auto-accept this dialog.
pub fn should_auto_accept(classification: &ConsentClassification, session_auto_accept: bool) -> bool {
    if !session_auto_accept {
        return false;
    }
    classification.auto_acceptable
}

/// Quick check: does a button label look like a consent/dialog button?
/// Used by hands_click to decide whether to run the full classifier.
pub fn looks_like_consent_button(text: &str) -> bool {
    let lower = text.to_lowercase();
    let consent_patterns = [
        "accept", "agree", "allow", "consent", "cookie", "got it",
        "ok", "dismiss", "i understand", "i accept", "continue",
        "confirm", "submit", "place order", "pay now", "purchase",
        "sign up", "register", "create account", "start trial",
    ];
    consent_patterns.iter().any(|p| lower.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payment_form_high_risk_blocked() {
        let classification = classify_consent(
            "Your total due is $49.99. By clicking Submit Payment, you agree to be charged.",
            &["Submit Payment", "Cancel"],
            Some("https://shop.example.com/checkout"),
            Some(&serde_json::json!({
                "fields": ["credit card", "CVV", "expiration"]
            })),
        );
        assert_eq!(classification.risk, RiskLevel::HighRisk);
        assert!(!classification.auto_acceptable);
        // Even with session flag, should NOT auto-accept
        assert!(!should_auto_accept(&classification, true));
    }

    #[test]
    fn test_cookie_banner_no_risk_auto_accepted() {
        let classification = classify_consent(
            "This site uses cookies to improve your experience. We use cookies for analytics and personalization.",
            &["Accept All Cookies", "Manage Preferences"],
            Some("https://blog.example.com/article/123"),
            None,
        );
        assert_eq!(classification.risk, RiskLevel::NoRisk);
        assert!(classification.auto_acceptable);
        assert!(should_auto_accept(&classification, true));
        // Without session flag, should not auto-accept
        assert!(!should_auto_accept(&classification, false));
    }

    #[test]
    fn test_gdpr_consent_auto_accepted() {
        let classification = classify_consent(
            "We value your privacy. This site uses cookies to provide a better experience. GDPR compliant.",
            &["Accept Cookies", "Dismiss"],
            Some("https://news.example.com/"),
            None,
        );
        assert!(matches!(classification.risk, RiskLevel::NoRisk | RiskLevel::LowRisk));
        assert!(classification.auto_acceptable);
        assert!(should_auto_accept(&classification, true));
    }

    #[test]
    fn test_account_creation_tos_medium_risk_not_auto() {
        // Simple account creation without extra data-sharing language
        let classification = classify_consent(
            "By continuing, you agree to our Terms of Service.",
            &["Create Account"],
            Some("https://app.example.com/register"),
            None,
        );
        assert_eq!(classification.risk, RiskLevel::MediumRisk);
        assert!(!classification.auto_acceptable);
        assert!(!should_auto_accept(&classification, true));
    }

    #[test]
    fn test_tos_browse_page_low_risk_auto_accepted() {
        let classification = classify_consent(
            "By continuing, you accept our Terms of Service and Privacy Policy.",
            &["I Agree", "Continue"],
            Some("https://example.com/articles"),
            None,
        );
        assert!(matches!(classification.risk, RiskLevel::LowRisk | RiskLevel::NoRisk));
        assert!(classification.auto_acceptable);
        assert!(should_auto_accept(&classification, true));
    }

    #[test]
    fn test_free_trial_medium_risk() {
        let classification = classify_consent(
            "Start your free trial today. After 14 days, you will be charged $9.99/month.",
            &["Start Trial"],
            Some("https://service.example.com/pricing"),
            None,
        );
        // "free trial" + "charged" → medium to high
        assert!(matches!(classification.risk, RiskLevel::MediumRisk | RiskLevel::HighRisk));
        assert!(!classification.auto_acceptable);
    }

    #[test]
    fn test_high_risk_url_boosts_score() {
        let classification = classify_consent(
            "Confirm your order details.",
            &["Confirm"],
            Some("https://shop.example.com/checkout/confirm"),
            None,
        );
        assert_eq!(classification.risk, RiskLevel::HighRisk);
    }

    #[test]
    fn test_no_risk_with_session_flag_off() {
        let classification = classify_consent(
            "We use cookies for analytics.",
            &["OK"],
            None,
            None,
        );
        assert!(classification.auto_acceptable);
        assert!(!should_auto_accept(&classification, false));
    }

    #[test]
    fn test_looks_like_consent_button() {
        assert!(looks_like_consent_button("Accept Cookies"));
        assert!(looks_like_consent_button("I Agree"));
        assert!(looks_like_consent_button("OK"));
        assert!(looks_like_consent_button("Got It"));
        assert!(looks_like_consent_button("Place Order"));
        assert!(!looks_like_consent_button("Next Page"));
        assert!(!looks_like_consent_button("Download"));
    }

    #[test]
    fn test_element_context_payment_bump() {
        // Even if dialog text is mild, payment fields bump to high
        let classification = classify_consent(
            "Please confirm your order.",
            &["Submit"],
            None,
            Some(&serde_json::json!({
                "nearby_fields": ["credit card number", "CVV", "expiration date"]
            })),
        );
        assert_eq!(classification.risk, RiskLevel::HighRisk);
    }

    #[test]
    fn test_auto_renew_high_risk() {
        let classification = classify_consent(
            "This subscription will auto-renew at $14.99/month. Non-refundable after activation.",
            &["Agree and Pay"],
            None,
            None,
        );
        assert_eq!(classification.risk, RiskLevel::HighRisk);
        assert!(!classification.auto_acceptable);
    }
}
