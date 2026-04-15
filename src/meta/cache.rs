//! A11yCache with hybrid invalidation.
//! Three-layer invalidation catches different failure modes:
//! 1. Event-based: invalidate on browser interaction (navigate/click/type/press/scroll/eval-with-mutation)
//! 2. Mutation observer: JS injected on navigate sets dirty flag on DOM mutations
//! 3. Content hash backstop: cheap fingerprint of a11y tree; mismatch = invalidate

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use serde_json::Value;

/// Events that invalidate the a11y cache.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InvalidationEvent {
    Navigate,
    Click,
    Type,
    Press,
    Scroll,
    EvalWithMutation,
}

/// Cached a11y snapshot with hybrid invalidation metadata.
#[derive(Debug)]
pub struct A11yMetaCache {
    /// The cached snapshot data
    snapshot: Option<Value>,
    /// Content hash of the snapshot for backstop comparison
    content_hash: u64,
    /// Whether mutation observer has signaled a change
    mutation_dirty: bool,
    /// Whether an event-based invalidation has occurred
    event_invalidated: bool,
    /// URL when the snapshot was taken
    url: String,
}

impl A11yMetaCache {
    pub fn new() -> Self {
        Self {
            snapshot: None,
            content_hash: 0,
            mutation_dirty: false,
            event_invalidated: false,
            url: String::new(),
        }
    }

    /// Store a new snapshot, computing its content hash.
    pub fn store(&mut self, snapshot: Value, url: &str) {
        self.content_hash = compute_content_hash(&snapshot);
        self.snapshot = Some(snapshot);
        self.url = url.to_string();
        self.mutation_dirty = false;
        self.event_invalidated = false;
    }

    /// Get the cached snapshot if still valid.
    /// Re-validates using all three invalidation layers.
    pub fn get(&self) -> Option<&Value> {
        if self.snapshot.is_none() {
            return None;
        }
        // Layer 1: event-based invalidation
        if self.event_invalidated {
            return None;
        }
        // Layer 2: mutation observer dirty flag
        if self.mutation_dirty {
            return None;
        }
        // Layer 3 (content hash) is checked externally via verify_hash()
        // because it requires a fresh snapshot to compare against
        self.snapshot.as_ref()
    }

    /// Verify the cached content hash against a fresh fingerprint.
    /// Returns true if hash matches (cache still valid).
    pub fn verify_hash(&self, fresh_snapshot: &Value) -> bool {
        if self.snapshot.is_none() {
            return false;
        }
        let fresh_hash = compute_content_hash(fresh_snapshot);
        self.content_hash == fresh_hash
    }

    /// Signal that a browser interaction event occurred.
    pub fn on_event(&mut self, _event: InvalidationEvent) {
        self.event_invalidated = true;
    }

    /// Signal that the mutation observer detected DOM changes.
    pub fn on_mutation(&mut self) {
        self.mutation_dirty = true;
    }

    /// Clear the cache entirely.
    pub fn clear(&mut self) {
        self.snapshot = None;
        self.content_hash = 0;
        self.mutation_dirty = false;
        self.event_invalidated = false;
        self.url.clear();
    }

    /// Get the URL the snapshot was taken on.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Check if cache has any stored data (regardless of validity).
    pub fn has_data(&self) -> bool {
        self.snapshot.is_some()
    }

    /// Force-get the snapshot ignoring validity (for hash comparison).
    pub fn get_unchecked(&self) -> Option<&Value> {
        self.snapshot.as_ref()
    }
}

/// Compute a cheap content hash of an a11y tree.
/// Uses the string representation's hash — fast enough for invalidation backstop.
fn compute_content_hash(value: &Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    // Hash the compact JSON string — captures all structural changes
    let s = value.to_string();
    s.hash(&mut hasher);
    hasher.finish()
}

/// JS snippet to inject mutation observer on page navigate.
/// Sets `window.__hands_mutation_dirty = true` on any DOM mutation.
pub const MUTATION_OBSERVER_JS: &str = r#"
(function() {
    if (window.__hands_observer) return;
    window.__hands_mutation_dirty = false;
    window.__hands_observer = new MutationObserver(function() {
        window.__hands_mutation_dirty = true;
    });
    window.__hands_observer.observe(document.body || document.documentElement, {
        childList: true, subtree: true, attributes: true, characterData: true
    });
})();
"#;

/// JS snippet to check and reset the mutation dirty flag.
pub const CHECK_MUTATION_JS: &str = r#"
(function() {
    var dirty = !!window.__hands_mutation_dirty;
    window.__hands_mutation_dirty = false;
    return dirty;
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_cache_store_and_get() {
        let mut cache = A11yMetaCache::new();
        assert!(cache.get().is_none());

        cache.store(json!({"role": "button", "name": "Submit"}), "https://example.com");
        assert!(cache.get().is_some());
    }

    #[test]
    fn test_event_invalidation() {
        let mut cache = A11yMetaCache::new();
        cache.store(json!({"role": "button"}), "https://example.com");
        assert!(cache.get().is_some());

        cache.on_event(InvalidationEvent::Click);
        assert!(cache.get().is_none());
    }

    #[test]
    fn test_mutation_invalidation() {
        let mut cache = A11yMetaCache::new();
        cache.store(json!({"role": "button"}), "https://example.com");
        assert!(cache.get().is_some());

        cache.on_mutation();
        assert!(cache.get().is_none());
    }

    #[test]
    fn test_hash_verification() {
        let mut cache = A11yMetaCache::new();
        let snapshot = json!({"role": "button", "name": "Submit"});
        cache.store(snapshot.clone(), "https://example.com");

        // Same content — hash matches
        assert!(cache.verify_hash(&json!({"role": "button", "name": "Submit"})));
        // Different content — hash mismatch
        assert!(!cache.verify_hash(&json!({"role": "button", "name": "Cancel"})));
    }

    #[test]
    fn test_clear() {
        let mut cache = A11yMetaCache::new();
        cache.store(json!({"role": "button"}), "https://example.com");
        assert!(cache.has_data());

        cache.clear();
        assert!(!cache.has_data());
        assert!(cache.get().is_none());
    }
}
