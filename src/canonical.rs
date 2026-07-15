use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

pub const CATALOG_TOOL_NAME: &str = "hands_capability_catalog";
pub const PROFILE_ENV: &str = "HANDS_TOOL_PROFILE";
pub const TEMPLATE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolProfile {
    Default,
    Full,
    Strict,
    Compatibility,
}

impl ToolProfile {
    fn active() -> Self {
        match std::env::var(PROFILE_ENV)
            .unwrap_or_else(|_| "default".to_string())
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "compat" | "compatibility" | "raw" => Self::Compatibility,
            "strict" => Self::Strict,
            "full" => Self::Full,
            _ => Self::Default,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Full => "full",
            Self::Strict => "strict",
            Self::Compatibility => "compatibility",
        }
    }

    fn exposure_field(self) -> Option<&'static str> {
        match self {
            Self::Default => Some("exposed_default"),
            Self::Full => Some("exposed_full"),
            Self::Strict => Some("exposed_strict"),
            Self::Compatibility => None,
        }
    }
}

fn manifest() -> &'static Value {
    static MANIFEST: OnceLock<Value> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        serde_json::from_str(include_str!("../manifest/unified_hands_manifest.json"))
            .expect("embedded unified Hands manifest must be valid JSON")
    })
}

fn manifest_tools() -> &'static [Value] {
    manifest()
        .get("tools")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .expect("unified Hands manifest must contain a tools array")
}

fn manifest_tool(name: &str) -> Option<&'static Value> {
    manifest_tools().iter().find(|tool| {
        tool.get("name")
            .and_then(Value::as_str)
            .is_some_and(|candidate| candidate == name)
    })
}

fn is_exposed(tool: &Value, profile: ToolProfile) -> bool {
    profile
        .exposure_field()
        .is_none_or(|field| tool.get(field).and_then(Value::as_bool).unwrap_or(false))
}

fn annotate_definition(mut definition: Value, tool: &Value) -> Value {
    if let Some(object) = definition.as_object_mut() {
        let name = object.get("name").and_then(Value::as_str).unwrap_or("");
        let behavior_note = match name {
            "vision_ocr" => Some(
                "Canonical OCR with automatic result caching and backend metadata included in every response.",
            ),
            "hands_health" => Some(
                "Canonical health and status surface, including subsystem probes, paths, profile, collection counts, and total advertised tools.",
            ),
            "browser_attach" => Some(
                "Cross-process debug-port locking is acquired automatically and held until browser_close.",
            ),
            _ => None,
        };
        if let Some(note) = behavior_note {
            let description = object
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            object.insert(
                "description".to_string(),
                Value::String(format!("{} {}", description, note)),
            );
        }
        object.insert(
            "x-hands-collection".to_string(),
            tool.get("collection").cloned().unwrap_or(Value::Null),
        );
        object.insert(
            "x-hands-disposition".to_string(),
            tool.get("disposition").cloned().unwrap_or(Value::Null),
        );
        object.insert(
            "x-hands-source-status".to_string(),
            tool.get("source_status").cloned().unwrap_or(Value::Null),
        );
    }
    definition
}

fn catalog_definition() -> Value {
    json!({
        "name": CATALOG_TOOL_NAME,
        "description": "List the unified Hands capability collections, canonical profiles, deduplication decisions, and optional per-tool records. This is the front door for choosing an accurate capability instead of guessing from tool names.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "collection": {
                    "type": "string",
                    "description": "Optional collection id to return, such as api-and-network-intelligence."
                },
                "include_tools": {
                    "type": "boolean",
                    "default": false,
                    "description": "Include the matching per-tool manifest records."
                }
            }
        },
        "x-hands-collection": "capability-catalog",
        "x-hands-disposition": "canonical-front-door",
        "x-hands-source-status": "Template"
    })
}

pub fn filter_tool_definitions(raw: Vec<Value>) -> Vec<Value> {
    let profile = ToolProfile::active();
    let mut by_name = HashMap::new();

    for definition in raw {
        let Some(name) = definition
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        by_name.entry(name).or_insert(definition);
    }

    let mut filtered = Vec::with_capacity(by_name.len() + 1);
    filtered.push(catalog_definition());

    let collections = manifest()
        .get("collections")
        .and_then(Value::as_array)
        .expect("unified Hands manifest must contain collections");

    for collection in collections {
        let Some(names) = collection.get("tools").and_then(Value::as_array) else {
            continue;
        };
        for name in names.iter().filter_map(Value::as_str) {
            let Some(tool) = manifest_tool(name) else {
                continue;
            };
            let definition = by_name.remove(name);
            if !is_exposed(tool, profile) {
                continue;
            }
            if let Some(definition) = definition {
                filtered.push(annotate_definition(definition, tool));
            }
        }
    }

    // Never silently erase a newly added source tool just because the pinned
    // manifest has not been regenerated yet. Expose it as unclassified so the
    // count check fails visibly and forces a taxonomy refresh.
    let mut unclassified: Vec<(String, Value)> = by_name.into_iter().collect();
    unclassified.sort_by(|left, right| left.0.cmp(&right.0));
    for (_, mut definition) in unclassified {
        if let Some(object) = definition.as_object_mut() {
            object.insert(
                "x-hands-collection".to_string(),
                json!("unclassified-new-source"),
            );
            object.insert(
                "x-hands-disposition".to_string(),
                json!("manifest-refresh-required"),
            );
            object.insert("x-hands-source-status".to_string(), json!("Unclassified"));
        }
        filtered.push(definition);
    }

    filtered
}

fn direct_dispatch_alias(name: &str) -> Option<&'static str> {
    match name {
        "hands_status" => Some("hands_health"),
        "browser_accessibility_snapshot" => Some("browser_a11y_snapshot"),
        _ => None,
    }
}

pub fn blocked_call_response(name: &str) -> Option<Value> {
    if name == CATALOG_TOOL_NAME {
        return None;
    }

    let profile = ToolProfile::active();

    if let Some(replacement) = direct_dispatch_alias(name) {
        if profile == ToolProfile::Compatibility {
            return None;
        }
        return Some(json!({
            "success": false,
            "error": format!(
                "Dispatcher alias '{}' is disabled by the '{}' canonical profile.",
                name,
                profile.name()
            ),
            "replacement": replacement,
            "reason": "unlisted_compatibility_alias",
            "profile_env": PROFILE_ENV,
            "compatibility_value": "compatibility"
        }));
    }

    let tool = manifest_tool(name)?;
    if is_exposed(tool, profile) {
        return None;
    }

    Some(json!({
        "success": false,
        "error": format!(
            "Tool '{}' is hidden by the '{}' canonical profile.",
            name,
            profile.name()
        ),
        "replacement": tool.get("replacement").cloned().unwrap_or(Value::Null),
        "reason": tool.get("disposition").cloned().unwrap_or(Value::Null),
        "profile_env": PROFILE_ENV,
        "compatibility_value": "compatibility"
    }))
}

pub fn catalog_response(args: &Value) -> Value {
    let profile = ToolProfile::active();
    let collection_filter = args.get("collection").and_then(Value::as_str);
    let include_tools = args
        .get("include_tools")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let collections: Vec<Value> = manifest()
        .get("collections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|collection| {
            collection_filter.is_none_or(|wanted| {
                collection
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id == wanted)
            })
        })
        .cloned()
        .collect();

    let tools: Vec<Value> = if include_tools {
        manifest_tools()
            .iter()
            .filter(|tool| is_exposed(tool, profile))
            .filter(|tool| {
                collection_filter.is_none_or(|wanted| {
                    tool.get("collection")
                        .and_then(Value::as_str)
                        .is_some_and(|id| id == wanted)
                })
            })
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    let exposed_count = manifest_tools()
        .iter()
        .filter(|tool| is_exposed(tool, profile))
        .count();

    let mut response = json!({
        "success": true,
        "version": TEMPLATE_VERSION,
        "active_profile": profile.name(),
        "profile_env": PROFILE_ENV,
        "listed_tool_count": exposed_count + 1,
        "catalog_tool_count": 1,
        "counts": manifest().get("counts").cloned().unwrap_or(Value::Null),
        "collections": collections,
        "replacements": manifest().get("replacements").cloned().unwrap_or(Value::Null),
        "snapshot": manifest().get("snapshot").cloned().unwrap_or(Value::Null)
    });

    if include_tools {
        response
            .as_object_mut()
            .expect("catalog response must be an object")
            .insert("tools".to_string(), Value::Array(tools));
    }

    response
}

pub fn health_tool_surface() -> Value {
    let profile = ToolProfile::active();
    let exposed_count = manifest_tools()
        .iter()
        .filter(|tool| is_exposed(tool, profile))
        .count();

    let collections: Vec<Value> = manifest()
        .get("collections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|collection| {
            let id = collection.get("id")?.as_str()?;
            let count = collection
                .get(match profile {
                    ToolProfile::Default => "default",
                    ToolProfile::Full => "full",
                    ToolProfile::Strict => "strict",
                    ToolProfile::Compatibility => "raw",
                })?
                .as_u64()?;
            Some(json!({"id": id, "count": count}))
        })
        .collect();

    json!({
        "active_profile": profile.name(),
        "profile_env": PROFILE_ENV,
        "capability_count": exposed_count,
        "catalog_tool_count": 1,
        "listed_tool_count": exposed_count + 1,
        "collections": collections
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_expected_exact_cover() {
        let checks = manifest().get("checks").expect("checks");
        assert_eq!(
            checks.get("exact_cover").and_then(Value::as_bool),
            Some(true)
        );
        let expected = manifest()
            .pointer("/counts/raw_union_unique")
            .and_then(Value::as_u64)
            .expect("raw union count") as usize;
        assert_eq!(manifest_tools().len(), expected);

        let tool_names = manifest_tools()
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(tool_names.len(), expected);

        let collection_names = manifest()
            .get("collections")
            .and_then(Value::as_array)
            .expect("collections")
            .iter()
            .flat_map(|collection| {
                collection
                    .get("tools")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
            })
            .collect::<Vec<_>>();
        assert_eq!(collection_names.len(), expected);
        assert_eq!(
            collection_names
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            expected
        );
        assert_eq!(
            collection_names
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
            tool_names
        );
    }

    #[test]
    fn manifest_uses_monitor_scope_instead_of_hands_recording_front_doors() {
        assert_eq!(
            manifest_tool("hands_monitor_scope")
                .and_then(|tool| tool.get("collection"))
                .and_then(Value::as_str),
            Some("monitor-scope-and-topology")
        );
        for removed in [
            "hands_self_record_lookup",
            "hands_self_record_start",
            "hands_self_record_stop_and_optimize",
        ] {
            assert!(
                manifest_tool(removed).is_none(),
                "{removed} must stay Workflow-owned"
            );
        }
    }

    #[test]
    fn every_manifest_tool_has_a_collection() {
        assert!(manifest_tools().iter().all(|tool| {
            tool.get("collection")
                .and_then(Value::as_str)
                .is_some_and(|collection| !collection.is_empty())
        }));
    }

    #[test]
    fn newly_added_source_tool_is_not_silently_dropped() {
        let listed = filter_tool_definitions(vec![json!({
            "name": "future_unclassified_tool",
            "description": "future",
            "inputSchema": {"type": "object", "properties": {}}
        })]);
        let future = listed
            .iter()
            .find(|tool| tool.get("name") == Some(&json!("future_unclassified_tool")))
            .expect("future tool must remain visible");
        assert_eq!(
            future.get("x-hands-collection"),
            Some(&json!("unclassified-new-source"))
        );
    }
}
