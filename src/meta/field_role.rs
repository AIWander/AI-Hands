//! Field role detection — classifies form inputs by semantic type.
//! Used by hands_type and hands_fill_form for appropriate handling strategies.
//! Sensitive fields refuse fast_set and require keystroke simulation.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Semantic classification of a form field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldRole {
    Text,
    Password,
    Email,
    Phone,
    Url,
    Search,
    Number,
    Date,
    Select,
    Checkbox,
    Radio,
    File,
    Unknown,
}

impl FieldRole {
    /// Detect field role from element attributes.
    /// Checks: element role, input type, autocomplete, inputmode.
    pub fn detect(element: &Value) -> Self {
        // Check explicit role/type first
        let role = element.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let input_type = element
            .get("type")
            .or_else(|| element.get("input_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let autocomplete = element
            .get("autocomplete")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let inputmode = element
            .get("inputmode")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let tag = element.get("tag").and_then(|v| v.as_str()).unwrap_or("");

        // Select elements
        if tag.eq_ignore_ascii_case("select") || role == "listbox" || role == "combobox" {
            return Self::Select;
        }

        // Checkbox / Radio
        if input_type == "checkbox" || role == "checkbox" {
            return Self::Checkbox;
        }
        if input_type == "radio" || role == "radiogroup" || role == "radio" {
            return Self::Radio;
        }

        // File upload
        if input_type == "file" {
            return Self::File;
        }

        // Password
        if input_type == "password" || autocomplete == "current-password"
            || autocomplete == "new-password"
        {
            return Self::Password;
        }

        // Email
        if input_type == "email" || autocomplete == "email" || inputmode == "email" {
            return Self::Email;
        }

        // Phone
        if input_type == "tel" || autocomplete == "tel" || inputmode == "tel" {
            return Self::Phone;
        }

        // URL
        if input_type == "url" || autocomplete == "url" || inputmode == "url" {
            return Self::Url;
        }

        // Search
        if input_type == "search" || role == "searchbox" {
            return Self::Search;
        }

        // Number — includes credit card fields detected by autocomplete
        if input_type == "number" || inputmode == "numeric" || inputmode == "decimal"
            || autocomplete.starts_with("cc-")
        {
            return Self::Number;
        }

        // Date variants
        if input_type == "date" || input_type == "datetime-local"
            || input_type == "month" || input_type == "week" || input_type == "time"
            || autocomplete == "bday"
        {
            return Self::Date;
        }

        // Text (explicit or default for input/textarea)
        if input_type == "text" || input_type.is_empty()
            || tag.eq_ignore_ascii_case("textarea")
            || role == "textbox"
        {
            return Self::Text;
        }

        Self::Unknown
    }

    /// Whether this field type is sensitive and requires extra care.
    /// Sensitive fields: refuse fast_set, always use per-keystroke simulation,
    /// never log values in instrumentation.
    pub fn is_sensitive(&self) -> bool {
        matches!(self, Self::Password | Self::Email | Self::Phone | Self::Number)
    }

    /// Whether this field type requires keystroke simulation (no JS direct-set).
    /// Masked/formatted inputs need keystroke to trigger formatters.
    pub fn requires_keystroke(&self) -> bool {
        matches!(self, Self::Password | Self::Phone | Self::Number)
    }

    /// Whether this field supports text input at all.
    pub fn is_text_input(&self) -> bool {
        matches!(
            self,
            Self::Text | Self::Password | Self::Email | Self::Phone
                | Self::Url | Self::Search | Self::Number | Self::Date
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_detect_password() {
        assert_eq!(
            FieldRole::detect(&json!({"type": "password"})),
            FieldRole::Password
        );
        assert_eq!(
            FieldRole::detect(&json!({"autocomplete": "current-password", "type": "text"})),
            FieldRole::Password
        );
    }

    #[test]
    fn test_detect_email() {
        assert_eq!(
            FieldRole::detect(&json!({"type": "email"})),
            FieldRole::Email
        );
        assert_eq!(
            FieldRole::detect(&json!({"inputmode": "email", "type": "text"})),
            FieldRole::Email
        );
    }

    #[test]
    fn test_detect_select() {
        assert_eq!(
            FieldRole::detect(&json!({"tag": "select"})),
            FieldRole::Select
        );
        assert_eq!(
            FieldRole::detect(&json!({"role": "combobox"})),
            FieldRole::Select
        );
    }

    #[test]
    fn test_detect_checkbox_radio() {
        assert_eq!(
            FieldRole::detect(&json!({"type": "checkbox"})),
            FieldRole::Checkbox
        );
        assert_eq!(
            FieldRole::detect(&json!({"type": "radio"})),
            FieldRole::Radio
        );
    }

    #[test]
    fn test_sensitive_fields() {
        assert!(FieldRole::Password.is_sensitive());
        assert!(FieldRole::Email.is_sensitive());
        assert!(FieldRole::Phone.is_sensitive());
        assert!(FieldRole::Number.is_sensitive());
        assert!(!FieldRole::Text.is_sensitive());
        assert!(!FieldRole::Search.is_sensitive());
    }

    #[test]
    fn test_requires_keystroke() {
        assert!(FieldRole::Password.requires_keystroke());
        assert!(FieldRole::Phone.requires_keystroke());
        assert!(!FieldRole::Text.requires_keystroke());
        assert!(!FieldRole::Email.requires_keystroke());
    }

    #[test]
    fn test_credit_card_autocomplete() {
        assert_eq!(
            FieldRole::detect(&json!({"autocomplete": "cc-number", "type": "text"})),
            FieldRole::Number
        );
    }

    #[test]
    fn test_default_text() {
        assert_eq!(
            FieldRole::detect(&json!({"type": "text"})),
            FieldRole::Text
        );
        assert_eq!(
            FieldRole::detect(&json!({"tag": "textarea"})),
            FieldRole::Text
        );
    }
}
