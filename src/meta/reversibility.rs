//! Action reversibility classification for Phase B interactions.
//! Separate from consent.rs (which handles dialog risk), this classifies
//! the reversibility of individual user actions on form elements.

use super::response::Reversibility;

/// Classify the reversibility of typing into a field.
/// Typing is always reversible — you can clear the field.
pub fn classify_type_action() -> Reversibility {
    Reversibility::Reversible
}

/// Classify the reversibility of a form submission.
/// Submit is RequiresConfirmation unless explicitly allowed.
pub fn classify_submit_action(allow_submit: bool) -> Reversibility {
    if allow_submit {
        Reversibility::Reversible // caller takes responsibility
    } else {
        Reversibility::RequiresConfirmation
    }
}

/// Classify the reversibility of a select/checkbox/radio change.
/// These are reversible — the old value can be restored.
pub fn classify_form_control_action() -> Reversibility {
    Reversibility::Reversible
}

/// Classify based on button text (delegates to targeting::classify_reversibility
/// but also handles submit-specific patterns).
pub fn classify_button_action(button_text: &str, auto_submit: bool) -> Reversibility {
    if auto_submit {
        // Caller explicitly allowed submission, but still check for destructive
        let base = super::targeting::classify_reversibility(button_text);
        if base == Reversibility::Destructive {
            Reversibility::Destructive // never auto-allow destructive
        } else {
            Reversibility::Reversible
        }
    } else {
        super::targeting::classify_reversibility(button_text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_always_reversible() {
        assert_eq!(classify_type_action(), Reversibility::Reversible);
    }

    #[test]
    fn test_submit_requires_confirmation_by_default() {
        assert_eq!(
            classify_submit_action(false),
            Reversibility::RequiresConfirmation
        );
    }

    #[test]
    fn test_submit_reversible_when_allowed() {
        assert_eq!(classify_submit_action(true), Reversibility::Reversible);
    }

    #[test]
    fn test_form_control_reversible() {
        assert_eq!(
            classify_form_control_action(),
            Reversibility::Reversible
        );
    }

    #[test]
    fn test_button_auto_submit_still_blocks_destructive() {
        assert_eq!(
            classify_button_action("Delete Account", true),
            Reversibility::Destructive
        );
    }

    #[test]
    fn test_button_auto_submit_allows_normal() {
        assert_eq!(
            classify_button_action("Submit", true),
            Reversibility::Reversible
        );
    }

    #[test]
    fn test_button_no_auto_submit_requires_confirmation() {
        assert_eq!(
            classify_button_action("Submit", false),
            Reversibility::RequiresConfirmation
        );
    }
}
