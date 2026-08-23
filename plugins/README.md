# Plugins

Installable agent harness packages for AI-Hands.

| Plugin | Status | Purpose |
|--------|--------|---------|
| [`ai-hands/`](./ai-hands/) | Current: hook-capable | MCP registration, five ability-separated skills, and inert SessionStart, prompt, pre-tool, post-tool, and failure hook templates |
| [`ai-hands-skills/`](./ai-hands-skills/) | Current: skills-only | The same MCP and skill coverage with no hook code, plus a behavioral instruction adapter for hookless hosts |
| [`../legacy/grok-ai-hands/`](../legacy/grok-ai-hands/) | Legacy, moved out of this tree | Original Grok-oriented package. Kept for existing installations only; it carries a model-supplied `allow_destructive` bypass and fails open on a parse error, so it lives in `legacy/` where it cannot be installed by walking `plugins/`. See [`../legacy/README.md`](../legacy/README.md). |

Install exactly one current AI-Hands plugin in a host. Choose `ai-hands` only when the host can review and run hooks; otherwise choose `ai-hands-skills`. Loading both duplicates guidance without adding tools.

Each current profile is distributed in two installer flavors: Hands only and Hands plus the separate Voice-Command plugin. The Voice flavor adds speech I/O and the Rust wrapper but does not add tool privileges, auto-start the microphone, or bundle the full Voice App/listener runtime.
