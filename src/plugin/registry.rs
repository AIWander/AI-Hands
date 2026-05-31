//! In-memory registry of loaded plugins.
//!
//! Phase 1 (this commit): the registry is a thread-safe `HashMap<name,
//! LoadedPlugin>` with insert/list/remove operations. No plugins ever get
//! inserted in phase 1 because the loader stub returns
//! `PhaseNotImplemented` for every path, but the registry is wired up and
//! tested so phase 2 can drop in real loading without restructuring.
//!
//! Phase 2 will populate this registry from `loader::load_from_path`
//! after a successful `LoadLibrary`/`dlopen` + symbol resolution + init
//! call.

// `insert`, `remove`, and `LoadedTool::input_schema_json` are reachable
// from tests + the phase-2 loader, but the phase-1 main.rs only calls
// `list()`. Allow the dead-code warning so phase 1 ships clean.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Mutex;

/// One loaded plugin's host-side metadata. Cloneable so callers can take a
/// snapshot of the registry without holding the lock across MCP responses.
#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub tools: Vec<LoadedTool>,
    pub abi_version_major: u32,
    pub abi_version_minor: u32,
    pub library_path: String,
    /// RFC3339 timestamp of when the plugin finished init.
    pub loaded_at: String,
}

/// One tool exposed by a loaded plugin. The JSON Schema string comes from
/// the plugin's `ToolDescriptor::input_schema_json` and is stored verbatim.
#[derive(Debug, Clone)]
pub struct LoadedTool {
    pub name: String,
    pub description: String,
    pub input_schema_json: String,
}

/// Global registry, lazily initialized on first use. We use `Option<HashMap>`
/// inside the `Mutex` so the static can be `const`-initialized without
/// requiring `OnceCell` (no new external dep — phase 1 constraint).
static REGISTRY: Mutex<Option<HashMap<String, LoadedPlugin>>> = Mutex::new(None);

/// Run `f` with exclusive access to the registry map. Lazy-inits on first
/// call. Recovers from poisoning by taking the inner data so a panicked
/// thread can't permanently lock out the rest of the process.
pub fn with_registry<R>(f: impl FnOnce(&mut HashMap<String, LoadedPlugin>) -> R) -> R {
    let mut g = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(HashMap::new());
    }
    f(g.as_mut().unwrap())
}

/// Snapshot of every loaded plugin, sorted by name for stable output.
pub fn list() -> Vec<LoadedPlugin> {
    with_registry(|m| {
        let mut v: Vec<LoadedPlugin> = m.values().cloned().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    })
}

/// Insert a plugin. Returns `Err` if a plugin with the same name is
/// already loaded — the host should reject the load attempt rather than
/// silently shadowing an earlier registration.
pub fn insert(plugin: LoadedPlugin) -> Result<(), String> {
    with_registry(|m| {
        if m.contains_key(&plugin.name) {
            return Err(format!("plugin '{}' already loaded", plugin.name));
        }
        m.insert(plugin.name.clone(), plugin);
        Ok(())
    })
}

/// Remove a plugin by name. Returns `true` if a plugin was present.
pub fn remove(name: &str) -> bool {
    with_registry(|m| m.remove(name).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};
    use std::thread;

    /// Tests share the global REGISTRY, so serialize them with a test-only
    /// mutex and clear state at the top of each test. Without this the
    /// concurrent_insert test would race the other tests.
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn reset_registry() {
        with_registry(|m| m.clear());
    }

    fn sample_plugin(name: &str) -> LoadedPlugin {
        LoadedPlugin {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            author: "test".to_string(),
            description: "sample".to_string(),
            tools: vec![LoadedTool {
                name: format!("{}_tool", name),
                description: "echo".to_string(),
                input_schema_json: r#"{"type":"object"}"#.to_string(),
            }],
            abi_version_major: 1,
            abi_version_minor: 0,
            library_path: format!("/tmp/{}.so", name),
            loaded_at: "1970-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn list_returns_empty_initially() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_registry();
        assert!(list().is_empty());
    }

    #[test]
    fn insert_then_list_returns_the_plugin() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_registry();
        insert(sample_plugin("alpha")).expect("first insert");
        let plugins = list();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "alpha");
        assert_eq!(plugins[0].tools.len(), 1);
        assert_eq!(plugins[0].tools[0].name, "alpha_tool");
    }

    #[test]
    fn insert_duplicate_returns_error() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_registry();
        insert(sample_plugin("beta")).expect("first insert");
        let err = insert(sample_plugin("beta")).expect_err("duplicate must error");
        assert!(
            err.contains("beta"),
            "error message should name the plugin: {err}"
        );
        assert_eq!(list().len(), 1, "registry must not double-register");
    }

    #[test]
    fn remove_returns_true_when_present_false_when_absent() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_registry();
        insert(sample_plugin("gamma")).expect("insert");
        assert!(remove("gamma"));
        assert!(!remove("gamma"), "second remove must be false");
        assert!(!remove("never-existed"));
        assert!(list().is_empty());
    }

    #[test]
    fn list_is_sorted_by_name() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_registry();
        insert(sample_plugin("zeta")).unwrap();
        insert(sample_plugin("alpha")).unwrap();
        insert(sample_plugin("mu")).unwrap();
        let names: Vec<String> = list().into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn concurrent_insert_does_not_crash() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_registry();

        // Spawn 4 threads each inserting 5 plugins with disjoint names.
        // Disjoint names mean every insert succeeds — the test exercises
        // the Mutex coordination, not the dedup path.
        let collisions = Arc::new(StdMutex::new(0usize));
        let mut handles = Vec::new();
        for t in 0..4 {
            let coll = Arc::clone(&collisions);
            handles.push(thread::spawn(move || {
                for i in 0..5 {
                    let name = format!("plug_{}_{}", t, i);
                    if insert(sample_plugin(&name)).is_err() {
                        *coll.lock().unwrap() += 1;
                    }
                }
            }));
        }
        for h in handles {
            h.join().expect("worker panic");
        }

        assert_eq!(
            *collisions.lock().unwrap(),
            0,
            "no name collisions expected"
        );
        assert_eq!(list().len(), 20, "all 20 inserts should land");
    }
}
