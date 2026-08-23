# Legacy packages

Kept for people who installed them before, and for nothing else. Nothing here is
advertised in either marketplace, and nothing here is covered by
`scripts/validate-plugin-package.ps1`.

## grok-ai-hands

The original Grok-oriented package. Superseded by `plugins/ai-hands`, which serves
Codex, Claude-compatible hosts and Grok from one reviewed policy.

It is kept out of `plugins/` deliberately, because it contains the exact bypass the
current engine was built to remove: its pre-tool guard reads `allow_destructive`
straight from the tool input, so the model authorizes itself simply by asserting
that it may, and its entry point fails open when a payload will not parse. The
current policy treats model-supplied booleans as worthless, denies on a parse
failure, and routes consent-class actions to the human.

Do not install it. If you already have, install `ai-hands` or `ai-hands-skills`
instead and remove this one - loading both gives you two competing policy owners.
