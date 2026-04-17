//! HTTP dashboard endpoint for the hands server.
//!
//! GET /api/status → JSON: browser state, recent actions, UIA state, vision state
//!
//! Port: CPC_DASHBOARD_PORT_HANDS env var, default 9102.
//! Binds 127.0.0.1 only. Falls back through +5 ports if primary is taken.
//! Graceful: if all ports fail, logs a warning — MCP continues normally.
//!
//! # Ring buffer
//! Each meta-tool call invokes `record_action(tool, target)` which appends to a
//! Mutex<VecDeque<ActionEntry>> (capacity 10) accessible from the HTTP thread.
//!
//! # Browser state
//! `update_browser_snapshot(status)` is called from the main dispatch after browser
//! tool calls so the dashboard can return a non-async snapshot.
//!
//! # UIA state
//! `update_uia_snapshot(window, action)` is called from UIA tool dispatch.

use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::thread;

const DEFAULT_PORT: u16 = 9102;
const ENV_PORT: &str = "CPC_DASHBOARD_PORT_HANDS";
const RING_CAPACITY: usize = 10;

// ── Shared state (written by MCP thread, read by HTTP thread) ─────────────────

#[derive(Clone)]
struct ActionEntry {
    tool_name: String,
    target: String,
    timestamp_utc: String,
    duration_ms: u64,
}

static RECENT_ACTIONS: OnceLock<Mutex<VecDeque<ActionEntry>>> = OnceLock::new();
static BROWSER_SNAPSHOT: OnceLock<Mutex<Value>> = OnceLock::new();
static UIA_SNAPSHOT: OnceLock<Mutex<Value>> = OnceLock::new();
static VISION_SNAPSHOT: OnceLock<Mutex<Value>> = OnceLock::new();

fn recent_actions() -> &'static Mutex<VecDeque<ActionEntry>> {
    RECENT_ACTIONS.get_or_init(|| Mutex::new(VecDeque::with_capacity(RING_CAPACITY)))
}

fn browser_snapshot() -> &'static Mutex<Value> {
    BROWSER_SNAPSHOT.get_or_init(|| Mutex::new(json!(null)))
}

fn uia_snapshot() -> &'static Mutex<Value> {
    UIA_SNAPSHOT.get_or_init(|| Mutex::new(json!(null)))
}

fn vision_snapshot() -> &'static Mutex<Value> {
    VISION_SNAPSHOT.get_or_init(|| Mutex::new(json!(null)))
}

// ── Public update API (called from MCP dispatch thread) ───────────────────────

/// Record a tool invocation in the ring buffer (capacity 10, oldest dropped).
pub fn record_action(tool: &str, args: &Value, duration_ms: u64) {
    let target = extract_target(args);
    let timestamp_utc = chrono::Utc::now().to_rfc3339();
    // Redact tool names that look like credential operations.
    let tool_name = {
        let lower = tool.to_ascii_lowercase();
        if lower.contains("password")
            || lower.contains("token")
            || lower.contains("api_key")
            || lower.contains("secret")
        {
            "[REDACTED]".to_string()
        } else {
            tool.to_string()
        }
    };
    if let Ok(mut guard) = recent_actions().lock() {
        if guard.len() >= RING_CAPACITY {
            guard.pop_front();
        }
        guard.push_back(ActionEntry {
            tool_name,
            target,
            timestamp_utc,
            duration_ms,
        });
    }
}

/// Update the cached browser status snapshot (from a sync try_read on SharedBrowser).
pub fn update_browser_snapshot(status: Value) {
    if let Ok(mut guard) = browser_snapshot().lock() {
        *guard = status;
    }
}

/// Update the cached UIA state after a UIA tool runs.
pub fn update_uia_snapshot(window: &str, action: &str) {
    let snap = json!({
        "last_focused_window": window,
        "last_action": action,
        "last_action_ts": chrono::Utc::now().to_rfc3339()
    });
    if let Ok(mut guard) = uia_snapshot().lock() {
        *guard = snap;
    }
}

/// Update the cached vision state after a screenshot or OCR runs.
pub fn update_vision_snapshot(screenshot_path: Option<&str>, ocr: bool) {
    let ts = chrono::Utc::now().to_rfc3339();
    if let Ok(mut guard) = vision_snapshot().lock() {
        if let Some(path) = screenshot_path {
            if let Some(obj) = guard.as_object_mut() {
                obj.insert("last_screenshot_path".into(), json!(path));
                if ocr {
                    obj.insert("last_ocr_ts".into(), json!(ts));
                }
            } else {
                let mut new_snap = json!({
                    "last_screenshot_path": path
                });
                if ocr {
                    new_snap["last_ocr_ts"] = json!(ts);
                }
                *guard = new_snap;
            }
        }
    }
}

/// Extract a meaningful target string from tool args (first non-empty known field).
fn extract_target(args: &Value) -> String {
    for field in &[
        "target", "selector", "url", "title", "text", "name", "query", "path",
    ] {
        if let Some(v) = args.get(field).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                return v.chars().take(100).collect();
            }
        }
    }
    String::new()
}

// ── HTTP server ────────────────────────────────────────────────────────────────

/// Spawn the dashboard HTTP server on an isolated thread.
pub fn spawn() {
    thread::Builder::new()
        .name("hands-dashboard".into())
        .spawn(move || {
            let base_port: u16 = std::env::var(ENV_PORT)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(DEFAULT_PORT);

            let server = match try_bind(base_port) {
                Some(s) => s,
                None => {
                    eprintln!(
                        "[hands/dashboard] Could not bind on ports {}–{}. \
                         MCP continues without dashboard endpoint.",
                        base_port,
                        base_port + 5
                    );
                    return;
                }
            };

            let port = server
                .server_addr()
                .to_ip()
                .map(|a| a.port())
                .unwrap_or(base_port);
            eprintln!(
                "[hands/dashboard] Listening on http://127.0.0.1:{}/api/status",
                port
            );

            for request in server.incoming_requests() {
                handle_request(request);
            }
        })
        .ok();
}

fn try_bind(base_port: u16) -> Option<tiny_http::Server> {
    for port in base_port..base_port + 6 {
        let addr = format!("127.0.0.1:{}", port);
        if let Ok(s) = tiny_http::Server::http(&addr) {
            return Some(s);
        }
    }
    None
}

fn cors_headers() -> Vec<tiny_http::Header> {
    vec![
        "Access-Control-Allow-Origin: *".parse().unwrap(),
        "Access-Control-Allow-Methods: GET, OPTIONS"
            .parse()
            .unwrap(),
        "Access-Control-Allow-Headers: Content-Type"
            .parse()
            .unwrap(),
        "Content-Type: application/json".parse().unwrap(),
    ]
}

fn respond(request: tiny_http::Request, status: u16, body: Value) {
    let body_str = serde_json::to_string(&body).unwrap_or_default();
    let mut response = tiny_http::Response::from_string(body_str).with_status_code(status);
    for h in cors_headers() {
        response = response.with_header(h);
    }
    let _ = request.respond(response);
}

fn handle_request(request: tiny_http::Request) {
    let method = request.method().as_str().to_uppercase();
    let url = request.url().split('?').next().unwrap_or("").to_string();

    match (method.as_str(), url.as_str()) {
        ("GET", "/api/status") => respond(request, 200, build_status()),
        ("OPTIONS", _) => respond(request, 204, json!({})),
        _ => respond(request, 404, json!({"error": "Not found"})),
    }
}

// ── Status builder ─────────────────────────────────────────────────────────────

fn build_status() -> Value {
    json!({
        "server": "hands",
        "version": "1.3.1",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "browser": build_browser_status(),
        "recent_tool_calls": build_recent_actions(),
        "uia": build_uia_status(),
        "vision": build_vision_status(),
    })
}

fn build_browser_status() -> Value {
    browser_snapshot()
        .lock()
        .ok()
        .map(|guard| {
            let snap = guard.clone();
            if snap.is_null() {
                // No browser activity yet
                json!({
                    "status": "unknown",
                    "current_url": null,
                    "tab_count": 0,
                    "contexts": [],
                    "routes_active": 0
                })
            } else {
                // browser_mcp BrowserManager::status() returns:
                // { active, headless, current_url, tab_count, active_tab }
                let launched = snap
                    .get("active")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let headless = snap
                    .get("headless")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let status_str = if !launched {
                    "closed"
                } else if headless {
                    "headless"
                } else {
                    "launched"
                };
                json!({
                    "status": status_str,
                    "current_url": snap.get("current_url"),
                    "tab_count": snap.get("tab_count").and_then(|v| v.as_u64()).unwrap_or(0),
                    "contexts": [],
                    "routes_active": 0
                })
            }
        })
        .unwrap_or_else(|| {
            json!({
                "status": "unknown",
                "current_url": null,
                "tab_count": 0,
                "contexts": [],
                "routes_active": 0
            })
        })
}

fn build_recent_actions() -> Vec<Value> {
    recent_actions()
        .lock()
        .ok()
        .map(|guard| {
            guard
                .iter()
                .map(|a| {
                    json!({
                        "tool_name": a.tool_name,
                        "target": a.target,
                        "timestamp_utc": a.timestamp_utc,
                        "duration_ms": a.duration_ms
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn build_uia_status() -> Value {
    uia_snapshot()
        .lock()
        .ok()
        .map(|guard| guard.clone())
        .unwrap_or(json!(null))
}

fn build_vision_status() -> Value {
    vision_snapshot()
        .lock()
        .ok()
        .map(|guard| guard.clone())
        .unwrap_or(json!(null))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_has_required_fields() {
        let status = build_status();
        assert_eq!(status["server"], "hands");
        assert_eq!(status["version"], "1.3.1");
        assert!(status["timestamp"].is_string());
        assert!(status["browser"].is_object());
        assert!(status["recent_tool_calls"].is_array());
        assert!(status["browser"]["status"].is_string());
        assert!(status["browser"]["tab_count"].is_number());
    }

    #[test]
    fn test_ring_buffer_capacity() {
        // Record 15 actions — only last 10 should remain.
        for i in 0..15 {
            record_action(
                &format!("tool_{}", i),
                &json!({"target": format!("sel_{}", i)}),
                0,
            );
        }
        let actions = build_recent_actions();
        assert!(
            actions.len() <= RING_CAPACITY,
            "ring buffer must not exceed capacity {}",
            RING_CAPACITY
        );
    }

    #[test]
    fn test_port_fallback_range() {
        let base = DEFAULT_PORT;
        let range: Vec<u16> = (base..base + 6).collect();
        assert_eq!(range.len(), 6);
        assert_eq!(range[0], 9102);
        assert_eq!(range[5], 9107);
    }

    #[test]
    fn test_extract_target_priority() {
        let args = json!({"selector": "#btn", "title": "My Window"});
        let target = extract_target(&args);
        // "target" field not present, next is "selector"
        assert_eq!(target, "#btn");
    }

    #[test]
    fn test_extract_target_empty_args() {
        let target = extract_target(&json!({}));
        assert!(target.is_empty());
    }
}
