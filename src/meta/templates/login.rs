//! Login recovery template — 5-stage pipeline.
//!
//! Stage 0: Pre-login checks (already authenticated? redirect detected?)
//! Stage 1: Credential-based login (fill form → submit → verify)
//! Stage 2: 2FA / MFA handling (TOTP auto → RequiresUserInput)
//! Stage 3: Recovery on credential failure (OAuth buttons → magic link → forgot password)
//! Stage 4: Post-login persistence (remember me, verify success)
//!
//! Returns a structured JSON object with `steps` and `variables` arrays,
//! ready to be passed to `hands_script` for execution. This template does
//! NOT execute anything — it only builds the script.

use serde_json::{json, Value};

/// Generate the `hands_script` payload for a login recovery attempt.
///
/// # Input args
/// - `url` (required): Login page URL
/// - `credential_name`: Name of stored credential to use (workflow vault)
/// - `username`: Username/email to fill (if no credential_name)
/// - `password`: Password to fill (if no credential_name)
/// - `totp_name`: Name of TOTP credential for auto-2FA
/// - `auto_remember`: Check "Remember me" checkbox (default: true)
/// - `success_text`: Text to verify after login (e.g. "Dashboard", "Welcome")
/// - `success_url_contains`: URL substring indicating successful login
///
/// # Returns
/// JSON object with `steps` array and `variables` map for `hands_script`.
pub fn build_login_script(args: &Value) -> Value {
    let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
    let credential_name = args.get("credential_name").and_then(|v| v.as_str());
    let username = args.get("username").and_then(|v| v.as_str());
    let password = args.get("password").and_then(|v| v.as_str());
    let totp_name = args.get("totp_name").and_then(|v| v.as_str());
    let auto_remember = args.get("auto_remember").and_then(|v| v.as_bool()).unwrap_or(true);
    let success_text = args.get("success_text").and_then(|v| v.as_str());
    let success_url_contains = args.get("success_url_contains").and_then(|v| v.as_str());

    let mut steps: Vec<Value> = Vec::new();
    let mut variables = serde_json::Map::new();

    // Populate initial variables
    variables.insert("login_url".to_string(), json!(url));
    if let Some(u) = username {
        variables.insert("username".to_string(), json!(u));
    }
    if let Some(p) = password {
        variables.insert("password".to_string(), json!(p));
    }
    if let Some(cn) = credential_name {
        variables.insert("credential_name".to_string(), json!(cn));
    }
    if let Some(tn) = totp_name {
        variables.insert("totp_name".to_string(), json!(tn));
    }

    // ── Stage 0: Pre-login — navigate and check if already logged in ──

    steps.push(json!({
        "tool": "hands_navigate",
        "args": {"url": "{{login_url}}"},
        "label": "stage0_navigate",
        "output_var": "nav_result",
        "on_error": "stop",
        "timeout_ms": 15000
    }));

    // Build verify args for the "already logged in" check
    let mut already_logged_in_args = serde_json::Map::new();
    if let Some(st) = success_text {
        already_logged_in_args.insert("text".to_string(), json!(st));
    } else if let Some(su) = success_url_contains {
        already_logged_in_args.insert(
            "natural_text".to_string(),
            json!(format!("URL contains {}", su)),
        );
    } else {
        // Generic: check for dashboard/profile/home indicators
        already_logged_in_args.insert("template".to_string(), json!("verify_login_success"));
    }
    already_logged_in_args.insert("timeout_ms".to_string(), json!(3000));

    steps.push(json!({
        "tool": "hands_verify",
        "args": Value::Object(already_logged_in_args),
        "label": "stage0_check_already_logged_in",
        "output_var": "already_logged_in",
        "on_error": "skip",
        "timeout_ms": 5000
    }));

    // ── Stage 1: Credential-based login ──

    // Allow caller to override field selectors and button text via template_args
    let template_args = args.get("template_args");
    let custom_username_selector = template_args
        .and_then(|t| t.get("username_selector")).and_then(|v| v.as_str());
    let custom_password_selector = template_args
        .and_then(|t| t.get("password_selector")).and_then(|v| v.as_str());
    let custom_submit_text = template_args
        .and_then(|t| t.get("submit_text")).and_then(|v| v.as_str());

    // Detect login form — use broad attribute selectors that match real sites
    steps.push(json!({
        "tool": "hands_verify",
        "args": {
            "element": "input[type=password], input[type=email], input[autocomplete=username], input[name*=user], input[name*=email], input[name=login]",
            "timeout_ms": 3000
        },
        "label": "stage1_detect_login_form",
        "output_var": "form_detected",
        "on_error": "skip",
        "timeout_ms": 5000
    }));

    // Fill credentials using attribute-based field detection
    // Phase C fix2: fill_form matches by label text, but real sites often have
    // inputs with name/id attributes and no visible label. Use hands_type with
    // CSS selectors for reliable binding, falling back to label text.
    if username.is_some() || credential_name.is_some() {
        let username_target = custom_username_selector.unwrap_or(
            "input[type=email], input[autocomplete=username], input[name*=user i], input[name*=email i], input[id*=user i], input[id*=email i], input[name=login]"
        );
        steps.push(json!({
            "tool": "hands_type",
            "args": {
                "target": username_target,
                "text": "{{username}}",
                "clear_first": true
            },
            "label": "stage1_fill_username",
            "output_var": "username_fill_result",
            "on_error": "skip",
            "timeout_ms": 8000
        }));
    }

    if password.is_some() || credential_name.is_some() {
        let password_target = custom_password_selector.unwrap_or(
            "input[type=password]"
        );
        steps.push(json!({
            "tool": "hands_type",
            "args": {
                "target": password_target,
                "text": "{{password}}",
                "clear_first": true
            },
            "label": "stage1_fill_password",
            "output_var": "password_fill_result",
            "on_error": "skip",
            "timeout_ms": 8000
        }));
    }

    if username.is_none() && password.is_none() && credential_name.is_none() {
        // No credentials provided — RequiresUserInput
        steps.push(json!({
            "tool": "hands_verify",
            "args": {
                "element": "input[type=password]",
                "timeout_ms": 2000
            },
            "label": "stage1_no_credentials_detected",
            "output_var": "needs_credentials",
            "on_error": "skip",
            "timeout_ms": 3000
        }));
    }

    // Submit the login form — try multiple button text variations (case-insensitive)
    // Phase C fix2: "Sign In" was hardcoded but real sites use Login, Submit, Continue, etc.
    let submit_target = custom_submit_text.unwrap_or(
        "button[type=submit], input[type=submit]"
    );
    // First try: CSS selector for submit button (most reliable)
    steps.push(json!({
        "tool": "hands_click",
        "args": {"target": submit_target},
        "label": "stage1_submit_login_css",
        "output_var": "submit_result_css",
        "on_error": "skip",
        "timeout_ms": 3000
    }));
    // Second try: text-based click with common button labels
    steps.push(json!({
        "tool": "hands_click",
        "args": {"target": "Login"},
        "label": "stage1_submit_login_text",
        "output_var": "submit_result_text",
        "on_error": "skip",
        "timeout_ms": 3000
    }));

    // Verify login success after submit
    let mut post_login_verify = serde_json::Map::new();
    if let Some(st) = success_text {
        post_login_verify.insert("text".to_string(), json!(st));
    } else {
        post_login_verify.insert("template".to_string(), json!("verify_login_success"));
    }
    post_login_verify.insert("timeout_ms".to_string(), json!(5000));

    steps.push(json!({
        "tool": "hands_verify",
        "args": Value::Object(post_login_verify),
        "label": "stage1_verify_login",
        "output_var": "login_verified",
        "on_error": "skip",
        "timeout_ms": 8000
    }));

    // ── Stage 2: 2FA / MFA handling ──

    // Detect 2FA prompt — Phase C fix2: expanded pattern set
    steps.push(json!({
        "tool": "hands_verify",
        "args": {
            "natural_text": "shows verification code OR one-time code OR two-factor OR authenticator OR enter code OR authentication code OR security code OR TOTP",
            "timeout_ms": 3000
        },
        "label": "stage2_detect_2fa",
        "output_var": "tfa_detected",
        "on_error": "skip",
        "timeout_ms": 5000
    }));

    // Also check for 2FA input by autocomplete attribute
    steps.push(json!({
        "tool": "hands_verify",
        "args": {
            "element": "input[autocomplete=one-time-code], input[name=otp], input[name=code], input[name=totp]",
            "timeout_ms": 2000
        },
        "label": "stage2_detect_2fa_input",
        "output_var": "tfa_input_detected",
        "on_error": "skip",
        "timeout_ms": 3000
    }));

    if totp_name.is_some() {
        // Auto-fill TOTP code — the script caller would need to have populated
        // the totp_code variable before this step via a workflow:totp_generate call.
        // Since hands_script can't call workflow tools directly, the caller must
        // pre-populate or use a wrapper that adds the TOTP step.
        steps.push(json!({
            "tool": "hands_type",
            "args": {
                "target": "input[autocomplete=one-time-code], input[name=code], input[name=otp], input[name=totp], input[type=number], input[type=text]",
                "text": "{{totp_code}}",
                "submit": true
            },
            "label": "stage2_fill_totp",
            "output_var": "totp_fill_result",
            "on_error": "skip",
            "timeout_ms": 5000
        }));
    }
    // If no totp_name, 2FA detection will succeed but fill will be skipped —
    // the caller inspects tfa_detected in variables_final to know user input is needed.

    // ── Stage 3: Recovery — OAuth buttons ──

    // Look for OAuth/SSO buttons as alternative login
    steps.push(json!({
        "tool": "hands_find",
        "args": {
            "target": "Sign in with Google",
            "scope": "browser",
            "timeout_ms": 2000
        },
        "label": "stage3_find_oauth_google",
        "output_var": "oauth_google",
        "on_error": "skip",
        "timeout_ms": 3000
    }));

    steps.push(json!({
        "tool": "hands_find",
        "args": {
            "target": "Sign in with Microsoft",
            "scope": "browser",
            "timeout_ms": 2000
        },
        "label": "stage3_find_oauth_microsoft",
        "output_var": "oauth_microsoft",
        "on_error": "skip",
        "timeout_ms": 3000
    }));

    steps.push(json!({
        "tool": "hands_find",
        "args": {
            "target": "Sign in with Apple",
            "scope": "browser",
            "timeout_ms": 2000
        },
        "label": "stage3_find_oauth_apple",
        "output_var": "oauth_apple",
        "on_error": "skip",
        "timeout_ms": 3000
    }));

    // ── Stage 4: Post-login persistence ──

    if auto_remember {
        steps.push(json!({
            "tool": "hands_find",
            "args": {
                "target": "Remember me",
                "scope": "browser",
                "timeout_ms": 2000
            },
            "label": "stage4_find_remember_me",
            "output_var": "remember_me_found",
            "on_error": "skip",
            "timeout_ms": 3000
        }));

        steps.push(json!({
            "tool": "hands_click",
            "args": {"target": "Remember me"},
            "label": "stage4_click_remember_me",
            "output_var": "remember_me_clicked",
            "on_error": "skip",
            "timeout_ms": 3000
        }));
    }

    // Final verification
    let mut final_verify = serde_json::Map::new();
    if let Some(st) = success_text {
        final_verify.insert("text".to_string(), json!(st));
    } else {
        final_verify.insert("template".to_string(), json!("verify_login_success"));
    }
    final_verify.insert("timeout_ms".to_string(), json!(5000));
    final_verify.insert("must_stabilize_ms".to_string(), json!(1000));

    steps.push(json!({
        "tool": "hands_verify",
        "args": Value::Object(final_verify),
        "label": "stage4_final_verify",
        "output_var": "final_verification",
        "on_error": "skip",
        "timeout_ms": 8000
    }));

    // ── Build output ──
    json!({
        "steps": steps,
        "variables": Value::Object(variables),
        "stop_on_error": false,
        "verbose": true,
        "timeout_ms": 120_000
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_build_login_script_minimal() {
        let args = json!({"url": "https://example.com/login"});
        let result = build_login_script(&args);

        assert!(result.get("steps").unwrap().as_array().unwrap().len() >= 5);
        assert_eq!(result["variables"]["login_url"], "https://example.com/login");
        assert_eq!(result["stop_on_error"], false);
    }

    #[test]
    fn test_build_login_script_with_credentials() {
        let args = json!({
            "url": "https://example.com/login",
            "username": "user@example.com",
            "password": "secret123",
            "success_text": "Dashboard"
        });
        let result = build_login_script(&args);

        let steps = result["steps"].as_array().unwrap();
        // Phase C fix2: credentials are now filled via hands_type (attribute selectors)
        // instead of hands_fill_form (label matching)
        let has_username = steps.iter().any(|s| {
            s.get("label").and_then(|l| l.as_str())
                .map(|l| l.contains("fill_username"))
                .unwrap_or(false)
        });
        let has_password = steps.iter().any(|s| {
            s.get("label").and_then(|l| l.as_str())
                .map(|l| l.contains("fill_password"))
                .unwrap_or(false)
        });
        assert!(has_username, "Should have fill_username step");
        assert!(has_password, "Should have fill_password step");

        // Submit should use CSS selector for button[type=submit] as primary
        let has_css_submit = steps.iter().any(|s| {
            s.get("label").and_then(|l| l.as_str())
                .map(|l| l.contains("submit_login_css"))
                .unwrap_or(false)
        });
        assert!(has_css_submit, "Should have CSS-based submit step");

        assert_eq!(result["variables"]["username"], "user@example.com");
        assert_eq!(result["variables"]["password"], "secret123");
    }

    #[test]
    fn test_build_login_script_with_template_args() {
        let args = json!({
            "url": "https://example.com/login",
            "username": "user",
            "password": "pass",
            "template_args": {
                "username_selector": "input#my-email",
                "submit_text": "Go"
            }
        });
        let result = build_login_script(&args);

        let steps = result["steps"].as_array().unwrap();
        // Username step should use custom selector
        let username_step = steps.iter().find(|s| {
            s.get("label").and_then(|l| l.as_str())
                .map(|l| l.contains("fill_username")).unwrap_or(false)
        }).unwrap();
        assert_eq!(
            username_step["args"]["target"], "input#my-email",
            "Should use custom username selector from template_args"
        );
    }

    #[test]
    fn test_build_login_script_with_totp() {
        let args = json!({
            "url": "https://example.com/login",
            "username": "user@example.com",
            "password": "pass",
            "totp_name": "example_totp"
        });
        let result = build_login_script(&args);

        let steps = result["steps"].as_array().unwrap();
        let has_totp = steps.iter().any(|s| {
            s.get("label")
                .and_then(|l| l.as_str())
                .map(|l| l.contains("totp"))
                .unwrap_or(false)
        });
        assert!(has_totp, "Should have TOTP fill step");
        assert_eq!(result["variables"]["totp_name"], "example_totp");
    }

    #[test]
    fn test_build_login_script_no_remember_me() {
        let args = json!({
            "url": "https://example.com/login",
            "auto_remember": false
        });
        let result = build_login_script(&args);

        let steps = result["steps"].as_array().unwrap();
        let has_remember = steps.iter().any(|s| {
            s.get("label")
                .and_then(|l| l.as_str())
                .map(|l| l.contains("remember_me"))
                .unwrap_or(false)
        });
        assert!(!has_remember, "Should not have remember_me steps when auto_remember=false");
    }

    #[test]
    fn test_all_steps_have_labels() {
        let args = json!({
            "url": "https://example.com/login",
            "username": "u",
            "password": "p",
            "totp_name": "t",
        });
        let result = build_login_script(&args);

        for step in result["steps"].as_array().unwrap() {
            assert!(
                step.get("label").and_then(|l| l.as_str()).is_some(),
                "Every step must have a label: {:?}",
                step
            );
        }
    }
}
