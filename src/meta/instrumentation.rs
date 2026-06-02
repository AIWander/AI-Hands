#![allow(dead_code)] // scaffolded module, awaiting integration
//! Instrumentation logger — writes rung attempts to hands_meta.jsonl.
//! Log directory resolved via legacy-fallback:
//!   1. Legacy C:\CPC\logs — if it exists with hands_meta data
//!   2. cpc_paths::data_path("hands") — fresh installs
//! One line per rung attempt, one aggregate line per call.
//! Rotate daily at midnight, keep 7 days.
//! Redaction: scrub 6-digit codes from args matching code|otp|2fa|verification fields.

use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
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
    // Best-effort logging — never panic on log failure
    let _ = fs::create_dir_all(log_dir());
    let path = log_path();

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let serialized = serde_json::to_string(line).unwrap_or_default();
        let _ = writeln!(file, "{}", serialized);
    }
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
    let mut ctx = context.clone();
    redact_sensitive_fields(&mut ctx);

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
    let mut error = error.unwrap_or(Value::Null);
    redact_sensitive_fields(&mut error);

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
        let mut ctx = context.clone();
        redact_sensitive_fields(&mut ctx);
        if let Some(obj) = line.as_object_mut() {
            obj.insert("context".to_string(), ctx);
        }
    }

    line
}

/// Redact sensitive fields from instrumentation context.
/// Scrubs 6-digit codes from args with field names matching code|otp|2fa|verification.
fn redact_sensitive_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let sensitive_keys: Vec<String> = map
                .keys()
                .filter(|k| {
                    let lower = k.to_lowercase();
                    lower.contains("code")
                        || lower.contains("otp")
                        || lower.contains("2fa")
                        || lower.contains("verification")
                        || lower.contains("password")
                        || lower.contains("secret")
                        || lower.contains("token")
                })
                .cloned()
                .collect();

            for key in sensitive_keys {
                map.insert(key, Value::String("[REDACTED]".into()));
            }

            // Recurse into remaining values
            for v in map.values_mut() {
                redact_sensitive_fields(v);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                redact_sensitive_fields(v);
            }
        }
        Value::String(s) if s.len() == 6 && s.chars().all(|c| c.is_ascii_digit()) => {
            // Redact inline 6-digit codes that look like OTPs
            *s = "[REDACTED-OTP]".into();
        }
        _ => {}
    }
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
    fn test_redact_otp_field() {
        let mut ctx = json!({
            "target": "Sign In",
            "verification_code": "123456",
            "otp": "789012"
        });
        redact_sensitive_fields(&mut ctx);
        assert_eq!(ctx["verification_code"], "[REDACTED]");
        assert_eq!(ctx["otp"], "[REDACTED]");
        assert_eq!(ctx["target"], "Sign In");
    }

    #[test]
    fn test_redact_inline_6digit() {
        let mut ctx = json!(["Submit", "123456", "button"]);
        redact_sensitive_fields(&mut ctx);
        assert_eq!(ctx[1], "[REDACTED-OTP]");
        assert_eq!(ctx[0], "Submit");
    }

    #[test]
    fn test_redact_nested() {
        let mut ctx = json!({
            "args": {
                "password": "secret123",
                "label": "Email"
            }
        });
        redact_sensitive_fields(&mut ctx);
        assert_eq!(ctx["args"]["password"], "[REDACTED]");
        assert_eq!(ctx["args"]["label"], "Email");
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
        assert_eq!(line["context"]["target"], "Delete Account");
        assert_eq!(line["context"]["otp"], "[REDACTED]");
    }
}
