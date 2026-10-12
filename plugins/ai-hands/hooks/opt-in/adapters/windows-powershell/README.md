# Windows PowerShell enforcement adapter

`ai-hands-hook.ps1` is the native Windows enforcement hook used in production on both
Claude Code and Grok CLI. It is a single script parameterized by `-Mode` and `-Client`,
with per-client state under `%LOCALAPPDATA%\AI-Hands\hook_state\<client>` (override with
`AI_HANDS_HOOK_ROOT` for tests).

What it enforces (PreToolUse, deny-only — allow paths exit silently so the host's own
permission flow is preserved):

- **Destructive/commerce-action gate** — click/press/key/submit/script calls, and the
  batch tools (`browser_batch`, `uia_batch`) and `browser_agent` whose nested steps can
  click, are denied when their input matches destruction or commerce words (delete, wipe,
  submit, approve, buy, sell, purchase, pay, transfer, ...) unless the input carries
  `allow_destructive: true` or an explicit user-confirm marker. Everyday UI words (confirm,
  cancel, reset, discard) intentionally do not trip it.
- **Payment-entry gate** — typing, form-fill, batch, and script inputs (including
  `browser_evaluate`, `browser_eval`, `browser_inject_script`, and `browser_script`)
  containing card or bank field names (card number, CVV, expiry, IBAN, routing number, ...)
  or card-like digit runs are denied with **no override marker**; payment information is
  entered by the human.
- **Plaintext-credential gate** — denied when a field labelled as a secret is paired with a
  typed value (`hands_type` target `Password` plus `text`, `browser_type` selector
  `#password` plus `text`), when a secret-holding key (`password`, `api_key`, `cookies`,
  `Authorization`, ...) carries a value, or when a `password=` / `Bearer` string appears
  anywhere in the input. Only vault writes (`credential_store`, `credential_update`,
  `credential_refresh`, `totp_register`) may carry secrets. Use `credential_name` /
  `credential_ref` instead.
- **Network-capture persistence gate** — network, trace, route, `learn_api`, and
  performance-log calls may not write into a protected location you name. Set
  `AI_HANDS_PROTECTED_SINKS` to your own directory names separated by `;` (for example
  `Archive;secrets`); each name matches only as a whole path segment. It is empty by
  default, as in the shared policy, so nothing is blocked until you name a sink.
- **`uia_list_window` rate limit** — full desktop window enumeration is cooldown-limited
  (default 45 s, `HANDS_LIST_COOLDOWN_S`; bypass with `HANDS_ALLOW_UIA_LIST=1`).

It also keeps an **unverified-mutation streak** per session (PostToolUse): mutations
increment it; verification reads clear it (`hands_verify`, `browser_verify_state`,
`vision_diff`, `wait_for_visual`, screenshots, DOM reads, network reads). Only the tool
identity counts: a click on a button labelled "Verify" is still a mutation. At
`UserPromptSubmit`, a streak of 5+ is recorded as an advisory audit event.

All events land in `ai-hands-events.jsonl` under the state root, and each denial is
recorded as a `denied` event naming its gate. Inputs are redacted before they are
written: typed text, scripts, commands, bodies, and headers are logged as metadata only
(kind and length); secret- or payment-labelled values, secret strings, and card-like digit
runs are masked.

## Register on Claude Code (`~/.claude/settings.json`)

```json
{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "type": "command",
      "command": "powershell -NoProfile -ExecutionPolicy Bypass -File <path>/ai-hands-hook.ps1 -Mode SessionStart -Client claude_code" }] }],
    "UserPromptSubmit": [{ "hooks": [{ "type": "command",
      "command": "powershell -NoProfile -ExecutionPolicy Bypass -File <path>/ai-hands-hook.ps1 -Mode UserPromptSubmit -Client claude_code" }] }],
    "PreToolUse": [{ "matcher": "mcp__hands__|mcp__workflow__", "hooks": [{ "type": "command",
      "command": "powershell -NoProfile -ExecutionPolicy Bypass -File <path>/ai-hands-hook.ps1 -Mode PreToolUse -Client claude_code" }] }],
    "PostToolUse": [{ "matcher": "mcp__hands__|mcp__workflow__", "hooks": [{ "type": "command",
      "command": "powershell -NoProfile -ExecutionPolicy Bypass -File <path>/ai-hands-hook.ps1 -Mode PostToolUse -Client claude_code" }] }],
    "PostToolUseFailure": [{ "matcher": "mcp__hands__|mcp__workflow__", "hooks": [{ "type": "command",
      "command": "powershell -NoProfile -ExecutionPolicy Bypass -File <path>/ai-hands-hook.ps1 -Mode PostToolUseFailure -Client claude_code" }] }]
  }
}
```

## Register on Grok CLI (`~/.grok/hooks/<name>.json`)

The same five registrations, with matcher `(hands|browser_|uia_|vision_|workflow)` and
`-Client grok`.

## Relationship to the Python adapters

The sibling `claude`, `grok`, and `codex` Python adapters implement the shared
`universal_policy.py` in a portable way. This PowerShell adapter is the
Windows-production variant of the same policy family: richer streak/audit behavior, no
Python dependency. Pick one per surface — do not register both.
