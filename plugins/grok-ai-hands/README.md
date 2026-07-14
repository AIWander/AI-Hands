# grok-ai-hands

Grok (and Claude Code–compatible) **plugin** that packages the AI-Hands agent skill set and safety/audit hooks.

This is **not** the MCP server binary. Install [AI-Hands](https://github.com/AIWander/AI-Hands) (`hands.exe`) separately, then install this plugin so agents know how to drive it safely.

## What you get

### Skills

| Skill | Slash / trigger | Purpose |
|-------|-----------------|---------|
| `ai-hands` | `/ai-hands` | Meta-tool routing: prefer `hands_*` over raw browser/UIA/vision |
| `ai-hands-safety` | `/ai-hands-safety` | Real-click safety, confirmation gates, injection hygiene |
| `ai-hands-workflows` | `/ai-hands-workflows` | Multi-step recipes (`hands_script`, login, forms, desktop) |

### Hooks

| Event | Script | Behavior |
|-------|--------|----------|
| `PreToolUse` | `hooks/bin/ai_hands_pre_safety.py` | Deny destructive-looking targets without `allow_destructive`; soft rate-limit `uia_list_window` |
| `PostToolUse` / `PostToolUseFailure` | `hooks/bin/ai_hands_post_audit.py` | Redacted JSONL audit of AI-Hands tool calls |

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
