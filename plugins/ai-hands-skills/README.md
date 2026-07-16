# AI-Hands skills-only plugin

This profile is for AI hosts that can load MCP tools and skills but cannot run lifecycle hooks. It contains no hook directory or executable policy adapter.

Its four skills divide setup, dense ability-first routing, action-boundary safety, and multi-step workflows without adding duplicate tool owners. The instruction guide contains a small dispatch block that asks the model to invoke those skills at hook-like moments. That improves consistency but remains behavioral guidance, not enforcement.

Install only one Hands profile in a host. Use the `ai-hands` profile when reviewed and runtime-proven hooks are available. Voice-inclusive installer variants add the separate Voice-Command plugin and Rust wrapper; they do not auto-start the microphone and do not include the full Voice App/listener runtime.
