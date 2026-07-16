//! A11y Ref Cache + Incremental Snapshot Diffing
//!
//! - Caches the last accessibility snapshot per tab (by URL as proxy for tab identity)
//! - Assigns stable ref IDs (e.g., "ref_0", "ref_1") to each node in DFS order
//! - Maps ref IDs → CSS selector paths for element resolution
//! - Provides tree diffing for incremental snapshots

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;

/// Global ref cache: maps ref_id → CSS selector for the most recent snapshot
static REF_CACHE: Mutex<Option<RefCache>> = Mutex::new(None);

/// Global snapshot cache for incremental diffing
static SNAPSHOT_CACHE: Mutex<Option<Value>> = Mutex::new(None);

pub struct RefCache {
    pub refs: HashMap<String, String>, // ref_id → CSS selector
    pub snapshot_url: String,          // URL when snapshot was taken
    #[allow(dead_code)]
    pub timestamp: u64, // epoch ms when cached (for future staleness checks)
}

/// Store a new ref cache (called after each a11y snapshot)
pub fn store_refs(refs: HashMap<String, String>, url: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let cache = RefCache {
        refs,
        snapshot_url: url.to_string(),
        timestamp: ts,
    };
    *REF_CACHE.lock().unwrap() = Some(cache);
}

/// Resolve a ref ID to a CSS selector. Returns None if stale or not found.
pub fn resolve_ref(ref_id: &str) -> Result<String, String> {
    let guard = REF_CACHE.lock().unwrap();
    match &*guard {
        None => Err("No a11y snapshot cached. Call browser_a11y_snapshot first.".into()),
        Some(cache) => match cache.refs.get(ref_id) {
            Some(selector) => Ok(selector.clone()),
            None => Err(format!(
                "Ref '{}' not found in cached snapshot ({}). Take a fresh browser_a11y_snapshot.",
                ref_id, cache.snapshot_url
            )),
        },
    }
}

/// Store the last full snapshot tree for incremental diffing
pub fn store_snapshot(tree: &Value) {
    *SNAPSHOT_CACHE.lock().unwrap() = Some(tree.clone());
}

/// Get the previous snapshot (if any) and replace with new one.
/// Returns the old snapshot for diffing.
pub fn swap_snapshot(new_tree: &Value) -> Option<Value> {
    let mut guard = SNAPSHOT_CACHE.lock().unwrap();
    let old = guard.take();
    *guard = Some(new_tree.clone());
    old
}

/// Get the current cached snapshot (read-only, for searching)
pub fn get_snapshot() -> Option<Value> {
    SNAPSHOT_CACHE.lock().unwrap().clone()
}

/// Walk the a11y tree and assign ref IDs. Returns a map of ref_id → CSS selector.
/// Also annotates each node in the tree with its ref_id.
pub fn assign_refs(tree: &mut Value) -> HashMap<String, String> {
    let mut refs = HashMap::new();
    let mut counter = 0u32;
    assign_refs_recursive(tree, &mut refs, &mut counter, "");
    refs
}

fn assign_refs_recursive(
    node: &mut Value,
    refs: &mut HashMap<String, String>,
    counter: &mut u32,
    parent_selector: &str,
) {
    if node.is_null() {
        return;
    }

    // Handle array nodes (pass-through)
    if let Some(arr) = node.as_array_mut() {
        for child in arr.iter_mut() {
            assign_refs_recursive(child, refs, counter, parent_selector);
        }
        return;
    }

    let obj = match node.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    let role = obj
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Build a CSS selector for this node based on role + name
    let selector = build_selector(&role, &name, parent_selector, *counter);

    let ref_id = format!("ref_{}", counter);
    *counter += 1;

    // Store the mapping
    refs.insert(ref_id.clone(), selector.clone());

    // Annotate the node with its ref
    obj.insert("ref".into(), json!(ref_id));

    // Recurse into children
    if let Some(children) = obj.get_mut("children") {
        if let Some(arr) = children.as_array_mut() {
            for child in arr.iter_mut() {
                assign_refs_recursive(child, refs, counter, &selector);
            }
        }
    }
}

/// Build a CSS selector that can locate this element.
/// Uses role→tag mapping + name-based attribute selectors.
fn build_selector(role: &str, name: &str, _parent: &str, index: u32) -> String {
    // Map ARIA roles back to likely CSS selectors
    let tag_part = match role {
        "button" => "button, input[type='button'], input[type='submit'], [role='button']",
        "link" => "a[href], [role='link']",
        "textbox" => "input[type='text'], input:not([type]), textarea, [role='textbox']",
        "searchbox" => "input[type='search'], [role='searchbox']",
        "checkbox" => "input[type='checkbox'], [role='checkbox']",
        "radio" => "input[type='radio'], [role='radio']",
        "combobox" => "select:not([multiple]), [role='combobox']",
        "listbox" => "select[multiple], [role='listbox']",
        "slider" => "input[type='range'], [role='slider']",
        "spinbutton" => "input[type='number'], [role='spinbutton']",
        "heading" => "h1, h2, h3, h4, h5, h6, [role='heading']",
        "img" => "img, [role='img']",
        "navigation" => "nav, [role='navigation']",
        "main" => "main, [role='main']",
        "form" => "form, [role='form']",
        "table" => "table, [role='table']",
        "row" => "tr, [role='row']",
        "cell" => "td, [role='cell']",
        "columnheader" => "th, [role='columnheader']",
        "list" => "ul, ol, [role='list']",
        "listitem" => "li, [role='listitem']",
        "dialog" => "dialog, [role='dialog']",
        "tab" => "[role='tab']",
        "tabpanel" => "[role='tabpanel']",
        "menu" => "[role='menu']",
        "menuitem" => "[role='menuitem']",
        "option" => "option, [role='option']",
        _ => "",
    };

    // If we have a name, use it for more specific targeting via JS-based resolution
    // The actual resolution uses JS `document.querySelectorAll` + text matching
    if !name.is_empty() && !tag_part.is_empty() {
        // We'll use a JS-resolvable format: role + name + index as fallback
        format!(
            "__a11y_ref__:{}:{}:{}",
            role,
            escape_selector_value(name),
            index
        )
    } else if !tag_part.is_empty() {
        format!("__a11y_ref__:{}::{}", role, index)
    } else {
        format!(
            "__a11y_ref__:{}::{}",
            if role.is_empty() { "generic" } else { role },
            index
        )
    }
}

fn escape_selector_value(s: &str) -> String {
    // Escape characters that would break our delimiter format
    s.replace(':', "\\:").replace('\\', "\\\\")
}

/// Generate JavaScript that resolves an a11y ref to a DOM element.
/// Returns JS code that evaluates to the element or null.
pub fn ref_resolution_js(ref_id: &str) -> Result<String, String> {
    let selector = resolve_ref(ref_id)?;

    // Parse our custom selector format: __a11y_ref__:role:name:index
    if let Some(rest) = selector.strip_prefix("__a11y_ref__:") {
        let parts: Vec<&str> = rest.splitn(3, ':').collect();
        let role = parts.first().unwrap_or(&"");
        let name = parts
            .get(1)
            .unwrap_or(&"")
            .replace("\\:", ":")
            .replace("\\\\", "\\");
        let _index = parts.get(2).unwrap_or(&"0");
        let role_js = serde_json::to_string(role).unwrap_or_else(|_| "\"\"".to_string());
        let name_js = serde_json::to_string(&name).unwrap_or_else(|_| "\"\"".to_string());

        // Build JS that finds element by role + accessible name
        let js = format!(
            r#"(() => {{
                const role = {};
                const targetName = {};

                // Map role to candidate selectors
                const ROLE_SELECTORS = {{
                    'button': 'button, input[type="button"], input[type="submit"], input[type="reset"], [role="button"], summary',
                    'link': 'a[href], [role="link"]',
                    'textbox': 'input[type="text"], input:not([type]), textarea, [role="textbox"], input[type="email"], input[type="tel"], input[type="url"], input[type="password"]',
                    'searchbox': 'input[type="search"], [role="searchbox"]',
                    'checkbox': 'input[type="checkbox"], [role="checkbox"]',
                    'radio': 'input[type="radio"], [role="radio"]',
                    'combobox': 'select:not([multiple]), [role="combobox"]',
                    'listbox': 'select[multiple], [role="listbox"]',
                    'slider': 'input[type="range"], [role="slider"]',
                    'spinbutton': 'input[type="number"], [role="spinbutton"]',
                    'heading': 'h1, h2, h3, h4, h5, h6, [role="heading"]',
                    'img': 'img, [role="img"]',
                    'navigation': 'nav, [role="navigation"]',
                    'main': 'main, [role="main"]',
                    'form': 'form, [role="form"]',
                    'table': 'table, [role="table"]',
                    'row': 'tr, [role="row"]',
                    'cell': 'td, [role="cell"]',
                    'columnheader': 'th, [role="columnheader"]',
                    'list': 'ul, ol, [role="list"]',
                    'listitem': 'li, [role="listitem"]',
                    'dialog': 'dialog, [role="dialog"]',
                    'tab': '[role="tab"]',
                    'tabpanel': '[role="tabpanel"]',
                    'menu': '[role="menu"]',
                    'menuitem': '[role="menuitem"]',
                    'option': 'option, [role="option"]',
                    'paragraph': 'p',
                    'region': '[role="region"], section[aria-label], section[aria-labelledby]',
                    'separator': 'hr, [role="separator"]',
                    'article': 'article, [role="article"]',
                    'complementary': 'aside, [role="complementary"]',
                    'banner': '[role="banner"]',
                    'contentinfo': '[role="contentinfo"]',
                    'figure': 'figure, [role="figure"]',
                    'group': 'fieldset, details, optgroup, [role="group"]',
                    'status': 'output, [role="status"]',
                    'meter': 'meter, [role="meter"]',
                    'progressbar': 'progress, [role="progressbar"]',
                }};

                function getAccessibleName(el) {{
                    const labelledBy = el.getAttribute('aria-labelledby');
                    if (labelledBy) {{
                        const parts = labelledBy.split(/\s+/).map(id => {{
                            const ref_ = document.getElementById(id);
                            return ref_ ? ref_.textContent.trim() : '';
                        }}).filter(Boolean);
                        if (parts.length) return parts.join(' ');
                    }}
                    const ariaLabel = el.getAttribute('aria-label');
                    if (ariaLabel) return ariaLabel;
                    if (['INPUT','SELECT','TEXTAREA'].includes(el.tagName)) {{
                        if (el.id) {{
                            const label = document.querySelector('label[for="' + CSS.escape(el.id) + '"]');
                            if (label) return label.textContent.trim();
                        }}
                        return el.getAttribute('placeholder') || el.getAttribute('title') || '';
                    }}
                    if (el.tagName === 'IMG') return el.getAttribute('alt') || el.getAttribute('title') || '';
                    if (['A','BUTTON','SUMMARY'].includes(el.tagName) || /^H[1-6]$/.test(el.tagName)) {{
                        return el.textContent.trim().slice(0, 200);
                    }}
                    return el.getAttribute('title') || '';
                }}

                const sel = ROLE_SELECTORS[role];
                if (!sel) return null;

                const candidates = document.querySelectorAll(sel);
                if (!targetName) {{
                    return candidates[0] || null;
                }}
                for (const el of candidates) {{
                    const accName = getAccessibleName(el);
                    if (accName === targetName || accName.includes(targetName) || targetName.includes(accName)) {{
                        return el;
                    }}
                }}
                // Fallback: try text content match
                for (const el of candidates) {{
                    if (el.textContent.trim().includes(targetName)) {{
                        return el;
                    }}
                }}
                return null;
            }})()"#,
            role_js, name_js,
        );
        Ok(js)
    } else {
        // Plain CSS selector (shouldn't happen with our format, but handle it)
        let selector_js = serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".to_string());
        Ok(format!("document.querySelector({selector_js})"))
    }
}

/// Diff two a11y trees. Returns a delta object with added, removed, changed nodes.
pub fn diff_trees(old: &Value, new: &Value) -> Value {
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    let mut unchanged_count = 0u32;

    let old_nodes = flatten_tree(old);
    let new_nodes = flatten_tree(new);

    // Build lookup by (role, name) key
    let mut old_map: HashMap<String, Vec<&Value>> = HashMap::new();
    for node in &old_nodes {
        let key = node_key(node);
        old_map.entry(key).or_default().push(node);
    }

    let mut new_map: HashMap<String, Vec<&Value>> = HashMap::new();
    for node in &new_nodes {
        let key = node_key(node);
        new_map.entry(key).or_default().push(node);
    }

    // Find added and changed
    let mut matched_old_keys: HashMap<String, usize> = HashMap::new();
    for node in &new_nodes {
        let key = node_key(node);
        let idx = matched_old_keys.entry(key.clone()).or_insert(0);
        if let Some(old_list) = old_map.get(&key) {
            if *idx < old_list.len() {
                let old_node = old_list[*idx];
                if nodes_differ(old_node, node) {
                    changed.push(json!({
                        "ref": node.get("ref"),
                        "role": node.get("role"),
                        "name": node.get("name"),
                        "old_states": old_node.get("states"),
                        "new_states": node.get("states"),
                        "old_properties": old_node.get("properties"),
                        "new_properties": node.get("properties"),
                    }));
                } else {
                    unchanged_count += 1;
                }
                *idx += 1;
            } else {
                added.push(compact_node(node));
            }
        } else {
            added.push(compact_node(node));
        }
    }

    // Find removed: old nodes not matched by new
    let mut new_matched: HashMap<String, usize> = HashMap::new();
    for node in &new_nodes {
        *new_matched.entry(node_key(node)).or_insert(0) += 1;
    }
    for (key, old_list) in &old_map {
        let new_count = new_matched.get(key).copied().unwrap_or(0);
        for node in old_list.iter().skip(new_count) {
            removed.push(compact_node(node));
        }
    }

    json!({
        "added": added,
        "removed": removed,
        "changed": changed,
        "unchanged_count": unchanged_count,
        "summary": format!(
            "+{} added, -{} removed, ~{} changed, {} unchanged",
            added.len(), removed.len(), changed.len(), unchanged_count
        )
    })
}

/// Flatten a tree into a list of leaf/meaningful nodes (DFS)
fn flatten_tree(node: &Value) -> Vec<&Value> {
    let mut result = Vec::new();
    flatten_recursive(node, &mut result);
    result
}

fn flatten_recursive<'a>(node: &'a Value, result: &mut Vec<&'a Value>) {
    if node.is_null() {
        return;
    }
    if let Some(arr) = node.as_array() {
        for child in arr {
            flatten_recursive(child, result);
        }
        return;
    }
    if node.is_object() {
        // Include this node if it has a role
        if node.get("role").is_some() {
            result.push(node);
        }
        if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
            for child in children {
                flatten_recursive(child, result);
            }
        }
    }
}

/// Create a key for a node based on role + name (for matching across snapshots)
fn node_key(node: &Value) -> String {
    let role = node.get("role").and_then(|v| v.as_str()).unwrap_or("");
    let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("");
    format!("{}::{}", role, name)
}

/// Check if two nodes differ in states or properties
fn nodes_differ(a: &Value, b: &Value) -> bool {
    a.get("states") != b.get("states")
        || a.get("properties") != b.get("properties")
        || a.get("text") != b.get("text")
}

/// Compact representation of a node for the diff output
fn compact_node(node: &Value) -> Value {
    json!({
        "ref": node.get("ref"),
        "role": node.get("role"),
        "name": node.get("name"),
        "states": node.get("states"),
        "properties": node.get("properties"),
    })
}
