# grok-ai-hands

> Legacy compatibility package. New installs should use [`../ai-hands/`](../ai-hands/), which provides the same AI-Hands ability owner across Codex, Claude-compatible hosts, and Grok. Do not load both packages in one host.

Grok (and Claude Code–compatible) **plugin** that packages the AI-Hands agent skill set and safety/audit hooks.

This is **not** the MCP server binary. Install [AI-Hands](https://github.com/AIWander/AI-Hands) (`hands.exe`) separately, then install this plugin so agents know how to drive it safely.

## What you get

### Skills

| Skill | Slash / trigger | Purpose |
|-------|-----------------|---------|
| `ai-hands` | `/ai-hands` | Meta-tool routing: prefer `hands_*` over raw browser/UIA/vision |
| `ai-hands-safety` | `/ai-hands-safety` | Real-click safety, confirmation gates, injection hygiene |
| `ai-hands-workflows` | `/ai-hands-workflows` | Safe visible-browser, fixed-batch, login, form, and desktop recipes |

The legacy package follows the same profile boundary as the current plugin: `default`, `full`, and `strict` are safe-advertised; `compatibility` is an unsafe debug escape hatch whose raw/direct-fetch/native-plugin tools require the profile, matching process environment gate, and matching per-call acknowledgement. The composite `browser_script` and `browser_evaluate` tools require both the direct-fetch and raw gate pairs. Whenever monitor scope is active, these vendor composites and aliases fail closed even when every compatibility gate is present: `browser_agent`/`agent`, `browser_batch`/`batch`, `browser_script`/`script`, `browser_evaluate`/`evaluate`, `browser_screenshot_burst`/`screenshot_burst`, `browser_scroll_collect`/`scroll_collect`, `browser_wait_stable`/`wait_stable`, and `retry_click` (with `browser_retry_click` treated defensively as an alias). Their nested vendor steps cannot revalidate the bound browser window. Use individually scoped browser calls or compatibility-gated `hands_script`, which centrally revalidates each nested call.

Before enabling a monitor fence, clear browser routes and stop any active trace; fence activation refuses while either persistent state is active. Under an active fence, `browser_route` and `browser_trace_start` fail closed, while `browser_route_remove`, `browser_route_clear`, and `browser_trace_stop` remain available so cleanup cannot be trapped.

### Hooks

| Event | Script | Behavior |
|-------|--------|----------|
| `PreToolUse` | `hooks/bin/ai_hands_pre_safety.py` | Deny destructive-looking targets without `allow_destructive`; soft rate-limit `uia_list_window` |
| `PostToolUse` / `PostToolUseFailure` | `hooks/bin/ai_hands_post_audit.py` | Minimized JSONL audit with best-effort pattern redaction |

Logs default to the plugin data dir (`GROK_PLUGIN_DATA` / `CLAUDE_PLUGIN_DATA`) under `logs/`, or `~/.grok/plugin-data/grok-ai-hands/logs`.

Override with `AI_HANDS_LOG_DIR`. Bypass window-list rate limit with `AI_HANDS_ALLOW_UIA_LIST=1`.

## Install (Grok)

```bash
# From this monorepo subdirectory (recommended)
grok plugin install AIWander/AI-Hands#plugins/grok-ai-hands --trust

# Or a local checkout
grok plugin install ./plugins/grok-ai-hands --trust
```

Then enable if needed:

```bash
grok plugin enable grok-ai-hands
grok plugin details grok-ai-hands
```

### MCP server (separate)

Point Grok at your installed binary, e.g. in `~/.grok/config.toml`:

```toml
[mcp_servers.AI-Hands]
command = "C:\\Path\\To\\hands.exe"
# or hang-safe build path
```

Without the MCP server running, skills still load but tools will not execute.

## Layout

```
plugins/grok-ai-hands/
├── .claude-plugin/plugin.json
├── README.md
├── hooks/
│   ├── hooks.json
│   └── bin/
│       ├── ai_hands_pre_safety.py
│       └── ai_hands_post_audit.py
└── skills/
    ├── ai-hands/
    │   ├── SKILL.md
    │   └── references/tool-map.md
    ├── ai-hands-safety/
    │   └── SKILL.md
    └── ai-hands-workflows/
        └── SKILL.md
```

## Requirements

- Python 3 on `PATH` (for hooks)
- Grok CLI with plugins/hooks support, **or** Claude Code–compatible plugin loader
- AI-Hands MCP binary for actual automation

## Relationship to repo-root `skills/`

The root `skills/hands` and `skills/hands-getting-started` docs remain the broader product skill pack. This plugin is the **Grok-oriented packaging**: meta-first routing, safety skill, workflows, and runtime hooks with portable `${CLAUDE_PLUGIN_ROOT}` paths.

## License

Apache-2.0 — same as [AI-Hands](https://github.com/AIWander/AI-Hands).
