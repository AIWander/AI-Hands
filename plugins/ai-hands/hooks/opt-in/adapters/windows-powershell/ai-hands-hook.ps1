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
#      overrides for tests (CPC_HANDS_HOOK_ROOT / CPC_GROK_HOOK_ROOT honored for back-compat).

$ErrorActionPreference = 'Stop'
$root = if ($env:AI_HANDS_HOOK_ROOT) { $env:AI_HANDS_HOOK_ROOT }
        elseif ($env:CPC_HANDS_HOOK_ROOT) { $env:CPC_HANDS_HOOK_ROOT }
        elseif ($env:CPC_GROK_HOOK_ROOT) { $env:CPC_GROK_HOOK_ROOT }
        else { Join-Path (Join-Path $env:LOCALAPPDATA 'AI-Hands\hook_state') $Client }
$logPath = Join-Path $root 'cpc-hands-events.jsonl'
$streakPath = Join-Path $root 'unverified-mutation-streak.json'
$listWindowStatePath = Join-Path $root 'list_window_last.json'
# Legacy path - clear on session start so old hard-gate markers never resurrect
$legacyPendingPath = Join-Path $root 'pending-hands-verify.json'
$listWindowCooldownS = if ($env:HANDS_LIST_COOLDOWN_S) { [double]$env:HANDS_LIST_COOLDOWN_S } elseif ($env:AI_HANDS_LIST_COOLDOWN_S) { [double]$env:AI_HANDS_LIST_COOLDOWN_S } else { 45 }
$doctrinePointer = 'See the ai-hands plugin docs (skills/hands-recommended-instructions.md). Meta-first; attach carefully; wait; DOM/a11y before vision; redact network; vault creds. Verify is skill/batch discipline (not a hard PreToolUse chain). Skills: ai-hands / ai-hands-safety / ai-hands-workflows.'

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
  return ($value | ConvertTo-Json -Depth 64 -Compress)
}

function Redact-Text([string]$text) {
  if ([string]::IsNullOrEmpty($text)) { return $text }
  $redacted = $text
  $redacted = [regex]::Replace($redacted, '(?i)("?(authorization|cookie|set-cookie|x-api-key|api[_-]?key|token|access[_-]?token|refresh[_-]?token|secret|password|passwd|credential|value)"?\s*[:=]\s*")([^"]*)(")', '$1[REDACTED]$4')
  $redacted = [regex]::Replace($redacted, '(?i)(Bearer\s+)[A-Za-z0-9._~+/\-=]+', '$1[REDACTED]')
  $redacted = [regex]::Replace($redacted, '(?i)(sk-[A-Za-z0-9_\-]{12,})', '[REDACTED_API_KEY]')
  return $redacted
}

function Write-Event($event, $inputObj, $extra = $null) {
  New-Item -ItemType Directory -Force -Path $root | Out-Null
  $toolInputJson = Convert-ToJsonSafe $inputObj.toolInput
  $entry = [ordered]@{
    ts = (Get-Date).ToUniversalTime().ToString('o')
    mode = $Mode
    client = $Client
    event = $event
    sessionId = $inputObj.sessionId
    cwd = $inputObj.cwd
    toolName = $inputObj.toolName
    toolUseId = $inputObj.toolUseId
    input = (Redact-Text $toolInputJson)
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

function Get-Text($inputObj) {
  $name = [string]$inputObj.toolName
  $json = Convert-ToJsonSafe $inputObj.toolInput
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

function Is-Verifier([string]$toolName, [string]$text) {
  if ($toolName -match '(?i)(hands_verify|verify_expectations|vision_diff|vision_screenshot|vision_ocr|browser_get_text|browser_get_html|browser_exists|browser_extract_content|browser_a11y_snapshot|browser_get_all_network|browser_get_network_log|hands_network|network_poll|browser_reload|browser_refresh|browser_screenshot|hands_capture|read_screen_text|window_screenshot|credential_list)') {
    return $true
  }
  if ($text -match '(?i)(refresh.*read|re-read|reread|verify|hands_verify|vision_diff|get_all_network|get_network_log|credential_list)') {
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
  if ($toolName -notmatch '(?i)(click|press|key|submit|app_action|api_call|script|route|evaluate|eval)') {
    return $false
  }
  return ($text -match '(?i)\b(delete|remove|destroy|drop|wipe|purge|archive|submit|confirm|approve|authorize|transfer|pay|purchase|buy|sell|cancel|discard|overwrite|reset)\b')
}

function Has-ExplicitConfirm([string]$text) {
  return ($text -match '(?i)("?(allow_destructive|confirmed_by_user|destructive_confirmed|user_confirmed)"?\s*:\s*true|confirm(ed)?\s+by\s+user)')
}

function Has-PlaintextCredential([string]$toolName, [string]$text) {
  if ($toolName -match '(?i)(credential_store|credential_update|credential_refresh|keyring|vault)') {
    return $false
  }
  if ($text -match '(?i)"(password|passwd|api[_-]?key|access[_-]?token|refresh[_-]?token|secret|credential)"\s*:\s*"[^"]{4,}"') {
    return $true
  }
  return $false
}

function Persists-NetworkToVolumes([string]$toolName, [string]$text) {
  if ($toolName -notmatch '(?i)(network|trace|route|learn_api|browser_get_all_network|browser_get_network_log)') {
    return $false
  }
  return ($text -match '(?i)(c:\\\\my drive\\\\volumes|c:/my drive/volumes|/volumes/|\\\\volumes\\\\)')
}

# Hooks fail OPEN on script error (only an explicit deny decision blocks).
try {
  $inputObj = Read-HookInput

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
      $reason = "CPC Hands anti-stuck: uia_list_window rate-limited (cooldown ${listWindowCooldownS}s). Cache the previous list or use uia_focus_window(title=...) / hands_app_action(focus|open). Need another list now: set HANDS_ALLOW_UIA_LIST=1 (or AI_HANDS_ALLOW_UIA_LIST=1), or wait for cooldown. $doctrinePointer"
      Write-Event 'list_window_rate_limited' $inputObj
      Emit-Deny $reason
      exit 0
    }
    Set-ListWindowStamp
    Write-Event 'list_window_allowed' $inputObj
  }

  # VERIFY MODEL (2026-07-14): no hard PreToolUse chain.
  # Verification is skill/batch discipline + audit streak, not a blocking gate.
  # Hard gates below remain: destructive confirm, plaintext creds, network->Volumes, list cooldown.

  if ((Is-Destructive $toolName $text) -and -not (Has-ExplicitConfirm $text)) {
    Emit-Deny "CPC Hands MUST-gate: destructive-tagged action needs explicit user confirmation or allow_destructive=true. $doctrinePointer"
    exit 0
  }

  if (Has-PlaintextCredential $toolName $text) {
    Emit-Deny "CPC Hands MUST-gate: plaintext secrets are allowed only for workflow credential_store/refresh/keyring/vault operations that write to the OS keyring. Ordinary hands/browser/workflow action calls must reference credential_name or credential_ref. $doctrinePointer"
    exit 0
  }

  if (Persists-NetworkToVolumes $toolName $text) {
    Emit-Deny "CPC Hands MUST-gate: captured network traffic cannot be persisted to Volumes or durable logs. Redact tokens/PII and keep capture ephemeral. $doctrinePointer"
    exit 0
  }

  # Silent allow (see header change 2).
} catch {
  $errMsg = $_.Exception.Message
  try { Write-Event "hook_error: $errMsg" $inputObj } catch {}
  # Fail open silently: no output means the normal CC permission flow decides.
}
