# AI-Hands hook-capable plugin

This is the AI-Hands profile for hosts that can run reviewed lifecycle hooks. It includes the `hands` MCP registration, four narrowly triggered skills, and inert hook templates modeled on the Grok Hands lifecycle: session start, prompt submit, pre-tool safety, post-tool audit, and post-tool failure.

The installer never edits or trusts a host hook file. Review, render, install, trust, and probe the exact hook definition through the host's supported controls. A definition on disk is not enforcement; the Rust monitor fence remains the hard display boundary.

The optional hook adapters require Python 3.10 or newer on `PATH` as `python`, without pinning one minor version. Hands itself and the skills-only profile do not require Python.

Install only one Hands profile in a host. Use `ai-hands-skills` when the host cannot run hooks. Voice-inclusive installer variants add the separate Voice-Command plugin and Rust wrapper; they do not auto-start the microphone and do not include the full Voice App/listener runtime.

## Skill pack

- `ai-hands-getting-started`: install, activation, and monitor-fence verification
- `ai-hands`: dense ability-first tool routing
- `ai-hands-safety`: action-boundary consent, origin, and secret checks
- `ai-hands-workflows`: short recipes, verification loops, and failure recovery

The split is by trigger, not by duplicate tool ownership. `ai-hands` owns selection; the other skills add setup, risk, or multi-step procedure only when that boundary is active.
