<#
.SYNOPSIS
    Plain-PowerShell tests for Register-HandsClients.ps1 (no Pester dependency).

.DESCRIPTION
    Every case runs the writer as a child process (so exit codes are real) against synthetic fixtures
    under a temp -ConfigRoot. Case (f) runs -Detect against this machine and proves, by hashing, that
    the live configs were not written. Case (g) dry-runs against temp COPIES of the live configs.

    The live %APPDATA%\Claude, %USERPROFILE%\.codex and %USERPROFILE%\.grok configs are only ever
    read (hashed, copied); nothing in this file writes to them.

.PARAMETER Engine
    powershell (Windows PowerShell 5.1, what the installer uses) or pwsh. Default: powershell.

.PARAMETER SkipLive
    Skip cases (f) and (g), which need the three clients installed on this machine.
#>
param(
    [ValidateSet('powershell', 'pwsh')][string]$Engine = 'powershell',
    [switch]$SkipLive,
    [switch]$KeepTemp
)

$ErrorActionPreference = 'Stop'
$Writer = Join-Path $PSScriptRoot '..\scripts\Register-HandsClients.ps1' | Resolve-Path | Select-Object -ExpandProperty Path
$Utf8NoBom = New-Object System.Text.UTF8Encoding $false
$TempBase = Join-Path ([System.IO.Path]::GetTempPath()) ("hands-writer-tests-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $TempBase | Out-Null
$Exe = 'C:\Users\Test User\AppData\Local\AIWander\Hands\hands.exe'

$script:Pass = 0
$script:Fail = 0
$script:Results = New-Object System.Collections.Generic.List[string]

function Assert([bool]$Cond, [string]$Name) {
    if ($Cond) { $script:Pass++; $script:Results.Add("PASS  $Name") }
    else { $script:Fail++; $script:Results.Add("FAIL  $Name") }
}

function Invoke-Writer([string[]]$WriterArgs) {
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $out = & $Engine -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $Writer @WriterArgs 2>&1
    $ErrorActionPreference = $prev
    return [pscustomobject]@{ Code = $LASTEXITCODE; Text = (($out | ForEach-Object { "$_" }) -join "`n") }
}

function New-Root([string]$Name, [switch]$Claude, [switch]$Codex, [switch]$Grok) {
    $root = Join-Path $TempBase $Name
    New-Item -ItemType Directory -Path $root | Out-Null
    if ($Claude) {
        New-Item -ItemType Directory -Path (Join-Path $root 'AnthropicClaude') | Out-Null
        [System.IO.File]::WriteAllBytes((Join-Path $root 'AnthropicClaude\claude.exe'), [byte[]]@())
        New-Item -ItemType Directory -Path (Join-Path $root 'Claude') | Out-Null
    }
    if ($Codex) { New-Item -ItemType Directory -Path (Join-Path $root '.codex') | Out-Null }
    if ($Grok) { New-Item -ItemType Directory -Path (Join-Path $root '.grok\bin') | Out-Null }
    return $root
}

function Set-Text([string]$Path, [string]$Text, [switch]$Bom) {
    $enc = if ($Bom) { New-Object System.Text.UTF8Encoding $true } else { $Utf8NoBom }
    [System.IO.File]::WriteAllText($Path, $Text, $enc)
}

function Get-Text([string]$Path) { return [System.IO.File]::ReadAllText($Path, $Utf8NoBom) }
function Get-Hash([string]$Path) { return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash }
function Test-Bom([string]$Path) {
    $b = [System.IO.File]::ReadAllBytes($Path)
    return ($b.Length -ge 3 -and $b[0] -eq 0xEF -and $b[1] -eq 0xBB -and $b[2] -eq 0xBF)
}
function Get-Backups([string]$Path) {
    $dir = Join-Path (Split-Path -Parent $Path) 'backups'
    if (-not (Test-Path $dir)) { return @() }
    return @(Get-ChildItem -LiteralPath $dir -Filter ((Split-Path -Leaf $Path) + '.pre_hands_*'))
}
function Count-Lines([string]$Text, [string]$Pattern) {
    return @(($Text -split "`r?`n") | Where-Object { $_ -match $Pattern }).Count
}
# canonical JSON for semantic comparison (engine-independent)
function ConvertTo-Canon($Text) {
    $t = $Text
    if ($t.Length -gt 0 -and $t[0] -eq [char]0xFEFF) { $t = $t.Substring(1) }
    return ($t | ConvertFrom-Json | ConvertTo-Json -Depth 50 -Compress)
}

# ---------------------------------------------------------------- (a) desktop, BOM, other servers
$desktopOriginal = @'
{
  "mcpServers": {
    "autonomous": {
      "command": "C:\\CPC\\servers\\autonomous.exe",
      "args": ["--stdio"],
      "env": { "CPC_MODE": "desktop", "GREETING": "h\u00e9llo <tag> & 'q'" }
    },
    "local": { "command": "C:\\CPC\\servers\\local.exe", "args": [] }
  },
  "preferences": { "quickEntryShortcut": "off", "lastSeen": "2026-10-11T07:09:00Z", "zoom": 1.25, "count": 3, "flag": true, "none": null },
  "globalShortcut": ""
}
'@
$ra = New-Root 'a' -Claude
$cfgA = Join-Path $ra 'Claude\claude_desktop_config.json'
Set-Text $cfgA $desktopOriginal -Bom
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'ClaudeDesktop', '-ConfigRoot', $ra)
Assert ($r.Code -eq 0) "(a) desktop write exits 0 [got $($r.Code)]"
Assert ($r.Text -match 'ClaudeDesktop: written') '(a) summary says written'
Assert (-not (Test-Bom $cfgA)) '(a) output has no BOM'
$ja = Get-Text $cfgA | ConvertFrom-Json
Assert ($ja.mcpServers.hands.command -eq $Exe) '(a) hands.command is HandsExe'
Assert (@($ja.mcpServers.hands.args).Count -eq 0 -and (Get-Text $cfgA) -match '"args": \[\]') '(a) hands.args is an empty array'
Assert ((ConvertTo-Canon ($ja.mcpServers.autonomous | ConvertTo-Json -Depth 20)) -eq (ConvertTo-Canon (($desktopOriginal | ConvertFrom-Json).mcpServers.autonomous | ConvertTo-Json -Depth 20))) '(a) other server autonomous preserved (incl. env, unicode)'
Assert (@($ja.mcpServers.PSObject.Properties.Name) -join ',' -eq 'autonomous,local,hands') '(a) server order kept, hands appended'
Assert ((Get-Text $cfgA) -match '"lastSeen": "2026-10-11T07:09:00Z"') '(a) ISO date string preserved verbatim'
Assert ((Get-Text $cfgA) -match '"zoom": 1.25' -and (Get-Text $cfgA) -match '"none": null' -and (Get-Text $cfgA) -match '"flag": true') '(a) number/null/bool preserved'
$ba = Get-Backups $cfgA
Assert ($ba.Count -eq 1) '(a) one backup in backups\'
Assert ($ba.Count -eq 1 -and (Get-Text $ba[0].FullName).TrimStart([char]0xFEFF) -eq $desktopOriginal.TrimStart([char]0xFEFF)) '(a) backup holds the original content'
Assert (@(Get-ChildItem (Split-Path $cfgA) -Force -Filter '.*hands_tmp*').Count -eq 0) '(a) no temp file left behind'

# ---------------------------------------------------------------- (b) desktop already has hands
$rb = New-Root 'b' -Claude
$cfgB = Join-Path $rb 'Claude\claude_desktop_config.json'
Set-Text $cfgB '{"mcpServers":{"hands":{"command":"C:\\CPC\\servers\\hands.exe","args":[],"env":{"X":"1"}},"voice":{"command":"v.exe"}},"other":1}'
$h0 = Get-Hash $cfgB
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'ClaudeDesktop', '-ConfigRoot', $rb)
Assert ($r.Code -eq 2) "(b) existing hands without -Force exits 2 [got $($r.Code)]"
Assert ($r.Text -match 'ClaudeDesktop: skipped existing') '(b) summary says skipped existing'
Assert ((Get-Hash $cfgB) -eq $h0) '(b) file untouched when skipped'
Assert ((Get-Backups $cfgB).Count -eq 0) '(b) no backup when skipped'
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'ClaudeDesktop', '-ConfigRoot', $rb, '-Force')
Assert ($r.Code -eq 0 -and $r.Text -match 'ClaudeDesktop: written') "(b) -Force replaces [got $($r.Code)]"
$jb = Get-Text $cfgB | ConvertFrom-Json
Assert ($jb.mcpServers.hands.command -eq $Exe -and -not ($jb.mcpServers.hands.PSObject.Properties.Name -contains 'env')) '(b) hands entry replaced exactly'
Assert ($jb.mcpServers.voice.command -eq 'v.exe' -and $jb.other -eq 1) '(b) other keys preserved'
Assert ((Get-Backups $cfgB).Count -eq 1) '(b) backup taken before -Force write'

# ---------------------------------------------------------------- (c) codex toml
$codexOriginal = @"
model = "gpt-6"
approval_policy = "on-request"

[mcp_servers.autonomous]
command = 'C:\CPC\servers\autonomous.exe'
args = [
  "--stdio",
]

[mcp_servers.autonomous.tools.vol_read]
approval = "never"

[mcp_servers.autocon]
command = 'C:\CPC\servers\autocon.exe'
cwd = 'C:\CPC\servers'
enabled = false

[profiles.fast]
model = "gpt-6-mini"
"@ -replace "`r?`n", "`r`n"
$rc = New-Root 'c' -Codex
$cfgC = Join-Path $rc '.codex\config.toml'
Set-Text $cfgC ($codexOriginal + "`r`n")
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'Codex', '-ConfigRoot', $rc)
Assert ($r.Code -eq 0 -and $r.Text -match 'Codex: written') "(c) codex write exits 0 [got $($r.Code)]"
$tc = Get-Text $cfgC
Assert ((Count-Lines $tc '^\[mcp_servers\.hands\]$') -eq 1) '(c) exactly one [mcp_servers.hands]'
Assert ($tc -match "\[mcp_servers\.hands\]\r\ncommand = 'C:\\Users\\Test User\\AppData\\Local\\AIWander\\Hands\\hands\.exe'\r\n$") '(c) table appended at end, single-quoted literal, CRLF kept'
Assert ($tc.StartsWith($codexOriginal + "`r`n`r`n[mcp_servers.hands]")) '(c) original content intact, blank-line separator'
$h1 = Get-Hash $cfgC
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'Codex', '-ConfigRoot', $rc)
Assert ($r.Code -eq 2 -and $r.Text -match 'Codex: skipped existing') "(c) re-run skipped existing [got $($r.Code)]"
Assert ((Get-Hash $cfgC) -eq $h1) '(c) re-run leaves file byte-identical'
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'Codex', '-ConfigRoot', $rc, '-Force')
Assert ((Count-Lines (Get-Text $cfgC) '^\[mcp_servers\.hands\]$') -eq 1) '(c) -Force re-run still exactly one table'

# codex: existing url-style hands with a sub-table, -Force replaces whole block incl. sub-table
$rc2 = New-Root 'c2' -Codex
$cfgC2 = Join-Path $rc2 '.codex\config.toml'
Set-Text $cfgC2 "[mcp_servers.hands]`nurl = `"http://127.0.0.1:7772/mcp`"`nbearer_token_env_var = `"CPC_CODEX_HANDS_TOKEN`"`n`n[mcp_servers.hands.tools.browser_click]`napproval = `"never`"`n`n# programmer server`n[mcp_servers.programmer]`ncommand = 'p.exe'`n"
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'Codex', '-ConfigRoot', $rc2)
Assert ($r.Code -eq 2 -and $r.Text -match 'skipped existing') '(c2) url-style hands entry is skipped existing'
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'Codex', '-ConfigRoot', $rc2, '-Force')
$tc2 = Get-Text $cfgC2
Assert ($r.Code -eq 0 -and $tc2 -notmatch 'url = ' -and $tc2 -notmatch 'hands\.tools') '(c2) -Force replaces hands block and its own sub-tables'
Assert ($tc2 -match "# programmer server`n\[mcp_servers\.programmer\]`ncommand = 'p.exe'") '(c2) next table and its leading comment untouched'

# ---------------------------------------------------------------- (d) grok toml
$grokOriginal = @"
model = "grok-5"
disabled_mcp_servers = [
  "autonomous",
  "hands",
  "heartbeat",
  "local",
]

[mcp_servers.autonomous]
command = 'C:\CPC\servers\autonomous.exe'

[mcp_servers.hands]
command = 'C:\CPC\servers\hands.exe'
enabled = false

[mcp_servers.local]
command = 'C:\CPC\servers\local.exe'
"@
$rd = New-Root 'd' -Grok
$cfgD = Join-Path $rd '.grok\config.toml'
Set-Text $cfgD ($grokOriginal + "`n")
$hd = Get-Hash $cfgD
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'GrokCli', '-ConfigRoot', $rd)
Assert ($r.Code -eq 2 -and $r.Text -match 'GrokCli: skipped existing' -and (Get-Hash $cfgD) -eq $hd) '(d) disabled hands without -Force: skipped existing, untouched'
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'GrokCli', '-ConfigRoot', $rd, '-Force')
$td = Get-Text $cfgD
Assert ($r.Code -eq 0 -and $r.Text -match 'GrokCli: written') "(d) -Force write exits 0 [got $($r.Code)]"
Assert ((Count-Lines $td '^\[mcp_servers\.hands\]$') -eq 1) '(d) exactly one hands table'
Assert ($td -notmatch 'enabled = false') '(d) enabled = false dropped from hands table'
Assert ($td -match "\[mcp_servers\.hands\]`ncommand = 'C:\\Users\\Test User\\AppData\\Local\\AIWander\\Hands\\hands\.exe'`n`n\[mcp_servers\.local\]") '(d) block replaced in place'
Assert ($td -match "disabled_mcp_servers = \[`n  `"autonomous`",`n  `"heartbeat`",`n  `"local`",`n\]") '(d) hands removed from multi-line disabled list, others kept'

$rd2 = New-Root 'd2' -Grok
$cfgD2 = Join-Path $rd2 '.grok\config.toml'
# the old grok-cpc-installer Add-Content anti-pattern: duplicate tables, single-line disabled list
Set-Text $cfgD2 "disabled_mcp_servers = [ `"ops`", `"hands`", `"stocks`" ]`n`n[mcp_servers.hands]`ncommand = 'old1.exe'`n`n[mcp_servers.x]`ncommand = 'x.exe'`n`n[mcp_servers.hands]`ncommand = 'old2.exe'`n"
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'GrokCli', '-ConfigRoot', $rd2, '-Force')
$td2 = Get-Text $cfgD2
Assert ($r.Code -eq 0 -and (Count-Lines $td2 '^\[mcp_servers\.hands\]$') -eq 1 -and $td2 -notmatch 'old[12]') '(d2) duplicate hands tables collapsed to one'
Assert ($td2 -match 'disabled_mcp_servers = \[ "ops", "stocks" \]') '(d2) hands removed from single-line disabled list'
Assert ($td2 -match "\[mcp_servers\.x\]`ncommand = 'x.exe'") '(d2) unrelated table kept'

$rd3 = New-Root 'd3' -Grok
$cfgD3 = Join-Path $rd3 '.grok\config.toml'
New-Item -ItemType File -Path (Join-Path $rd3 '.grok\bin\grok.exe') | Out-Null
Remove-Item (Join-Path $rd3 '.grok\bin\grok.exe'); New-Item -ItemType File -Path (Join-Path $rd3 '.grok\bin\grok.exe') | Out-Null
$r = Invoke-Writer @('-HandsExe', "C:\Users\O'Neil\hands.exe", '-Clients', 'GrokCli', '-ConfigRoot', $rd3)
Assert ($r.Code -eq 0 -and (Get-Text $cfgD3) -eq "[mcp_servers.hands]`ncommand = `"C:\\Users\\O'Neil\\hands.exe`"`n") '(d3) grok binary only: config created; apostrophe path uses a basic string'

# ---------------------------------------------------------------- (e) uninstall removes only hands
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'ClaudeDesktop,Codex', '-ConfigRoot', $ra, '-Uninstall')
$r2 = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'Codex', '-ConfigRoot', $rc, '-Uninstall')
Assert ($r.Code -eq 0 -and $r.Text -match 'ClaudeDesktop: removed') "(e) desktop uninstall removed [got $($r.Code)]"
Assert ((ConvertTo-Canon (Get-Text $cfgA)) -eq (ConvertTo-Canon $desktopOriginal)) '(e) desktop config semantically equal to the original after uninstall'
Assert ($r2.Code -eq 0 -and $r2.Text -match 'Codex: removed') '(e) codex uninstall removed'
Assert ((Get-Text $cfgC) -eq ($codexOriginal + "`r`n")) '(e) codex config byte-equal to the original after install+uninstall'
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'GrokCli', '-ConfigRoot', $rd, '-Uninstall')
$tde = Get-Text $cfgD
Assert ($r.Code -eq 0 -and (Count-Lines $tde 'mcp_servers\.hands') -eq 0) '(e) grok uninstall removes the hands table'
Assert ($tde -match '\[mcp_servers\.autonomous\]' -and $tde -match '\[mcp_servers\.local\]' -and $tde -notmatch '"hands"') '(e) grok: other tables kept; disabled list not restored (nothing else restored)'
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'Codex', '-ConfigRoot', $rc2, '-Uninstall', '-DryRun')
Assert ($r.Code -eq 0 -and $r.Text -match 'would remove') '(e) dry-run uninstall reports would remove'
Set-Text $cfgB '{"mcpServers":{"hands":{"command":"C:\\CPC\\servers\\hands.exe","args":[]}}}'
$hb = Get-Hash $cfgB
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'ClaudeDesktop', '-ConfigRoot', $rb, '-Uninstall')
Assert ($r.Code -eq 2 -and $r.Text -match 'skipped foreign' -and (Get-Hash $cfgB) -eq $hb) '(e) uninstall leaves a hands entry that is not ours'
$r = Invoke-Writer @('-HandsExe', $Exe, '-ConfigRoot', $rb, '-Uninstall')
Assert ($r.Text -match 'Codex: not detected' -and $r.Text -match 'GrokCli: not detected') '(e) undetected clients reported, nothing created'
Assert (-not (Test-Path (Join-Path $rb '.codex')) -and -not (Test-Path (Join-Path $rb '.grok'))) '(e) no config dirs created for undetected clients'

# ---------------------------------------------------------------- extra: MSIX layout, errors
$rm = New-Root 'msix'
New-Item -ItemType Directory -Path (Join-Path $rm 'AnthropicClaude') | Out-Null
New-Item -ItemType File -Path (Join-Path $rm 'AnthropicClaude\claude.exe') | Out-Null
$msixDir = Join-Path $rm 'Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude'
New-Item -ItemType Directory -Path $msixDir | Out-Null
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'ClaudeDesktop', '-ConfigRoot', $rm)
Assert ($r.Code -eq 0 -and ((Get-Text (Join-Path $msixDir 'claude_desktop_config.json')) | ConvertFrom-Json).mcpServers.hands.command -eq $Exe) '(x) MSIX-only layout: config created under LocalCache\Roaming\Claude'
$rx = New-Root 'bad' -Claude
Set-Text (Join-Path $rx 'Claude\claude_desktop_config.json') '{ "mcpServers": { "a": 1, } '
$hx = Get-Hash (Join-Path $rx 'Claude\claude_desktop_config.json')
$r = Invoke-Writer @('-HandsExe', $Exe, '-Clients', 'ClaudeDesktop', '-ConfigRoot', $rx)
Assert ($r.Code -eq 1 -and $r.Text -match 'ClaudeDesktop: error' -and (Get-Hash (Join-Path $rx 'Claude\claude_desktop_config.json')) -eq $hx) '(x) unparseable JSON: exit 1, file untouched'
$r = Invoke-Writer @('-Clients', 'GrokBot', '-ConfigRoot', $rx)
Assert ($r.Code -eq 1) '(x) unknown client name exits 1'
$r = Invoke-Writer @('-HandsExe', $Exe, '-ConfigRoot', (New-Root 'empty'))
Assert ($r.Code -eq 2 -and ([regex]::Matches($r.Text, 'not detected')).Count -eq 3) '(x) nothing installed: three not detected, exit 2'

# ---------------------------------------------------------------- (f) + (g) live machine, read-only
$LiveSummary = ''
if (-not $SkipLive) {
    $live = @(
        (Join-Path $env:APPDATA 'Claude\claude_desktop_config.json'),
        (Join-Path $env:USERPROFILE '.codex\config.toml'),
        (Join-Path $env:USERPROFILE '.grok\config.toml')
    )
    $before = @($live | ForEach-Object { if (Test-Path $_) { (Get-FileHash $_).Hash + '|' + (Get-Item $_).LastWriteTimeUtc.Ticks } else { 'absent' } })
    $r = Invoke-Writer @('-Detect')
    $after = @($live | ForEach-Object { if (Test-Path $_) { (Get-FileHash $_).Hash + '|' + (Get-Item $_).LastWriteTimeUtc.Ticks } else { 'absent' } })
    Assert ($r.Code -eq 0) '(f) -Detect exits 0'
    Assert ($r.Text -match 'ClaudeDesktop: present' -and $r.Text -match 'Codex: present' -and $r.Text -match 'GrokCli: present') '(f) -Detect reports all three present on this machine'
    Assert ((($before -join ',') -eq ($after -join ','))) '(f) live configs unchanged (hash + mtime) after -Detect'
    $LiveSummary += "--- -Detect (this machine)`n" + $r.Text + "`n"

    $rg = New-Root 'live-copy' -Claude -Codex -Grok
    Copy-Item $live[0] (Join-Path $rg 'Claude\claude_desktop_config.json')
    Copy-Item $live[1] (Join-Path $rg '.codex\config.toml')
    Copy-Item $live[2] (Join-Path $rg '.grok\config.toml')
    $copies = @((Join-Path $rg 'Claude\claude_desktop_config.json'), (Join-Path $rg '.codex\config.toml'), (Join-Path $rg '.grok\config.toml'))
    $cb = @($copies | ForEach-Object { (Get-FileHash $_).Hash })
    $r = Invoke-Writer @('-DryRun', '-ConfigRoot', $rg)
    Assert ($r.Code -eq 2) "(g) dry-run on copies of the live configs exits 2 [got $($r.Code)]"
    Assert ($r.Text -match 'ClaudeDesktop: skipped existing' -and $r.Text -match 'Codex: skipped existing' -and $r.Text -match 'GrokCli: skipped existing') '(g) all three skipped existing'
    Assert ((@($copies | ForEach-Object { (Get-FileHash $_).Hash }) -join ',') -eq ($cb -join ',')) '(g) copies unchanged'
    $LiveSummary += "--- -DryRun -ConfigRoot <temp copy of the live configs>`n" + ($r.Text -replace [regex]::Escape($rg), '<copy>') + "`n"
}

# ---------------------------------------------------------------- report
if (-not $KeepTemp) { Remove-Item -LiteralPath $TempBase -Recurse -Force }
$ver = (& $Engine -NoProfile -Command '$PSVersionTable.PSVersion.ToString()')
Write-Output "Register-HandsClients tests, engine $Engine $ver"
$script:Results | ForEach-Object { Write-Output $_ }
if ($LiveSummary) { Write-Output $LiveSummary.TrimEnd() }
Write-Output ("RESULT: {0} passed, {1} failed" -f $script:Pass, $script:Fail)
if ($script:Fail -gt 0) { exit 1 }
exit 0
