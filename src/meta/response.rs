//! MetaToolResult envelope — standardized response for all meta-tools.
//! Per spec: success, method, rungs_tried, confidence, reversibility, elapsed_ms, warnings, result, error.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Primary response envelope for every meta-tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaToolResult {
    pub success: bool,
    /// Rung that succeeded (empty string on failure)
    pub method: String,
    /// Full ladder history — every rung attempted
    pub rungs_tried: Vec<RungAttempt>,
    /// Dual-model confidence (method-based + optional location-based)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    /// Reversibility classification of the action taken
    pub reversibility: Reversibility,
    /// Total wall-clock time for the meta-tool call
    pub elapsed_ms: u64,
    /// Non-fatal issues encountered during execution
    pub warnings: Vec<String>,
    /// Tool-specific payload (content, screenshot data, click result, etc.)
    pub result: Value,
    /// Error details on failure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<super::error::MetaError>,
}

/// Record of a single rung attempt within a ladder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RungAttempt {
    pub name: String,
    pub success: bool,
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Distinct from other failures — triggers adaptive timeout widening
    pub timed_out: bool,
}

/// Dual-scale confidence model per spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Confidence {
    /// How the element was found (a11y exact=1.0, partial=0.8, OCR=0.5-0.7, coord-only=0.4)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<f32>,
    /// Where in content it was found (heading=1.0, body=0.7, nav/footer=0.4, hidden=0.2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<f32>,
}

/// Reversibility classification for actions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Reversibility {
    /// back button, clear field, undo available
    Reversible,
    /// submit, close-with-unsaved
    RequiresConfirmation,
    /// delete, payment, publish
    Destructive,
}

/// Certainty level for ambiguous results (Phase B/C forward-compat).
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "level", rename_all = "snake_case")]
pub enum Certainty {
    Certain,
    Likely {
        reasoning: String,
    },
    Unclear {
        options: Vec<String>,
        reasoning: String,
    },
}

impl MetaToolResult {
    /// Create a successful result.
    pub fn success(
        method: impl Into<String>,
        rungs_tried: Vec<RungAttempt>,
        result: Value,
        elapsed_ms: u64,
    ) -> Self {
        Self {
            success: true,
            method: method.into(),
            rungs_tried,
            confidence: None,
            reversibility: Reversibility::Reversible,
            elapsed_ms,
            warnings: Vec::new(),
            result,
            error: None,
        }
    }

    /// Create a failed result.
    pub fn failure(
        rungs_tried: Vec<RungAttempt>,
        error: super::error::MetaError,
        elapsed_ms: u64,
    ) -> Self {
        Self {
            success: false,
            method: String::new(),
            rungs_tried,
            confidence: None,
            reversibility: Reversibility::Reversible,
            elapsed_ms,
            warnings: Vec::new(),
            result: Value::Null,
            error: Some(error),
        }
    }

    /// Set confidence on the result.
    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = Some(confidence);
        self
    }

    /// Set reversibility.
    pub fn with_reversibility(mut self, rev: Reversibility) -> Self {
        self.reversibility = rev;
        self
    }

    /// Add a warning.
    pub fn with_warning(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }

    /// Convert to JSON Value for MCP response.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|e| {
            serde_json::json!({
                "success": false,
                "error": format!("Failed to serialize MetaToolResult: {}", e)
            })
        })
    }
}

impl RungAttempt {
    pub fn ok(name: impl Into<String>, elapsed_ms: u64) -> Self {
        Self {
            name: name.into(),
            success: true,
            elapsed_ms,
            error: None,
            timed_out: false,
        }
    }

    pub fn failed(name: impl Into<String>, elapsed_ms: u64, error: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            success: false,
            elapsed_ms,
            error: Some(error.into()),
            timed_out: false,
        }
    }

    pub fn timed_out(name: impl Into<String>, elapsed_ms: u64) -> Self {
        Self {
            name: name.into(),
            success: false,
            elapsed_ms,
            error: Some("Timed out".into()),
            timed_out: true,
        }
    }
}

impl Confidence {
    pub fn method_only(score: f32) -> Self {
        Self {
            method: Some(score),
            location: None,
        }
    }

    pub fn location_only(score: f32) -> Self {
        Self {
            method: None,
            location: Some(score),
        }
    }

    pub fn dual(method: f32, location: f32) -> Self {
        Self {
            method: Some(method),
            location: Some(location),
        }
    }
}

/// Adaptive timeout multiplier based on initial timeout tightness.
/// Tighter first tries get more widening headroom.
pub fn adaptive_timeout_multiplier(initial_ms: u64) -> u64 {
    match initial_ms {
        0..=500 => 4,
        501..=2000 => 3,
        2001..=5000 => 2,
        _ => 1, // >5s: no widening, already generous
    }
}
