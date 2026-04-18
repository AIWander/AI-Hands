#![allow(dead_code)] // scaffolded module, awaiting integration
//! Session state — thread-safe shared state for meta-tools.
//! Lives for the session, cleared on Claude restart.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Per-window monitor tracking for stickiness.
#[derive(Debug, Clone)]
pub struct MonitorRecord {
    pub monitor_index: i32,
    pub dpi: f64,
    pub last_bounds: Option<(i32, i32, i32, i32)>, // x, y, w, h
}

/// Subsystem health status from startup probes.
#[derive(Debug, Clone, PartialEq)]
pub enum SubsystemStatus {
    Available,
    Unavailable { reason: String },
    Unknown,
}

/// Health probe results for all subsystems.
#[derive(Debug, Clone)]
pub struct SubsystemHealth {
    pub browser: SubsystemStatus,
    pub uia: SubsystemStatus,
    pub vision: SubsystemStatus,
    pub ntp_drift_ms: Option<i64>,
}

impl Default for SubsystemHealth {
    fn default() -> Self {
        Self {
            browser: SubsystemStatus::Unknown,
            uia: SubsystemStatus::Unknown,
            vision: SubsystemStatus::Unknown,
            ntp_drift_ms: None,
        }
    }
}

/// Session-level state shared across all meta-tool calls.
#[derive(Debug)]
#[derive(Default)]
pub struct SessionState {
    /// Per-session user opt-in for low-risk auto-accept (Phase C consent).
    pub auto_accept_low_risk: bool,

    /// Window → monitor mapping for stickiness.
    pub window_monitors: HashMap<String, MonitorRecord>,

    /// Cached subsystem health from startup probes.
    pub subsystem_health: SubsystemHealth,

    /// A11y cache dirty flag — set by mutation observer JS callback.
    pub a11y_cache_dirty: bool,

    /// Last a11y content hash for backstop invalidation.
    pub a11y_content_hash: Option<u64>,

    /// Active tool context (for instrumentation — which tool is currently running).
    pub active_tool: Option<String>,

    /// Call counter for unique call_id generation.
    pub call_counter: u64,
}


impl SessionState {
    /// Check if a subsystem is available.
    pub fn subsystem_available(&self, name: &str) -> Result<(), super::error::MetaError> {
        let status = match name {
            "browser" => &self.subsystem_health.browser,
            "uia" => &self.subsystem_health.uia,
            "vision" => &self.subsystem_health.vision,
            _ => return Ok(()), // unknown subsystems assumed available
        };
        match status {
            SubsystemStatus::Available | SubsystemStatus::Unknown => Ok(()),
            SubsystemStatus::Unavailable { reason } => {
                Err(super::error::MetaError::subsystem(name, reason.clone()))
            }
        }
    }

    /// Record a window's monitor assignment.
    pub fn record_window_monitor(&mut self, window_id: &str, monitor_index: i32, dpi: f64) {
        self.window_monitors.insert(
            window_id.to_string(),
            MonitorRecord {
                monitor_index,
                dpi,
                last_bounds: None,
            },
        );
    }

    /// Get the owning monitor for a window.
    pub fn get_window_monitor(&self, window_id: &str) -> Option<&MonitorRecord> {
        self.window_monitors.get(window_id)
    }

    /// Generate a unique call ID for instrumentation.
    pub fn next_call_id(&mut self) -> String {
        self.call_counter += 1;
        format!("call_{:06}", self.call_counter)
    }

    /// Mark a11y cache as dirty (called from mutation observer callback).
    pub fn mark_a11y_dirty(&mut self) {
        self.a11y_cache_dirty = true;
    }

    /// Check and clear dirty flag. Returns true if cache was dirty.
    pub fn check_and_clear_a11y_dirty(&mut self) -> bool {
        let was_dirty = self.a11y_cache_dirty;
        self.a11y_cache_dirty = false;
        was_dirty
    }
}

/// Thread-safe session handle — cloneable, passed to all meta-tools.
pub type SharedSession = Arc<RwLock<SessionState>>;

/// Create a new shared session.
pub fn new_session() -> SharedSession {
    Arc::new(RwLock::new(SessionState::default()))
}
