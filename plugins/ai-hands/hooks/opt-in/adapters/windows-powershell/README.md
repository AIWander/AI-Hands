# Windows PowerShell enforcement adapter

`ai-hands-hook.ps1` is the native Windows enforcement hook used in production on both
Claude Code and Grok CLI. It is a single script parameterized by `-Mode` and `-Client`,
with per-client state under `%LOCALAPPDATA%\AI-Hands\hook_state\<client>` (override with
`AI_HANDS_HOOK_ROOT` for tests).

What it enforces (PreToolUse, deny-only — allow paths exit silently so the host's own
permission flow is preserved):

- **Destructive/commerce-action gate** — click/press/key/submit calls whose input matches
  destruction or commerce words (delete, wipe, submit, approve, buy, sell, purchase, pay,
  transfer, ...) are denied unless the input carries `allow_destructive: true` or an
  explicit user-confirm marker. Everyday UI words (confirm, cancel, reset, discard)
  intentionally do not trip it.
- **Payment-entry gate** — typing/fill/script inputs containing card or bank field names
  (card number, CVV, expiry, IBAN, routing number, ...) or PAN-like digit runs are denied
  with **no override marker**; payment information is entered by the human.
- **Plaintext-credential gate** — secrets in ordinary tool inputs are denied; only
  keyring/vault credential operations may carry them. Use `credential_name` /
  `credential_ref` instead.
- **Network-capture persistence gate** — captured network traffic may not be persisted to
  durable knowledge stores; keep captures ephemeral and redacted.
- **`uia_list_window` rate limit** — full desktop window enumeration is cooldown-limited
  (default 45 s, `HANDS_LIST_COOLDOWN_S`; bypass with `HANDS_ALLOW_UIA_LIST=1`).

It also keeps an **unverified-mutation streak** (PostToolUse): mutations increment it,
verification reads (`hands_verify`, `vision_diff`, screenshots, DOM reads, network reads)
clear it, and a streak of 5+ is surfaced as an advisory audit event. All events land in
`cpc-hands-events.jsonl` under the state root, with secret redaction.

## Register on Claude Code (`~/.claude/settings.json`)

```json
{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "type": "command",
      "command": "powershell -NoProfile -ExecutionPolicy Bypass -File <path>/ai-hands-hook.ps1 -Mode SessionStart -Client claude_code" }] }],
    "PreToolUse": [{ "matcher": "mcp__hands__|mcp__workflow__", "hooks": [{ "type": "command",
      "command": "powershell -NoProfile -ExecutionPolicy Bypass -File <path>/ai-hands-hook.ps1 -Mode PreToolUse -Client claude_code" }] }],
    "PostToolUse": [{ "matcher": "mcp__hands__|mcp__workflow__", "hooks": [{ "type": "command",
      "command": "powershell -NoProfile -ExecutionPolicy Bypass -File <path>/ai-hands-hook.ps1 -Mode PostToolUse -Client claude_code" }] }]
  }
}
```

## Register on Grok CLI (`~/.grok/hooks/<name>.json`)

Same five-mode shape with matcher `(hands|browser_|uia_|vision_|workflow)` and
`-Client grok`; Grok additionally supports `UserPromptSubmit` and `PostToolUseFailure`
modes, which this script handles.

## Relationship to the Python adapters

The sibling `claude-grok` / `codex` Python adapters implement the shared
`universal_policy.py` in a portable way. This PowerShell adapter is the
Windows-production variant of the same policy family: richer streak/audit behavior, no
Python dependency. Pick one per surface — do not register both.
