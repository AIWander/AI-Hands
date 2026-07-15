//! Ingredient-based Planner/Router with per-server Requirements
//!
//! Returns: ingredients, dependencies, requirements, breadcrumb recommendation
//! Requirements are the server's own pre/post conditions for its tools.
//! When a lead planner assembles a cross-server plan, it collects requirements
//! from each involved server to build the complete protocol chain.
//!
//! Any server with a planner can lead. Format is identical across all servers.
//! Claude decides order and execution - the planner just maps the terrain.

use serde_json::{json, Value};

pub fn get_definition() -> Value {
    json!({
        "name": "plan",
        "description": "Analyze a task and return its ingredients: what tools are needed, which depend on each other, and whether breadcrumbing is warranted. Does NOT prescribe step order - Claude decides execution. Any server's planner can lead or hand off.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task": {"type": "string", "description": "What needs to be done"},
                "context": {"type": "string", "description": "Additional context (current topic, recent actions, etc.)"}
            },
            "required": ["task"]
        }
    })
}

pub fn plan(args: &Value) -> Value {
    let task = args.get("task").and_then(|v| v.as_str()).unwrap_or("");
    let context = args.get("context").and_then(|v| v.as_str()).unwrap_or("");
    if task.is_empty() {
        return json!({"error": "task is required"});
    }
    let t = task.to_lowercase();

    if t.contains("extract") || t.contains("insight") || t.contains("learn") {
        extraction_recipe(&t, context)
    } else if t.contains("write") || t.contains("update") || t.contains("edit") {
        file_write_recipe(&t, context)
    } else if t.contains("search") || t.contains("find") || t.contains("look up") {
        search_recipe(&t, context)
    } else if t.contains("consolidat") || t.contains("maintenance") || t.contains("cleanup") {
        maintenance_recipe(&t, context)
    } else if t.contains("build")
        || t.contains("compile")
        || t.contains("rebuild")
        || t.contains("deploy")
    {
        build_recipe(&t, context)
    } else if t.contains("research") || t.contains("investigate") {
        research_recipe(&t, context)
    } else if t.contains("topic") && (t.contains("create") || t.contains("new")) {
        new_topic_recipe(&t, context)
    } else if t.contains("boot") || t.contains("startup") || t.contains("status") {
        boot_recipe(&t, context)
    } else {
        json!({
            "task": task,
            "ingredients": [{"tool": "route", "role": "analysis", "why": "Analyze task to determine approach"}],
            "dependencies": [],
            "requirements": {"before": {}, "after": {}},
            "breadcrumb": {"recommended": false, "reason": "Single-tool task"},
            "lead": "self", "handoff_if": {}
        })
    }
}

fn extraction_recipe(task: &str, _ctx: &str) -> Value {
    json!({
        "task": task,
        "ingredients": [
            {"tool": "check_catalog", "role": "validation", "why": "Verify target topic exists in CATALOG.md"},
            {"tool": "check_dedup", "role": "validation", "why": "Avoid writing duplicate insight"},
            {"tool": "read", "role": "context", "why": "See existing content in Operating file before adding"},
            {"tool": "extract_to_topic", "role": "action", "why": "Write the insight to the correct topic folder"},
            {"tool": "check_propagation", "role": "protocol", "why": "See if changes need to spread to related files"},
            {"tool": "log_extraction", "role": "tracking", "why": "Record what was extracted and where"}
        ],
        "dependencies": [
            {"tool": "extract_to_topic", "needs": ["check_catalog", "check_dedup"], "why": "Validate before writing"},
            {"tool": "check_propagation", "needs": ["extract_to_topic"], "why": "Only meaningful after a write"},
            {"tool": "log_extraction", "needs": ["extract_to_topic"], "why": "Log the outcome"}
        ],
        "requirements": {
            "before": {
                "extract_to_topic": ["check_catalog must pass", "check_dedup must return is_duplicate=false"],
                "write": ["read current content first", "backup if file >50KB"]
            },
            "after": {
                "extract_to_topic": ["check_propagation", "log_extraction"],
                "write": ["check_propagation", "update_refs if structure changed"]
            }
        },
        "breadcrumb": {"recommended": false, "reason": "Standard extraction is routine. Breadcrumb if extracting 3+ insights in one pass."},
        "lead": "self",
        "handoff_if": {"needs_search": "utonomous", "needs_semantic": "echo"},
        "cross_server_requirements": {
            "knowledge_server": {
                "pre": ["check_catalog - verify topic exists", "check_dedup - avoid duplicate"],
                "post": ["check_propagation - spread changes", "log_extraction - record decision"]
            }
        }
    })
}

fn file_write_recipe(task: &str, _ctx: &str) -> Value {
    json!({
        "task": task,
        "ingredients": [
            {"tool": "read", "role": "context", "why": "Read current content before modifying"},
            {"tool": "write", "role": "action", "why": "Write updated content"},
            {"tool": "check_propagation", "role": "protocol", "why": "Check if changes affect other files"},
            {"tool": "update_refs", "role": "protocol", "why": "Update cross-references if file was renamed or restructured"}
        ],
        "dependencies": [
            {"tool": "write", "needs": ["read"], "why": "Must see current state before overwriting"},
            {"tool": "check_propagation", "needs": ["write"], "why": "Only after write completes"},
            {"tool": "update_refs", "needs": ["write"], "why": "Only if structure changed"}
        ],
        "requirements": {
            "before": {
                "write": ["read current content", "verify file path exists or create_topic first"],
                "batch_edit": ["read current content", "verify each edit target string exists in file"]
            },
            "after": {
                "write": ["check_propagation", "update_refs if renamed"],
                "batch_edit": ["check_propagation"]
            }
        },
        "breadcrumb": {"recommended": true, "reason": "File modifications are stateful and should be tracked for rollback."},
        "lead": "self", "handoff_if": {}
    })
}

fn search_recipe(task: &str, _ctx: &str) -> Value {
    json!({
        "task": task,
        "ingredients": [
            {"tool": "search", "role": "discovery", "why": "BM25 keyword search across Volumes"},
            {"tool": "read", "role": "context", "why": "Read full content of relevant results"},
            {"tool": "enrich", "role": "context", "why": "Surface related patterns from Operating files"}
        ],
        "dependencies": [
            {"tool": "read", "needs": ["search"], "why": "Need search results before reading specific files"},
            {"tool": "enrich", "needs": ["search"], "why": "Enrich based on what search found"}
        ],
        "requirements": {
            "before": {},
            "after": {
                "search": ["evaluate relevance before reading full files - skip low-scoring results"]
            }
        },
        "breadcrumb": {"recommended": false, "reason": "Read-only operation."},
        "lead": "self",
        "handoff_if": {"needs_semantic_search": "echo", "needs_web_search": "ops/local/programmer"}
    })
}

fn maintenance_recipe(task: &str, _ctx: &str) -> Value {
    json!({
        "task": task,
        "ingredients": [
            {"tool": "detect_stale", "role": "discovery", "why": "Find files not modified recently"},
            {"tool": "find_dupes", "role": "discovery", "why": "Identify duplicate content"},
            {"tool": "pending_consolidations", "role": "discovery", "why": "Check which topics need merging"},
            {"tool": "consolidate_topic", "role": "action", "why": "Merge dated insights into Operating files"},
            {"tool": "audit", "role": "validation", "why": "Scan for antipatterns in recent operations"}
        ],
        "dependencies": [
            {"tool": "consolidate_topic", "needs": ["pending_consolidations"], "why": "Only consolidate flagged topics"}
        ],
        "requirements": {
            "before": {
                "consolidate_topic": ["pending_consolidations must flag the topic", "preview with execute=false first"]
            },
            "after": {
                "consolidate_topic": ["verify Operating file updated correctly", "check_propagation"]
            }
        },
        "breadcrumb": {"recommended": true, "reason": "Maintenance modifies multiple files across topics."},
        "lead": "self", "handoff_if": {}
    })
}

fn build_recipe(task: &str, _ctx: &str) -> Value {
    json!({
        "task": task,
        "ingredients": [
            {"tool": "preflight_deploy", "role": "validation", "why": "Safety checks before build"},
            {"tool": "powershell/psession", "role": "execution", "why": "Backup current binary (archive-first rule)"},
            {"tool": "psession_create", "role": "setup", "why": "Persistent session for long builds (survives timeouts)"},
            {"tool": "psession_run", "role": "action", "why": "Run cargo build in persistent session"},
            {"tool": "build_learning", "role": "tracking", "why": "Extract learnings from build outcome"}
        ],
        "dependencies": [
            {"tool": "psession_run", "needs": ["psession_create", "preflight_deploy"], "why": "Session and safety first"},
            {"tool": "build_learning", "needs": ["psession_run"], "why": "Only after build completes"}
        ],
        "requirements": {
            "before": {
                "psession_run": ["psession_create must exist", "preflight_deploy must pass", "backup the current binary to the configured backup directory"],
                "any_build": ["NEVER use one-shot powershell with -Wait for cargo builds"]
            },
            "after": {
                "psession_run": ["verify binary exists and size > 0", "build_learning to capture outcome"],
                "deploy": ["backup old binary before overwriting", "verify new binary runs"]
            }
        },
        "breadcrumb": {"recommended": true, "reason": "Builds are multi-step with rollback potential."},
        "lead": "self",
        "handoff_if": {"needs_execution": "ops/local/programmer"},
        "warnings": ["NEVER use one-shot powershell with -Wait for cargo builds. Use psession.", "Always backup binary before overwriting (archive-first)."],
        "cross_server_requirements": {
            "execution_server": {
                "pre": ["preflight_deploy - safety check", "powershell/psession - backup current binary"],
                "post": ["psession_run - verify binary exists and is non-zero"]
            },
            "knowledge_server": {
                "post": ["build_learning - extract learnings from build", "check_propagation - if build changed tool definitions"]
            }
        }
    })
}

fn research_recipe(task: &str, _ctx: &str) -> Value {
    json!({
        "task": task,
        "ingredients": [
            {"tool": "search", "role": "discovery", "why": "Search existing Volumes knowledge"},
            {"tool": "read", "role": "context", "why": "Read relevant files in full"},
            {"tool": "extract_to_topic", "role": "action", "why": "Save new insights to Volumes"},
            {"tool": "check_dedup", "role": "validation", "why": "Avoid duplicating existing knowledge"}
        ],
        "dependencies": [
            {"tool": "read", "needs": ["search"], "why": "Read what search found"},
            {"tool": "check_dedup", "needs": ["search"], "why": "Compare findings against existing"},
            {"tool": "extract_to_topic", "needs": ["check_dedup"], "why": "Only extract if genuinely new"}
        ],
        "requirements": {
            "before": {
                "extract_to_topic": ["check_dedup must return is_duplicate=false", "check_catalog for target topic"]
            },
            "after": {
                "extract_to_topic": ["check_propagation", "log_extraction"]
            }
        },
        "breadcrumb": {"recommended": true, "reason": "Research produces multiple extractions."},
        "lead": "self",
        "handoff_if": {"needs_web": "ops/local/programmer", "needs_semantic": "echo"},
        "cross_server_requirements": {
            "execution_server": {
                "during": ["http_fetch/smart_fetch - web research", "powershell - file operations"]
            },
            "knowledge_server": {
                "pre": ["search - check existing knowledge first"],
                "post": ["extract_to_topic - save findings", "check_propagation - update related"]
            }
        }
    })
}

fn new_topic_recipe(task: &str, _ctx: &str) -> Value {
    json!({
        "task": task,
        "ingredients": [
            {"tool": "check_catalog", "role": "validation", "why": "Verify topic does not already exist"},
            {"tool": "create_topic", "role": "action", "why": "Scaffold STATUS.md and README.md"},
            {"tool": "write", "role": "action", "why": "Create Operating_{topic}.md with initial structure"},
            {"tool": "check_catalog", "role": "validation", "why": "Confirm catalog entry was created"}
        ],
        "dependencies": [
            {"tool": "create_topic", "needs": ["check_catalog"], "why": "Must confirm topic is new"},
            {"tool": "write", "needs": ["create_topic"], "why": "Topic folder must exist first"}
        ],
        "requirements": {
            "before": {
                "create_topic": ["check_catalog must return in_catalog=false"]
            },
            "after": {
                "create_topic": ["write Operating file with frontmatter, ToC placeholder, and backmatter"],
                "write": ["verify check_catalog now returns in_catalog=true"]
            }
        },
        "breadcrumb": {"recommended": false, "reason": "Small operation, easily reversible."},
        "lead": "self", "handoff_if": {}
    })
}

fn boot_recipe(task: &str, _ctx: &str) -> Value {
    json!({
        "task": task,
        "ingredients": [
            {"tool": "breadcrumb_status", "role": "discovery", "why": "Check for interrupted operations"},
            {"tool": "status", "role": "discovery", "why": "Overall system health"},
            {"tool": "analytics", "role": "context", "why": "Extraction and learning stats"}
        ],
        "dependencies": [],
        "requirements": {
            "before": {},
            "after": {
                "breadcrumb_status": ["if interrupted operation found, offer to resume or abort"],
                "boot": ["check reminders, check inbox, check recent activity"]
            }
        },
        "breadcrumb": {"recommended": false, "reason": "Boot is read-only discovery."},
        "lead": "self",
        "handoff_if": {"interrupted_operation_found": "atlas for breadcrumb resume"}
    })
}

/// Cross-server requirement maps.
/// When a plan has handoff_if pointing to another server domain,
/// these maps tell the lead what that domain's server would require.
/// This eliminates the need to call multiple planners for common workflows.
fn cross_server_requirements(domain: &str) -> Value {
    match domain {
        "knowledge" | "atlas" | "utonomous" | "learning" => json!({
            "domain": "knowledge",
            "servers": ["atlas", "utonomous", "learning"],
            "common_requirements": {
                "before_write": ["read current content", "check_catalog for target topic", "check_dedup if extracting"],
                "after_write": ["check_propagation", "update_refs if structure changed"],
                "before_extract": ["check_dedup against Operating file Learned Patterns section"],
                "after_extract": ["log_extraction", "check_propagation"]
            }
        }),
        "execution" | "ops" | "local" | "programmer" => json!({
            "domain": "execution",
            "servers": ["ops", "local", "programmer"],
            "common_requirements": {
                "before_build": ["preflight_deploy", "backup current binary (archive-first)"],
                "during_build": ["use psession for builds >30s (never one-shot powershell with -Wait)"],
                "after_build": ["verify binary exists and size >0", "build_learning for extraction"],
                "before_file_edit": ["read current content first"],
                "after_file_edit": ["verify changes applied"]
            }
        }),
        "automation" | "browser" | "vision" => json!({
            "domain": "automation",
            "servers": ["browser", "vision"],
            "common_requirements": {
                "before_navigate": ["ensure browser is launched"],
                "before_click": ["verify element exists (use exists or get_clickables)"],
                "after_action": ["screenshot to verify result"],
                "for_forms": ["screenshot before + after for verification"]
            }
        }),
        "ai_delegation" | "manager" | "claude-bridge" | "claude-runner" | "codex"
        | "gemini-mcp" => json!({
            "domain": "ai_delegation",
            "servers": ["manager", "claude-bridge", "claude-runner", "codex", "gemini-mcp"],
            "common_requirements": {
                "before_delegate": ["clear task description with expected output format"],
                "for_long_tasks": ["use persistent session (bridge) not fire-and-forget"],
                "after_complete": ["review output before using", "build_learning if code was produced"]
            }
        }),
        "services" | "google" | "reminder" | "echo" => json!({
            "domain": "services",
            "servers": ["google", "reminder", "echo", "voice"],
            "common_requirements": {
                "google_calendar": ["check for conflicts before creating events"],
                "google_drive": ["verify file doesn't already exist before upload"],
                "reminders": ["check existing reminders to avoid duplicates"],
                "echo_semantic": ["requires Ollama running - check echo:health first"]
            }
        }),
        _ => json!({"domain": "unknown", "note": "No known requirements for this domain"}),
    }
}

/// Assemble a unified plan from a primary plan and its handoff targets.
/// Call this with the result of plan() to get the full cross-server picture.
pub fn assemble(args: &Value) -> Value {
    let empty = json!({});
    let primary_plan = args.get("plan").unwrap_or(&empty);

    // Collect domains from handoff_if
    let empty_obj = json!({});
    let handoff = primary_plan.get("handoff_if").unwrap_or(&empty_obj);
    let mut domains_seen = std::collections::HashSet::new();
    let mut cross_reqs = Vec::new();

    if let Some(obj) = handoff.as_object() {
        for (_key, val) in obj {
            if let Some(target) = val.as_str() {
                // Extract server name from target like "ops/local/programmer" or "utonomous"
                let server = target.split('/').next().unwrap_or(target).trim();
                let reqs = cross_server_requirements(server);
                let domain = reqs
                    .get("domain")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();
                if !domain.is_empty() && domains_seen.insert(domain.clone()) {
                    cross_reqs.push(reqs);
                }
            }
        }
    }

    // Also check ingredients for server references
    if let Some(ingredients) = primary_plan.get("ingredients").and_then(|i| i.as_array()) {
        for ing in ingredients {
            if let Some(tool) = ing.get("tool").and_then(|t| t.as_str()) {
                // Map tools to domains
                let domain = match tool {
                    t if t.contains("breadcrumb")
                        || t.contains("read")
                        || t.contains("write")
                        || t.contains("extract")
                        || t.contains("catalog") =>
                    {
                        "knowledge"
                    }
                    t if t.contains("powershell")
                        || t.contains("psession")
                        || t.contains("build") =>
                    {
                        "execution"
                    }
                    t if t.contains("navigate")
                        || t.contains("click")
                        || t.contains("screenshot") =>
                    {
                        "automation"
                    }
                    t if t.contains("submit") || t.contains("delegate") => "ai_delegation",
                    _ => "",
                };
                if !domain.is_empty() && domains_seen.insert(domain.to_string()) {
                    cross_reqs.push(cross_server_requirements(domain));
                }
            }
        }
    }

    json!({
        "primary_plan": primary_plan,
        "cross_server_requirements": cross_reqs,
        "domains_involved": domains_seen.iter().cloned().collect::<Vec<_>>(),
        "note": "Primary plan ingredients + cross-server requirements = complete protocol. Claude executes in order respecting dependencies."
    })
}
