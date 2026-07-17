# Plugins

Installable agent harness packages for AI-Hands.

| Plugin | Status | Purpose |
|--------|--------|---------|
| [`ai-hands/`](./ai-hands/) | Current: hook-capable | MCP registration, four ability-separated skills, and inert SessionStart, prompt, pre-tool, post-tool, and failure hook templates |
| [`ai-hands-skills/`](./ai-hands-skills/) | Current: skills-only | The same MCP and skill coverage with no hook code, plus a behavioral instruction adapter for hookless hosts |
| [`grok-ai-hands/`](./grok-ai-hands/) | Legacy compatibility | Original Grok-oriented package retained for existing installations; do not use for new installs |

Install exactly one current AI-Hands plugin in a host. Choose `ai-hands` only when the host can review and run hooks; otherwise choose `ai-hands-skills`. Loading both duplicates guidance without adding tools.

Each current profile is distributed in two installer flavors: Hands only and Hands plus the separate Voice-Command plugin. The Voice flavor adds speech I/O and the Rust wrapper but does not add tool privileges, auto-start the microphone, or bundle the full Voice App/listener runtime.
