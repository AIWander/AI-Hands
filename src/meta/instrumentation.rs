#![allow(dead_code)] // scaffolded module, awaiting integration
//! Instrumentation logger — writes rung attempts to hands_meta.jsonl.
//! Log directory resolved via legacy-fallback:
//!   1. Legacy C:\CPC\logs — if it exists with hands_meta data
//!   2. cpc_paths::data_path("hands") — fresh installs
//! One line per rung attempt, one aggregate line per call.
//! Rotate daily at midnight, keep 7 days.
//! Persistence is an allowlisted telemetry projection. Raw arguments, URLs,
//! selectors, typed text, bodies, headers, cookies, and paths never enter JSONL.

use serde_json::{json, Map, Value};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const LEGACY_LOG_DIR: &str = r"C:\CPC\logs";
const LOG_FILE: &str = "hands_meta.jsonl";
const RETENTION_DAYS: u64 = 7;

static LOG_DIR_RESOLVED: OnceLock<PathBuf> = OnceLock::new();

/// Get the resolved log directory (resolved once at first call).
fn log_dir() -> &'static PathBuf {
    LOG_DIR_RESOLVED.get_or_init(|| {
        _resolve_hands_log_dir(Path::new(LEGACY_LOG_DIR))
            .unwrap_or_else(|_| PathBuf::from(LEGACY_LOG_DIR))
    })
}

/// Resolve the hands log directory.
/// 1. Legacy `C:\CPC\logs` — if it exists AND contains hands_meta data.
/// 2. `cpc_paths::data_path("hands")` — fresh installs.
///
/// Testable inner function — takes `legacy` as a parameter so tests can inject tempdirs.
pub(crate) fn _resolve_hands_log_dir(legacy: &Path) -> anyhow::Result<PathBuf> {
    if legacy.exists() && has_hands_log_data(legacy) {
        return Ok(legacy.to_path_buf());
    }
    cpc_paths::data_path("hands")
}

/// Returns true if `dir` contains at least one hands_meta log file.
/// An empty-but-existing legacy dir falls through to cpc-paths.
pub(crate) fn has_hands_log_data(dir: &Path) -> bool {
    if dir.join(LOG_FILE).exists() {
        return true;
    }
    // Also check for rotated hands_meta_{date}.jsonl files
    dir.read_dir()
        .map(|mut d| {
            d.any(|e| {
                e.ok()
                    .and_then(|e| e.file_name().into_string().ok())
                    .map(|n| n.starts_with("hands_meta_") && n.ends_with(".jsonl"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// Get the current log file path.
fn log_path() -> PathBuf {
    log_dir().join(LOG_FILE)
}

/// Get the rotated log file path for a given date.
fn rotated_path(date: &str) -> PathBuf {
    log_dir().join(format!("hands_meta_{}.jsonl", date))
}

/// Write a single JSON line to the instrumentation log.
/// Creates the log directory if it doesn't exist.
fn write_line(line: &Value) {
    // Best-effort logging — never panic on log failure.
    let _ = append_safe_line(&log_path(), line);
}

/// Final persistence boundary. The central network redactor runs before the
/// strict telemetry projection, so a future caller cannot bypass the sink by
/// adding a new raw field to an event.
fn append_safe_line(path: &Path, line: &Value) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let recursively_redacted = crate::network_redaction::redact_network_value(line);
    let projected = project_telemetry_line(&recursively_redacted);
    let serialized = serde_json::to_string(&projected)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serialized)
}

fn safe_identifier(raw: &str, max_len: usize) -> String {
    if raw.is_empty() {
        return String::new();
    }
    if raw.len() <= max_len
        && raw
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
    {
        raw.to_string()
    } else {
        "[REDACTED]".to_string()
    }
}

fn safe_tool_name(raw: &str) -> String {
    let candidate = safe_identifier(raw, 64);
    if candidate.starts_with("hands_") {
        candidate
    } else {
        "unknown_tool".to_string()
    }
}

fn safe_call_id(raw: &str) -> String {
    let candidate = safe_identifier(raw, 80);
    if candidate.starts_with("call_") {
        candidate
    } else {
        "[REDACTED]".to_string()
    }
}

fn tool_category(tool: &str) -> &'static str {
    match tool {
        "hands_navigate" => "navigation",
        "hands_click" | "hands_type" | "hands_fill_form" | "hands_app_action" => "interaction",
        "hands_read_page" | "hands_find" | "hands_capture" | "hands_verify" | "hands_qr_scan" => {
            "observation"
        }
        "hands_script" => "automation",
        _ => "meta_tool",
    }
}

fn safe_timestamp(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|parsed| Value::String(parsed.with_timezone(&chrono::Utc).to_rfc3339()))
        .unwrap_or(Value::Null)
}

/// Keep only low-risk aggregate context. String values are intentionally not
/// copied: even an innocent-looking label can hold a selector, typed text,
/// bearer token, URL, or local path.
fn project_context(context: &Value) -> Value {
    const SAFE_BOOL_KEYS: &[&str] = &[
        "auto_submit",
        "clear_first",
        "double_click",
        "fast_set",
        "force_close",
        "negated",
        "ocr",
        "offset_active",
        "strict",
        "submit",
        "verify_focus",
        "visible",
    ];
    const SAFE_NUMBER_KEYS: &[&str] = &[
        "attempt_count",
        "candidate_count",
        "field_count",
        "item_count",
        "match_count",
        "monitor",
        "result_count",
        "step_count",
        "text_len",
        "timeout_ms",
    ];

    let redacted = crate::network_redaction::redact_network_value(context);
    let mut projected = Map::new();
    projected.insert("redacted".to_string(), Value::Bool(true));

    if let Some(object) = redacted.as_object() {
        for (key, value) in object {
            if (SAFE_BOOL_KEYS.contains(&key.as_str()) && value.is_boolean())
                || (SAFE_NUMBER_KEYS.contains(&key.as_str()) && value.is_number())
            {
                projected.insert(key.clone(), value.clone());
            }
        }
    }

    Value::Object(projected)
}

fn classify_error(value: &Value) -> &'static str {
    let redacted = crate::network_redaction::redact_network_value(value);
    let normalized = redacted.to_string().to_ascii_lowercase();

    if normalized.contains("confirm") {
        "requires_confirmation"
    } else if normalized.contains("permission")
        || normalized.contains("denied")
        || normalized.contains("forbidden")
        || normalized.contains("unauthor")
    {
        "permission_denied"
    } else if normalized.contains("timeout") || normalized.contains("timed out") {
        "timeout"
    } else if normalized.contains("not found")
        || normalized.contains("not_found")
        || normalized.contains("no match")
    {
        "not_found"
    } else if normalized.contains("no browser") || normalized.contains("browser unavailable") {
        "no_browser"
    } else if normalized.contains("launch") || normalized.contains("attach") {
        "browser_startup"
    } else if normalized.contains("invalid")
        || normalized.contains("missing")
        || normalized.contains("argument")
        || normalized.contains("parameter")
    {
        "invalid_input"
    } else if normalized.contains("unsupported") {
        "unsupported"
    } else if normalized.contains("cancel") {
        "cancelled"
    } else if normalized.contains("security") || normalized.contains("policy") {
        "security_policy"
    } else if normalized.contains("network")
        || normalized.contains("dns")
        || normalized.contains("connect")
        || normalized.contains("http")
    {
        "network"
    } else if normalized.contains("block") {
        "blocked"
    } else {
        "operation_failed"
    }
}

fn project_error(error: &Value) -> Value {
    if error.is_null() {
        Value::Null
    } else {
        json!({"category": classify_error(error)})
    }
}

/// Project an event onto the fields consumed by `hands_summarize_run` plus
/// bounded aggregate telemetry. Unknown fields are dropped at the sink.
fn project_telemetry_line(line: &Value) -> Value {
    let object = match line.as_object() {
        Some(object) => object,
        None => return json!({"error": {"category": "invalid_telemetry_event"}}),
    };

    let tool = object
        .get("tool")
        .and_then(Value::as_str)
        .map(safe_tool_name)
        .unwrap_or_else(|| "unknown_tool".to_string());
    let mut projected = Map::new();
    projected.insert("ts".to_string(), safe_timestamp(object.get("ts")));
    projected.insert("category".to_string(), json!(tool_category(&tool)));
    projected.insert("tool".to_string(), Value::String(tool));
    projected.insert(
        "call_id".to_string(),
        Value::String(
            object
                .get("call_id")
                .and_then(Value::as_str)
                .map(safe_call_id)
                .unwrap_or_else(|| "[REDACTED]".to_string()),
        ),
    );

    for key in ["aggregate", "success"] {
        if let Some(value) = object.get(key).and_then(Value::as_bool) {
            projected.insert(key.to_string(), Value::Bool(value));
        }
    }
    for key in ["elapsed_ms", "rungs_tried"] {
        if let Some(value) = object.get(key).and_then(Value::as_u64) {
            projected.insert(key.to_string(), json!(value));
        }
    }
    if let Some(value) = object.get("confidence").filter(|value| value.is_number()) {
        projected.insert("confidence".to_string(), value.clone());
    } else if object.contains_key("confidence") {
        projected.insert("confidence".to_string(), Value::Null);
    }
    for key in ["rung", "method"] {
        if let Some(value) = object.get(key).and_then(Value::as_str) {
            projected.insert(key.to_string(), Value::String(safe_identifier(value, 64)));
        }
    }
    if let Some(error) = object.get("error") {
        projected.insert("error".to_string(), project_error(error));
    }
    if let Some(context) = object.get("context") {
        projected.insert("context".to_string(), project_context(context));
    }

    Value::Object(projected)
}

/// Log a single rung attempt.
pub fn log_rung_attempt(
    tool: &str,
    call_id: &str,
    rung: &str,
    success: bool,
    elapsed_ms: u64,
    confidence: Option<f32>,
    context: &Value,
) {
    let ts = chrono::Utc::now().to_rfc3339();
    let ctx = project_context(context);

    let line = json!({
        "ts": ts,
        "tool": tool,
        "call_id": call_id,
        "rung": rung,
        "success": success,
        "elapsed_ms": elapsed_ms,
        "confidence": confidence,
        "context": ctx,
    });

    write_line(&line);
}

/// Log an aggregate result for a complete meta-tool call.
pub fn log_aggregate(
    tool: &str,
    call_id: &str,
    success: bool,
    method: &str,
    rungs_tried: usize,
    total_elapsed_ms: u64,
    confidence: Option<f32>,
    error: Option<&str>,
) {
    let error_value = error.map(|e| Value::String(e.to_string()));
    let line = build_aggregate_line(
        tool,
        call_id,
        success,
        method,
        rungs_tried,
        total_elapsed_ms,
        confidence,
        error_value,
        None,
    );

    write_line(&line);
}

/// Log an aggregate result with structured error and call context.
pub fn log_aggregate_with_context(
    tool: &str,
    call_id: &str,
    success: bool,
    method: &str,
    rungs_tried: usize,
    total_elapsed_ms: u64,
    confidence: Option<f32>,
    error: Option<&Value>,
    context: Option<&Value>,
) {
    let line = build_aggregate_line(
        tool,
        call_id,
        success,
        method,
        rungs_tried,
        total_elapsed_ms,
        confidence,
        error.cloned(),
        context,
    );

    write_line(&line);
}

fn build_aggregate_line(
    tool: &str,
    call_id: &str,
    success: bool,
    method: &str,
    rungs_tried: usize,
    total_elapsed_ms: u64,
    confidence: Option<f32>,
    error: Option<Value>,
    context: Option<&Value>,
) -> Value {
    let ts = chrono::Utc::now().to_rfc3339();
    let error = error.as_ref().map(project_error).unwrap_or(Value::Null);

    let mut line = json!({
        "ts": ts,
        "tool": tool,
        "call_id": call_id,
        "aggregate": true,
        "success": success,
        "method": method,
        "rungs_tried": rungs_tried,
        "elapsed_ms": total_elapsed_ms,
        "confidence": confidence,
        "error": error,
    });

    if let Some(context) = context {
        if let Some(obj) = line.as_object_mut() {
            obj.insert("context".to_string(), project_context(context));
        }
    }

    line
}

/// Rotate the log file if it's from a previous day.
/// Called before first write of the day.
pub fn rotate_if_needed() {
    let path = log_path();
    if !path.exists() {
        return;
    }

    if let Ok(metadata) = fs::metadata(&path) {
        if let Ok(modified) = metadata.modified() {
            let modified_date = chrono::DateTime::<chrono::Utc>::from(modified)
                .format("%Y-%m-%d")
                .to_string();
            let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

            if modified_date != today {
                let dest = rotated_path(&modified_date);
                let _ = fs::rename(&path, &dest);
            }
        }
    }

    // Clean up old rotated files (keep 7 days)
    cleanup_old_logs();
}

/// Remove rotated log files older than RETENTION_DAYS.
fn cleanup_old_logs() {
    let dir = log_dir();
    if let Ok(entries) = fs::read_dir(dir) {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(RETENTION_DAYS as i64);
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("hands_meta_") && name.ends_with(".jsonl") {
                // Extract date from filename
                let date_part = name
                    .strip_prefix("hands_meta_")
                    .and_then(|s| s.strip_suffix(".jsonl"));
                if let Some(date_str) = date_part {
                    if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                        let dt = date.and_hms_opt(0, 0, 0).unwrap();
                        let dt_utc = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                            dt,
                            chrono::Utc,
                        );
                        if dt_utc < cutoff {
                            let _ = fs::remove_file(entry.path());
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_legacy_path_wins() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // Create hands_meta.jsonl — simulates an existing legacy install
        std::fs::write(dir.path().join("hands_meta.jsonl"), "").unwrap();

        let result = _resolve_hands_log_dir(dir.path()).unwrap();
        assert_eq!(
            result,
            dir.path(),
            "legacy dir with hands_meta.jsonl should be returned"
        );
    }

    #[test]
    fn test_no_legacy_falls_through() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // Empty tempdir — no hands_meta files
        assert!(
            !has_hands_log_data(dir.path()),
            "empty dir must not be detected as legacy hands log data"
        );
    }

    #[test]
    fn test_rotated_marker_detected() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hands_meta_2026-04-14.jsonl"), "").unwrap();
        assert!(
            has_hands_log_data(dir.path()),
            "rotated hands_meta_{{date}}.jsonl should be detected"
        );
    }

    #[test]
    fn test_context_projection_keeps_only_bounded_aggregates() {
        let ctx = json!({
            "target": "Sign In",
            "url": "https://example.test/?token=QUERY_SENTINEL",
            "selector": "#password",
            "typed_text": "PASSWORD_SENTINEL",
            "headers": {"Authorization": "Bearer BEARER_SENTINEL"},
            "cookie": "session=COOKIE_SENTINEL",
            "path": "C:\\Users\\josep\\Private\\secret.txt",
            "otp": "654321",
            "monitor": 2,
            "text_len": 17,
            "visible": true
        });

        let projected = project_context(&ctx);
        assert_eq!(projected["redacted"], true);
        assert_eq!(projected["monitor"], 2);
        assert_eq!(projected["text_len"], 17);
        assert_eq!(projected["visible"], true);
        assert_eq!(projected.as_object().unwrap().len(), 4);
    }

    #[test]
    fn test_aggregate_line_can_record_zero_rung_block() {
        let ctx = json!({"target": "Delete Account", "otp": "123456"});
        let err = json!({
            "category": "requires_confirmation",
            "detail": {
                "action": "Delete Account",
                "reason": "blocked by reversibility classifier"
            }
        });

        let line = build_aggregate_line(
            "hands_click",
            "call_test",
            false,
            "",
            0,
            3,
            None,
            Some(err),
            Some(&ctx),
        );

        assert_eq!(line["aggregate"], true);
        assert_eq!(line["tool"], "hands_click");
        assert_eq!(line["success"], false);
        assert_eq!(line["rungs_tried"], 0);
        assert_eq!(line["method"], "");
        assert_eq!(line["error"]["category"], "requires_confirmation");
        assert_eq!(line["context"]["redacted"], true);
        assert!(line["context"].get("target").is_none());
        assert!(line["context"].get("otp").is_none());
    }

    #[test]
    fn test_actual_jsonl_sink_contains_no_secret_url_or_private_path_sentinels() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hands_meta.jsonl");
        let raw_event = json!({
            "ts": "2026-07-15T12:00:00Z",
            "tool": "hands_navigate",
            "call_id": "call_000001",
            "aggregate": true,
            "success": false,
            "method": "navigate",
            "rungs_tried": 1,
            "elapsed_ms": 9,
            "confidence": null,
            "error": {
                "category": "network",
                "detail": "Bearer BEARER_SENTINEL at https://example.test/?token=QUERY_SENTINEL from C:\\Users\\josep\\Private"
            },
            "context": {
                "url": "https://alice:password@example.test/reset?token=QUERY_SENTINEL",
                "selector": "#password",
                "typed_text": "PASSWORD_SENTINEL",
                "password": "hunter2",
                "otp": "654321",
                "headers": {
                    "Authorization": "Bearer BEARER_SENTINEL",
                    "Cookie": "session=COOKIE_SENTINEL"
                },
                "cookie": "session=COOKIE_SENTINEL",
                "path": "C:\\Users\\josep\\Private\\secret.txt",
                "monitor": 2,
                "text_len": 17,
                "visible": true
            },
            "future_raw_field": "Bearer FUTURE_FIELD_SENTINEL"
        });

        append_safe_line(&path, &raw_event).unwrap();
        let serialized = std::fs::read_to_string(&path).unwrap();
        for forbidden in [
            "BEARER_SENTINEL",
            "COOKIE_SENTINEL",
            "PASSWORD_SENTINEL",
            "QUERY_SENTINEL",
            "FUTURE_FIELD_SENTINEL",
            "654321",
            "hunter2",
            "C:\\\\Users\\\\josep",
            "example.test",
            "#password",
            "alice",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "serialized JSONL leaked sentinel {forbidden}: {serialized}"
            );
        }

        let persisted: Value = serde_json::from_str(serialized.trim()).unwrap();
        assert_eq!(persisted["category"], "navigation");
        assert_eq!(persisted["tool"], "hands_navigate");
        assert_eq!(persisted["call_id"], "call_000001");
        assert_eq!(persisted["method"], "navigate");
        assert_eq!(persisted["error"]["category"], "network");
        assert_eq!(persisted["context"]["monitor"], 2);
        assert_eq!(persisted["context"]["text_len"], 17);
        assert_eq!(persisted["context"]["visible"], true);
        assert!(persisted.get("future_raw_field").is_none());
        assert_eq!(persisted["context"].as_object().unwrap().len(), 4);
    }
}
