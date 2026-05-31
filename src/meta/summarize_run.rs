#![allow(dead_code)] // public handle() invoked via main.rs dispatch
//! `hands_summarize_run` — agent-facing telemetry export tool.
//!
//! Resolves a breadcrumb id (full, short-prefix, or bare timestamp) to a
//! summary suitable for self-optimization: tools_used breakdown, total/avg
//! ms, success rate, failure steps, files_changed counts. Pure local-state:
//! no browser, no session, no async.
//!
//! Inputs come from two sources:
//!   1. Breadcrumb JSON at C:/My Drive/Volumes/breadcrumbs/{active,completed/<date>}/bc_*.json
//!   2. Instrumentation log at <resolved_log_dir>/hands_meta.jsonl
//!
//! Acceptance criterion (aspirational, warm cache): <50ms per breadcrumb_id.
//! Cold-cache timings are reported in the response's `timing` block so callers
//! can verify.

use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Instant;

const VOLUMES_BREADCRUMBS_ROOT: &str = r"C:\My Drive\Volumes\breadcrumbs";
const LEGACY_LOG_DIR: &str = r"C:\CPC\logs";
const LOG_FILE: &str = "hands_meta.jsonl";

/// Maximum failure_steps entries returned (cap to avoid explosion on long runs).
const FAILURE_STEPS_CAP: usize = 50;

/// Public entry point — called by main.rs dispatch.
pub fn handle(args: &Value) -> Value {
    let input = match args.get("breadcrumb_id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s,
        _ => {
            return json!({
                "ok": false,
                "error": "missing required parameter: breadcrumb_id",
                "breadcrumb_id_input": Value::Null
            });
        }
    };

    let breadcrumbs_root = Path::new(VOLUMES_BREADCRUMBS_ROOT);
    let log_path = resolve_log_path();
    _summarize_from_paths(input, breadcrumbs_root, log_path.as_deref())
}

/// Resolve the hands_meta.jsonl path via the same legacy-fallback logic used by
/// instrumentation.rs. We don't duplicate the resolver — we call it directly.
fn resolve_log_path() -> Option<PathBuf> {
    crate::meta::instrumentation::_resolve_hands_log_dir(Path::new(LEGACY_LOG_DIR))
        .ok()
        .map(|dir| dir.join(LOG_FILE))
}

/// Testable inner function — accepts injected paths so tests can mock the
/// filesystem layout with tempdirs.
pub(crate) fn _summarize_from_paths(
    input: &str,
    breadcrumbs_root: &Path,
    log_path: Option<&Path>,
) -> Value {
    let overall_start = Instant::now();

    // === 1. Normalize breadcrumb id and locate the file ===
    let bc_lookup_start = Instant::now();
    let normalized = normalize_breadcrumb_id(input);

    let lookup_result = find_breadcrumb_file(&normalized, breadcrumbs_root);
    let bc_lookup_ms = bc_lookup_start.elapsed().as_millis() as u64;

    let (bc_path, location) = match lookup_result {
        Ok((p, loc)) => (p, loc),
        Err(LookupError::NotFound) => {
            return json!({
                "ok": false,
                "error": "breadcrumb_id not found in active/ or completed/<date>/",
                "breadcrumb_id_input": input,
                "normalized": normalized,
                "timing": {
                    "breadcrumb_lookup_ms": bc_lookup_ms,
                    "instrumentation_scan_ms": 0,
                    "total_ms": overall_start.elapsed().as_millis() as u64
                }
            });
        }
        Err(LookupError::Ambiguous(matches)) => {
            return json!({
                "ok": false,
                "error": "ambiguous prefix",
                "breadcrumb_id_input": input,
                "normalized": normalized,
                "matches": matches,
                "timing": {
                    "breadcrumb_lookup_ms": bc_lookup_ms,
                    "instrumentation_scan_ms": 0,
                    "total_ms": overall_start.elapsed().as_millis() as u64
                }
            });
        }
    };

    // === 2. Parse the breadcrumb JSON ===
    let raw = match fs::read_to_string(&bc_path) {
        Ok(s) => s,
        Err(e) => {
            return json!({
                "ok": false,
                "error": format!("failed to read breadcrumb file: {}", e),
                "breadcrumb_id_input": input,
                "path": bc_path.display().to_string()
            });
        }
    };
    let bc_json: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            return json!({
                "ok": false,
                "error": format!("breadcrumb JSON unparseable: {}", e),
                "breadcrumb_id_input": input,
                "path": bc_path.display().to_string()
            });
        }
    };

    let bc_summary = build_breadcrumb_summary(&bc_json, &location);

    // === 3. Determine time window and scan instrumentation log ===
    let started_at_ms = bc_summary
        .get("started_at")
        .and_then(|v| v.as_str())
        .and_then(parse_rfc3339_ms);
    let last_activity_ms = bc_summary
        .get("last_activity_at")
        .and_then(|v| v.as_str())
        .and_then(parse_rfc3339_ms);

    let scan_start = Instant::now();
    let scan = scan_instrumentation_log(log_path, started_at_ms, last_activity_ms);
    let scan_ms = scan_start.elapsed().as_millis() as u64;

    // === 4. Assemble response ===
    let total_ms = overall_start.elapsed().as_millis() as u64;
    json!({
        "ok": true,
        "breadcrumb": bc_summary,
        "tools_used": scan.tools_used,
        "totals": scan.totals,
        "failure_steps": scan.failure_steps,
        "tokens_in": Value::Null,
        "tokens_out": Value::Null,
        "note": scan.note,
        "timing": {
            "breadcrumb_lookup_ms": bc_lookup_ms,
            "instrumentation_scan_ms": scan_ms,
            "total_ms": total_ms
        }
    })
}

// ─────────────────────────────── breadcrumb id ────────────────────────────────

/// Normalize a breadcrumb id input:
///   - "bc_1780092049_..." → unchanged
///   - "bc_1780092049"     → unchanged (prefix matching handled at lookup time)
///   - "1780092049"        → "bc_1780092049"
///   - "  bc_xyz  "        → trim whitespace
pub(crate) fn normalize_breadcrumb_id(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.starts_with("bc_") {
        trimmed.to_string()
    } else if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        format!("bc_{}", trimmed)
    } else {
        trimmed.to_string()
    }
}

#[derive(Debug)]
enum LookupError {
    NotFound,
    Ambiguous(Vec<String>),
}

/// Find the breadcrumb file by id, scanning active/ first then completed/<date>/.
///
/// Lookup order:
///   1. active/<id>.json (or active/<prefix>*.json)
///   2. completed/<today>/, completed/<yesterday>/, completed/last 7 days
///   3. Full completed/ directory scan as fallback
///
/// Returns (path, location_label like "active" or "completed/2026-05-30").
fn find_breadcrumb_file(
    normalized: &str,
    breadcrumbs_root: &Path,
) -> Result<(PathBuf, String), LookupError> {
    // Exact-match candidate filename
    let filename = format!("{}.json", normalized);

    // 1. Check active/
    let active_dir = breadcrumbs_root.join("active");
    let exact = active_dir.join(&filename);
    if exact.is_file() {
        return Ok((exact, "active".to_string()));
    }
    if let Some(found) = scan_dir_for_prefix(&active_dir, normalized) {
        match found {
            ScanResult::Single(p) => return Ok((p, "active".to_string())),
            ScanResult::Multiple(names) => return Err(LookupError::Ambiguous(names)),
        }
    }

    // 2. Check completed/<date>/ — today, yesterday, then last 7 days, then full
    let completed_dir = breadcrumbs_root.join("completed");
    let today = Utc::now().date_naive();
    let mut tried = Vec::new();

    for delta in 0..=7 {
        let date = today - chrono::Duration::days(delta);
        let date_str = date.format("%Y-%m-%d").to_string();
        let date_dir = completed_dir.join(&date_str);
        if !date_dir.is_dir() {
            continue;
        }
        tried.push(date_str.clone());

        let exact = date_dir.join(&filename);
        if exact.is_file() {
            return Ok((exact, format!("completed/{}", date_str)));
        }

        if let Some(found) = scan_dir_for_prefix(&date_dir, normalized) {
            match found {
                ScanResult::Single(p) => return Ok((p, format!("completed/{}", date_str))),
                ScanResult::Multiple(names) => return Err(LookupError::Ambiguous(names)),
            }
        }
    }

    // 3. Full fallback scan — every date dir under completed/
    if let Ok(entries) = fs::read_dir(&completed_dir) {
        let mut all_matches: Vec<(PathBuf, String, String)> = Vec::new(); // (path, location, name)
        for entry in entries.flatten() {
            let date_dir = entry.path();
            if !date_dir.is_dir() {
                continue;
            }
            let date_str = entry.file_name().to_string_lossy().to_string();
            if tried.contains(&date_str) {
                continue; // already scanned in the 7-day window
            }

            let exact = date_dir.join(&filename);
            if exact.is_file() {
                return Ok((exact, format!("completed/{}", date_str)));
            }

            // Collect prefix matches across the whole tree to detect ambiguity
            if let Ok(files) = fs::read_dir(&date_dir) {
                for f in files.flatten() {
                    let n = f.file_name().to_string_lossy().to_string();
                    if let Some(stem) = n.strip_suffix(".json") {
                        if stem.starts_with(normalized) {
                            all_matches.push((
                                f.path(),
                                format!("completed/{}", date_str),
                                stem.to_string(),
                            ));
                        }
                    }
                }
            }
        }

        match all_matches.len() {
            0 => Err(LookupError::NotFound),
            1 => {
                let (p, loc, _) = all_matches.into_iter().next().unwrap();
                Ok((p, loc))
            }
            _ => Err(LookupError::Ambiguous(
                all_matches.into_iter().map(|(_, _, n)| n).collect(),
            )),
        }
    } else {
        Err(LookupError::NotFound)
    }
}

enum ScanResult {
    Single(PathBuf),
    Multiple(Vec<String>),
}

/// Scan a single directory for filenames starting with `prefix` (followed by
/// anything before .json). Returns Single if exactly one match, Multiple if
/// more than one, None if zero.
fn scan_dir_for_prefix(dir: &Path, prefix: &str) -> Option<ScanResult> {
    let entries = fs::read_dir(dir).ok()?;
    let mut matches: Vec<(PathBuf, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let stem = match name.strip_suffix(".json") {
            Some(s) => s,
            None => continue,
        };
        if stem.starts_with(prefix) {
            matches.push((entry.path(), stem.to_string()));
        }
    }
    match matches.len() {
        0 => None,
        1 => Some(ScanResult::Single(matches.into_iter().next().unwrap().0)),
        _ => Some(ScanResult::Multiple(
            matches.into_iter().map(|(_, n)| n).collect(),
        )),
    }
}

// ───────────────────────────── breadcrumb summary ─────────────────────────────

/// Build the `breadcrumb` summary object from a parsed breadcrumb JSON.
fn build_breadcrumb_summary(bc: &Value, location: &str) -> Value {
    let started_at = bc.get("started_at").and_then(|v| v.as_str()).unwrap_or("");
    let last_activity_at = bc
        .get("last_activity_at")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let duration_ms = match (
        parse_rfc3339_ms(started_at),
        parse_rfc3339_ms(last_activity_at),
    ) {
        (Some(s), Some(e)) => (e - s).max(0) as u64,
        _ => 0,
    };

    let step_results_count = parse_maybe_double_encoded_array(bc.get("step_results"))
        .map(|arr| arr.len())
        .unwrap_or(0);
    let files_changed_count = parse_maybe_double_encoded_array(bc.get("files_changed"))
        .map(|arr| arr.len())
        .unwrap_or(0);

    json!({
        "id": bc.get("id").cloned().unwrap_or(Value::Null),
        "name": bc.get("name").cloned().unwrap_or(Value::Null),
        "owner": bc.get("owner").cloned().unwrap_or(Value::Null),
        "writer_actor": bc.get("writer_actor").cloned().unwrap_or(Value::Null),
        "writer_session": bc.get("writer_session").cloned().unwrap_or(Value::Null),
        "started_at": started_at,
        "last_activity_at": last_activity_at,
        "duration_ms": duration_ms,
        "current_step": bc.get("current_step").cloned().unwrap_or(Value::Null),
        "total_steps": bc.get("total_steps").cloned().unwrap_or(Value::Null),
        "step_results_count": step_results_count,
        "files_changed_count": files_changed_count,
        "stale": bc.get("stale").cloned().unwrap_or(json!(false)),
        "aborted": bc.get("aborted").cloned().unwrap_or(json!(false)),
        "location": location
    })
}

/// Parse a field that *might* be a proper JSON array, *or* might be a
/// string-encoded JSON array (the breadcrumb double-encoding quirk). Returns
/// the array contents in either case, or None if it's neither.
pub(crate) fn parse_maybe_double_encoded_array(v: Option<&Value>) -> Option<Vec<Value>> {
    let v = v?;
    if let Some(arr) = v.as_array() {
        return Some(arr.clone());
    }
    if let Some(s) = v.as_str() {
        if let Ok(parsed) = serde_json::from_str::<Value>(s) {
            if let Some(arr) = parsed.as_array() {
                return Some(arr.clone());
            }
        }
    }
    None
}

// ──────────────────────────── instrumentation scan ────────────────────────────

struct ScanOutput {
    tools_used: Value,
    totals: Value,
    failure_steps: Value,
    note: Option<String>,
}

#[derive(Default)]
struct ToolStats {
    calls: u64,
    ok: u64,
    failed: u64,
    ms_total: u64,
}

/// Stream the instrumentation log, filter aggregates by time window, and
/// aggregate per-tool counts. Rung attempts are skipped (they'd double-count).
fn scan_instrumentation_log(
    log_path: Option<&Path>,
    window_start_ms: Option<i64>,
    window_end_ms: Option<i64>,
) -> ScanOutput {
    let mut tools: std::collections::BTreeMap<String, ToolStats> =
        std::collections::BTreeMap::new();
    let mut total_calls: u64 = 0;
    let mut total_ok: u64 = 0;
    let mut total_failed: u64 = 0;
    let mut total_ms: u64 = 0;
    let mut failures: Vec<Value> = Vec::new();

    let path = match log_path {
        Some(p) => p,
        None => {
            return empty_scan_output(Some(
                "no instrumentation log present (resolver returned no path)".into(),
            ));
        }
    };

    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => {
            return empty_scan_output(Some("no instrumentation log present".into()));
        }
    };

    let metadata = fs::metadata(path).ok();
    let is_empty = metadata.map(|m| m.len() == 0).unwrap_or(false);
    if is_empty {
        return empty_scan_output(Some("no instrumentation log present".into()));
    }

    let reader = BufReader::new(file);

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let rec: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue, // skip malformed lines silently
        };

        // Skip rung attempts — only aggregates count toward tools_used totals.
        let is_aggregate = rec
            .get("aggregate")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !is_aggregate {
            continue;
        }

        // Parse timestamp and filter by window
        let ts_ms = match rec
            .get("ts")
            .and_then(|v| v.as_str())
            .and_then(parse_rfc3339_ms)
        {
            Some(t) => t,
            None => continue,
        };
        if let Some(start) = window_start_ms {
            if ts_ms < start {
                continue;
            }
        }
        if let Some(end) = window_end_ms {
            if ts_ms > end {
                continue;
            }
        }

        let tool = match rec.get("tool").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => continue,
        };
        let success = rec
            .get("success")
            .or_else(|| rec.get("ok"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let ms = rec
            .get("elapsed_ms")
            .or_else(|| rec.get("ms"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let entry = tools.entry(tool.clone()).or_default();
        entry.calls += 1;
        if success {
            entry.ok += 1;
            total_ok += 1;
        } else {
            entry.failed += 1;
            total_failed += 1;
        }
        entry.ms_total = entry.ms_total.saturating_add(ms);
        total_calls += 1;
        total_ms = total_ms.saturating_add(ms);

        if !success {
            let error_field = rec.get("error").cloned().unwrap_or(Value::Null);
            let error_str = match &error_field {
                Value::Null => None,
                Value::String(s) => Some(s.clone()),
                other => Some(other.to_string()),
            };
            failures.push(json!({
                "tool": tool,
                "call_id": rec.get("call_id").cloned().unwrap_or(Value::Null),
                "rungs_tried": rec.get("rungs_tried").cloned().unwrap_or(json!(0)),
                "selected_rung": rec.get("method").cloned().unwrap_or(Value::Null),
                "error": error_str,
                "ts": rec.get("ts").cloned().unwrap_or(Value::Null)
            }));
        }
    }

    // Cap failure_steps at FAILURE_STEPS_CAP most-recent entries
    if failures.len() > FAILURE_STEPS_CAP {
        // Records are written chronologically; keep the tail.
        let drop = failures.len() - FAILURE_STEPS_CAP;
        failures.drain(0..drop);
    }

    // Build tools_used object
    let mut tools_used = Map::new();
    for (name, stats) in tools.iter() {
        let ms_avg = if stats.calls > 0 {
            (stats.ms_total as f64) / (stats.calls as f64)
        } else {
            0.0
        };
        tools_used.insert(
            name.clone(),
            json!({
                "calls": stats.calls,
                "ok": stats.ok,
                "failed": stats.failed,
                "ms_total": stats.ms_total,
                "ms_avg": round_2(ms_avg)
            }),
        );
    }

    let success_rate = if total_calls > 0 {
        round_3((total_ok as f64) / (total_calls as f64))
    } else {
        0.0
    };

    let note = if total_calls == 0 {
        Some("no aggregate records in window".into())
    } else {
        None
    };

    ScanOutput {
        tools_used: Value::Object(tools_used),
        totals: json!({
            "tool_calls": total_calls,
            "ms_total": total_ms,
            "success_count": total_ok,
            "failure_count": total_failed,
            "success_rate": success_rate
        }),
        failure_steps: Value::Array(failures),
        note,
    }
}

fn empty_scan_output(note: Option<String>) -> ScanOutput {
    ScanOutput {
        tools_used: json!({}),
        totals: json!({
            "tool_calls": 0,
            "ms_total": 0,
            "success_count": 0,
            "failure_count": 0,
            "success_rate": 0.0
        }),
        failure_steps: json!([]),
        note,
    }
}

// ───────────────────────────────── helpers ────────────────────────────────────

fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc).timestamp_millis())
}

fn round_2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn round_3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

// ───────────────────────────────── tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use tempfile::tempdir;

    // ── id normalization ──

    #[test]
    fn breadcrumb_id_normalize_accepts_full_id() {
        let id = "bc_1780092049_bootstrap_topic_manifest_domains_toc_aut";
        assert_eq!(normalize_breadcrumb_id(id), id);
    }

    #[test]
    fn breadcrumb_id_normalize_accepts_short_prefix() {
        let id = "bc_1780092049";
        assert_eq!(normalize_breadcrumb_id(id), id);
    }

    #[test]
    fn breadcrumb_id_normalize_accepts_bare_timestamp() {
        assert_eq!(normalize_breadcrumb_id("1780092049"), "bc_1780092049");
    }

    #[test]
    fn breadcrumb_id_normalize_trims_whitespace() {
        assert_eq!(normalize_breadcrumb_id("  bc_xyz  "), "bc_xyz");
    }

    // ── double-encoded array quirk ──

    #[test]
    fn parse_double_encoded_json_array_field() {
        // Proper array
        let proper = json!(["a", "b", "c"]);
        let parsed = parse_maybe_double_encoded_array(Some(&proper)).unwrap();
        assert_eq!(parsed.len(), 3);

        // String-encoded array
        let encoded = json!(r#"["x", "y"]"#);
        let parsed = parse_maybe_double_encoded_array(Some(&encoded)).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], json!("x"));

        // Null/None
        assert!(parse_maybe_double_encoded_array(None).is_none());
        let nullv = Value::Null;
        assert!(parse_maybe_double_encoded_array(Some(&nullv)).is_none());

        // Non-array string (garbage)
        let garbage = json!("not an array");
        assert!(parse_maybe_double_encoded_array(Some(&garbage)).is_none());
    }

    // ── instrumentation window filtering ──

    fn write_log(path: &Path, lines: &[Value]) {
        let mut f = std::fs::File::create(path).unwrap();
        for line in lines {
            writeln!(f, "{}", serde_json::to_string(line).unwrap()).unwrap();
        }
    }

    #[test]
    fn instrumentation_aggregate_inside_window_counted() {
        let dir = tempdir().unwrap();
        let log = dir.path().join("hands_meta.jsonl");
        write_log(
            &log,
            &[json!({
                "aggregate": true,
                "tool": "hands_click",
                "call_id": "call_a",
                "success": true,
                "method": "a11y",
                "rungs_tried": 1,
                "elapsed_ms": 100,
                "ts": "2026-05-30T12:00:00Z"
            })],
        );

        // Window covers 2026-05-30 noon
        let start = parse_rfc3339_ms("2026-05-30T11:00:00Z");
        let end = parse_rfc3339_ms("2026-05-30T13:00:00Z");
        let out = scan_instrumentation_log(Some(&log), start, end);

        let tools = out.tools_used.as_object().unwrap();
        assert!(tools.contains_key("hands_click"));
        let click = &tools["hands_click"];
        assert_eq!(click["calls"], 1);
        assert_eq!(click["ok"], 1);
        assert_eq!(out.totals["tool_calls"], 1);
    }

    #[test]
    fn instrumentation_aggregate_outside_window_skipped() {
        let dir = tempdir().unwrap();
        let log = dir.path().join("hands_meta.jsonl");
        write_log(
            &log,
            &[
                json!({
                    "aggregate": true,
                    "tool": "hands_click",
                    "call_id": "call_old",
                    "success": true,
                    "method": "a11y",
                    "rungs_tried": 1,
                    "elapsed_ms": 100,
                    "ts": "2020-01-01T00:00:00Z"
                }),
                json!({
                    "aggregate": true,
                    "tool": "hands_type",
                    "call_id": "call_in",
                    "success": true,
                    "method": "browser_type",
                    "rungs_tried": 1,
                    "elapsed_ms": 50,
                    "ts": "2026-05-30T12:30:00Z"
                }),
            ],
        );
        let start = parse_rfc3339_ms("2026-05-30T11:00:00Z");
        let end = parse_rfc3339_ms("2026-05-30T13:00:00Z");
        let out = scan_instrumentation_log(Some(&log), start, end);

        let tools = out.tools_used.as_object().unwrap();
        assert!(!tools.contains_key("hands_click"));
        assert!(tools.contains_key("hands_type"));
        assert_eq!(out.totals["tool_calls"], 1);
    }

    #[test]
    fn instrumentation_rung_attempts_not_counted() {
        let dir = tempdir().unwrap();
        let log = dir.path().join("hands_meta.jsonl");
        write_log(
            &log,
            &[
                // Rung attempt — should be skipped
                json!({
                    "tool": "hands_click",
                    "call_id": "call_x",
                    "rung": "a11y",
                    "success": true,
                    "elapsed_ms": 50,
                    "ts": "2026-05-30T12:00:00Z"
                }),
                // Aggregate — should be counted
                json!({
                    "aggregate": true,
                    "tool": "hands_click",
                    "call_id": "call_x",
                    "success": true,
                    "method": "a11y",
                    "rungs_tried": 1,
                    "elapsed_ms": 50,
                    "ts": "2026-05-30T12:00:00Z"
                }),
            ],
        );
        let out = scan_instrumentation_log(Some(&log), None, None);
        assert_eq!(out.totals["tool_calls"], 1);
    }

    #[test]
    fn failure_steps_capped_at_50() {
        let dir = tempdir().unwrap();
        let log = dir.path().join("hands_meta.jsonl");
        let mut lines: Vec<Value> = Vec::new();
        for i in 0..60 {
            lines.push(json!({
                "aggregate": true,
                "tool": "hands_click",
                "call_id": format!("call_{}", i),
                "success": false,
                "method": "",
                "rungs_tried": 7,
                "elapsed_ms": 1,
                "error": format!("fail {}", i),
                "ts": "2026-05-30T12:00:00Z"
            }));
        }
        write_log(&log, &lines);

        let out = scan_instrumentation_log(Some(&log), None, None);
        let failures = out.failure_steps.as_array().unwrap();
        assert_eq!(failures.len(), 50);
        // The cap drops the OLDEST and keeps the most recent 50 — the last
        // entry should still be call_59 (the most-recent in the file).
        assert_eq!(failures.last().unwrap()["call_id"], "call_59");
        // The first kept entry should be call_10 (60 records, drop 10).
        assert_eq!(failures.first().unwrap()["call_id"], "call_10");
    }

    #[test]
    fn empty_instrumentation_log_returns_note() {
        let dir = tempdir().unwrap();
        let log = dir.path().join("hands_meta.jsonl");
        // Create empty file
        std::fs::write(&log, "").unwrap();
        let out = scan_instrumentation_log(Some(&log), None, None);
        assert!(out.note.is_some());
        assert!(out.note.unwrap().contains("no instrumentation log"));
    }

    #[test]
    fn missing_instrumentation_log_returns_note() {
        let out = scan_instrumentation_log(
            Some(Path::new("nonexistent_path_for_test.jsonl")),
            None,
            None,
        );
        assert!(out.note.is_some());
    }

    // ── breadcrumb lookup ──

    fn write_breadcrumb(dir: &Path, id: &str, started: &str, last: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let bc = json!({
            "id": id,
            "name": format!("test breadcrumb {}", id),
            "owner": "claude",
            "writer_actor": "claude",
            "writer_session": "sess_test",
            "started_at": started,
            "last_activity_at": last,
            "current_step": 1,
            "total_steps": 4,
            "step_results": [{"step_idx": 0}],
            "files_changed": ["a.rs", "b.rs"],
            "stale": false,
            "aborted": false
        });
        std::fs::write(
            dir.join(format!("{}.json", id)),
            serde_json::to_string_pretty(&bc).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn lookup_finds_active_breadcrumb_exact() {
        let dir = tempdir().unwrap();
        let active = dir.path().join("active");
        write_breadcrumb(
            &active,
            "bc_1780241128_test",
            "2026-05-31T15:00:00Z",
            "2026-05-31T15:30:00Z",
        );
        let result = _summarize_from_paths("bc_1780241128_test", dir.path(), None);
        assert_eq!(result["ok"], true);
        assert_eq!(result["breadcrumb"]["location"], "active");
        assert_eq!(result["breadcrumb"]["files_changed_count"], 2);
        assert_eq!(result["breadcrumb"]["step_results_count"], 1);
    }

    #[test]
    fn lookup_finds_active_by_short_prefix() {
        let dir = tempdir().unwrap();
        let active = dir.path().join("active");
        write_breadcrumb(
            &active,
            "bc_1780241128_test",
            "2026-05-31T15:00:00Z",
            "2026-05-31T15:30:00Z",
        );
        let result = _summarize_from_paths("bc_1780241128", dir.path(), None);
        assert_eq!(result["ok"], true);
    }

    #[test]
    fn lookup_finds_active_by_bare_timestamp() {
        let dir = tempdir().unwrap();
        let active = dir.path().join("active");
        write_breadcrumb(
            &active,
            "bc_1780241128_test",
            "2026-05-31T15:00:00Z",
            "2026-05-31T15:30:00Z",
        );
        let result = _summarize_from_paths("1780241128", dir.path(), None);
        assert_eq!(result["ok"], true);
    }

    #[test]
    fn lookup_ambiguous_prefix_returns_error() {
        let dir = tempdir().unwrap();
        let active = dir.path().join("active");
        write_breadcrumb(
            &active,
            "bc_1780241128_test_a",
            "2026-05-31T15:00:00Z",
            "2026-05-31T15:30:00Z",
        );
        write_breadcrumb(
            &active,
            "bc_1780241128_test_b",
            "2026-05-31T15:00:00Z",
            "2026-05-31T15:30:00Z",
        );
        let result = _summarize_from_paths("bc_1780241128", dir.path(), None);
        assert_eq!(result["ok"], false);
        assert_eq!(result["error"], "ambiguous prefix");
        let matches = result["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn lookup_not_found_returns_error() {
        let dir = tempdir().unwrap();
        // Create empty structure
        std::fs::create_dir_all(dir.path().join("active")).unwrap();
        std::fs::create_dir_all(dir.path().join("completed")).unwrap();
        let result = _summarize_from_paths("bc_nope", dir.path(), None);
        assert_eq!(result["ok"], false);
        assert!(result["error"].as_str().unwrap().contains("not found"));
    }

    #[test]
    fn lookup_finds_completed_breadcrumb() {
        let dir = tempdir().unwrap();
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let completed = dir.path().join("completed").join(&today);
        write_breadcrumb(
            &completed,
            "bc_1780000000_done",
            "2026-05-31T10:00:00Z",
            "2026-05-31T11:00:00Z",
        );
        let result = _summarize_from_paths("bc_1780000000_done", dir.path(), None);
        assert_eq!(result["ok"], true);
        assert_eq!(
            result["breadcrumb"]["location"],
            format!("completed/{}", today)
        );
    }

    #[test]
    fn duration_ms_computed_correctly() {
        let dir = tempdir().unwrap();
        let active = dir.path().join("active");
        write_breadcrumb(
            &active,
            "bc_dur",
            "2026-05-31T15:00:00Z",
            "2026-05-31T15:00:05Z", // 5 seconds = 5000ms
        );
        let result = _summarize_from_paths("bc_dur", dir.path(), None);
        assert_eq!(result["ok"], true);
        assert_eq!(result["breadcrumb"]["duration_ms"], 5000);
    }
}
