<#
.SYNOPSIS
    Register the AI-Hands stdio MCP server with installed AI hosts.

.DESCRIPTION
    Resolves the Hands home (%LOCALAPPDATA%\AIWander\Hands, or HANDS_HOME,
    or CPC_ROOT as a back-compat alias) and the absolute path of hands.exe.
    Copies first-run guidance into that home when it is missing.
    For each installed host that does not already have a hands entry, writes
    a timestamped backup and then the absolute command path.

    Hosts: Claude Desktop (classic and MSIX/Store), Claude Code, Codex CLI/IDE,
    Gemini CLI, Grok CLI, and Cursor. ChatGPT desktop is reported and not
    written, because it does not launch local stdio servers.

.PARAMETER HandsExe
    Absolute path to hands.exe. Defaults to `hands` on PATH, then the Hands home.

.PARAMETER HandsHome
    Override the install folder for this run.

.PARAMETER Force
    Replace an existing hands entry. Without this switch an existing entry is skipped.

.PARAMETER DryRun
    Print the actions without writing.
#>

[CmdletBinding()]
param(
    [string]$HandsExe,
    [string]$HandsHome,
    [switch]$Force,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'

function Get-ResolvedHandsHome {
    if ($HandsHome) { return $HandsHome }
    if ($env:HANDS_HOME -and $env:HANDS_HOME.Trim()) { return $env:HANDS_HOME.Trim() }
    if ($env:CPC_ROOT -and $env:CPC_ROOT.Trim()) { return $env:CPC_ROOT.Trim() }
    return (Join-Path (Join-Path $env:LOCALAPPDATA 'AIWander') 'Hands')
}

function Resolve-HandsExecutable {
    param([string]$Home)
    if ($HandsExe) {
        if (-not (Test-Path -LiteralPath $HandsExe)) {
            throw "HandsExe not found: $HandsExe"
        }
        return (Resolve-Path -LiteralPath $HandsExe).Path
    }
    $command = Get-Command hands -ErrorAction SilentlyContinue
    if (-not $command) { $command = Get-Command hands.exe -ErrorAction SilentlyContinue }
    if ($command -and $command.Source) { return $command.Source }
    foreach ($relative in @('bin\hands.exe', 'hands.exe')) {
        $candidate = Join-Path $Home $relative
        if (Test-Path -LiteralPath $candidate) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }
    throw @"
hands.exe was not found.
Pass -HandsExe, put hands on PATH, or place it in:
    $(Join-Path $Home 'bin\hands.exe')
"@
}

function Write-Utf8File {
    param([string]$Path, [string]$Text)
    $parent = Split-Path -Parent $Path
    if ($parent -and -not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
    $encoding = New-Object System.Text.UTF8Encoding $false
    $temp = "$Path.tmp.$([guid]::NewGuid().ToString('N'))"
    [System.IO.File]::WriteAllText($temp, $Text, $encoding)
    Move-Item -LiteralPath $temp -Destination $Path -Force
}

function Backup-ConfigFile {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    $dest = "$Path.bak.$(Get-Date -Format 'yyyyMMdd-HHmmss')"
    if ($DryRun) {
        Write-Host "[DryRun] Would back up $Path -> $dest"
        return $dest
    }
    Copy-Item -LiteralPath $Path -Destination $dest -Force
    Write-Host "Backup written: $dest"
    return $dest
}

function Test-HasHandsName {
    param([string[]]$Names)
    foreach ($name in $Names) {
        foreach ($candidate in @('hands', 'ai-hands', 'ai_hands')) {
            if ($name -eq $candidate) { return $true }
        }
    }
    return $false
}

function Install-GuidanceFiles {
    param([string]$Home)
    $payload = Join-Path (Split-Path -Parent $PSScriptRoot) 'payload'
    if (-not (Test-Path -LiteralPath $payload)) {
        Write-Host "Guidance payload not found beside the script. Existing install files were left as they are."
        return
    }
    $copies = @(
        @{ Source = 'README.md'; Dest = 'README.md' },
        @{ Source = 'AGENTS.md'; Dest = 'AGENTS.md' },
        @{ Source = 'skills\hands-getting-started\SKILL.md'; Dest = 'skills\hands-getting-started\SKILL.md' }
    )
    foreach ($copy in $copies) {
        $source = Join-Path $payload $copy.Source
        $dest = Join-Path $Home $copy.Dest
        if (-not (Test-Path -LiteralPath $source)) {
            Write-Warning "Missing payload file: $source"
            continue
        }
        if (Test-Path -LiteralPath $dest) {
            Write-Host "Guidance already present: $dest"
            continue
        }
        if ($DryRun) {
            Write-Host "[DryRun] Would copy $dest"
            continue
        }
        $parent = Split-Path -Parent $dest
        if (-not (Test-Path -LiteralPath $parent)) {
            New-Item -ItemType Directory -Force -Path $parent | Out-Null
        }
        Copy-Item -LiteralPath $source -Destination $dest
        Write-Host "Copied guidance: $dest"
    }
    foreach ($scriptName in @('Register-HandsClients.ps1', 'Initialize-HandsOnboarding.ps1')) {
        $source = Join-Path $PSScriptRoot $scriptName
        $dest = Join-Path $Home $scriptName
        if (-not (Test-Path -LiteralPath $source)) { continue }
        if ((Resolve-Path -LiteralPath $source).Path -eq $(if (Test-Path -LiteralPath $dest) { (Resolve-Path -LiteralPath $dest).Path } else { '' })) {
            continue
        }
        if (Test-Path -LiteralPath $dest) {
            Backup-ConfigFile -Path $dest | Out-Null
        }
        if ($DryRun) {
            Write-Host "[DryRun] Would copy $scriptName into $Home"
            continue
        }
        Copy-Item -LiteralPath $source -Destination $dest -Force
        Write-Host "Installed script: $dest"
    }
}

function New-HandsJsonEntry {
    param([string]$Command)
    return [pscustomobject]@{
        command = $Command
        args    = @()
        env     = [pscustomobject]@{ HANDS_TOOL_PROFILE = 'default' }
    }
}

function Register-JsonMcp {
    param(
        [string]$Id,
        [string]$Path,
        [string]$Command,
        [bool]$Create
    )
    $existed = Test-Path -LiteralPath $Path
    if (-not $existed -and -not $Create) {
        Write-Host "Skip ${Id}: host not installed ($Path)"
        return 'skipped-absent'
    }
    $raw = ''
    if ($existed) { $raw = Get-Content -LiteralPath $Path -Raw -Encoding UTF8 }
    if ([string]::IsNullOrWhiteSpace($raw)) {
        $config = [pscustomobject]@{}
    } else {
        try {
            $config = $raw | ConvertFrom-Json
        } catch {
            throw "Failed to parse ${Path}: $_"
        }
    }
    if (-not ($config.PSObject.Properties.Name -contains 'mcpServers') -or $null -eq $config.mcpServers) {
        $config | Add-Member -NotePropertyName mcpServers -NotePropertyValue ([pscustomobject]@{}) -Force
    }
    $names = @($config.mcpServers.PSObject.Properties.Name)
    if ((Test-HasHandsName -Names $names) -and -not $Force) {
        Write-Host "Skip ${Id}: hands entry already exists ($Path)"
        return 'skipped-existing'
    }
    if ($existed) { Backup-ConfigFile -Path $Path | Out-Null }
    $entry = New-HandsJsonEntry -Command $Command
    $replaced = $false
    foreach ($name in @($config.mcpServers.PSObject.Properties.Name)) {
        if (Test-HasHandsName -Names @($name)) {
            $config.mcpServers.$name = $entry
            $replaced = $true
        }
    }
    if (-not $replaced) {
        $config.mcpServers | Add-Member -NotePropertyName hands -NotePropertyValue $entry -Force
    }
    $json = ($config | ConvertTo-Json -Depth 100) + "`n"
    if ($DryRun) {
        Write-Host "[DryRun] Would write ${Id} -> $Path"
        Write-Host "         command: $Command"
        return 'dry-run'
    }
    Write-Utf8File -Path $Path -Text $json
    Write-Host "Registered ${Id}: $Path"
    return 'written'
}

function Test-TomlHasHands {
    param([string]$Text)
    return [regex]::IsMatch(
        $Text,
        '(?im)^\s*\[mcp_servers\.(?:"hands"|"ai-hands"|"ai_hands"|hands|ai-hands|ai_hands)\]\s*$'
    )
}

function Get-TomlHandsBlock {
    param([string]$Command)
    $escaped = $Command.Replace('\', '\\').Replace('"', '\"')
    return @"

[mcp_servers.hands]
command = "$escaped"
args = []

[mcp_servers.hands.env]
HANDS_TOOL_PROFILE = "default"
"@
}

function Register-TomlMcp {
    param(
        [string]$Id,
        [string]$Path,
        [string]$Command,
        [bool]$Create
    )
    $existed = Test-Path -LiteralPath $Path
    if (-not $existed -and -not $Create) {
        Write-Host "Skip ${Id}: host not installed ($Path)"
        return 'skipped-absent'
    }
    $text = ''
    if ($existed) { $text = Get-Content -LiteralPath $Path -Raw -Encoding UTF8 }
    if ($null -eq $text) { $text = '' }
    $hasHands = Test-TomlHasHands -Text $text
    if ($hasHands -and -not $Force) {
        Write-Host "Skip ${Id}: hands entry already exists ($Path)"
        return 'skipped-existing'
    }
    if ($existed) { Backup-ConfigFile -Path $Path | Out-Null }
    $escaped = $Command.Replace('\', '\\').Replace('"', '\"')
    if ($hasHands) {
        $pattern = '(?im)(?<=^\s*\[mcp_servers\.(?:"hands"|"ai-hands"|"ai_hands"|hands|ai-hands|ai_hands)\][^\[]*?)(^\s*command\s*=\s*.*$)'
        if ([regex]::IsMatch($text, $pattern)) {
            $text = [regex]::Replace($text, $pattern, "command = `"$escaped`"", 1)
        } else {
            $text = [regex]::Replace(
                $text,
                '(?im)(^\s*\[mcp_servers\.(?:"hands"|"ai-hands"|"ai_hands"|hands|ai-hands|ai_hands)\]\s*$)',
                "`$1`r`ncommand = `"$escaped`"",
                1
            )
        }
    } else {
        if ($text.Length -gt 0 -and -not $text.EndsWith("`n")) { $text += "`r`n" }
        $text += (Get-TomlHandsBlock -Command $Command)
        if (-not $text.EndsWith("`n")) { $text += "`r`n" }
    }
    if ($DryRun) {
        Write-Host "[DryRun] Would write ${Id} -> $Path"
        Write-Host "         command: $Command"
        return 'dry-run'
    }
    Write-Utf8File -Path $Path -Text $text
    Write-Host "Registered ${Id}: $Path"
    return 'written'
}

function Test-CommandPresent {
    param([string]$Name)
    return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

$homeDir = Get-ResolvedHandsHome
$exe = Resolve-HandsExecutable -Home $homeDir
Write-Host "Hands home: $homeDir"
Write-Host "Hands exe:  $exe"
if (-not $DryRun -and -not (Test-Path -LiteralPath $homeDir)) {
    New-Item -ItemType Directory -Force -Path $homeDir | Out-Null
}
Install-GuidanceFiles -Home $homeDir

$profileDir = $env:USERPROFILE
if (-not $profileDir) { $profileDir = $env:HOME }
$results = @()

# Claude Desktop classic
$claudeDir = Join-Path $env:APPDATA 'Claude'
$claudeConfig = Join-Path $claudeDir 'claude_desktop_config.json'
$claudeInstalled = (Test-Path -LiteralPath $claudeDir) -or (Test-Path -LiteralPath $claudeConfig)
$results += [pscustomobject]@{
    Host = 'claude_desktop'
    Result = (Register-JsonMcp -Id 'claude_desktop' -Path $claudeConfig -Command $exe -Create $claudeInstalled)
}

# Claude Desktop MSIX / Microsoft Store
$packages = Join-Path $env:LOCALAPPDATA 'Packages'
if (Test-Path -LiteralPath $packages) {
    $msixPackages = @(Get-ChildItem -LiteralPath $packages -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like 'Claude_*' } |
        Sort-Object Name)
    foreach ($package in $msixPackages) {
        $msixConfig = Join-Path $package.FullName 'LocalCache\Roaming\Claude\claude_desktop_config.json'
        $results += [pscustomobject]@{
            Host = 'claude_desktop_msix'
            Result = (Register-JsonMcp -Id "claude_desktop_msix $($package.Name)" -Path $msixConfig -Command $exe -Create $true)
        }
    }
}

# Claude Code user scope
$claudeCodePath = if ($env:CLAUDE_CONFIG_DIR) {
    Join-Path $env:CLAUDE_CONFIG_DIR '.claude.json'
} else {
    Join-Path $profileDir '.claude.json'
}
$claudeCodeInstalled = (Test-CommandPresent 'claude') -or (Test-Path -LiteralPath (Join-Path $profileDir '.claude')) -or (Test-Path -LiteralPath $claudeCodePath)
$results += [pscustomobject]@{
    Host = 'claude_code'
    Result = (Register-JsonMcp -Id 'claude_code' -Path $claudeCodePath -Command $exe -Create $claudeCodeInstalled)
}

# Codex CLI and Codex IDE share config.toml
$codexPath = Join-Path (Join-Path $profileDir '.codex') 'config.toml'
$codexInstalled = (Test-CommandPresent 'codex') -or (Test-Path -LiteralPath (Join-Path $profileDir '.codex'))
$results += [pscustomobject]@{
    Host = 'codex'
    Result = (Register-TomlMcp -Id 'codex' -Path $codexPath -Command $exe -Create $codexInstalled)
}

# Gemini CLI
$geminiPath = Join-Path (Join-Path $profileDir '.gemini') 'settings.json'
$geminiInstalled = (Test-CommandPresent 'gemini') -or (Test-Path -LiteralPath (Join-Path $profileDir '.gemini'))
$results += [pscustomobject]@{
    Host = 'gemini_cli'
    Result = (Register-JsonMcp -Id 'gemini_cli' -Path $geminiPath -Command $exe -Create $geminiInstalled)
}

# Grok CLI
$grokPath = if ($env:GROK_HOME) {
    Join-Path $env:GROK_HOME 'config.toml'
} else {
    Join-Path (Join-Path $profileDir '.grok') 'config.toml'
}
$grokInstalled = (Test-CommandPresent 'grok') -or (Test-Path -LiteralPath (Join-Path $profileDir '.grok')) -or [bool]$env:GROK_HOME
$results += [pscustomobject]@{
    Host = 'grok_cli'
    Result = (Register-TomlMcp -Id 'grok_cli' -Path $grokPath -Command $exe -Create $grokInstalled)
}

# Cursor
$cursorExe = Join-Path (Join-Path $env:LOCALAPPDATA 'Programs\cursor') 'Cursor.exe'
$cursorPath = Join-Path (Join-Path $profileDir '.cursor') 'mcp.json'
$cursorInstalled = (Test-CommandPresent 'cursor') -or (Test-Path -LiteralPath (Join-Path $profileDir '.cursor')) -or (Test-Path -LiteralPath $cursorExe)
$results += [pscustomobject]@{
    Host = 'cursor'
    Result = (Register-JsonMcp -Id 'cursor' -Path $cursorPath -Command $exe -Create $cursorInstalled)
}

Write-Host ""
Write-Host "ChatGPT desktop is not a stdio host. It needs a remote HTTPS MCP endpoint reached through an authenticated tunnel. See docs/chatgpt-desktop-mcp.md. This script does not open a public URL."
Write-Host ""
Write-Host "Summary:"
$results | ForEach-Object { Write-Host ("  {0}: {1}" -f $_.Host, $_.Result) }
Write-Host ""
Write-Host "Restart each host after a written config change."
