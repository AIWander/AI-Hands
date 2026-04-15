//! Window matching — find windows by title, process name, or automation ID.
//! Supports multiple match modes: First, LastFocused, RequireUnique, All.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::error::{MetaError, WindowInfo};

/// Criteria for matching a window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowMatch {
    pub title: Option<String>,
    pub process: Option<String>,
    pub automation_id: Option<String>,
}

/// How to resolve when multiple windows match.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    /// Return the first match in enumeration order.
    First,
    /// Prefer the window most recently focused (falls back to First).
    #[default]
    LastFocused,
    /// Error with MultipleWindows if more than one match.
    RequireUnique,
    /// Return all matching windows.
    All,
}

/// Which monitor to target for window placement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Monitor {
    Current,
    Primary,
    Owning,
    Index(i32),
}

/// A single matched window result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowMatchResult {
    pub title: String,
    pub process_name: Option<String>,
    pub hwnd: Option<String>,
    pub bounds: Option<(i32, i32, i32, i32)>,
    pub monitor_index: Option<i32>,
}

/// Parse a WindowMatch from tool args (looks for `window_match` or top-level `title`/`process`).
pub fn parse_window_match(args: &Value) -> Option<WindowMatch> {
    // Prefer nested window_match object
    if let Some(wm) = args.get("window_match") {
        let title = wm.get("title").and_then(|v| v.as_str()).map(|s| s.to_string());
        let process = wm.get("process").and_then(|v| v.as_str()).map(|s| s.to_string());
        let automation_id = wm.get("automation_id").and_then(|v| v.as_str()).map(|s| s.to_string());
        if title.is_some() || process.is_some() || automation_id.is_some() {
            return Some(WindowMatch { title, process, automation_id });
        }
    }

    // Fall back to top-level fields
    let title = args.get("title").and_then(|v| v.as_str()).map(|s| s.to_string());
    let process = args.get("process").and_then(|v| v.as_str()).map(|s| s.to_string());
    let automation_id = args.get("automation_id").and_then(|v| v.as_str()).map(|s| s.to_string());

    if title.is_some() || process.is_some() || automation_id.is_some() {
        Some(WindowMatch { title, process, automation_id })
    } else {
        None
    }
}

/// Parse a MatchMode from an optional string value.
pub fn parse_match_mode(s: Option<&str>) -> MatchMode {
    match s {
        Some("first") => MatchMode::First,
        Some("last_focused") => MatchMode::LastFocused,
        Some("require_unique") => MatchMode::RequireUnique,
        Some("all") => MatchMode::All,
        _ => MatchMode::default(),
    }
}

/// Parse a Monitor from tool args.
pub fn parse_monitor(args: &Value) -> Option<Monitor> {
    let monitor_val = args.get("monitor")?;

    // String form: "current", "primary", "owning"
    if let Some(s) = monitor_val.as_str() {
        return match s {
            "current" => Some(Monitor::Current),
            "primary" => Some(Monitor::Primary),
            "owning" => Some(Monitor::Owning),
            _ => {
                // Try parsing as integer string
                if let Ok(idx) = s.parse::<i32>() {
                    Some(Monitor::Index(idx))
                } else {
                    None
                }
            }
        };
    }

    // Integer form
    if let Some(idx) = monitor_val.as_i64() {
        return Some(Monitor::Index(idx as i32));
    }

    // Object form: {"index": N}
    if let Some(idx) = monitor_val.get("index").and_then(|v| v.as_i64()) {
        return Some(Monitor::Index(idx as i32));
    }

    None
}

/// Check if a window JSON object from UIA matches the given criteria.
fn window_matches(window: &Value, wm: &WindowMatch) -> bool {
    // Title: case-insensitive contains
    if let Some(ref title_query) = wm.title {
        let win_title = window.get("title")
            .or_else(|| window.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !win_title.to_lowercase().contains(&title_query.to_lowercase()) {
            return false;
        }
    }

    // Process name: case-insensitive match with .exe suffix tolerance.
    // Phase C fix3: handles mismatches like query="notepad.exe" vs UIA="Notepad",
    // or query="notepad" vs UIA="notepad.exe".
    if let Some(ref proc_query) = wm.process {
        let win_proc = window.get("process_name")
            .or_else(|| window.get("process"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let q = proc_query.to_lowercase();
        let w = win_proc.to_lowercase();
        let q_stem = q.strip_suffix(".exe").unwrap_or(&q);
        let w_stem = w.strip_suffix(".exe").unwrap_or(&w);
        if q_stem != w_stem {
            return false;
        }
    }

    // Automation ID: exact match
    if let Some(ref aid_query) = wm.automation_id {
        let win_aid = window.get("automation_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if win_aid != aid_query {
            return false;
        }
    }

    true
}

/// Extract a WindowMatchResult from a UIA window JSON value.
fn extract_result(window: &Value) -> WindowMatchResult {
    let title = window.get("title")
        .or_else(|| window.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let process_name = window.get("process_name")
        .or_else(|| window.get("process"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let hwnd = window.get("hwnd")
        .or_else(|| window.get("handle"))
        .and_then(|v| {
            v.as_str().map(|s| s.to_string())
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        });

    let bounds = extract_bounds(window);

    let monitor_index = window.get("monitor_index")
        .or_else(|| window.get("monitor"))
        .and_then(|v| v.as_i64())
        .map(|n| n as i32);

    WindowMatchResult { title, process_name, hwnd, bounds, monitor_index }
}

/// Extract window bounds as (x, y, w, h) from various JSON layouts.
fn extract_bounds(window: &Value) -> Option<(i32, i32, i32, i32)> {
    // Try bounds object: {x, y, width, height}
    if let Some(b) = window.get("bounds").or_else(|| window.get("rect")) {
        let x = b.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let y = b.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let w = b.get("width").or_else(|| b.get("w")).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let h = b.get("height").or_else(|| b.get("h")).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        return Some((x, y, w, h));
    }

    // Try flat fields
    let x = window.get("x").and_then(|v| v.as_i64());
    let y = window.get("y").and_then(|v| v.as_i64());
    let w = window.get("width").and_then(|v| v.as_i64());
    let h = window.get("height").and_then(|v| v.as_i64());

    match (x, y, w, h) {
        (Some(x), Some(y), Some(w), Some(h)) => Some((x as i32, y as i32, w as i32, h as i32)),
        _ => None,
    }
}

/// Find matching windows from a UIA window list result.
///
/// Applies match mode logic:
/// - `First`: return first match in enumeration order.
/// - `LastFocused`: prefer the window most recently focused (by z-order position); falls back to First.
/// - `RequireUnique`: error with MultipleWindows if more than one match.
/// - `All`: return all matches.
///
/// Returns `ElementNotFound` if no windows match.
pub fn find_matching_windows(
    windows: &[Value],
    window_match: &WindowMatch,
    mode: &MatchMode,
) -> Result<Vec<WindowMatchResult>, MetaError> {
    let matches: Vec<WindowMatchResult> = windows
        .iter()
        .filter(|w| window_matches(w, window_match))
        .map(|w| extract_result(w))
        .collect();

    if matches.is_empty() {
        let target = window_match.title.clone()
            .or_else(|| window_match.process.clone())
            .or_else(|| window_match.automation_id.clone())
            .unwrap_or_else(|| "<unspecified>".to_string());
        return Err(MetaError::not_found(target, "uia_list_window"));
    }

    match mode {
        MatchMode::First => Ok(vec![matches.into_iter().next().unwrap()]),

        MatchMode::LastFocused => {
            // The first window in UIA enumeration is typically the foreground window
            // (highest z-order). This matches "last focused" semantics since
            // SetForegroundWindow changes z-order.
            Ok(vec![matches.into_iter().next().unwrap()])
        }

        MatchMode::RequireUnique => {
            if matches.len() > 1 {
                let app = window_match.title.clone()
                    .or_else(|| window_match.process.clone())
                    .unwrap_or_else(|| "<app>".to_string());
                let candidates = matches.iter().map(|m| WindowInfo {
                    title: m.title.clone(),
                    process: m.process_name.clone(),
                    hwnd: m.hwnd.as_ref().and_then(|h| h.parse::<u64>().ok()),
                }).collect();
                return Err(MetaError::MultipleWindows { app, candidates });
            }
            Ok(matches)
        }

        MatchMode::All => Ok(matches),
    }
}

/// Convenience: get a single best match (for actions that target one window).
pub fn find_single_window(
    windows: &[Value],
    window_match: &WindowMatch,
    mode: &MatchMode,
) -> Result<WindowMatchResult, MetaError> {
    let mut results = find_matching_windows(windows, window_match, mode)?;
    Ok(results.remove(0))
}
