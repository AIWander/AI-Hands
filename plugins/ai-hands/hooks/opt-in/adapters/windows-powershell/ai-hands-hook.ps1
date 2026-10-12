param(
  [ValidateSet('SessionStart', 'UserPromptSubmit', 'PreToolUse', 'PostToolUse', 'PostToolUseFailure')]
  [string]$Mode,
  [string]$Client = 'claude_code'
)

# AI-Hands enforcement hook (native Windows PowerShell) for Claude Code and Grok CLI.
# Register with -Client claude_code / -Client grok so each surface gets its own state root.
# Design notes:
#   1. stdin key normalization: Claude Code sends snake_case (tool_name/tool_input/session_id/
#      tool_use_id); Grok CLI sends camelCase. Both accepted.
#   2. DENY-ONLY output: Claude Code treats an emitted permissionDecision 'allow' as AUTO-APPROVE,
#      bypassing the user permission dialog. Grok treats explicit allow as a no-op. All allow paths
#      here exit silently so the host's normal permission flow is preserved.
#   3. Deny shape: emits legacy decision 'block' plus hookSpecificOutput.permissionDecision 'deny'
#      (belt and suspenders - covers both hook API generations).
#   4. Per-client state root: %LOCALAPPDATA%\AI-Hands\hook_state\<client>; AI_HANDS_HOOK_ROOT
#      overrides for tests.
#   5. Audit redaction follows redact() in shared/policy/universal_policy.py: typed text, scripts,
#      commands, bodies and headers are logged as metadata only, and secret- or payment-labelled
#      values, secret strings and PAN-like digit runs are masked before anything reaches disk.
#   6. The unverified-mutation streak is kept per session (one state file per hashed session id),
#      so one session's verification or SessionStart never clears another session's streak.

$ErrorActionPreference = 'Stop'
$root = if ($env:AI_HANDS_HOOK_ROOT) { $env:AI_HANDS_HOOK_ROOT }
        else { Join-Path (Join-Path $env:LOCALAPPDATA 'AI-Hands\hook_state') $Client }
$logPath = Join-Path $root 'ai-hands-events.jsonl'
# Per-session streak file; set once the hook input (and its session id) has been read.
$streakPath = $null
# Pre-2026-10 single streak file shared by every session - removed on session start
$legacyStreakPath = Join-Path $root 'unverified-mutation-streak.json'
$listWindowStatePath = Join-Path $root 'list_window_last.json'
# Legacy path - clear on session start so old hard-gate markers never resurrect
$legacyPendingPath = Join-Path $root 'pending-hands-verify.json'
$listWindowCooldownS = if ($env:HANDS_LIST_COOLDOWN_S) { [double]$env:HANDS_LIST_COOLDOWN_S } elseif ($env:AI_HANDS_LIST_COOLDOWN_S) { [double]$env:AI_HANDS_LIST_COOLDOWN_S } else { 45 }
$doctrinePointer = 'See the ai-hands-safety skill and hooks/opt-in/README.md in the ai-hands plugin. Meta-first; attach carefully; wait; DOM/a11y before vision; redact network; vault creds. Verify is skill/batch discipline (not a hard PreToolUse chain). Skills: ai-hands / ai-hands-safety / ai-hands-workflows.'

# Key vocabulary shared with shared/policy/universal_policy.py. Label keys add the field
# locators Hands tools use (hands_type target, browser_type match_text/xpath).
$sensitiveFragments = @('password', 'passwd', 'secret', 'token', 'api_key', 'apikey', 'authorization', 'cookie', 'totp', 'credential')
$safeReferenceKeys = @('credential_name', 'credential_ref', 'totp_name', 'totp_ref')
$nonSecretKeySuffixes = @('_selector', '_field', '_label', '_name', '_id', '_path', '_url', '_uri', '_hint', '_pattern')
$labelKeys = @('field', 'key', 'label', 'name', 'type', 'aria_label', 'placeholder', 'selector', 'target', 'match_text', 'xpath')
$labelKeySuffixes = @('_field', '_key', '_label', '_name', '_placeholder', '_selector', '_type')
$valueKeys = @('content', 'input', 'text', 'value', 'val')
$valueKeySuffixes = @('_content', '_input', '_text', '_val', '_value')
$metadataOnlyKeys = @('bodies', 'body', 'cmd', 'cmds', 'command', 'commands', 'header', 'headers', 'script', 'scripts', 'text', 'texts')
$metadataOnlySuffixes = @('_body', '_cmd', '_command', '_header', '_headers', '_script', '_text')
$secretTextPatterns = @(
  '(?i)\bauthorization\s*[:=]\s*(?:bearer\s+|basic\s+)?[^\s"'']{3,}',
  '(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{4,}',
  '(?i)(?:--(?:password|passwd|token|api[-_]?key)\s+|\b(?:password|passwd|token|api[_-]?key|apikey)\s*[:=]\s*)["'']?[^\s"'';]{1,}|["''](?:password|passwd|token|api[_-]?key|apikey)["'']\s*(?:\]|:)\s*=*\s*["''][^"'']+["'']'
)
# Short bank codes (iban/swift/bic) match only as whole tokens, so 'public' or 'swiftly' never
# reads as a payment field; snake_case such as billing_iban still matches.
$paymentFieldPattern = '(?i)(card[_\s-]?number|cardnum|credit[_\s-]?card|debit[_\s-]?card|cvv2?|cvc|security[_\s-]?code|card[_\s-]?verification|expir(y|ation)|name[_\s-]?on[_\s-]?card|(?<![a-z])(?:iban|swift|bic)(?![a-z])|routing[_\s-]?number|account[_\s-]?number|billing[_\s-]?(zip|address)|payment[_\s-]?(method|info|details))'
# 13-19 digits, optionally grouped by spaces or dashes (card numbers and similar)
$panLikePattern = '(?<!\d)\d(?:[ -]?\d){12,18}(?!\d)'

function Read-HookInput {
  $raw = [Console]::In.ReadToEnd()
  if ([string]::IsNullOrWhiteSpace($raw)) {
    return [pscustomobject]@{}
  }
  $j = $raw | ConvertFrom-Json
  $names = @($j.PSObject.Properties.Name)
  $pick = {
    param($camel, $snake)
    if ($names -contains $camel -and $null -ne $j.$camel) { return $j.$camel }
    if ($names -contains $snake -and $null -ne $j.$snake) { return $j.$snake }
    return $null
  }
  $norm = [pscustomobject]@{
    toolName  = (& $pick 'toolName' 'tool_name')
    toolInput = (& $pick 'toolInput' 'tool_input')
    sessionId = (& $pick 'sessionId' 'session_id')
    toolUseId = (& $pick 'toolUseId' 'tool_use_id')
    cwd       = $j.cwd
  }
  foreach ($errKey in @('error', 'toolError', 'message', 'errorMessage')) {
    if ($names -contains $errKey -and $null -ne $j.$errKey) {
      $norm | Add-Member -NotePropertyName $errKey -NotePropertyValue $j.$errKey
    }
  }
  return $norm
}

function Convert-ToJsonSafe($value) {
  if ($null -eq $value) { return '{}' }
  return (ConvertTo-Json -InputObject $value -Depth 64 -Compress)
}

function Redact-Text([string]$text) {
  if ([string]::IsNullOrEmpty($text)) { return $text }
  $redacted = $text
  $redacted = [regex]::Replace($redacted, '(?i)("?(authorization|cookie|set-cookie|x-api-key|api[_-]?key|token|access[_-]?token|refresh[_-]?token|secret|password|passwd|credential|value)"?\s*[:=]\s*")([^"]*)(")', '$1[REDACTED]$4')
  $redacted = [regex]::Replace($redacted, '(?i)(Bearer\s+)[A-Za-z0-9._~+/\-=]+', '$1[REDACTED]')
  $redacted = [regex]::Replace($redacted, '(?i)(sk-[A-Za-z0-9_\-]{12,})', '[REDACTED_API_KEY]')
  $redacted = [regex]::Replace($redacted, $panLikePattern, '[REDACTED_DIGITS]')
  return $redacted
}

# ---- Structured inspection of tool input (ConvertFrom-Json objects, hashtables, arrays) ----

function Test-IsMap($value) {
  return (($value -is [System.Collections.IDictionary]) -or ($value -is [System.Management.Automation.PSCustomObject]))
}

function Test-IsList($value) {
  return (($value -is [System.Collections.IList]) -and -not ($value -is [string]))
}

function Get-MapPairs($map) {
  if ($map -is [System.Collections.IDictionary]) {
    foreach ($k in @($map.Keys)) { [pscustomobject]@{ Name = [string]$k; Value = $map[$k] } }
  } else {
    foreach ($p in $map.PSObject.Properties) { [pscustomobject]@{ Name = $p.Name; Value = $p.Value } }
  }
}

# camelCase -> snake_case, lower-case, '-' and ' ' -> '_' (so 'API key' reads like api_key)
function ConvertTo-PolicyKey($key) {
  $raw = ([string]$key).Trim()
  $snake = [regex]::Replace($raw, '(?<=[a-z0-9])(?=[A-Z])', '_')
  return $snake.ToLowerInvariant().Replace('-', '_').Replace(' ', '_')
}

function Test-EndsWithAny([string]$s, $suffixes) {
  foreach ($suffix in $suffixes) { if ($s.EndsWith($suffix)) { return $true } }
  return $false
}

function Test-HasSensitiveFragment([string]$normalized) {
  foreach ($fragment in $sensitiveFragments) { if ($normalized.Contains($fragment)) { return $true } }
  return $false
}

# A key that holds a secret (password, api_key, cookies), not one that names a field.
function Test-SecretishKey($key) {
  $k = ConvertTo-PolicyKey $key
  return (($safeReferenceKeys -notcontains $k) -and -not (Test-EndsWithAny $k $nonSecretKeySuffixes) -and (Test-HasSensitiveFragment $k))
}

# A label that names a secret field ('Password', '#password'); 'Token name' or
# 'Password hint' name ordinary fields, as in the shared policy.
function Test-SecretLabel($value) {
  if (-not ($value -is [string])) { return $false }
  return (Test-SecretishKey $value)
}

function Test-PaymentLabel($value) {
  return (($value -is [string]) -and ($value -match $paymentFieldPattern))
}

# credential_name / totp_name hold the name of a stored secret, not a field label.
function Test-LabelKey($key) {
  $k = ConvertTo-PolicyKey $key
  if ($safeReferenceKeys -contains $k) { return $false }
  return (($labelKeys -contains $k) -or (Test-EndsWithAny $k $labelKeySuffixes))
}

# Label keys win, so a locator like match_text is never read as the typed value.
function Test-ValueKey($key) {
  if (Test-LabelKey $key) { return $false }
  $k = ConvertTo-PolicyKey $key
  return (($valueKeys -contains $k) -or (Test-EndsWithAny $k $valueKeySuffixes))
}

function Test-MetadataOnlyKey($key) {
  $k = ConvertTo-PolicyKey $key
  return (($metadataOnlyKeys -contains $k) -or (Test-EndsWithAny $k $metadataOnlySuffixes))
}

function Test-SecretText([string]$text) {
  foreach ($pattern in $secretTextPatterns) { if ($text -match $pattern) { return $true } }
  return $false
}

# Booleans never carry a secret: ticking 'Remember password' is not typing one.
function Test-NonEmptyScalar($value, [int]$depth = 0) {
  if ($depth -gt 6 -or $null -eq $value -or $value -is [bool]) { return $false }
  if (Test-IsMap $value) {
    foreach ($p in (Get-MapPairs $value)) { if (Test-NonEmptyScalar $p.Value ($depth + 1)) { return $true } }
    return $false
  }
  if (Test-IsList $value) {
    foreach ($item in $value) { if (Test-NonEmptyScalar $item ($depth + 1)) { return $true } }
    return $false
  }
  return (-not [string]::IsNullOrWhiteSpace([string]$value))
}

function Test-SensitiveLabelPair($pairs) {
  foreach ($p in $pairs) {
    if ((Test-LabelKey $p.Name) -and ((Test-SecretLabel $p.Value) -or (Test-PaymentLabel $p.Value))) { return $true }
  }
  return $false
}

function Get-MetadataOnly($value) {
  if ($value -is [string]) { return [ordered]@{ storage = 'metadata-only'; kind = 'string'; length = $value.Length } }
  if (Test-IsMap $value) { return [ordered]@{ storage = 'metadata-only'; kind = 'object'; item_count = @(Get-MapPairs $value).Count } }
  if (Test-IsList $value) { return [ordered]@{ storage = 'metadata-only'; kind = 'array'; item_count = $value.Count } }
  if ($null -eq $value) { return [ordered]@{ storage = 'metadata-only'; kind = 'null' } }
  return [ordered]@{ storage = 'metadata-only'; kind = $value.GetType().Name }
}

function Redact-String([string]$text) {
  if (Test-SecretText $text) { return '[REDACTED]' }
  $masked = [regex]::Replace($text, $panLikePattern, '[REDACTED_DIGITS]')
  if ($masked.Length -gt 1000) { return $masked.Substring(0, 1000) + '[TRUNCATED]' }
  return $masked
}

# Redacted copy of a tool input, safe to write to the audit log.
function Redact-Value($value, [int]$depth = 0) {
  if ($depth -gt 7) { return '[TRUNCATED]' }
  if ($null -eq $value) { return $null }
  if ($value -is [string]) { return (Redact-String $value) }
  if (Test-IsMap $value) {
    $pairs = @(Get-MapPairs $value)
    $sensitiveLabel = Test-SensitiveLabelPair $pairs
    $out = [ordered]@{}
    foreach ($p in $pairs) {
      if (Test-MetadataOnlyKey $p.Name) {
        $out[$p.Name] = (Get-MetadataOnly $p.Value)
      } elseif ((Test-SecretishKey $p.Name) -or ($p.Name -match $paymentFieldPattern) -or ($sensitiveLabel -and (Test-ValueKey $p.Name))) {
        $out[$p.Name] = '[REDACTED]'
      } else {
        $out[$p.Name] = (Redact-Value $p.Value ($depth + 1))
      }
    }
    return $out
  }
  if (Test-IsList $value) {
    $items = New-Object System.Collections.ArrayList
    $n = [Math]::Min($value.Count, 100)
    for ($i = 0; $i -lt $n; $i++) { [void]$items.Add((Redact-Value $value[$i] ($depth + 1))) }
    return ,($items.ToArray())
  }
  if ($value -is [bool]) { return $value }
  if (([string]$value) -match '\d{13,}') { return '[REDACTED_DIGITS]' }
  return $value
}

# Every key and string value in a tool input, one per line of output.
function Get-AllStrings($value, [int]$depth = 0) {
  if ($depth -gt 7 -or $null -eq $value) { return }
  if ($value -is [string]) { return $value }
  if (Test-IsMap $value) {
    foreach ($p in (Get-MapPairs $value)) {
      $p.Name
      Get-AllStrings $p.Value ($depth + 1)
    }
    return
  }
  if (Test-IsList $value) {
    $n = [Math]::Min($value.Count, 100)
    for ($i = 0; $i -lt $n; $i++) { Get-AllStrings $value[$i] ($depth + 1) }
    return
  }
  return [string]$value
}

function Write-Event($event, $inputObj, $extra = $null) {
  New-Item -ItemType Directory -Force -Path $root | Out-Null
  # Structural redaction first, then the string pass. If redaction itself fails, log a
  # marker rather than the raw input.
  try {
    $inputJson = Redact-Text (Convert-ToJsonSafe (Redact-Value $inputObj.toolInput))
  } catch {
    $inputJson = '"[REDACTION_FAILED]"'
  }
  $entry = [ordered]@{
    ts = (Get-Date).ToUniversalTime().ToString('o')
    mode = $Mode
    client = $Client
    event = $event
    sessionId = $inputObj.sessionId
    cwd = $inputObj.cwd
    toolName = $inputObj.toolName
    toolUseId = $inputObj.toolUseId
    input = $inputJson
  }
  if ($null -ne $extra) {
    foreach ($p in $extra.PSObject.Properties) {
      $entry[$p.Name] = $p.Value
    }
  }
  foreach ($errKey in @('error', 'toolError', 'message', 'errorMessage')) {
    if ($inputObj.PSObject.Properties.Name -contains $errKey -and $null -ne $inputObj.$errKey) {
      $entry['error'] = (Redact-Text ([string]$inputObj.$errKey))
      break
    }
  }
  # Serialized append (2026-07-19 audit): concurrent session starts raced Add-Content and
  # produced "process cannot access the file" hook_error entries. Retry briefly on lock.
  $line = ($entry | ConvertTo-Json -Compress -Depth 8)
  for ($i = 0; $i -lt 5; $i++) {
    try {
      Add-Content -Path $logPath -Value $line -Encoding UTF8 -ErrorAction Stop
      return
    } catch [System.IO.IOException] {
      Start-Sleep -Milliseconds (20 * ($i + 1))
    }
  }
}

# Deny-only output (see header change 2). Allow paths return silently.
function Emit-Deny([string]$reason) {
  $obj = [ordered]@{
    decision = 'block'
    reason = $reason
    hookSpecificOutput = [ordered]@{
      hookEventName = 'PreToolUse'
      permissionDecision = 'deny'
      permissionDecisionReason = $reason
    }
  }
  $obj | ConvertTo-Json -Compress -Depth 5
}

# Log which gate fired, then deny and end the hook.
function Deny-Call([string]$gate, [string]$reason, $inputObj) {
  try { Write-Event 'denied' $inputObj ([pscustomobject]@{ gate = $gate }) } catch { }
  Emit-Deny $reason
  exit 0
}

function Get-Text($inputObj) {
  $name = [string]$inputObj.toolName
  $json = Convert-ToJsonSafe $inputObj.toolInput
  # Windows PowerShell 5.1 writes ' < > & as backslash-u escapes (u0027, u003c, u003e, u0026).
  # Decode them so the word-boundary checks read value='4111...' and name='buy' the same way
  # as PowerShell 7. The backslash is built at runtime to keep the escapes out of this file.
  foreach ($code in @('0027', '003c', '003e', '0026')) {
    $json = $json.Replace(([string][char]92) + 'u' + $code, [string][char][Convert]::ToInt32($code, 16))
  }
  return (($name + "`n" + $json).ToLowerInvariant())
}

function Is-HandsLike([string]$toolName) {
  return ($toolName -match '(?i)(hands|browser_|uia_|vision_|workflow)')
}

function Is-ListWindow([string]$toolName) {
  return ($toolName -match '(?i)uia_list_window')
}

function Is-ListWindowBypass {
  $v = if ($env:HANDS_ALLOW_UIA_LIST) { $env:HANDS_ALLOW_UIA_LIST } else { $env:AI_HANDS_ALLOW_UIA_LIST }
  return ($v -match '^(1|true|yes)$')
}

function Test-ListWindowCooldown {
  if (-not (Test-Path -Path $listWindowStatePath)) { return $false }
  try {
    $state = Get-Content -Path $listWindowStatePath -Raw | ConvertFrom-Json
    $last = [DateTimeOffset]::Parse($state.ts)
    return (([DateTimeOffset]::UtcNow - $last).TotalSeconds -lt $listWindowCooldownS)
  } catch {
    return $false
  }
}

function Set-ListWindowStamp {
  New-Item -ItemType Directory -Force -Path $root | Out-Null
  $marker = [pscustomobject]@{
    ts = (Get-Date).ToUniversalTime().ToString('o')
  }
  Set-Content -Path $listWindowStatePath -Value ($marker | ConvertTo-Json -Compress) -Encoding UTF8
}

# Only read/verification tool identities clear the streak. Argument text is not evidence of
# a fresh observation (a click on a "Verify purchase" button is still a mutation); the text
# check only recognizes verifier tool names nested in batch or script steps.
function Is-Verifier([string]$toolName, [string]$text) {
  if ($toolName -match '(?i)(hands_verify|verify_expectations|browser_verify|wait_for_visual|vision_diff|vision_screenshot|vision_ocr|browser_get_text|browser_get_html|browser_exists|browser_extract_content|browser_a11y_snapshot|browser_get_all_network|browser_get_network_log|hands_network|network_poll|browser_reload|browser_refresh|browser_screenshot|hands_capture|read_screen_text|window_screenshot|credential_list)') {
    return $true
  }
  if ($text -match '(?i)(hands_verify|verify_expectations|browser_verify|vision_diff|get_all_network|get_network_log|credential_list)') {
    return $true
  }
  return $false
}

# Window chrome only - not a real UI mutation that needs state proof
function Is-FocusOnly([string]$toolName, [string]$text) {
  if ($toolName -match '(?i)(uia_focus_window|browser_focus)') { return $true }
  if ($toolName -match '(?i)hands_app_action' -and $text -match '(?i)"action"\s*:\s*"(focus|maximize|minimize|restore|snap_)') {
    return $true
  }
  return $false
}

function Is-Mutating([string]$toolName, [string]$text) {
  if (Is-FocusOnly $toolName $text) { return $false }
  return ($toolName -match '(?i)(hands_click|hands_type|hands_fill_form|hands_app_action|hands_login_recovery|hands_script|browser_click|browser_type|browser_fill|browser_press|browser_select|browser_submit|browser_eval|browser_evaluate|browser_inject|browser_route|uia_click|uia_type|uia_key|uia_shortcut|uia_menu|uia_set|workflow_api_call|workflow_replay|workflow_run|credential_store|credential_refresh|credential_delete|totp_register|totp_delete)')
}

# Hosts that send no session id share the 'default' streak, as in the shared policy.
function Get-StreakPath($inputObj) {
  $id = [string]$inputObj.sessionId
  if ([string]::IsNullOrWhiteSpace($id)) { $id = [string]$env:CLAUDE_SESSION_ID }
  if ([string]::IsNullOrWhiteSpace($id)) { $id = [string]$env:GROK_SESSION_ID }
  if ([string]::IsNullOrWhiteSpace($id)) { $id = 'default' }
  $sha = [System.Security.Cryptography.SHA256]::Create()
  try {
    $hash = $sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($id))
  } finally {
    $sha.Dispose()
  }
  $key = (($hash | ForEach-Object { $_.ToString('x2') }) -join '').Substring(0, 16)
  return (Join-Path $root "unverified-mutation-streak-$key.json")
}

function Get-Streak {
  if (-not (Test-Path -Path $streakPath)) {
    return [pscustomobject]@{ count = 0; lastTool = $null; ts = $null }
  }
  try {
    return (Get-Content -Path $streakPath -Raw | ConvertFrom-Json)
  } catch {
    return [pscustomobject]@{ count = 0; lastTool = $null; ts = $null }
  }
}

function Set-Streak([int]$count, $inputObj) {
  New-Item -ItemType Directory -Force -Path $root | Out-Null
  $marker = [pscustomobject]@{
    ts = (Get-Date).ToUniversalTime().ToString('o')
    count = $count
    lastTool = $inputObj.toolName
    toolUseId = $inputObj.toolUseId
  }
  Set-Content -Path $streakPath -Value ($marker | ConvertTo-Json -Compress) -Encoding UTF8
}

function Clear-Streak {
  if (Test-Path -Path $streakPath) {
    Remove-Item -Path $streakPath -Force
  }
}

function Is-Destructive([string]$toolName, [string]$text) {
  # batch/agent tools carry nested clicks and key presses, so their whole input is checked.
  if ($toolName -notmatch '(?i)(click|press|key|submit|app_action|api_call|script|route|evaluate|eval|batch|browser_agent)') {
    return $false
  }
  # Retune 2026-08-12: plain UI words (confirm/cancel/reset/discard) removed - they are everyday
  # game/app chrome and payment entry is hard-blocked separately. Commerce words stay.
  return ($text -match '(?i)\b(delete|remove|destroy|drop|wipe|purge|archive|submit|approve|authorize|transfer|pay|purchase|buy|sell|overwrite)\b')
}

function Has-ExplicitConfirm([string]$text) {
  return ($text -match '(?i)("?(allow_destructive|confirmed_by_user|destructive_confirmed|user_confirmed)"?\s*:\s*true|confirm(ed)?\s+by\s+user)')
}

# Mirrors _has_plaintext_secret() in the shared policy: a field labelled as a secret together
# with a typed value ({"target":"Password","text":"..."}), a secret-holding key with a value,
# or a secret assignment / bearer string anywhere in the input.
function Test-PlaintextSecret($value, [int]$depth = 0) {
  if ($depth -gt 7 -or $null -eq $value) { return $false }
  if ($value -is [string]) { return (Test-SecretText $value) }
  if (Test-IsMap $value) {
    $pairs = @(Get-MapPairs $value)
    $secretLabel = $false
    foreach ($p in $pairs) {
      if ((Test-LabelKey $p.Name) -and (Test-SecretLabel $p.Value)) { $secretLabel = $true; break }
    }
    if ($secretLabel) {
      foreach ($p in $pairs) {
        if ((Test-ValueKey $p.Name) -and (Test-NonEmptyScalar $p.Value)) { return $true }
      }
    }
    foreach ($p in $pairs) {
      if ((Test-SecretishKey $p.Name) -and (Test-NonEmptyScalar $p.Value)) { return $true }
      if (Test-PlaintextSecret $p.Value ($depth + 1)) { return $true }
    }
    return $false
  }
  if (Test-IsList $value) {
    $n = [Math]::Min($value.Count, 100)
    for ($i = 0; $i -lt $n; $i++) { if (Test-PlaintextSecret $value[$i] ($depth + 1)) { return $true } }
  }
  return $false
}

function Has-PlaintextCredential([string]$toolName, $inputObj) {
  # Vault writes (credential_store/refresh, TOTP registration) carry the secret on purpose.
  if ($toolName -match '(?i)(credential_store|credential_update|credential_refresh|totp_register|keyring|vault)') {
    return $false
  }
  return (Test-PlaintextSecret $inputObj.toolInput)
}

# Payment-information entry is HUMAN-ONLY. No override marker - unlike the destructive gate,
# allow_destructive does not bypass this. Card/bank field names or PAN-like digit runs in
# typing/fill/script inputs are denied outright.
function Is-PaymentEntry([string]$toolName, [string]$text) {
  if ($toolName -notmatch '(?i)(hands_type|hands_fill_form|browser_type|browser_fill|browser_fill_form|uia_type|type_into_window|hands_script|uia_batch|browser_batch|browser_eval|browser_inject|browser_script)') {
    return $false
  }
  if ($text -match $paymentFieldPattern) {
    return $true
  }
  if ($text -match '\b(?:\d[ -]?){13,19}\b') {
    return $true
  }
  return $false
}

# Durable sinks that captured network traffic must never be written into. Empty by default
# and operator-supplied, as in the shared policy: list your own directory names in
# AI_HANDS_PROTECTED_SINKS separated by ';' (for example: Volumes;Archive). A name matches
# only as a whole path segment.
function Get-ProtectedSinks {
  $names = @()
  foreach ($part in ([string]$env:AI_HANDS_PROTECTED_SINKS).Split(';')) {
    $name = $part.Trim().Trim([char[]]@('/', '\'))
    if ($name -and ($names -notcontains $name)) { $names += $name }
  }
  return $names
}

function Get-NetworkPersistSink([string]$toolName, $inputObj) {
  if ($toolName -notmatch '(?i)(network|route|trace|learn_api|performance_log)') {
    return $null
  }
  $sinks = @(Get-ProtectedSinks)
  if ($sinks.Count -eq 0) { return $null }
  $strings = @(Get-AllStrings $inputObj.toolInput)
  foreach ($name in $sinks) {
    $n = [regex]::Escape($name)
    $pattern = '(?i)(?:^|[\\/])' + $n + '[\\/]|[\\/]' + $n + '$'
    foreach ($s in $strings) {
      if ($s -match $pattern) { return $name }
    }
  }
  return $null
}

# Hooks fail OPEN on script error (only an explicit deny decision blocks).
try {
  $inputObj = Read-HookInput
  $streakPath = Get-StreakPath $inputObj

  if ($Mode -eq 'SessionStart') {
    # State-change-only logging (2026-07-19 audit): the unconditional heartbeat produced
    # 542 of 549 weekly entries. Liveness is already covered by claude_code_sessions.log;
    # this ledger now records only actual state transitions.
    # Daily liveness floor (2026-07-26 audit): zero post-tune entries made healthy silence
    # indistinguishable from a dead hook. ONE heartbeat per calendar day (first session)
    # keeps absence diagnostic without restoring the noise.
    try {
      $hbStamp = Join-Path $root 'liveness_day.txt'
      $today = (Get-Date).ToString('yyyy-MM-dd')
      $lastHb = if (Test-Path $hbStamp) { (Get-Content $hbStamp -TotalCount 1) } else { '' }
      if ($lastHb -ne $today) {
        New-Item -ItemType Directory -Force -Path $root | Out-Null
        Write-Event 'daily_liveness_heartbeat' $inputObj
        Set-Content -Path $hbStamp -Value $today -NoNewline
      }
    } catch { }
    if (Test-Path -Path $legacyPendingPath) {
      Remove-Item -Path $legacyPendingPath -Force
      Write-Event 'legacy_pending_cleared_session_start' $inputObj
    }
    # Drop the old shared streak file and streak files of sessions idle for a week.
    try {
      if (Test-Path -Path $legacyStreakPath) { Remove-Item -Path $legacyStreakPath -Force }
      $staleBefore = (Get-Date).ToUniversalTime().AddDays(-7)
      Get-ChildItem -Path $root -Filter 'unverified-mutation-streak-*.json' -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTimeUtc -lt $staleBefore } |
        Remove-Item -Force -ErrorAction SilentlyContinue
    } catch { }
    $streak = Get-Streak
    if ([int]$streak.count -gt 0) {
      Write-Event 'streak_cleared_session_start' $inputObj ([pscustomobject]@{ cleared_streak = $streak.count; lastTool = $streak.lastTool })
    }
    Clear-Streak
    return
  }

  if ($Mode -eq 'UserPromptSubmit') {
    $streak = Get-Streak
    if ($streak.count -ge 5) {
      # Advisory only - surface streak in audit; do not block the prompt.
      Write-Event 'unverified_mutation_streak_prompt' $inputObj ([pscustomobject]@{ streak = $streak.count; lastTool = $streak.lastTool })
    }
    # Non-event prompt_submitted entries dropped (2026-07-19 audit): state changes only.
    return
  }

  $toolName = [string]$inputObj.toolName
  $text = Get-Text $inputObj

  if ($Mode -eq 'PostToolUseFailure') {
    if (-not (Is-HandsLike $toolName)) { return }
    # Failures never advance the verify streak and never block.
    Write-Event 'post_tool_failure' $inputObj
    return
  }

  if ($Mode -eq 'PostToolUse') {
    if (-not (Is-HandsLike $toolName)) { return }

    if (Is-Verifier $toolName $text) {
      Clear-Streak
      Write-Event 'verification_observed' $inputObj
      return
    }

    if (Is-Mutating $toolName $text) {
      $streak = Get-Streak
      $next = [int]$streak.count + 1
      Set-Streak $next $inputObj
      Write-Event 'mutation_observed' $inputObj ([pscustomobject]@{ unverified_streak = $next })
      return
    }

    Write-Event 'post_tool_observed' $inputObj
    return
  }

  if ($Mode -ne 'PreToolUse') {
    return
  }

  if (-not (Is-HandsLike $toolName)) {
    # Silent allow: emitting an allow decision would auto-approve in Claude Code.
    return
  }

  Write-Event 'pre_tool_checked' $inputObj

  # Anti-stuck: rate-limit full desktop window enumeration (do not ban).
  if (Is-ListWindow $toolName) {
    if ((-not (Is-ListWindowBypass)) -and (Test-ListWindowCooldown)) {
      $reason = "AI-Hands anti-stuck: uia_list_window rate-limited (cooldown ${listWindowCooldownS}s). Cache the previous list or use uia_focus_window(title=...) / hands_app_action(focus|open). Need another list now: set HANDS_ALLOW_UIA_LIST=1 (or AI_HANDS_ALLOW_UIA_LIST=1), or wait for cooldown. $doctrinePointer"
      Write-Event 'list_window_rate_limited' $inputObj
      Emit-Deny $reason
      exit 0
    }
    Set-ListWindowStamp
    Write-Event 'list_window_allowed' $inputObj
  }

  # VERIFY MODEL (2026-07-14): no hard PreToolUse chain.
  # Verification is skill/batch discipline + audit streak, not a blocking gate.
  # Hard gates below remain: payment entry (no override), destructive confirm, plaintext creds,
  # network->durable store, list cooldown.

  if (Is-PaymentEntry $toolName $text) {
    Deny-Call 'payment_entry' "AI-Hands MUST-gate: payment information entry (card/bank numbers, CVV, expiry, billing fields) is human-only. No override. Hand control to the user for this step. $doctrinePointer" $inputObj
  }

  if ((Is-Destructive $toolName $text) -and -not (Has-ExplicitConfirm $text)) {
    Deny-Call 'destructive_action' "AI-Hands MUST-gate: destructive-tagged action needs explicit user confirmation or allow_destructive=true. $doctrinePointer" $inputObj
  }

  if (Has-PlaintextCredential $toolName $inputObj) {
    Deny-Call 'plaintext_credential' "AI-Hands MUST-gate: plaintext secrets are allowed only for workflow credential_store/refresh/keyring/vault operations that write to the OS keyring. Ordinary hands/browser/workflow action calls must reference credential_name or credential_ref. $doctrinePointer" $inputObj
  }

  $sink = Get-NetworkPersistSink $toolName $inputObj
  if ($sink) {
    Deny-Call 'network_persist' "AI-Hands MUST-gate: captured network traffic cannot be persisted into the protected location '$sink' (AI_HANDS_PROTECTED_SINKS). Redact tokens/PII and keep capture ephemeral. $doctrinePointer" $inputObj
  }

  # Silent allow (see header change 2).
} catch {
  try {
    $errMsg = Redact-Text $_.Exception.Message
    Write-Event "hook_error: $errMsg" $inputObj
  } catch {}
  # Fail open silently: no output means the normal CC permission flow decides.
}
