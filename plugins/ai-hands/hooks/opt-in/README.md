# Optional AI-Hands hooks

These fragments are inert templates. The plugin and installer do not merge them into any host configuration.

Use only one policy owner for AI-Hands. Replace `__AI_HANDS_PLUGIN_ROOT__` with the absolute plugin path, review the rendered JSON, archive the host's live hook file, then apply it through that host's supported mechanism.

The shared engine:

- normalizes `hands__`, `AI-Hands__`, and `mcp__hands__` names;
- denies managed pre-tool calls when parsing, policy evaluation, or audit writing fails;
- rejects plaintext secrets and raw network captures aimed at durable Volumes;
- accepts risky-action consent only as a short-lived HMAC token bound to the exact host, tool, and argument hash;
- writes metadata-only command, script, text, body, and header audit fields.

No consent broker or signing key ships in this package. The adapters never inject consent. Enabling these hooks therefore denies covered risky calls until a separate trusted host integration supplies exact-call tokens.

A hook definition is not enforcement merely because it exists. It becomes a hard boundary only when the host trusts that exact definition, the runtime can block that event, and a harmless probe proves the hook actually fired. The Rust monitor fence remains independent of host hooks.
