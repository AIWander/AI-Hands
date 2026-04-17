//! Verify template library — named bundles of structured verification checks.
//! Each template maps to a function that returns TemplateExpansion with checks + config.
//!
//! Templates encode common verification patterns so Claude doesn't need to
//! manually compose multi-check sequences for standard post-action verification.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Template-expanded verification config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateExpansion {
    pub checks: Vec<TemplateCheck>,
    pub description: String,
    pub default_timeout_ms: u64,
    pub requires_browser: bool,
}

/// A single check within a template expansion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateCheck {
    /// Check type: "url_changed", "element_absent", "element_present",
    /// "text_present", "loading_absent", "network_idle", "text_absent"
    pub check_type: String,
    /// Primary target (selector, text, or URL pattern)
    pub target: Option<String>,
    /// For url_changed: the original URL to compare against
    pub from: Option<String>,
    /// Multiple patterns to check (any match = pass for this check)
    pub patterns: Vec<String>,
    /// Must pass for template to succeed
    pub required: bool,
}

/// All available template names.
const TEMPLATES: &[&str] = &[
    "verify_page_loaded",
    "verify_login_success",
    "verify_form_submitted",
    "verify_error_displayed",
    "verify_modal_present",
    "verify_navigation_completed",
];

/// List all available template names.
pub fn list_templates() -> Vec<&'static str> {
    TEMPLATES.to_vec()
}

/// Resolve a named template into a TemplateExpansion.
/// `args` can contain context-specific overrides (e.g., `from_url`, `target_url`, `success_text`).
pub fn resolve_template(name: &str, args: &Value) -> Result<TemplateExpansion, String> {
    match name {
        "verify_page_loaded" => Ok(template_page_loaded(args)),
        "verify_login_success" => Ok(template_login_success(args)),
        "verify_form_submitted" => Ok(template_form_submitted(args)),
        "verify_error_displayed" => Ok(template_error_displayed(args)),
        "verify_modal_present" => Ok(template_modal_present(args)),
        "verify_navigation_completed" => Ok(template_navigation_completed(args)),
        _ => {
            let available = TEMPLATES.join(", ");
            Err(format!(
                "Unknown template '{}'. Available: {}",
                name, available
            ))
        }
    }
}

// ── Template implementations ──

fn template_page_loaded(_args: &Value) -> TemplateExpansion {
    TemplateExpansion {
        description: "Verify page loaded: URL settled, body non-trivial, no loading indicators"
            .into(),
        default_timeout_ms: 10000,
        requires_browser: true,
        checks: vec![
            TemplateCheck {
                check_type: "text_present".into(),
                target: None,
                from: None,
                patterns: vec![],
                required: true,
            },
            TemplateCheck {
                check_type: "loading_absent".into(),
                target: None,
                from: None,
                patterns: vec![
                    "Loading...".into(),
                    "Please wait".into(),
                    "Loading".into(),
                    "Spinner".into(),
                ],
                required: true,
            },
            TemplateCheck {
                check_type: "element_absent".into(),
                target: Some("[class*='loading'], [class*='spinner'], [class*='skeleton']".into()),
                from: None,
                patterns: vec![],
                required: false,
            },
        ],
    }
}

fn template_login_success(args: &Value) -> TemplateExpansion {
    let from_url = args
        .get("from_url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    TemplateExpansion {
        description: "Verify login success: URL changed from login, login form absent, dashboard indicator present".into(),
        default_timeout_ms: 15000,
        requires_browser: true,
        checks: vec![
            TemplateCheck {
                check_type: "url_changed".into(),
                target: None,
                from: if from_url.is_empty() { None } else { Some(from_url) },
                patterns: vec![],
                required: true,
            },
            TemplateCheck {
                check_type: "element_absent".into(),
                target: Some("form".into()),
                from: None,
                patterns: vec![
                    "[type='password']".into(),
                    "input[name='password']".into(),
                    "#password".into(),
                ],
                required: true,
            },
            TemplateCheck {
                check_type: "element_present".into(),
                target: None,
                from: None,
                patterns: vec![
                    "[class*='dashboard']".into(),
                    "[class*='home']".into(),
                    "[class*='profile']".into(),
                    "[class*='account']".into(),
                    "nav".into(),
                ],
                required: false,
            },
        ],
    }
}

fn template_form_submitted(args: &Value) -> TemplateExpansion {
    let from_url = args
        .get("from_url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let success_text = args
        .get("success_text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut checks = vec![
        TemplateCheck {
            check_type: "url_changed".into(),
            target: None,
            from: if from_url.is_empty() {
                None
            } else {
                Some(from_url)
            },
            patterns: vec![],
            required: false,
        },
        TemplateCheck {
            check_type: "element_absent".into(),
            target: None,
            from: None,
            patterns: vec!["form[data-submitted]".into()],
            required: false,
        },
    ];

    if !success_text.is_empty() {
        checks.push(TemplateCheck {
            check_type: "text_present".into(),
            target: Some(success_text),
            from: None,
            patterns: vec![],
            required: false,
        });
    } else {
        checks.push(TemplateCheck {
            check_type: "text_present".into(),
            target: None,
            from: None,
            patterns: vec![
                "Success".into(),
                "Thank you".into(),
                "Submitted".into(),
                "Confirmed".into(),
                "Complete".into(),
            ],
            required: false,
        });
    }

    TemplateExpansion {
        description:
            "Verify form submitted: URL changed OR success element OR original form absent".into(),
        default_timeout_ms: 10000,
        requires_browser: true,
        checks,
    }
}

fn template_error_displayed(_args: &Value) -> TemplateExpansion {
    TemplateExpansion {
        description:
            "Verify error displayed: role=alert OR .error class OR common error text patterns"
                .into(),
        default_timeout_ms: 5000,
        requires_browser: true,
        checks: vec![
            TemplateCheck {
                check_type: "element_present".into(),
                target: None,
                from: None,
                patterns: vec![
                    "[role='alert']".into(),
                    "[role='alertdialog']".into(),
                    "[class*='error']".into(),
                    "[class*='danger']".into(),
                    "[class*='alert-error']".into(),
                    ".toast-error".into(),
                ],
                required: false,
            },
            TemplateCheck {
                check_type: "text_present".into(),
                target: None,
                from: None,
                patterns: vec![
                    "Error".into(),
                    "Failed".into(),
                    "Invalid".into(),
                    "Something went wrong".into(),
                    "Please try again".into(),
                    "Unable to".into(),
                ],
                required: false,
            },
        ],
    }
}

fn template_modal_present(_args: &Value) -> TemplateExpansion {
    TemplateExpansion {
        description: "Verify modal present: role=dialog OR modal class + focus within".into(),
        default_timeout_ms: 5000,
        requires_browser: true,
        checks: vec![TemplateCheck {
            check_type: "element_present".into(),
            target: None,
            from: None,
            patterns: vec![
                "[role='dialog']".into(),
                "[role='alertdialog']".into(),
                "[class*='modal']".into(),
                "[class*='dialog']".into(),
                "[class*='overlay']".into(),
                ".modal.show".into(),
                ".modal.is-active".into(),
            ],
            required: true,
        }],
    }
}

fn template_navigation_completed(args: &Value) -> TemplateExpansion {
    let target_url = args
        .get("target_url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut checks = vec![];

    if !target_url.is_empty() {
        checks.push(TemplateCheck {
            check_type: "url_changed".into(),
            target: Some(target_url),
            from: None,
            patterns: vec![],
            required: true,
        });
    }

    checks.push(TemplateCheck {
        check_type: "network_idle".into(),
        target: None,
        from: None,
        patterns: vec![],
        required: false,
    });

    checks.push(TemplateCheck {
        check_type: "loading_absent".into(),
        target: None,
        from: None,
        patterns: vec![
            "Loading...".into(),
            "Please wait".into(),
            "Navigating".into(),
        ],
        required: true,
    });

    TemplateExpansion {
        description: "Verify navigation completed: URL matches pattern + network idle + no loading indicators".into(),
        default_timeout_ms: 15000,
        requires_browser: true,
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_page_loaded_resolves() {
        let expansion = resolve_template("verify_page_loaded", &json!({})).unwrap();
        assert!(expansion.requires_browser);
        assert!(expansion.default_timeout_ms >= 5000);
        assert!(!expansion.checks.is_empty());
        // Must have a loading_absent check
        assert!(expansion
            .checks
            .iter()
            .any(|c| c.check_type == "loading_absent"));
    }

    #[test]
    fn test_login_success_resolves() {
        let expansion = resolve_template(
            "verify_login_success",
            &json!({"from_url": "https://example.com/login"}),
        )
        .unwrap();
        assert!(expansion.requires_browser);
        // Must have url_changed check with from set
        let url_check = expansion
            .checks
            .iter()
            .find(|c| c.check_type == "url_changed")
            .unwrap();
        assert_eq!(url_check.from.as_deref(), Some("https://example.com/login"));
        // Must have element_absent check for password fields
        assert!(expansion
            .checks
            .iter()
            .any(|c| c.check_type == "element_absent"));
    }

    #[test]
    fn test_form_submitted_resolves() {
        let expansion = resolve_template("verify_form_submitted", &json!({})).unwrap();
        assert!(!expansion.checks.is_empty());
        // Should have text_present with default success patterns
        let text_check = expansion
            .checks
            .iter()
            .find(|c| c.check_type == "text_present")
            .unwrap();
        assert!(!text_check.patterns.is_empty());
    }

    #[test]
    fn test_form_submitted_custom_text() {
        let expansion = resolve_template(
            "verify_form_submitted",
            &json!({"success_text": "Order Placed"}),
        )
        .unwrap();
        let text_check = expansion
            .checks
            .iter()
            .find(|c| c.check_type == "text_present")
            .unwrap();
        assert_eq!(text_check.target.as_deref(), Some("Order Placed"));
    }

    #[test]
    fn test_error_displayed_resolves() {
        let expansion = resolve_template("verify_error_displayed", &json!({})).unwrap();
        // Should check for role=alert and error text
        assert!(expansion
            .checks
            .iter()
            .any(|c| c.check_type == "element_present"));
        assert!(expansion
            .checks
            .iter()
            .any(|c| c.check_type == "text_present"));
    }

    #[test]
    fn test_modal_present_resolves() {
        let expansion = resolve_template("verify_modal_present", &json!({})).unwrap();
        let elem_check = expansion
            .checks
            .iter()
            .find(|c| c.check_type == "element_present")
            .unwrap();
        assert!(elem_check.patterns.iter().any(|p| p.contains("dialog")));
        assert!(elem_check.required);
    }

    #[test]
    fn test_navigation_completed_resolves() {
        let expansion = resolve_template(
            "verify_navigation_completed",
            &json!({"target_url": "https://example.com/dashboard"}),
        )
        .unwrap();
        let url_check = expansion
            .checks
            .iter()
            .find(|c| c.check_type == "url_changed")
            .unwrap();
        assert_eq!(
            url_check.target.as_deref(),
            Some("https://example.com/dashboard")
        );
        assert!(url_check.required);
    }

    #[test]
    fn test_unknown_template_returns_error() {
        let result = resolve_template("verify_nonexistent", &json!({}));
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("Unknown template"));
        assert!(msg.contains("verify_page_loaded"));
        assert!(msg.contains("verify_login_success"));
    }

    #[test]
    fn test_list_templates_returns_all() {
        let templates = list_templates();
        assert_eq!(templates.len(), 6);
        assert!(templates.contains(&"verify_page_loaded"));
        assert!(templates.contains(&"verify_login_success"));
        assert!(templates.contains(&"verify_form_submitted"));
        assert!(templates.contains(&"verify_error_displayed"));
        assert!(templates.contains(&"verify_modal_present"));
        assert!(templates.contains(&"verify_navigation_completed"));
    }
}
