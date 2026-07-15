#![allow(dead_code)] // invalidate_* hooks are public surface for v2 event wiring
//! Element-region vision cache (Item 9, Milestone A).
//!
//! TTL-based MVP. Event-driven invalidation hooks are exposed (`invalidate_all`,
//! `invalidate_browser`, `invalidate_uia`) but the v2 work wires them into the
//! browser/UIA event streams. For now the cache trusts its TTL.
//!
//! Acceptance: re-OCR the same region within 5s returns the cached result in
//! <100ms. The cache wraps `vision_core::execute(...)` and is opt-in per tool —
//! only the read-only vision tools (`vision_ocr`, `vision_screenshot_ocr`,
//! `vision_find_template`) are cached. Mutating or rapidly-changing calls
//! (`vision_screenshot` raw capture, `vision_diff`) bypass the cache.
//!
//! Concurrency: a single `Mutex<Option<VisionCache>>` static guards all state.
//! Cache reads and writes are short critical sections — they touch a HashMap
//! and increment counters; no async work happens under the lock.

use crate::vision_core;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default TTL for cached vision results.
const DEFAULT_TTL_MS: u64 = 5_000;
/// Soft cap on entries — when exceeded, the oldest entry is evicted.
const MAX_ENTRIES: usize = 256;

struct CacheEntry {
    result: Value,
    stored_at: Instant,
    hits: u32,
}

pub struct VisionCache {
    entries: HashMap<String, CacheEntry>,
    // Aggregate stats — never reset except via `handle_stats(reset=true)`.
    total_hits: u64,
    total_misses: u64,
    total_evictions: u64,
    total_invalidations: u64,
    ttl_ms: u64,
}

impl VisionCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            total_hits: 0,
            total_misses: 0,
            total_evictions: 0,
            total_invalidations: 0,
            ttl_ms: DEFAULT_TTL_MS,
        }
    }
}

impl Default for VisionCache {
    fn default() -> Self {
        Self::new()
    }
}

static CACHE: Mutex<Option<VisionCache>> = Mutex::new(None);

fn with_cache<F, R>(f: F) -> R
where
    F: FnOnce(&mut VisionCache) -> R,
{
    let mut g = CACHE.lock().expect("vision_cache mutex poisoned");
    if g.is_none() {
        *g = Some(VisionCache::new());
    }
    f(g.as_mut().expect("vision_cache just initialized"))
}

/// Tools we know are safe to cache. Read-only with stable outputs for the
/// same (image, region) pair.
fn is_cacheable(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "vision_ocr" | "vision_screenshot_ocr" | "vision_find_template"
    )
}

/// Compute a stable cache key from (tool_name, args).
///
/// `serde_json::to_string` walks a `Value` in insertion order. Because args are
/// constructed via `json!({...})` literals (string-keyed objects in source
/// order), the resulting string is stable across calls with identical inputs —
/// `cache_key_is_deterministic_for_same_inputs` enforces this.
fn cache_key(tool_name: &str, args: &Value) -> String {
    let canonical = serde_json::to_string(args).unwrap_or_default();
    format!("{}|{}", tool_name, canonical)
}

/// Cached wrapper around `vision_core::execute`.
///
/// Returns the cached result if a non-expired entry exists, otherwise calls
/// through to vision-core and stores the response.
pub async fn cached_execute(tool_name: &str, args: &Value) -> Value {
    if !is_cacheable(tool_name) {
        return vision_core::execute(tool_name, args).await;
    }

    let key = cache_key(tool_name, args);

    // Hit?
    let hit = with_cache(|c| {
        if let Some(e) = c.entries.get_mut(&key) {
            if e.stored_at.elapsed() < Duration::from_millis(c.ttl_ms) {
                e.hits += 1;
                c.total_hits += 1;
                return Some(e.result.clone());
            }
        }
        c.total_misses += 1;
        None
    });

    if let Some(cached) = hit {
        return cached;
    }

    // Miss — execute and store. No lock held during the await.
    let result = vision_core::execute(tool_name, args).await;
    with_cache(|c| {
        if c.entries.len() >= MAX_ENTRIES {
            // Simple eviction: drop oldest by stored_at.
            if let Some(oldest_key) = c
                .entries
                .iter()
                .min_by_key(|(_, e)| e.stored_at)
                .map(|(k, _)| k.clone())
            {
                c.entries.remove(&oldest_key);
                c.total_evictions += 1;
            }
        }
        c.entries.insert(
            key,
            CacheEntry {
                result: result.clone(),
                stored_at: Instant::now(),
                hits: 0,
            },
        );
    });
    result
}

/// Invalidate all cached entries. Exposed for v2 event integration.
pub fn invalidate_all() {
    with_cache(|c| {
        let n = c.entries.len() as u64;
        c.entries.clear();
        c.total_invalidations += n;
    });
}

/// Invalidate browser-originated entries. v2 hook — currently a no-op stub
/// that records the invalidation intent. Reserved for future wiring when
/// the cache key encodes subsystem origin.
pub fn invalidate_browser() {
    // v2: when keys are extended with a subsystem tag, drop only browser-
    // originated entries. For now we conservatively drop everything to match
    // the documented contract.
    invalidate_all();
}

/// Invalidate UIA-originated entries. v2 hook — see `invalidate_browser`.
pub fn invalidate_uia() {
    invalidate_all();
}

/// Stats handler for the `vision_cache_stats` tool.
///
/// If `reset` is true, both the entries and the aggregate counters are zeroed
/// AFTER the snapshot is captured (so the returned `stats` reflects the
/// pre-reset state).
pub fn handle_stats(args: &Value) -> Value {
    let reset = args.get("reset").and_then(|v| v.as_bool()).unwrap_or(false);
    let stats = with_cache(|c| {
        let total = c.total_hits + c.total_misses;
        let hit_rate = if total > 0 {
            c.total_hits as f64 / total as f64
        } else {
            0.0
        };
        let snapshot = json!({
            "entries": c.entries.len(),
            "max_entries": MAX_ENTRIES,
            "ttl_ms": c.ttl_ms,
            "total_hits": c.total_hits,
            "total_misses": c.total_misses,
            "hit_rate": hit_rate,
            "total_evictions": c.total_evictions,
            "total_invalidations": c.total_invalidations,
        });
        if reset {
            c.entries.clear();
            c.total_hits = 0;
            c.total_misses = 0;
            c.total_evictions = 0;
            c.total_invalidations = 0;
        }
        snapshot
    });
    json!({ "ok": true, "stats": stats, "reset": reset })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reset cache between tests — tests share process-global state.
    fn reset_for_test() {
        with_cache(|c| {
            c.entries.clear();
            c.total_hits = 0;
            c.total_misses = 0;
            c.total_evictions = 0;
            c.total_invalidations = 0;
        });
    }

    #[test]
    fn cache_key_is_deterministic_for_same_inputs() {
        let args = json!({"image_path": "C:/tmp/foo.png", "region": [0, 0, 100, 100]});
        let k1 = cache_key("vision_ocr", &args);
        let k2 = cache_key("vision_ocr", &args);
        assert_eq!(k1, k2);
        // Same key in a fresh json! literal too.
        let args2 = json!({"image_path": "C:/tmp/foo.png", "region": [0, 0, 100, 100]});
        let k3 = cache_key("vision_ocr", &args2);
        assert_eq!(k1, k3);
    }

    #[test]
    fn cache_key_differs_for_different_args() {
        let a = json!({"image_path": "C:/tmp/foo.png"});
        let b = json!({"image_path": "C:/tmp/bar.png"});
        assert_ne!(cache_key("vision_ocr", &a), cache_key("vision_ocr", &b));
    }

    #[test]
    fn cache_key_differs_for_different_tools() {
        let args = json!({});
        assert_ne!(
            cache_key("vision_ocr", &args),
            cache_key("vision_screenshot_ocr", &args)
        );
    }

    #[test]
    fn non_cacheable_tools_bypass_cache() {
        // vision_screenshot, vision_diff, and any unknown tool are not cached.
        assert!(!is_cacheable("vision_screenshot"));
        assert!(!is_cacheable("vision_diff"));
        assert!(!is_cacheable("uia_list_window"));
        assert!(!is_cacheable("hands_health"));
        // Cacheable set is the documented trio.
        assert!(is_cacheable("vision_ocr"));
        assert!(is_cacheable("vision_screenshot_ocr"));
        assert!(is_cacheable("vision_find_template"));
    }

    #[test]
    fn handle_stats_returns_zero_initial_state() {
        reset_for_test();
        let out = handle_stats(&json!({}));
        assert_eq!(out["ok"], json!(true));
        assert_eq!(out["reset"], json!(false));
        let stats = &out["stats"];
        assert_eq!(stats["entries"], json!(0));
        assert_eq!(stats["max_entries"], json!(MAX_ENTRIES));
        assert_eq!(stats["ttl_ms"], json!(DEFAULT_TTL_MS));
        assert_eq!(stats["total_hits"], json!(0));
        assert_eq!(stats["total_misses"], json!(0));
        assert_eq!(stats["hit_rate"], json!(0.0));
        assert_eq!(stats["total_evictions"], json!(0));
        assert_eq!(stats["total_invalidations"], json!(0));
    }

    #[test]
    fn handle_stats_reset_clears_counters() {
        reset_for_test();
        // Seed some state directly so we don't depend on vision_core.
        with_cache(|c| {
            c.total_hits = 7;
            c.total_misses = 3;
            c.total_evictions = 1;
            c.total_invalidations = 2;
            c.entries.insert(
                "fake_key".to_string(),
                CacheEntry {
                    result: json!({"text": "hello"}),
                    stored_at: Instant::now(),
                    hits: 5,
                },
            );
        });

        // Snapshot before reset.
        let out = handle_stats(&json!({"reset": true}));
        assert_eq!(out["reset"], json!(true));
        let stats = &out["stats"];
        assert_eq!(stats["entries"], json!(1));
        assert_eq!(stats["total_hits"], json!(7));
        assert_eq!(stats["total_misses"], json!(3));
        assert_eq!(stats["hit_rate"], json!(0.7));

        // After reset everything is zero.
        let out2 = handle_stats(&json!({}));
        let stats2 = &out2["stats"];
        assert_eq!(stats2["entries"], json!(0));
        assert_eq!(stats2["total_hits"], json!(0));
        assert_eq!(stats2["total_misses"], json!(0));
        assert_eq!(stats2["total_evictions"], json!(0));
        assert_eq!(stats2["total_invalidations"], json!(0));
    }

    #[test]
    fn invalidate_all_clears_entries_and_tracks_count() {
        reset_for_test();
        with_cache(|c| {
            for i in 0..3 {
                c.entries.insert(
                    format!("k{}", i),
                    CacheEntry {
                        result: json!({"i": i}),
                        stored_at: Instant::now(),
                        hits: 0,
                    },
                );
            }
        });
        invalidate_all();
        let out = handle_stats(&json!({}));
        assert_eq!(out["stats"]["entries"], json!(0));
        assert_eq!(out["stats"]["total_invalidations"], json!(3));
    }

    // TODO(item-9 v2): async cache hit/miss/expiry tests require a working
    // vision_core probe in the test env. The current MVP relies on the
    // deterministic key + stats machinery covered above; integration tests
    // will land alongside event-driven invalidation in v2.
}
