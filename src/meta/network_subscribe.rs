//! `hands_network_*` — Poll-based subscription model over `browser_get_network_log`
//! (Item 5, Milestone A).
//!
//! True CDP event streaming would require modifying the `browser-mcp` crate, so
//! this module wraps `browser_get_network_log` with cursor-based delta polling
//! and inline event filtering — all at the hands.exe layer. No background
//! polling, no new external crates.
//!
//! ## Tools
//!
//!   1. `hands_network_subscribe(filter?, history_capacity?)`
//!      → register a subscription, return `subscription_id` + cursor=0.
//!   2. `hands_network_poll(subscription_id, max_events?)`
//!      → fetch the network log, filter, return events strictly newer than the
//!      stored cursor; advance cursor.
//!   3. `hands_network_unsubscribe(subscription_id)`
//!      → remove a subscription. Returns ok:true even if not found.
//!   4. `hands_network_subscriptions()`
//!      → list all active subscriptions.
//!
//! ## Cursor strategy
//!
//! `browser_get_network_log` is backed by the `__mcp_intercepted` JS array
//! injected by the browser-mcp interception layer. Each entry is a JSON object
//! pushed in chronological order; there is **no stable `request_id` field** in
//! that response (the JS push records `{url, method, time, action}` only).
//!
//! The closest stable identifier is the **array index** within the per-poll
//! response. Because the underlying array is append-only between calls to
//! `clear_intercepted()`, the cursor is `last_seen_index + 1` and only the
//! tail of the array (entries at index ≥ cursor) is returned. We pass
//! `clear: false` so the log accumulates and the index is stable across polls.
//!
//! If the underlying response includes a numeric `request_id` field (defensive
//! check — some Phase-G feature work or browser-mcp upgrade may add one), the
//! cursor strategy upgrades transparently to "max(request_id) seen" via
//! `event_id_for`. The decision is per-event, not per-subscription.
//!
//! ## Filter shape
//!
//! ```json
//! {
//!   "url_glob": "**/api/**",
//!   "methods": ["GET", "POST"],
//!   "status_range": {"min": 400, "max": 599},
//!   "mime_contains": "json"
//! }
//! ```
//!
//! All fields are optional. Missing field = no filter on that axis.
//! Empty `methods: []` = match all (same as omitted).
//!
//! Glob: minimal `*` matcher implemented inline (matches any sequence including
//! empty). `**` collapses to `*`. Anchored: whole-string match.
//!
//! ## State
//!
//! Subscriptions live in a process-local `Mutex<Option<HashMap<...>>>`. No disk
//! persistence — restarting hands.exe clears all subscriptions.

#![allow(dead_code)] // public handle_* invoked via meta::handle_meta_tool dispatch

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use super::session::SharedSession;

// ── Tunables ──────────────────────────────────────────────────────────

/// Default per-poll event cap if the caller omits `history_capacity`.
const DEFAULT_HISTORY_CAPACITY: usize = 256;
/// Default `max_events` per `hands_network_poll` call (still capped by
/// the subscription's `history_capacity`).
const DEFAULT_MAX_EVENTS_PER_POLL: usize = 256;

// ── Public types ──────────────────────────────────────────────────────

/// Subscription filter. All fields are optional — a missing axis means
/// "no filter on this axis".
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Filter {
    /// Glob pattern for the request URL. Supports `*` (any sequence).
    /// `**` collapses to `*`. Anchored: whole-string match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_glob: Option<String>,
    /// Uppercase HTTP method whitelist. Empty array = no filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub methods: Option<Vec<String>>,
    /// Inclusive status code range. `None` = no filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_range: Option<StatusRange>,
    /// Substring match against `Content-Type` (case-insensitive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_contains: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StatusRange {
    pub min: u32,
    pub max: u32,
}

/// In-memory subscription record. Process-local — not persisted.
#[derive(Debug, Clone)]
pub struct Subscription {
    pub id: String,
    pub filter: Filter,
    pub history_capacity: usize,
    /// Monotonic cursor. Events with `event_id >= cursor` are "new".
    /// On first poll cursor=0; advances to `max(event_id) + 1` of returned events.
    pub cursor: u64,
    pub created_at: String,
    pub last_polled_at: Option<String>,
    pub events_seen: u64,
}

// ── Process-local store ───────────────────────────────────────────────

static SUBSCRIPTIONS: Mutex<Option<HashMap<String, Subscription>>> = Mutex::new(None);

fn with_subs<F, R>(f: F) -> R
where
    F: FnOnce(&mut HashMap<String, Subscription>) -> R,
{
    // Recover from poisoning silently — subscription state is process-local
    // and tolerating a panic in one tool call shouldn't take down the rest.
    let mut guard = SUBSCRIPTIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    f(guard.as_mut().expect("SUBSCRIPTIONS just initialized"))
}

// ── UUID v4-shape generator (duplicated from attach_lock, no crate) ───
//
// Per spec note: prefer duplicating over cross-module coupling so this and
// attach_lock stay independent.

fn new_subscription_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let elapsed = Instant::now().elapsed().as_nanos() as u64;
    let pid = std::process::id() as u64;

    // splitmix64 mix
    let mut state = nanos
        .wrapping_mul(0x9E3779B97F4A7C15)
        .wrapping_add(elapsed)
        .wrapping_add(pid << 17)
        .wrapping_add(counter.wrapping_mul(0xBF58476D1CE4E5B9));

    fn next(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    let mut bytes = [0u8; 16];
    let a = next(&mut state);
    let b = next(&mut state);
    bytes[0..8].copy_from_slice(&a.to_le_bytes());
    bytes[8..16].copy_from_slice(&b.to_le_bytes());

    // Set version (4) and variant (10xx) per RFC 4122.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

// ── Filter parsing & matching ─────────────────────────────────────────

fn parse_filter(v: Option<&Value>) -> Filter {
    let Some(v) = v else { return Filter::default() };
    let url_glob = v
        .get("url_glob")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    let methods = v.get("methods").and_then(|x| x.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|x| x.as_str().map(|s| s.to_uppercase()))
            .collect::<Vec<_>>()
    });
    let status_range = v.get("status_range").and_then(|x| {
        let min = x.get("min").and_then(|m| m.as_u64())? as u32;
        let max = x.get("max").and_then(|m| m.as_u64())? as u32;
        Some(StatusRange { min, max })
    });
    let mime_contains = v
        .get("mime_contains")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    Filter {
        url_glob,
        methods,
        status_range,
        mime_contains,
    }
}

/// Minimal `*`-glob matcher. `**` collapses to `*`. Anchored to the whole
/// string. Empty pattern = match only empty input. `*` alone matches everything.
fn glob_match(pattern: &str, input: &str) -> bool {
    // Collapse runs of `*` → single `*` (handles `**`, `***`, etc.).
    let mut collapsed = String::with_capacity(pattern.len());
    let mut prev_star = false;
    for c in pattern.chars() {
        if c == '*' {
            if !prev_star {
                collapsed.push('*');
            }
            prev_star = true;
        } else {
            collapsed.push(c);
            prev_star = false;
        }
    }

    let p: Vec<char> = collapsed.chars().collect();
    let s: Vec<char> = input.chars().collect();

    // Iterative two-pointer with backtracking — handles `*` matching any
    // (possibly empty) sequence. Whole-string anchored.
    let (mut pi, mut si) = (0usize, 0usize);
    let mut star_p: Option<usize> = None;
    let mut star_s: usize = 0;

    while si < s.len() {
        if pi < p.len() && p[pi] == '*' {
            star_p = Some(pi);
            star_s = si;
            pi += 1;
        } else if pi < p.len() && p[pi] == s[si] {
            pi += 1;
            si += 1;
        } else if let Some(sp) = star_p {
            // Backtrack: extend the previous `*` to swallow one more char.
            pi = sp + 1;
            star_s += 1;
            si = star_s;
        } else {
            return false;
        }
    }

    // Trailing `*` consumes nothing more.
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Apply a filter to a single event Value. Each axis is independently
/// optional; all *present* axes must match (logical AND).
fn event_matches_filter(event: &Value, filter: &Filter) -> bool {
    // URL glob
    if let Some(pat) = &filter.url_glob {
        let url = event.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if !glob_match(pat, url) {
            return false;
        }
    }

    // Method whitelist (empty = match all)
    if let Some(methods) = &filter.methods {
        if !methods.is_empty() {
            let method = event
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_uppercase();
            if !methods.iter().any(|m| m.eq_ignore_ascii_case(&method)) {
                return false;
            }
        }
    }

    // Status range (inclusive). If event has no status, treat as no-match
    // when the filter is set — defensive, since absence is ambiguous.
    if let Some(range) = &filter.status_range {
        let status = event
            .get("status")
            .and_then(|v| v.as_u64())
            .map(|s| s as u32);
        match status {
            Some(s) if s >= range.min && s <= range.max => {}
            _ => return false,
        }
    }

    // MIME substring (case-insensitive). Check common shape variations:
    //   - event["mime_type"]
    //   - event["content_type"]
    //   - event["headers"]["content-type"]
    if let Some(needle) = &filter.mime_contains {
        let needle_lower = needle.to_lowercase();
        let mime = event
            .get("mime_type")
            .or_else(|| event.get("content_type"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                event
                    .get("headers")
                    .and_then(|h| h.get("content-type").or_else(|| h.get("Content-Type")))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();
        if !mime.to_lowercase().contains(&needle_lower) {
            return false;
        }
    }

    true
}

/// Resolve a per-event id. Prefers a numeric `request_id` if present (forward
/// compatibility), else falls back to array index. The index is passed in by
/// the caller so the caller controls the sequence semantics.
fn event_id_for(event: &Value, fallback_index: u64) -> u64 {
    if let Some(rid) = event.get("request_id").and_then(|v| v.as_u64()) {
        return rid;
    }
    // Some implementations might use a numeric `id` field.
    if let Some(id) = event.get("id").and_then(|v| v.as_u64()) {
        return id;
    }
    fallback_index
}

// ── Public handlers ───────────────────────────────────────────────────

pub async fn handle_subscribe(
    args: &Value,
    _browser: &browser_mcp::browser::SharedBrowser,
    _session: &SharedSession,
) -> Value {
    let filter = parse_filter(args.get("filter"));
    let history_capacity = args
        .get("history_capacity")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_HISTORY_CAPACITY)
        .max(1);

    let id = new_subscription_id();
    let created_at = Utc::now().to_rfc3339();

    let sub = Subscription {
        id: id.clone(),
        filter: filter.clone(),
        history_capacity,
        cursor: 0,
        created_at: created_at.clone(),
        last_polled_at: None,
        events_seen: 0,
    };

    with_subs(|m| m.insert(id.clone(), sub));

    json!({
        "ok": true,
        "subscription_id": id,
        "filter": serde_json::to_value(&filter).unwrap_or(json!({})),
        "history_capacity": history_capacity,
        "created_at": created_at,
    })
}

pub async fn handle_poll(
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
    session: &SharedSession,
) -> Value {
    let subscription_id = match args.get("subscription_id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return json!({
                "ok": false,
                "error": "missing required parameter: subscription_id"
            });
        }
    };

    let max_events = args
        .get("max_events")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_MAX_EVENTS_PER_POLL)
        .max(1);

    // Confirm the subscription exists before paying for a browser round-trip.
    let exists = with_subs(|m| m.contains_key(&subscription_id));
    if !exists {
        return json!({
            "ok": false,
            "error": "subscription_id not found",
            "subscription_id": subscription_id,
        });
    }

    let events = fetch_network_events(browser, session).await;
    poll_inner(&subscription_id, events, max_events)
}

/// Pure-data poll: takes the resolved event vector and updates the
/// subscription. Factored out so tests can drive it without a real browser.
pub fn poll_inner(subscription_id: &str, events: Vec<Value>, max_events: usize) -> Value {
    let polled_at = Utc::now().to_rfc3339();

    let result = with_subs(|m| {
        let sub = match m.get_mut(subscription_id) {
            Some(s) => s,
            None => {
                return Err(());
            }
        };
        let cap = sub.history_capacity.min(max_events);

        // Apply cursor: keep events whose id >= cursor.
        let mut out: Vec<Value> = Vec::new();
        let mut new_cursor = sub.cursor;
        for (idx, event) in events.iter().enumerate() {
            let eid = event_id_for(event, idx as u64);
            if eid < sub.cursor {
                continue;
            }
            if !event_matches_filter(event, &sub.filter) {
                // Filter rejects, but still advance cursor past it so we
                // don't re-evaluate stale entries forever.
                if eid + 1 > new_cursor {
                    new_cursor = eid + 1;
                }
                continue;
            }
            out.push(event.clone());
            if eid + 1 > new_cursor {
                new_cursor = eid + 1;
            }
            if out.len() >= cap {
                break;
            }
        }

        sub.cursor = new_cursor;
        sub.last_polled_at = Some(polled_at.clone());
        sub.events_seen = sub.events_seen.saturating_add(out.len() as u64);

        Ok((out, sub.events_seen, new_cursor))
    });

    let (events_out, events_seen_total, new_cursor) = match result {
        Ok(t) => t,
        Err(()) => {
            return json!({
                "ok": false,
                "error": "subscription_id not found",
                "subscription_id": subscription_id,
            });
        }
    };

    let events_returned = events_out.len();
    json!({
        "ok": true,
        "subscription_id": subscription_id,
        "events": events_out,
        "events_returned": events_returned,
        "events_seen_total": events_seen_total,
        "new_cursor": new_cursor,
        "polled_at": polled_at,
    })
}

pub async fn handle_unsubscribe(
    args: &Value,
    _browser: &browser_mcp::browser::SharedBrowser,
    _session: &SharedSession,
) -> Value {
    let subscription_id = match args.get("subscription_id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return json!({
                "ok": false,
                "error": "missing required parameter: subscription_id"
            });
        }
    };

    let removed = with_subs(|m| m.remove(&subscription_id).is_some());

    json!({
        "ok": true,
        "subscription_id": subscription_id,
        "removed": removed,
    })
}

pub async fn handle_subscriptions(
    _args: &Value,
    _browser: &browser_mcp::browser::SharedBrowser,
    _session: &SharedSession,
) -> Value {
    let subs = with_subs(|m| {
        let mut v: Vec<Value> = m
            .values()
            .map(|s| {
                json!({
                    "subscription_id": s.id,
                    "filter": serde_json::to_value(&s.filter).unwrap_or(json!({})),
                    "history_capacity": s.history_capacity,
                    "cursor": s.cursor,
                    "created_at": s.created_at,
                    "last_polled_at": s.last_polled_at,
                    "events_seen": s.events_seen,
                })
            })
            .collect();
        // Stable order: by created_at asc (then by id for ties).
        v.sort_by(|a, b| {
            let ka = (
                a.get("created_at").and_then(|x| x.as_str()).unwrap_or(""),
                a.get("subscription_id")
                    .and_then(|x| x.as_str())
                    .unwrap_or(""),
            );
            let kb = (
                b.get("created_at").and_then(|x| x.as_str()).unwrap_or(""),
                b.get("subscription_id")
                    .and_then(|x| x.as_str())
                    .unwrap_or(""),
            );
            ka.cmp(&kb)
        });
        v
    });

    let count = subs.len();
    json!({
        "ok": true,
        "subscriptions": subs,
        "count": count,
    })
}

// ── Event source resolution ───────────────────────────────────────────

/// Fetch the current network log via browser-mcp (`get_network_log`). We pass
/// `clear: false` so the underlying `__mcp_intercepted` array accumulates,
/// making the array-index cursor stable across calls.
///
/// Failures (no page, JS eval error, missing routes) return an empty Vec —
/// `hands_network_poll` simply yields no events that turn.
async fn fetch_network_events(
    browser: &browser_mcp::browser::SharedBrowser,
    _session: &SharedSession,
) -> Vec<Value> {
    let result =
        browser_mcp::tools::handle_tool(browser, "get_network_log", json!({ "clear": false }))
            .await;

    if result.is_error {
        return Vec::new();
    }

    let text: String = result
        .content
        .iter()
        .filter_map(|c| match c {
            browser_mcp::types::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let parsed: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    // Response may be:
    //   - bare array
    //   - {"entries": [...]}
    //   - JSON-encoded string of either above (sometimes the eval-result is
    //     double-stringified).
    if let Some(arr) = parsed.as_array() {
        return arr.clone();
    }
    if let Some(entries) = parsed.get("entries").and_then(|v| v.as_array()) {
        return entries.clone();
    }
    if let Some(s) = parsed.as_str() {
        if let Ok(inner) = serde_json::from_str::<Value>(s) {
            if let Some(arr) = inner.as_array() {
                return arr.clone();
            }
            if let Some(entries) = inner.get("entries").and_then(|v| v.as_array()) {
                return entries.clone();
            }
        }
    }
    Vec::new()
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    /// Tests mutate process-global SUBSCRIPTIONS — serialize them so they
    /// don't race each other. Async-aware (tokio) lock so the async
    /// `#[tokio::test]`s can hold it across `.await` without the std-Mutex
    /// `await_holding_lock` hazard; sync `#[test]`s take it via `blocking_lock()`.
    static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Sync `#[test]`s (no runtime): acquire via `blocking_lock()`.
    fn test_guard() -> tokio::sync::MutexGuard<'static, ()> {
        TEST_LOCK.blocking_lock()
    }

    /// Async `#[tokio::test]`s: acquire via `.await` so the guard is held
    /// safely across await points (no std-Mutex hazard).
    async fn test_guard_async() -> tokio::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().await
    }

    fn reset_subs() {
        with_subs(|m| m.clear());
    }

    fn make_sub(filter: Filter, capacity: usize) -> String {
        let id = new_subscription_id();
        let sub = Subscription {
            id: id.clone(),
            filter,
            history_capacity: capacity,
            cursor: 0,
            created_at: Utc::now().to_rfc3339(),
            last_polled_at: None,
            events_seen: 0,
        };
        with_subs(|m| m.insert(id.clone(), sub));
        id
    }

    // ── UUID-shape tests ───────────────────────────────────────────────

    #[test]
    fn subscribe_returns_unique_uuid_v4() {
        let _g = test_guard();
        reset_subs();
        let ids: Vec<String> = (0..32).map(|_| new_subscription_id()).collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "expected all unique");
        for id in &ids {
            assert_eq!(id.len(), 36, "expected 36-char UUID, got {}", id);
            let parts: Vec<&str> = id.split('-').collect();
            assert_eq!(parts.len(), 5);
            assert_eq!(parts[0].len(), 8);
            assert_eq!(parts[1].len(), 4);
            assert_eq!(parts[2].len(), 4);
            assert_eq!(parts[3].len(), 4);
            assert_eq!(parts[4].len(), 12);
            assert!(parts[2].starts_with('4'), "version nibble: {}", id);
            let variant = parts[3].chars().next().unwrap();
            assert!(
                matches!(variant, '8' | '9' | 'a' | 'b'),
                "variant nibble: {}",
                id
            );
        }
    }

    // ── Filter parsing ─────────────────────────────────────────────────

    #[test]
    fn subscribe_stores_filter() {
        let _g = test_guard();
        reset_subs();
        let filter_json = json!({
            "url_glob": "**/api/**",
            "methods": ["GET", "POST"],
            "status_range": {"min": 200, "max": 299},
            "mime_contains": "json"
        });
        let parsed = parse_filter(Some(&filter_json));
        assert_eq!(parsed.url_glob.as_deref(), Some("**/api/**"));
        assert_eq!(
            parsed.methods.as_ref().unwrap(),
            &vec!["GET".to_string(), "POST".to_string()]
        );
        let r = parsed.status_range.unwrap();
        assert_eq!(r.min, 200);
        assert_eq!(r.max, 299);
        assert_eq!(parsed.mime_contains.as_deref(), Some("json"));
    }

    // ── Glob matcher ───────────────────────────────────────────────────

    #[test]
    fn glob_matches_simple_star() {
        assert!(glob_match("*", ""));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*.json", "package.json"));
        assert!(glob_match("api/*", "api/users"));
        assert!(!glob_match("api/*", "v1/api/users"));
        assert!(glob_match("/api/*", "/api/users"));
        assert!(glob_match("a*b", "ab"));
        assert!(glob_match("a*b", "aXYZb"));
        assert!(!glob_match("a*b", "ax"));
    }

    #[test]
    fn glob_matches_double_star_as_single_star() {
        // `**` is just collapsed to `*` per spec.
        assert!(glob_match("**", "https://example.com/anything"));
        assert!(glob_match("**/api/**", "/api/users"));
        assert!(glob_match("**/api/**", "https://example.com/v1/api/users"));
        assert!(glob_match("***", "abc"));
        // Equivalence: "**" should match same set as "*".
        assert_eq!(glob_match("**", "abc"), glob_match("*", "abc"));
        assert_eq!(glob_match("**", ""), glob_match("*", ""));
    }

    #[test]
    fn glob_anchors_whole_string() {
        // Pattern without leading `*` should NOT match a substring.
        assert!(!glob_match("api/users", "v1/api/users"));
        assert!(glob_match("api/users", "api/users"));
        // Pattern without trailing `*` should NOT match if input has trailing content.
        assert!(!glob_match("api/users", "api/users/123"));
        assert!(glob_match("api/*", "api/users/123"));
        // Empty pattern matches only empty input.
        assert!(glob_match("", ""));
        assert!(!glob_match("", "x"));
    }

    // ── Per-axis filter behavior ───────────────────────────────────────

    #[test]
    fn filter_method_case_insensitive() {
        let f = Filter {
            methods: Some(vec!["get".to_string(), "post".to_string()]),
            ..Default::default()
        };
        let evt_upper = json!({"url": "/x", "method": "GET"});
        let evt_lower = json!({"url": "/x", "method": "post"});
        let evt_other = json!({"url": "/x", "method": "DELETE"});
        assert!(event_matches_filter(&evt_upper, &f));
        assert!(event_matches_filter(&evt_lower, &f));
        assert!(!event_matches_filter(&evt_other, &f));
    }

    #[test]
    fn filter_status_range_inclusive() {
        let f = Filter {
            status_range: Some(StatusRange { min: 400, max: 599 }),
            ..Default::default()
        };
        assert!(event_matches_filter(
            &json!({"url": "/x", "method": "GET", "status": 400}),
            &f
        ));
        assert!(event_matches_filter(
            &json!({"url": "/x", "method": "GET", "status": 500}),
            &f
        ));
        assert!(event_matches_filter(
            &json!({"url": "/x", "method": "GET", "status": 599}),
            &f
        ));
        assert!(!event_matches_filter(
            &json!({"url": "/x", "method": "GET", "status": 399}),
            &f
        ));
        assert!(!event_matches_filter(
            &json!({"url": "/x", "method": "GET", "status": 600}),
            &f
        ));
        // Missing status field → fails the (set) status filter.
        assert!(!event_matches_filter(&json!({"url": "/x"}), &f));
    }

    #[test]
    fn filter_mime_contains_substring() {
        let f = Filter {
            mime_contains: Some("json".to_string()),
            ..Default::default()
        };
        assert!(event_matches_filter(
            &json!({"url": "/x", "mime_type": "application/json"}),
            &f
        ));
        assert!(event_matches_filter(
            &json!({"url": "/x", "mime_type": "application/JSON; charset=utf-8"}),
            &f
        ));
        assert!(event_matches_filter(
            &json!({"url": "/x", "content_type": "text/json"}),
            &f
        ));
        // Headers nested lookup.
        assert!(event_matches_filter(
            &json!({"url": "/x", "headers": {"content-type": "application/json"}}),
            &f
        ));
        assert!(!event_matches_filter(
            &json!({"url": "/x", "mime_type": "text/html"}),
            &f
        ));
        assert!(!event_matches_filter(&json!({"url": "/x"}), &f));
    }

    #[test]
    fn filter_combined_all_axes_required() {
        let f = Filter {
            url_glob: Some("*/api/*".to_string()),
            methods: Some(vec!["POST".to_string()]),
            status_range: Some(StatusRange { min: 200, max: 299 }),
            mime_contains: Some("json".to_string()),
        };
        // All four axes pass.
        let good = json!({
            "url": "/api/users",
            "method": "POST",
            "status": 200,
            "mime_type": "application/json",
        });
        assert!(event_matches_filter(&good, &f));

        // Each individual axis broken → reject.
        let bad_url = json!({
            "url": "/v1/users",  // no /api/
            "method": "POST", "status": 200, "mime_type": "application/json",
        });
        assert!(!event_matches_filter(&bad_url, &f));
        let bad_method = json!({
            "url": "/api/users",
            "method": "GET", "status": 200, "mime_type": "application/json",
        });
        assert!(!event_matches_filter(&bad_method, &f));
        let bad_status = json!({
            "url": "/api/users", "method": "POST",
            "status": 404, "mime_type": "application/json",
        });
        assert!(!event_matches_filter(&bad_status, &f));
        let bad_mime = json!({
            "url": "/api/users", "method": "POST", "status": 200,
            "mime_type": "text/html",
        });
        assert!(!event_matches_filter(&bad_mime, &f));
    }

    #[test]
    fn filter_missing_axes_match_all() {
        // No axes set = match everything.
        let f = Filter::default();
        assert!(event_matches_filter(&json!({}), &f));
        assert!(event_matches_filter(&json!({"url": "/x"}), &f));
        assert!(event_matches_filter(
            &json!({"url": "/x", "method": "DELETE", "status": 500}),
            &f
        ));

        // Empty `methods: []` = match all per spec.
        let f2 = Filter {
            methods: Some(vec![]),
            ..Default::default()
        };
        assert!(event_matches_filter(&json!({"method": "GET"}), &f2));
        assert!(event_matches_filter(&json!({"method": "DELETE"}), &f2));
        assert!(event_matches_filter(&json!({}), &f2));
    }

    // ── Cursor & poll_inner ────────────────────────────────────────────

    fn evt(idx: u64, url: &str, method: &str) -> Value {
        // Use array index as the implicit event_id; no request_id field.
        json!({"url": url, "method": method, "_idx": idx})
    }

    #[test]
    fn poll_advances_cursor() {
        let _g = test_guard();
        reset_subs();
        let sub_id = make_sub(Filter::default(), 100);
        let events = vec![
            evt(0, "/a", "GET"),
            evt(1, "/b", "GET"),
            evt(2, "/c", "GET"),
        ];
        let out = poll_inner(&sub_id, events, 100);
        assert_eq!(out["ok"], json!(true));
        assert_eq!(out["events_returned"], json!(3));
        assert_eq!(out["new_cursor"], json!(3));

        // State should reflect cursor advance.
        let cursor = with_subs(|m| m.get(&sub_id).unwrap().cursor);
        assert_eq!(cursor, 3);
    }

    #[test]
    fn poll_returns_only_events_after_cursor() {
        let _g = test_guard();
        reset_subs();
        let sub_id = make_sub(Filter::default(), 100);

        // First poll: returns all 3 events, cursor = 3.
        let events1 = vec![
            evt(0, "/a", "GET"),
            evt(1, "/b", "GET"),
            evt(2, "/c", "GET"),
        ];
        let out1 = poll_inner(&sub_id, events1, 100);
        assert_eq!(out1["events_returned"], json!(3));

        // Second poll: same 3 + 2 new. Should only return the 2 new.
        let events2 = vec![
            evt(0, "/a", "GET"),
            evt(1, "/b", "GET"),
            evt(2, "/c", "GET"),
            evt(3, "/d", "GET"),
            evt(4, "/e", "GET"),
        ];
        let out2 = poll_inner(&sub_id, events2, 100);
        assert_eq!(out2["events_returned"], json!(2));
        let urls: Vec<&str> = out2["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["url"].as_str().unwrap())
            .collect();
        assert_eq!(urls, vec!["/d", "/e"]);
        assert_eq!(out2["new_cursor"], json!(5));
    }

    #[test]
    fn poll_respects_history_capacity_cap() {
        let _g = test_guard();
        reset_subs();
        // capacity = 2 → poll caps even if max_events is bigger.
        let sub_id = make_sub(Filter::default(), 2);
        let events = vec![
            evt(0, "/a", "GET"),
            evt(1, "/b", "GET"),
            evt(2, "/c", "GET"),
            evt(3, "/d", "GET"),
            evt(4, "/e", "GET"),
        ];
        let out = poll_inner(&sub_id, events, 1000);
        assert_eq!(out["events_returned"], json!(2));
        let urls: Vec<&str> = out["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["url"].as_str().unwrap())
            .collect();
        assert_eq!(urls, vec!["/a", "/b"]);

        // max_events cap also independently bounds (here: smaller than capacity).
        let sub_id2 = make_sub(Filter::default(), 10);
        let events2 = vec![
            evt(0, "/a", "GET"),
            evt(1, "/b", "GET"),
            evt(2, "/c", "GET"),
        ];
        let out2 = poll_inner(&sub_id2, events2, 1);
        assert_eq!(out2["events_returned"], json!(1));
    }

    #[test]
    fn poll_updates_last_polled_at_and_events_seen() {
        let _g = test_guard();
        reset_subs();
        let sub_id = make_sub(Filter::default(), 100);
        let before = with_subs(|m| m.get(&sub_id).unwrap().clone());
        assert!(before.last_polled_at.is_none());
        assert_eq!(before.events_seen, 0);

        let events = vec![evt(0, "/a", "GET"), evt(1, "/b", "GET")];
        let _ = poll_inner(&sub_id, events, 100);

        let after = with_subs(|m| m.get(&sub_id).unwrap().clone());
        assert!(after.last_polled_at.is_some(), "expected polled_at set");
        assert_eq!(after.events_seen, 2);

        // Second poll with one new event → events_seen accumulates.
        let events2 = vec![
            evt(0, "/a", "GET"),
            evt(1, "/b", "GET"),
            evt(2, "/c", "GET"),
        ];
        let _ = poll_inner(&sub_id, events2, 100);
        let after2 = with_subs(|m| m.get(&sub_id).unwrap().clone());
        assert_eq!(after2.events_seen, 3);
    }

    #[test]
    fn poll_unknown_subscription_returns_error() {
        let _g = test_guard();
        reset_subs();
        let out = poll_inner("no-such-subscription-id", vec![evt(0, "/a", "GET")], 100);
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"], json!("subscription_id not found"));
    }

    // ── Unsubscribe / list ─────────────────────────────────────────────

    #[tokio::test]
    async fn unsubscribe_removes_subscription() {
        let _g = test_guard_async().await;
        reset_subs();
        let sub_id = make_sub(Filter::default(), 10);
        assert!(with_subs(|m| m.contains_key(&sub_id)));

        // Construct stand-in browser/session — they're not used by these handlers.
        let browser = browser_mcp::browser::create_shared();
        let session = super::super::session::new_session();

        let out = handle_unsubscribe(
            &json!({"subscription_id": sub_id.clone()}),
            &browser,
            &session,
        )
        .await;
        assert_eq!(out["ok"], json!(true));
        assert_eq!(out["removed"], json!(true));
        assert!(!with_subs(|m| m.contains_key(&sub_id)));
    }

    #[tokio::test]
    async fn unsubscribe_unknown_returns_ok_anyway() {
        let _g = test_guard_async().await;
        reset_subs();
        let browser = browser_mcp::browser::create_shared();
        let session = super::super::session::new_session();
        let out = handle_unsubscribe(
            &json!({"subscription_id": "ghost-id-does-not-exist"}),
            &browser,
            &session,
        )
        .await;
        assert_eq!(out["ok"], json!(true), "got: {}", out);
        assert_eq!(out["removed"], json!(false));
    }

    #[tokio::test]
    async fn subscriptions_list_returns_all() {
        let _g = test_guard_async().await;
        reset_subs();
        let _id1 = make_sub(Filter::default(), 10);
        std::thread::sleep(std::time::Duration::from_millis(2));
        let _id2 = make_sub(
            Filter {
                url_glob: Some("*/api/*".to_string()),
                ..Default::default()
            },
            20,
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
        let _id3 = make_sub(Filter::default(), 30);

        let browser = browser_mcp::browser::create_shared();
        let session = super::super::session::new_session();
        let out = handle_subscriptions(&json!({}), &browser, &session).await;
        assert_eq!(out["ok"], json!(true));
        assert_eq!(out["count"], json!(3));
        let arr = out["subscriptions"].as_array().unwrap();
        assert_eq!(arr.len(), 3);
        // Each entry has the documented shape.
        for sub in arr {
            assert!(sub.get("subscription_id").is_some());
            assert!(sub.get("filter").is_some());
            assert!(sub.get("history_capacity").is_some());
            assert!(sub.get("cursor").is_some());
            assert!(sub.get("created_at").is_some());
            assert!(sub.get("events_seen").is_some());
        }
    }

    // ── Bonus: forward-compatible request_id cursor strategy ───────────

    #[test]
    fn poll_uses_request_id_when_present() {
        let _g = test_guard();
        reset_subs();
        let sub_id = make_sub(Filter::default(), 100);

        // First poll: request_ids 10, 11, 12 → cursor advances to 13.
        let events = vec![
            json!({"url": "/a", "method": "GET", "request_id": 10}),
            json!({"url": "/b", "method": "GET", "request_id": 11}),
            json!({"url": "/c", "method": "GET", "request_id": 12}),
        ];
        let out = poll_inner(&sub_id, events, 100);
        assert_eq!(out["events_returned"], json!(3));
        assert_eq!(out["new_cursor"], json!(13));

        // Second poll: same three + one new at request_id=20.
        let events2 = vec![
            json!({"url": "/a", "method": "GET", "request_id": 10}),
            json!({"url": "/b", "method": "GET", "request_id": 11}),
            json!({"url": "/c", "method": "GET", "request_id": 12}),
            json!({"url": "/d", "method": "GET", "request_id": 20}),
        ];
        let out2 = poll_inner(&sub_id, events2, 100);
        assert_eq!(out2["events_returned"], json!(1));
        assert_eq!(out2["events"][0]["url"], json!("/d"));
        assert_eq!(out2["new_cursor"], json!(21));
    }
}
