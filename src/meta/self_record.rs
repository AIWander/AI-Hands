//! Self-record + replay loop (Phase D — Self-improvement).
//!
//! Three tools that wrap workflow:flow_record_* to give hands a
//! cosine-similarity-indexed memory of past automation sessions. The hands
//! server NEVER calls workflow MCP tools directly — it returns *call plans*
//! for the caller (Claude) to invoke. This is a load-bearing architectural
//! rule (Operating_system_architecture.md): hands is plan-not-action wrt
//! workflow:*.
//!
//! ## Tools
//! - `hands_self_record_start`              — generate flow_name, persist intent
//! - `hands_self_record_lookup`             — cosine-similarity search prior records
//! - `hands_self_record_stop_and_optimize`  — two-stage orchestrator: stop, replay
//!                                             (dry_run), prune, optionally replace
//!
//! ## Storage
//! Records live in `cpc_paths::data_path("hands")/self_records.json`. Writes are
//! atomic (write `.tmp` then rename), matching the workflow MCP pattern documented
//! in Operating_workflow.md.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

// ──────────────────────────────────────────────────────────────────────
// Constants
// ──────────────────────────────────────────────────────────────────────

const SELF_RECORDS_FILE: &str = "self_records.json";
const DEFAULT_SIMILARITY_THRESHOLD: f32 = 0.7;
const FLOW_NAME_PREFIX: &str = "self_";
const FLOW_NAME_HASH_LEN: usize = 16;
const MARKER: &str = "hands_self_record";
/// Threshold (ms) below which a `browser_wait_for` is considered an "instant"
/// no-op — the target element was already present and the wait can be pruned.
const INSTANT_WAIT_THRESHOLD_MS: u64 = 100;

/// ~50 common English stopwords. Compiled once into a HashSet on demand.
/// Keep this list stable — changing it changes tokenization, which changes
/// cosine similarity scores against records already on disk.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "is", "are", "was", "were", "be", "been", "being",
    "have", "has", "had", "do", "does", "did", "will", "would", "should", "could", "can", "may",
    "might", "must", "shall", "to", "of", "in", "on", "at", "by", "for", "with", "about",
    "against", "between", "into", "through", "during", "before", "after", "above", "below", "from",
    "up", "down", "out", "off", "over", "under", "again", "further", "then", "once",
];

// ──────────────────────────────────────────────────────────────────────
// Record schema
// ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfRecord {
    pub flow_name: String,
    pub task_description: String,
    pub tokens: Vec<String>,
    pub created_at: i64,
    #[serde(default)]
    pub step_count: Option<u32>,
    #[serde(default)]
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct SelfRecordsFile {
    #[serde(default)]
    records: Vec<SelfRecord>,
}

// ──────────────────────────────────────────────────────────────────────
// Tokenization
// ──────────────────────────────────────────────────────────────────────

/// Lowercase, split on whitespace + ASCII punctuation, drop stopwords + empties.
/// Stable: same input always yields same output (no randomness, no hashing).
fn tokenize(text: &str) -> Vec<String> {
    let stopwords: std::collections::HashSet<&str> = STOPWORDS.iter().copied().collect();
    text.to_lowercase()
        .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .filter(|t| !t.is_empty() && !stopwords.contains(t))
        .map(|t| t.to_string())
        .collect()
}

/// Build a term-frequency vector for cosine similarity.
fn term_frequencies(tokens: &[String]) -> HashMap<String, usize> {
    let mut tf = HashMap::new();
    for tok in tokens {
        *tf.entry(tok.clone()).or_insert(0) += 1;
    }
    tf
}

/// Cosine similarity between two term-frequency vectors.
/// Returns 0.0 if either vector is empty.
fn cosine_similarity(a: &HashMap<String, usize>, b: &HashMap<String, usize>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    // Dot product — iterate the smaller map for fewer lookups.
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let mut dot: f64 = 0.0;
    for (term, &count) in small.iter() {
        if let Some(&other) = large.get(term) {
            dot += (count as f64) * (other as f64);
        }
    }
    let norm_a: f64 = a
        .values()
        .map(|&c| (c as f64) * (c as f64))
        .sum::<f64>()
        .sqrt();
    let norm_b: f64 = b
        .values()
        .map(|&c| (c as f64) * (c as f64))
        .sum::<f64>()
        .sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    (dot / (norm_a * norm_b)) as f32
}

// ──────────────────────────────────────────────────────────────────────
// Flow name generation
// ──────────────────────────────────────────────────────────────────────

/// FNV-1a 64-bit — small, no_std-friendly, no external deps.
/// Used purely as a deterministic-given-input flow-name suffix. NOT a security hash.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Generate a deterministic flow_name: `self_` + first 16 hex chars of
/// FNV-1a(task_description + timestamp_secs).
///
/// Deviation from spec: spec calls for SHA-256, but `sha2` is not a dep and
/// the spec forbids adding new external crates. FNV-1a is sufficient here
/// because the timestamp is mixed in and collisions are checked against the
/// existing records before write.
fn generate_flow_name(task_description: &str, timestamp_secs: i64) -> String {
    let mut buf = Vec::with_capacity(task_description.len() + 16);
    buf.extend_from_slice(task_description.as_bytes());
    buf.extend_from_slice(&timestamp_secs.to_le_bytes());
    let h = fnv1a_64(&buf);
    let hex = format!("{:016x}", h);
    format!(
        "{}{}",
        FLOW_NAME_PREFIX,
        &hex[..FLOW_NAME_HASH_LEN.min(hex.len())]
    )
}

// ──────────────────────────────────────────────────────────────────────
// Storage
// ──────────────────────────────────────────────────────────────────────

fn records_path() -> Result<PathBuf, String> {
    let dir = cpc_paths::data_path("hands")
        .map_err(|e| format!("cpc_paths::data_path(hands) failed: {}", e))?;
    Ok(dir.join(SELF_RECORDS_FILE))
}

fn load_records() -> Result<SelfRecordsFile, String> {
    let path = records_path()?;
    if !path.exists() {
        return Ok(SelfRecordsFile::default());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    if raw.trim().is_empty() {
        return Ok(SelfRecordsFile::default());
    }
    serde_json::from_str(&raw).map_err(|e| format!("parse {}: {}", path.display(), e))
}

/// Atomic save: write `.tmp` then rename.
fn save_records(file: &SelfRecordsFile) -> Result<(), String> {
    let path = records_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("create dir {}: {}", parent.display(), e))?;
    }
    let tmp = path.with_extension("json.tmp");
    let serialized =
        serde_json::to_string_pretty(file).map_err(|e| format!("serialize records: {}", e))?;
    fs::write(&tmp, serialized).map_err(|e| format!("write {}: {}", tmp.display(), e))?;
    fs::rename(&tmp, &path)
        .map_err(|e| format!("rename {} -> {}: {}", tmp.display(), path.display(), e))?;
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────
// Public handlers
// ──────────────────────────────────────────────────────────────────────

/// `hands_self_record_start` — generate flow_name, persist intent, return
/// the workflow:flow_record_start call plan for the caller to invoke.
pub async fn handle_self_record_start(args: &Value) -> Value {
    let task_description = match args.get("task_description").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return err("missing required parameter: task_description (non-empty string)"),
    };

    let now = chrono::Utc::now().timestamp();
    let flow_name = generate_flow_name(&task_description, now);
    let tokens = tokenize(&task_description);

    let record = SelfRecord {
        flow_name: flow_name.clone(),
        task_description: task_description.clone(),
        tokens,
        created_at: now,
        step_count: None,
        last_used_at: None,
    };

    let mut file = match load_records() {
        Ok(f) => f,
        Err(e) => return err(&format!("load self_records.json: {}", e)),
    };

    // Collision guard: if flow_name already exists, append a disambiguator.
    let mut effective_name = flow_name.clone();
    if file.records.iter().any(|r| r.flow_name == effective_name) {
        let suffix = fnv1a_64(format!("{}|{}", flow_name, now).as_bytes());
        effective_name = format!("{}_{:08x}", flow_name, suffix & 0xffff_ffff);
    }
    let mut record = record;
    record.flow_name = effective_name.clone();

    file.records.push(record);
    if let Err(e) = save_records(&file) {
        return err(&format!("persist self_records.json: {}", e));
    }

    json!({
        "flow_name": effective_name,
        "task_description": task_description,
        "flow_record_call_plan": {
            "tool": "workflow:flow_record_start",
            "args": { "name": effective_name, "description": task_description },
            "marker": MARKER
        },
        "hint": "Call workflow:flow_record_start with the provided args, then perform the task. Every hands_* / browser_* / uia_* / vision_* tool call between record_start and record_stop will be captured."
    })
}

/// `hands_self_record_lookup` — find prior records similar to `task_description`.
pub async fn handle_self_record_lookup(args: &Value) -> Value {
    let task_description = match args.get("task_description").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return err("missing required parameter: task_description (non-empty string)"),
    };
    let threshold = args
        .get("threshold")
        .and_then(|v| v.as_f64())
        .map(|f| f as f32)
        .unwrap_or(DEFAULT_SIMILARITY_THRESHOLD);

    let query_tokens = tokenize(&task_description);
    let query_tf = term_frequencies(&query_tokens);

    let file = match load_records() {
        Ok(f) => f,
        Err(e) => return err(&format!("load self_records.json: {}", e)),
    };
    let total_records_scanned = file.records.len();

    let mut candidates: Vec<Value> = Vec::new();
    for r in &file.records {
        let tf = term_frequencies(&r.tokens);
        let sim = cosine_similarity(&query_tf, &tf);
        if sim >= threshold {
            candidates.push(json!({
                "flow_name": r.flow_name,
                "similarity": sim,
                "task_description": r.task_description,
                "step_count": r.step_count,
                "last_used_at": r.last_used_at,
                "replay_call_plan": {
                    "tool": "workflow:flow_replay",
                    "args": { "name": r.flow_name },
                    "marker": MARKER
                }
            }));
        }
    }
    // Sort highest similarity first.
    candidates.sort_by(|a, b| {
        let sa = a.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let sb = b.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.0);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });

    json!({
        "task_description": task_description,
        "threshold": threshold,
        "candidates": candidates,
        "total_records_scanned": total_records_scanned,
        "hint": "If a candidate looks right, invoke its replay_call_plan to replay. If none look right, call hands_self_record_start to begin a fresh recording."
    })
}

/// `hands_self_record_stop_and_optimize` — two-stage orchestrator.
///
/// **Stage 1** (no `recorded_steps`): return a plan to call
/// `workflow:flow_record_stop` + `workflow:flow_replay(dry_run=true)`, then
/// re-invoke this tool with the replay output.
///
/// **Stage 2** (`recorded_steps` provided): prune the step list, look up
/// existing records by similarity, optionally return a replace plan if the
/// pruned version is shorter than the best existing match.
pub async fn handle_self_record_stop_and_optimize(args: &Value) -> Value {
    let name = match args.get("name").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return err("missing required parameter: name (non-empty string)"),
    };

    let recorded_steps = args.get("recorded_steps");

    // ──────────────── Stage 1: awaiting_replay ────────────────
    if recorded_steps.is_none() || recorded_steps == Some(&Value::Null) {
        return json!({
            "name": name,
            "stage": "awaiting_replay",
            "orchestration_plan": [
                { "tool": "workflow:flow_record_stop", "args": { "name": name }, "marker": MARKER },
                { "tool": "workflow:flow_replay",     "args": { "name": name, "dry_run": true }, "marker": MARKER }
            ],
            "callback_plan": {
                "tool": "hands_self_record_stop_and_optimize",
                "args": { "name": name, "recorded_steps": "<paste the steps array from flow_replay response>" },
                "marker": MARKER
            },
            "hint": "Execute the orchestration_plan in order, then call this tool again with recorded_steps populated from the flow_replay output."
        });
    }

    // ──────────────── Stage 2: prune + decide ────────────────
    let steps = match recorded_steps.unwrap().as_array() {
        Some(arr) => arr.clone(),
        None => return err("recorded_steps must be a JSON array"),
    };
    let original_step_count = steps.len();

    let pruned = prune_steps(&steps);
    let pruned_step_count = pruned.len();

    // Find the originally stored record for this `name` to get its task_description.
    let mut file = match load_records() {
        Ok(f) => f,
        Err(e) => return err(&format!("load self_records.json: {}", e)),
    };
    let stored_description = file
        .records
        .iter()
        .find(|r| r.flow_name == name)
        .map(|r| r.task_description.clone());

    // Update the current record's step_count + last_used_at.
    let now = chrono::Utc::now().timestamp();
    if let Some(rec) = file.records.iter_mut().find(|r| r.flow_name == name) {
        rec.step_count = Some(pruned_step_count as u32);
        rec.last_used_at = Some(now);
    }

    // Search for a better existing match via cosine similarity against the
    // stored description. If none stored, no replacement decision is possible.
    let best_match: Option<SelfRecord> = if let Some(desc) = stored_description.as_ref() {
        let query_tf = term_frequencies(&tokenize(desc));
        file.records
            .iter()
            .filter(|r| r.flow_name != name)
            .filter_map(|r| {
                let sim = cosine_similarity(&query_tf, &term_frequencies(&r.tokens));
                if sim >= DEFAULT_SIMILARITY_THRESHOLD {
                    Some((sim, r.clone()))
                } else {
                    None
                }
            })
            .max_by(|(a, _), (b, _)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(_, rec)| rec)
    } else {
        None
    };

    // Persist updated record metadata regardless of decision.
    if let Err(e) = save_records(&file) {
        return err(&format!("persist self_records.json: {}", e));
    }

    // Decision: replace existing if the best match has more steps than our pruned version.
    if let Some(best) = best_match.as_ref() {
        if let Some(existing_count) = best.step_count {
            if (existing_count as usize) > pruned_step_count {
                let description = stored_description.clone().unwrap_or_default();
                let mut plan: Vec<Value> = Vec::with_capacity(pruned.len() + 3);
                plan.push(json!({
                    "tool": "workflow:flow_delete",
                    "args": { "name": best.flow_name },
                    "marker": MARKER
                }));
                plan.push(json!({
                    "tool": "workflow:flow_record_start",
                    "args": { "name": name, "description": description },
                    "marker": MARKER
                }));
                for step in &pruned {
                    let tool_name = step
                        .get("tool_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    let tool_params = step.get("tool_params").cloned().unwrap_or(json!({}));
                    plan.push(json!({
                        "tool": "workflow:flow_record_step",
                        "args": { "name": name, "tool_name": tool_name, "tool_params": tool_params },
                        "marker": MARKER
                    }));
                }
                plan.push(json!({
                    "tool": "workflow:flow_record_stop",
                    "args": { "name": name },
                    "marker": MARKER
                }));

                return json!({
                    "name": name,
                    "stage": "complete",
                    "pruned_step_count": pruned_step_count,
                    "original_step_count": original_step_count,
                    "existing_best_match": {
                        "flow_name": best.flow_name,
                        "step_count": existing_count
                    },
                    "decision": "replace_existing",
                    "replace_plan": plan
                });
            }
        }
    }

    json!({
        "name": name,
        "stage": "complete",
        "pruned_step_count": pruned_step_count,
        "original_step_count": original_step_count,
        "decision": "keep",
        "existing_best_match": best_match.as_ref().map(|r| json!({
            "flow_name": r.flow_name,
            "step_count": r.step_count
        }))
    })
}

// ──────────────────────────────────────────────────────────────────────
// Pruning
// ──────────────────────────────────────────────────────────────────────

/// Apply the four prune heuristics from the spec, in order:
///
/// a. Drop consecutive `browser_scroll` calls with matching `scroll_to`.
/// b. When a click on a target FAILS and the next click on the same target
///    succeeds, drop the failure.
/// c. Drop `*_screenshot` calls whose `result` field is never referenced in
///    subsequent steps' `tool_params`.
/// d. Drop `browser_wait_for` calls with `timeout_ms <= INSTANT_WAIT_THRESHOLD_MS`
///    (the element was already present).
fn prune_steps(steps: &[Value]) -> Vec<Value> {
    let mut kept: Vec<Value> = Vec::with_capacity(steps.len());

    // ── Pass 1: heuristic (a) — collapse consecutive same-target browser_scroll
    for step in steps {
        if step_tool_name(step) == "browser_scroll" {
            if let Some(prev) = kept.last() {
                if step_tool_name(prev) == "browser_scroll" && scroll_targets_match(prev, step) {
                    continue; // skip duplicate scroll
                }
            }
        }
        kept.push(step.clone());
    }

    // ── Pass 2: heuristic (b) — drop failed click followed by successful click on same target
    let mut i = 0;
    let mut after_b: Vec<Value> = Vec::with_capacity(kept.len());
    while i < kept.len() {
        let step = &kept[i];
        let next = kept.get(i + 1);
        if is_click(step) && step_failed(step) {
            if let Some(n) = next {
                if is_click(n) && click_targets_match(step, n) {
                    // Drop the failure; keep the next (it'll be appended on the next iter)
                    i += 1;
                    continue;
                }
            }
        }
        after_b.push(step.clone());
        i += 1;
    }

    // ── Pass 3: heuristic (d) — drop instant browser_wait_for
    // (Do (d) before (c) so referenced-result detection runs on the final shape.)
    let after_d: Vec<Value> = after_b
        .into_iter()
        .filter(|s| !is_instant_wait_for(s))
        .collect();

    // ── Pass 4: heuristic (c) — drop unreferenced screenshots
    drop_unreferenced_screenshots(&after_d)
}

fn step_tool_name(step: &Value) -> &str {
    step.get("tool_name").and_then(|v| v.as_str()).unwrap_or("")
}

fn step_failed(step: &Value) -> bool {
    // workflow:flow_replay output typically includes a `success` or `result.success` field.
    if let Some(b) = step.get("success").and_then(|v| v.as_bool()) {
        return !b;
    }
    if let Some(b) = step
        .get("result")
        .and_then(|r| r.get("success"))
        .and_then(|v| v.as_bool())
    {
        return !b;
    }
    if let Some(e) = step.get("error") {
        if !e.is_null() {
            return true;
        }
    }
    false
}

fn is_click(step: &Value) -> bool {
    matches!(step_tool_name(step), "hands_click" | "browser_click")
}

fn click_targets_match(a: &Value, b: &Value) -> bool {
    let pa = a.get("tool_params").cloned().unwrap_or(json!({}));
    let pb = b.get("tool_params").cloned().unwrap_or(json!({}));
    for key in ["a11y_ref", "selector", "target", "match_text"] {
        let va = pa.get(key).and_then(|v| v.as_str());
        let vb = pb.get(key).and_then(|v| v.as_str());
        if let (Some(x), Some(y)) = (va, vb) {
            return x == y;
        }
    }
    false
}

fn scroll_targets_match(a: &Value, b: &Value) -> bool {
    let pa = a.get("tool_params").cloned().unwrap_or(json!({}));
    let pb = b.get("tool_params").cloned().unwrap_or(json!({}));
    let va = pa.get("scroll_to");
    let vb = pb.get("scroll_to");
    match (va, vb) {
        (Some(x), Some(y)) => x == y,
        // If both omit scroll_to, treat as matching (likely default scroll).
        (None, None) => true,
        _ => false,
    }
}

fn is_instant_wait_for(step: &Value) -> bool {
    if step_tool_name(step) != "browser_wait_for" {
        return false;
    }
    let params = match step.get("tool_params") {
        Some(p) => p,
        None => return false,
    };
    match params.get("timeout_ms").and_then(|v| v.as_u64()) {
        Some(t) => t <= INSTANT_WAIT_THRESHOLD_MS,
        None => false,
    }
}

/// Heuristic (c): drop `*_screenshot` calls whose `result` field is never
/// referenced by any subsequent step's `tool_params` (e.g. no later step
/// embeds the screenshot path/data).
fn drop_unreferenced_screenshots(steps: &[Value]) -> Vec<Value> {
    // Build the set of stringified tool_params of *all* steps. A screenshot is
    // "referenced" if any later step's tool_params string-contains the
    // screenshot's result text/path.
    let mut out: Vec<Value> = Vec::with_capacity(steps.len());

    for (idx, step) in steps.iter().enumerate() {
        let tn = step_tool_name(step);
        let is_screenshot = tn.ends_with("_screenshot") || tn == "vision_screenshot";
        if !is_screenshot {
            out.push(step.clone());
            continue;
        }

        // Build a list of substrings worth searching for in later params.
        let mut needles: Vec<String> = Vec::new();
        if let Some(r) = step.get("result") {
            collect_strings(r, &mut needles);
        }
        // Always also try the screenshot's own save_path arg if present.
        if let Some(p) = step
            .get("tool_params")
            .and_then(|p| p.get("save_path"))
            .and_then(|v| v.as_str())
        {
            needles.push(p.to_string());
        }
        // Drop empties / very short strings to avoid spurious matches.
        let needles: Vec<String> = needles.into_iter().filter(|n| n.len() >= 4).collect();

        // If we have no needle to search for, keep the screenshot (we can't
        // prove it's unreferenced).
        if needles.is_empty() {
            out.push(step.clone());
            continue;
        }

        let mut referenced = false;
        for later in &steps[idx + 1..] {
            let later_params = later.get("tool_params").cloned().unwrap_or(json!({}));
            let serialized = serde_json::to_string(&later_params).unwrap_or_default();
            if needles.iter().any(|n| serialized.contains(n)) {
                referenced = true;
                break;
            }
        }
        if referenced {
            out.push(step.clone());
        }
    }

    out
}

/// Recursively collect string leaves from a JSON value into `acc`.
fn collect_strings(v: &Value, acc: &mut Vec<String>) {
    match v {
        Value::String(s) => acc.push(s.clone()),
        Value::Array(arr) => {
            for x in arr {
                collect_strings(x, acc);
            }
        }
        Value::Object(map) => {
            for x in map.values() {
                collect_strings(x, acc);
            }
        }
        _ => {}
    }
}

// ──────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────

fn err(message: &str) -> Value {
    json!({
        "success": false,
        "error": message,
    })
}

// ──────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_drops_stopwords_and_punctuation() {
        let toks = tokenize("Login to Gmail, send a draft!");
        // "to" and "a" are stopwords; punctuation is stripped.
        assert_eq!(toks, vec!["login", "gmail", "send", "draft"]);
    }

    #[test]
    fn tokenize_lowercases() {
        let toks = tokenize("OPEN Chrome");
        assert_eq!(toks, vec!["open", "chrome"]);
    }

    #[test]
    fn cosine_zero_when_disjoint() {
        let a = term_frequencies(&tokenize("login gmail"));
        let b = term_frequencies(&tokenize("buy stocks"));
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn cosine_one_when_identical() {
        let a = term_frequencies(&tokenize("login gmail send"));
        let b = term_frequencies(&tokenize("login gmail send"));
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-5, "expected 1.0, got {}", sim);
    }

    #[test]
    fn cosine_below_threshold_for_loose_overlap() {
        // Spec example: "login to gmail and send a draft" vs "send gmail email"
        // After stopword removal: [login, gmail, send, draft] vs [send, gmail, email]
        // Dot = 2 (gmail + send), |a|=2, |b|=sqrt(3), cos = 2/(2*sqrt(3)) ≈ 0.577 < 0.7
        let a = term_frequencies(&tokenize("login to gmail and send a draft"));
        let b = term_frequencies(&tokenize("send gmail email"));
        let sim = cosine_similarity(&a, &b);
        assert!(
            sim < DEFAULT_SIMILARITY_THRESHOLD,
            "expected <0.7, got {}",
            sim
        );
        assert!(sim > 0.0, "expected >0.0, got {}", sim);
    }

    #[test]
    fn flow_name_is_deterministic_for_same_inputs() {
        let a = generate_flow_name("open chrome", 1730000000);
        let b = generate_flow_name("open chrome", 1730000000);
        assert_eq!(a, b);
        assert!(a.starts_with(FLOW_NAME_PREFIX));
        assert_eq!(a.len(), FLOW_NAME_PREFIX.len() + FLOW_NAME_HASH_LEN);
    }

    #[test]
    fn flow_name_differs_with_timestamp() {
        let a = generate_flow_name("open chrome", 1730000000);
        let b = generate_flow_name("open chrome", 1730000001);
        assert_ne!(a, b);
    }

    #[test]
    fn prune_drops_consecutive_scrolls() {
        let steps = vec![
            json!({"tool_name": "browser_scroll", "tool_params": {"scroll_to": "bottom"}}),
            json!({"tool_name": "browser_scroll", "tool_params": {"scroll_to": "bottom"}}),
            json!({"tool_name": "browser_scroll", "tool_params": {"scroll_to": "top"}}),
        ];
        let pruned = prune_steps(&steps);
        assert_eq!(pruned.len(), 2);
    }

    #[test]
    fn prune_drops_failed_then_successful_click() {
        let steps = vec![
            json!({
                "tool_name": "hands_click",
                "tool_params": {"target": "Submit"},
                "success": false
            }),
            json!({
                "tool_name": "hands_click",
                "tool_params": {"target": "Submit"},
                "success": true
            }),
        ];
        let pruned = prune_steps(&steps);
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0]["success"], true);
    }

    #[test]
    fn prune_drops_instant_wait_for() {
        let steps = vec![
            json!({"tool_name": "browser_wait_for", "tool_params": {"timeout_ms": 50}}),
            json!({"tool_name": "browser_wait_for", "tool_params": {"timeout_ms": 5000}}),
        ];
        let pruned = prune_steps(&steps);
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0]["tool_params"]["timeout_ms"], 5000);
    }

    #[test]
    fn prune_drops_unreferenced_screenshot() {
        let steps = vec![
            json!({
                "tool_name": "vision_screenshot",
                "tool_params": {"save_path": "C:/tmp/screenshot_abc.png"},
                "result": {"path": "C:/tmp/screenshot_abc.png"}
            }),
            json!({"tool_name": "hands_click", "tool_params": {"target": "Submit"}}),
        ];
        let pruned = prune_steps(&steps);
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0]["tool_name"], "hands_click");
    }

    #[test]
    fn prune_keeps_referenced_screenshot() {
        let steps = vec![
            json!({
                "tool_name": "vision_screenshot",
                "tool_params": {"save_path": "C:/tmp/screenshot_xyz.png"},
                "result": {"path": "C:/tmp/screenshot_xyz.png"}
            }),
            json!({
                "tool_name": "vision_ocr",
                "tool_params": {"image_path": "C:/tmp/screenshot_xyz.png"}
            }),
        ];
        let pruned = prune_steps(&steps);
        assert_eq!(pruned.len(), 2);
    }
}
