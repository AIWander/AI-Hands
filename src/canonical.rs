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

/// True only for the deliberately unrestricted compatibility/debug profile.
pub fn compatibility_profile_active() -> bool {
    ToolProfile::active() == ToolProfile::Compatibility
}

pub type UnsafeGate = (&'static str, &'static str, &'static str);

/// Return every dual-gate contract for a compatibility-only capability.
///
/// The manifest is the authority for which tools are unsafe. Only the smaller
/// direct-fetch subset is named here so it can use a distinct process and
/// per-call acknowledgement. This prevents the advertised surface and runtime
/// dispatcher from drifting into different safety classifications.
pub fn unsafe_compatibility_gates(name: &str) -> Vec<UnsafeGate> {
    let effective_name = direct_dispatch_alias(name).unwrap_or(name);
    let Some(tool) = manifest_tool(effective_name) else {
        return Vec::new();
    };
    let manifest_marks_unsafe =
        tool.get("safety").and_then(Value::as_str) == Some("unsafe-debug-compatibility-only");
    if !manifest_marks_unsafe {
        return Vec::new();
    }

    if matches!(
        effective_name,
        "hands_plugin_call" | "hands_plugin_load" | "hands_plugin_unload"
    ) {
        vec![(
            "native plugin execution",
            "HANDS_ALLOW_UNSAFE_PLUGINS",
            "allow_unsafe_plugin",
        )]
    } else if matches!(
        effective_name,
        "browser_script" | "browser_evaluate" | "browser_eval" | "browser_inject_script"
    ) {
        vec![
            (
                "direct network fetch",
                "HANDS_ALLOW_UNSAFE_DIRECT_FETCH",
                "allow_unsafe_fetch",
            ),
            (
                "raw or credential-adjacent output",
                "HANDS_ALLOW_UNSAFE_RAW_TOOLS",
                "allow_unsafe_raw",
            ),
        ]
    } else if matches!(
        effective_name,
        "browser_http_scrape"
            | "browser_crawl"
            | "browser_map"
            | "browser_smart_browse"
            | "browser_bulk_extract"
            | "browser_js_extract"
            | "hands_read_page"
    ) {
        vec![(
            "direct network fetch",
            "HANDS_ALLOW_UNSAFE_DIRECT_FETCH",
            "allow_unsafe_fetch",
        )]
    } else {
        vec![(
            "raw or credential-adjacent output",
            "HANDS_ALLOW_UNSAFE_RAW_TOOLS",
            "allow_unsafe_raw",
        )]
    }
}

/// Backward-compatible primary gate accessor. Composite capabilities may have
/// additional required gates; enforcement and tool schemas use the full list.
#[cfg_attr(not(test), allow(dead_code))]
pub fn unsafe_compatibility_gate(name: &str) -> Option<UnsafeGate> {
    unsafe_compatibility_gates(name).into_iter().next()
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

fn profile_safety(profile: ToolProfile) -> &'static str {
    manifest()
        .pointer(&format!("/profile_safety/{}", profile.name()))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}

fn compatibility_warning() -> &'static str {
    manifest()
        .pointer("/profile_safety/compatibility_warning")
        .and_then(Value::as_str)
        .unwrap_or("Compatibility is an unsafe debug profile.")
}

fn annotate_definition(mut definition: Value, tool: &Value, profile: ToolProfile) -> Value {
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
        object.insert("x-hands-profile".to_string(), json!(profile.name()));
        object.insert(
            "x-hands-profile-safety".to_string(),
            json!(profile_safety(profile)),
        );
        object.insert(
            "x-hands-safety".to_string(),
            tool.get("safety")
                .cloned()
                .unwrap_or_else(|| json!(profile_safety(profile))),
        );
    }
    definition
}

fn catalog_definition(profile: ToolProfile) -> Value {
    json!({
        "name": CATALOG_TOOL_NAME,
        "description": "List the unified Hands capability collections, canonical profiles, safety classification, deduplication decisions, and optional per-tool records. Compatibility is an explicitly unsafe debug profile, not a security-safe operating mode.",
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
        "x-hands-source-status": "Template",
        "x-hands-profile": profile.name(),
        "x-hands-profile-safety": profile_safety(profile)
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
    filtered.push(catalog_definition(profile));

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
                filtered.push(annotate_definition(definition, tool, profile));
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
        "uia_poll_events" => Some("uia_poll_event"),
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
            "compatibility_value": "compatibility",
            "compatibility_warning": compatibility_warning()
        }));
    }

    let Some(tool) = manifest_tool(name) else {
        return Some(json!({
            "success": false,
            "error": format!("Tool '{}' is not classified in the canonical Hands manifest.", name),
            "reason": "unmanifested_tool",
            "profile_env": PROFILE_ENV,
            "active_profile": profile.name(),
            "required_action": "classify the tool or use an advertised canonical capability"
        }));
    };
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
        "compatibility_value": "compatibility",
        "compatibility_warning": compatibility_warning()
    }))
}

/// Enforce the process-level and per-call acknowledgement required by every
/// manifest-classified unsafe compatibility capability. Composite dispatchers
/// call this same boundary before invoking nested tools.
pub fn unsafe_call_block_response(name: &str, args: &Value) -> Option<Value> {
    let gates = unsafe_compatibility_gates(name);
    if gates.is_empty() {
        return None;
    }
    if !compatibility_profile_active() {
        return Some(json!({
            "success": false,
            "error": format!("Tool '{name}' is an unsafe compatibility capability and is unavailable in safe profiles."),
            "reason": "compatibility_profile_required",
            "capability_kinds": gates.iter().map(|(kind, _, _)| *kind).collect::<Vec<_>>(),
            "profile_env": PROFILE_ENV,
            "required_profile": "compatibility"
        }));
    }

    let missing = gates
        .iter()
        .filter_map(|(kind, env_name, argument_name)| {
            let env_enabled = std::env::var(env_name).ok().is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes"
                )
            });
            let call_enabled = args
                .get(*argument_name)
                .and_then(Value::as_bool)
                .unwrap_or(false);
            (!env_enabled || !call_enabled).then(|| {
                let mut gate = serde_json::Map::new();
                gate.insert(
                    "capability_kind".to_owned(),
                    Value::String((*kind).to_owned()),
                );
                gate.insert(
                    "required_env".to_owned(),
                    Value::String((*env_name).to_owned()),
                );
                gate.insert(
                    "required_env_value".to_owned(),
                    Value::String("1".to_owned()),
                );
                gate.insert(
                    "required_argument".to_owned(),
                    Value::String((*argument_name).to_owned()),
                );
                gate.insert("required_argument_value".to_owned(), Value::Bool(true));
                Value::Object(gate)
            })
        })
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Some(json!({
            "success": false,
            "error": format!("Tool '{name}' requires every listed process-level unsafe gate and per-call acknowledgement."),
            "reason": "unsafe_dual_gate_required",
            "missing_gates": missing,
            "unsafe_effects_possible": true
        }));
    }
    None
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
        "active_profile_safety": profile_safety(profile),
        "profile_env": PROFILE_ENV,
        "compatibility_warning": compatibility_warning(),
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
        "active_profile_safety": profile_safety(profile),
        "profile_env": PROFILE_ENV,
        "compatibility_warning": compatibility_warning(),
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

    #[test]
    fn declared_profile_and_collection_counts_match_exposure_flags() {
        let cases = [
            (
                ToolProfile::Compatibility,
                "raw",
                "raw_union_unique",
                "compatibility_tools_list",
            ),
            (
                ToolProfile::Strict,
                "strict",
                "strict_unique",
                "strict_tools_list",
            ),
            (ToolProfile::Full, "full", "full_unique", "full_tools_list"),
            (
                ToolProfile::Default,
                "default",
                "default_unique",
                "default_tools_list",
            ),
        ];

        for (profile, collection_field, unique_field, listed_field) in cases {
            let actual = manifest_tools()
                .iter()
                .filter(|tool| is_exposed(tool, profile))
                .count() as u64;
            assert_eq!(
                manifest()
                    .pointer(&format!("/counts/{unique_field}"))
                    .and_then(Value::as_u64),
                Some(actual),
                "{unique_field} must be derived from exposure flags"
            );
            assert_eq!(
                manifest()
                    .pointer(&format!("/counts/{listed_field}"))
                    .and_then(Value::as_u64),
                Some(actual + 1),
                "{listed_field} must include the catalog tool"
            );

            for collection in manifest()
                .get("collections")
                .and_then(Value::as_array)
                .expect("collections")
            {
                let collection_id = collection
                    .get("id")
                    .and_then(Value::as_str)
                    .expect("collection id");
                let actual_collection_count = collection
                    .get("tools")
                    .and_then(Value::as_array)
                    .expect("collection tools")
                    .iter()
                    .filter_map(Value::as_str)
                    .filter_map(manifest_tool)
                    .filter(|tool| is_exposed(tool, profile))
                    .count() as u64;
                assert_eq!(
                    collection.get(collection_field).and_then(Value::as_u64),
                    Some(actual_collection_count),
                    "{collection_id}.{collection_field} must be derived from exposure flags"
                );
            }
        }
    }

    #[test]
    fn raw_value_and_direct_fetch_surfaces_are_compatibility_only() {
        const UNSAFE_DEBUG_TOOLS: [&str; 30] = [
            "browser_a11y_find",
            "browser_a11y_snapshot",
            "browser_bulk_extract",
            "browser_crawl",
            "browser_eval",
            "browser_evaluate",
            "browser_get_clickables",
            "browser_get_forms",
            "browser_get_html",
            "browser_http_scrape",
            "browser_inject_script",
            "browser_js_extract",
            "browser_map",
            "browser_page_capture",
            "browser_page_dump",
            "browser_script",
            "browser_smart_browse",
            "browser_trace_save",
            "browser_trace_start",
            "browser_trace_stop",
            "hands_read_page",
            "hands_scan_qr",
            "hands_script",
            "hands_plugin_call",
            "hands_plugin_load",
            "hands_plugin_unload",
            "uia_poll_event",
            "uia_read_value",
            "uia_watch",
            "vision_check_user_input",
        ];

        for name in UNSAFE_DEBUG_TOOLS {
            let tool = manifest_tool(name).unwrap_or_else(|| panic!("missing {name}"));
            for safe_profile in [ToolProfile::Default, ToolProfile::Full, ToolProfile::Strict] {
                assert!(
                    !is_exposed(tool, safe_profile),
                    "{name} must be hidden from {}",
                    safe_profile.name()
                );
            }
            assert!(
                is_exposed(tool, ToolProfile::Compatibility),
                "{name} must remain discoverable in compatibility"
            );
            assert_eq!(
                tool.get("disposition").and_then(Value::as_str),
                Some("unsafe_debug_compatibility")
            );
            assert_eq!(
                tool.get("safety").and_then(Value::as_str),
                Some("unsafe-debug-compatibility-only")
            );
            assert!(
                tool.get("replacement")
                    .and_then(Value::as_str)
                    .is_some_and(|replacement| !replacement.is_empty()),
                "{name} needs a safe-profile replacement hint"
            );
            assert!(
                unsafe_compatibility_gate(name).is_some(),
                "{name} must have a runtime dual gate derived from the manifest"
            );
        }

        let (kind, env_name, argument_name) =
            unsafe_compatibility_gate("browser_http_scrape").expect("direct fetch gate");
        assert_eq!(kind, "direct network fetch");
        assert_eq!(env_name, "HANDS_ALLOW_UNSAFE_DIRECT_FETCH");
        assert_eq!(argument_name, "allow_unsafe_fetch");

        for composite in [
            "browser_script",
            "browser_evaluate",
            "browser_eval",
            "browser_inject_script",
        ] {
            let (kind, env_name, argument_name) =
                unsafe_compatibility_gate(composite).expect("nested direct fetch gate");
            assert_eq!(kind, "direct network fetch");
            assert_eq!(env_name, "HANDS_ALLOW_UNSAFE_DIRECT_FETCH");
            assert_eq!(argument_name, "allow_unsafe_fetch");
            let gates = unsafe_compatibility_gates(composite);
            assert_eq!(gates.len(), 2);
            assert!(gates.contains(&(
                "raw or credential-adjacent output",
                "HANDS_ALLOW_UNSAFE_RAW_TOOLS",
                "allow_unsafe_raw"
            )));
        }

        for plugin_tool in [
            "hands_plugin_call",
            "hands_plugin_load",
            "hands_plugin_unload",
        ] {
            let (kind, env_name, argument_name) =
                unsafe_compatibility_gate(plugin_tool).expect("native plugin gate");
            assert_eq!(kind, "native plugin execution");
            assert_eq!(env_name, "HANDS_ALLOW_UNSAFE_PLUGINS");
            assert_eq!(argument_name, "allow_unsafe_plugin");
        }

        let inject_gates = unsafe_compatibility_gates("browser_inject_script");
        assert_eq!(inject_gates.len(), 2);
        assert!(inject_gates.contains(&(
            "direct network fetch",
            "HANDS_ALLOW_UNSAFE_DIRECT_FETCH",
            "allow_unsafe_fetch"
        )));
        assert!(inject_gates.contains(&(
            "raw or credential-adjacent output",
            "HANDS_ALLOW_UNSAFE_RAW_TOOLS",
            "allow_unsafe_raw"
        )));
        for alias in [
            "browser_accessibility_snapshot",
            "browser_page_dump",
            "uia_poll_events",
        ] {
            let (kind, env_name, argument_name) =
                unsafe_compatibility_gate(alias).expect("unsafe alias gate");
            assert_eq!(kind, "raw or credential-adjacent output");
            assert_eq!(env_name, "HANDS_ALLOW_UNSAFE_RAW_TOOLS");
            assert_eq!(argument_name, "allow_unsafe_raw");
        }
        assert!(unsafe_compatibility_gate("browser_click").is_none());
    }

    #[test]
    fn runtime_dispatch_fails_closed_for_unmanifested_tools() {
        let blocked = blocked_call_response("uia_poll_events_unclassified")
            .expect("unknown tools must be blocked");
        assert_eq!(blocked.get("reason"), Some(&json!("unmanifested_tool")));
    }

    #[test]
    fn compatibility_tool_annotations_are_explicitly_unsafe_debug() {
        let tool = manifest_tool("browser_eval").expect("browser_eval");
        let annotated = annotate_definition(
            json!({
                "name": "browser_eval",
                "description": "test",
                "inputSchema": {"type": "object", "properties": {}}
            }),
            tool,
            ToolProfile::Compatibility,
        );
        assert_eq!(
            annotated.get("x-hands-profile"),
            Some(&json!("compatibility"))
        );
        assert_eq!(
            annotated.get("x-hands-profile-safety"),
            Some(&json!("unsafe-debug"))
        );
        assert_eq!(
            annotated.get("x-hands-safety"),
            Some(&json!("unsafe-debug-compatibility-only"))
        );
        assert!(compatibility_warning().contains("not a security-safe profile"));
    }
}
