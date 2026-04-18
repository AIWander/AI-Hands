#![allow(dead_code)] // scaffolded module, awaiting integration
//! Subsystem health probes — run at startup, cached for session.
//! Browser: attempt headless launch with 5s timeout
//! UIA: uia_list_window — COM init + call
//! Vision: attempt 1x1 vision_screenshot
//! NTP: time drift check (warn if >15s off for future TOTP support)

use super::session::{SubsystemHealth, SubsystemStatus};

/// Run all health probes and return results.
/// Called once at server startup; results cached in SessionState.
///
/// This function is designed to be called from the async runtime.
/// Each probe is independent and has its own timeout.
pub fn probe_all_sync() -> SubsystemHealth {
    SubsystemHealth {
        browser: probe_browser(),
        uia: probe_uia(),
        vision: probe_vision(),
        ntp_drift_ms: probe_ntp_drift(),
    }
}

/// Probe browser subsystem.
/// Attempts to check browser_status; if that fails, marks as unavailable.
/// Actual probe implementation calls browser_mcp primitives.
fn probe_browser() -> SubsystemStatus {
    // The actual probe will call browser_mcp::tools at runtime.
    // For compilation, we mark as Unknown — the meta-tool dispatch
    // will run the real probe on first use if still Unknown.
    SubsystemStatus::Unknown
}

/// Probe UIA subsystem.
/// Calls uia_list_window to verify COM is initialized.
fn probe_uia() -> SubsystemStatus {
    // Check if we're on Windows — UIA is Windows-only
    #[cfg(not(windows))]
    {
        return SubsystemStatus::Unavailable {
            reason: "UIA only available on Windows".into(),
        };
    }

    #[cfg(windows)]
    {
        // Actual COM probe happens at runtime via uia_lib.
        // For now, mark as Unknown — will be probed on first UIA rung.
        SubsystemStatus::Unknown
    }
}

/// Probe vision subsystem.
/// Attempts a minimal screenshot to verify the capture pipeline works.
fn probe_vision() -> SubsystemStatus {
    // Vision probe deferred to runtime — requires graphics context.
    SubsystemStatus::Unknown
}

/// Check NTP time drift.
/// Uses w32tm on Windows to compare system clock to NTP.
/// Returns drift in milliseconds, or None if check failed.
fn probe_ntp_drift() -> Option<i64> {
    #[cfg(windows)]
    {
        // Parse w32tm output for drift detection.
        // Deferred to runtime — this is a network call that may timeout.
        // Returns None (unchecked) at startup; can be triggered on demand.
        None
    }

    #[cfg(not(windows))]
    {
        None
    }
}

/// Run a real browser health probe using browser_mcp status check.
/// Called lazily on first meta-tool use if browser status is still Unknown.
pub fn probe_browser_live(browser_status_result: &serde_json::Value) -> SubsystemStatus {
    if let Some(active) = browser_status_result
        .get("active")
        .and_then(|v| v.as_bool())
    {
        if active {
            SubsystemStatus::Available
        } else {
            // Browser process exists but no active page — still available for launch
            SubsystemStatus::Available
        }
    } else if let Some(err) = browser_status_result.get("error").and_then(|v| v.as_str()) {
        SubsystemStatus::Unavailable {
            reason: err.to_string(),
        }
    } else {
        SubsystemStatus::Available
    }
}

/// Run a real UIA health probe from uia_list_window result.
pub fn probe_uia_live(list_window_result: &serde_json::Value) -> SubsystemStatus {
    if list_window_result.get("error").is_some() {
        SubsystemStatus::Unavailable {
            reason: list_window_result["error"]
                .as_str()
                .unwrap_or("UIA COM error")
                .to_string(),
        }
    } else {
        SubsystemStatus::Available
    }
}

/// Run a real vision health probe from vision_screenshot result.
pub fn probe_vision_live(screenshot_result: &serde_json::Value) -> SubsystemStatus {
    if screenshot_result.get("error").is_some() {
        SubsystemStatus::Unavailable {
            reason: screenshot_result["error"]
                .as_str()
                .unwrap_or("Vision capture error")
                .to_string(),
        }
    } else {
        SubsystemStatus::Available
    }
}

/// Check NTP drift from w32tm output.
/// Parses the offset value and returns milliseconds.
pub fn parse_ntp_drift(w32tm_output: &str) -> Option<i64> {
    // Look for "CurrentTimeOffset" or time offset in seconds
    for line in w32tm_output.lines() {
        if line.contains("CurrentTimeOffset") || line.contains("offset") {
            // Extract numeric value
            let parts: Vec<&str> = line.split_whitespace().collect();
            for part in parts.iter().rev() {
                if let Ok(seconds) = part.trim_end_matches('s').parse::<f64>() {
                    return Some((seconds * 1000.0) as i64);
                }
            }
        }
    }
    None
}

/// Aggregated health report for the hands MCP tool.
/// Combines cpc-paths path status with browser and vision subsystem probes.
pub fn hands_health() -> serde_json::Value {
    let paths = serde_json::to_value(cpc_paths::health_check())
        .unwrap_or_else(|e| serde_json::json!({"error": format!("serialize: {}", e)}));

    let browser_status = probe_browser();
    let vision_status = probe_vision();
    let uia_status = probe_uia();

    let browser_str = match &browser_status {
        SubsystemStatus::Available => "available",
        SubsystemStatus::Unavailable { .. } => "unavailable",
        SubsystemStatus::Unknown => "unknown",
    };
    let vision_str = match &vision_status {
        SubsystemStatus::Available => "available",
        SubsystemStatus::Unavailable { .. } => "unavailable",
        SubsystemStatus::Unknown => "unknown",
    };
    let uia_str = match &uia_status {
        SubsystemStatus::Available => "available",
        SubsystemStatus::Unavailable { .. } => "unavailable",
        SubsystemStatus::Unknown => "unknown",
    };

    serde_json::json!({
        "server": "hands",
        "version": "1.3.0-dev",
        "paths": paths,
        "browser": {
            "status": browser_str,
            "note": "call browser_launch or browser_attach to activate"
        },
        "vision": {
            "status": vision_str,
            "note": "screenshot + OCR + template matching"
        },
        "uia": {
            "status": uia_str,
            "note": "Windows UI Automation (desktop element inspection + input)"
        }
    })
}

/// Check if NTP drift is concerning (>15s off — TOTP codes fail silently).
pub fn ntp_drift_warning(drift_ms: Option<i64>) -> Option<String> {
    match drift_ms {
        Some(ms) if ms.abs() > 15_000 => Some(format!(
            "System clock drift {}ms from NTP — TOTP codes may fail. Run 'w32tm /resync'.",
            ms
        )),
        _ => None,
    }
}
