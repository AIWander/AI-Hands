//! MetaError enum — locked error taxonomy for all phases (A/B/C).
//! Adding variants later is non-breaking. Renaming is breaking.

use serde::{Deserialize, Serialize};

/// Match candidate info for MultipleMatches error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchCandidate {
    pub text: String,
    pub role: Option<String>,
    pub selector: Option<String>,
    pub confidence: f32,
}

/// Window info for MultipleWindows error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub title: String,
    pub process: Option<String>,
    pub hwnd: Option<u64>,
}

/// Locked error taxonomy — all Phase A/B/C variants defined now for forward compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "category", content = "detail", rename_all = "snake_case")]
pub enum MetaError {
    // ── Targeting ──
    ElementNotFound {
        target: String,
        scope: String,
    },
    MultipleMatches {
        target: String,
        candidates: Vec<MatchCandidate>,
    },
    RoleMismatch {
        expected: String,
        found: String,
    },
    NotInteractable {
        reason: String,
    },

    // ── State ──
    StaleRef {
        ref_id: String,
    },
    FocusLost {
        expected: String,
        actual: String,
    },
    DynamicContentChanged,

    // ── Infrastructure ──
    BrowserNotRunning,
    SubsystemUnavailable {
        subsystem: String,
        reason: String,
    },
    NoPage,
    Timeout {
        operation: String,
        elapsed_ms: u64,
    },

    // ── Content ──
    VerificationFailed {
        evidence: String,
        confidence: f32,
    },
    RequiresConfirmation {
        action: String,
        reason: String,
    },
    InsufficientContent {
        chars: usize,
        reason: String,
    },

    // ── Windows ──
    MultipleWindows {
        app: String,
        candidates: Vec<WindowInfo>,
    },
    DialogBlocking {
        dialog_title: String,
        buttons: Vec<String>,
    },

    // ── Script (Phase C) ──
    ScriptStepFailed {
        step_index: usize,
        step_label: Option<String>,
        inner: Box<MetaError>,
    },

    // ── Input (Phase B) ──
    RequiresUserInput {
        needs: String,
        field_label: Option<String>,
        field_type: Option<String>,
        reason: String,
    },

    // ── Catch-all ──
    Other {
        message: String,
    },
}

impl MetaError {
    /// Convenience: element not found in a scope.
    pub fn not_found(target: impl Into<String>, scope: impl Into<String>) -> Self {
        Self::ElementNotFound {
            target: target.into(),
            scope: scope.into(),
        }
    }

    /// Convenience: subsystem unavailable.
    pub fn subsystem(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::SubsystemUnavailable {
            subsystem: name.into(),
            reason: reason.into(),
        }
    }

    /// Convenience: timeout.
    pub fn timeout(operation: impl Into<String>, elapsed_ms: u64) -> Self {
        Self::Timeout {
            operation: operation.into(),
            elapsed_ms,
        }
    }

    /// Convenience: other/generic error.
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other {
            message: msg.into(),
        }
    }

    /// Convenience: browser not running.
    pub fn no_browser() -> Self {
        Self::BrowserNotRunning
    }

    /// Convenience: no page loaded.
    pub fn no_page() -> Self {
        Self::NoPage
    }

    /// Convenience: requires confirmation for irreversible action.
    pub fn requires_confirmation(action: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::RequiresConfirmation {
            action: action.into(),
            reason: reason.into(),
        }
    }

    /// Get the error category as a string.
    pub fn category(&self) -> &'static str {
        match self {
            Self::ElementNotFound { .. }
            | Self::MultipleMatches { .. }
            | Self::RoleMismatch { .. }
            | Self::NotInteractable { .. } => "targeting",

            Self::StaleRef { .. } | Self::FocusLost { .. } | Self::DynamicContentChanged => "state",

            Self::BrowserNotRunning
            | Self::SubsystemUnavailable { .. }
            | Self::NoPage
            | Self::Timeout { .. } => "infrastructure",

            Self::VerificationFailed { .. }
            | Self::RequiresConfirmation { .. }
            | Self::InsufficientContent { .. } => "content",

            Self::MultipleWindows { .. } | Self::DialogBlocking { .. } => "windows",

            Self::ScriptStepFailed { .. } => "script",

            Self::RequiresUserInput { .. } => "input",

            Self::Other { .. } => "other",
        }
    }
}

impl std::fmt::Display for MetaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ElementNotFound { target, scope } => {
                write!(f, "Element '{}' not found in {}", target, scope)
            }
            Self::MultipleMatches { target, candidates } => {
                write!(
                    f,
                    "Multiple matches for '{}': {} candidates",
                    target,
                    candidates.len()
                )
            }
            Self::RoleMismatch { expected, found } => {
                write!(
                    f,
                    "Role mismatch: expected '{}', found '{}'",
                    expected, found
                )
            }
            Self::NotInteractable { reason } => {
                write!(f, "Element not interactable: {}", reason)
            }
            Self::StaleRef { ref_id } => {
                write!(f, "Stale ref: {}", ref_id)
            }
            Self::FocusLost { expected, actual } => {
                write!(f, "Focus lost: expected '{}', got '{}'", expected, actual)
            }
            Self::DynamicContentChanged => write!(f, "Dynamic content changed mid-operation"),
            Self::BrowserNotRunning => write!(f, "Browser not running"),
            Self::SubsystemUnavailable { subsystem, reason } => {
                write!(f, "Subsystem '{}' unavailable: {}", subsystem, reason)
            }
            Self::NoPage => write!(f, "No page loaded in browser"),
            Self::Timeout {
                operation,
                elapsed_ms,
            } => {
                write!(f, "Timeout on '{}' after {}ms", operation, elapsed_ms)
            }
            Self::VerificationFailed {
                evidence,
                confidence,
            } => {
                write!(
                    f,
                    "Verification failed (confidence {}): {}",
                    confidence, evidence
                )
            }
            Self::RequiresConfirmation { action, reason } => {
                write!(f, "Requires confirmation for '{}': {}", action, reason)
            }
            Self::InsufficientContent { chars, reason } => {
                write!(f, "Insufficient content ({} chars): {}", chars, reason)
            }
            Self::MultipleWindows { app, candidates } => {
                write!(
                    f,
                    "Multiple windows for '{}': {} found",
                    app,
                    candidates.len()
                )
            }
            Self::DialogBlocking { dialog_title, .. } => {
                write!(f, "Dialog blocking: '{}'", dialog_title)
            }
            Self::ScriptStepFailed {
                step_index,
                step_label,
                inner,
            } => {
                write!(
                    f,
                    "Script step {} ({}) failed: {}",
                    step_index,
                    step_label.as_deref().unwrap_or("unnamed"),
                    inner
                )
            }
            Self::RequiresUserInput { needs, reason, .. } => {
                write!(f, "Requires user input ({}): {}", needs, reason)
            }
            Self::Other { message } => write!(f, "{}", message),
        }
    }
}

impl std::error::Error for MetaError {}
