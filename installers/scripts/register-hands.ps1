<#
.SYNOPSIS
    Registers AI-Hands as an MCP server in Claude Desktop's claude_desktop_config.json.

.DESCRIPTION
    Locates Claude Desktop's config at $env:APPDATA\Claude\claude_desktop_config.json,
    makes a timestamped backup, then adds (or replaces) the `hands` entry under
    `mcpServers` pointing at the binary resolved via `where.exe hands`.

    The script is ARCHIVE-FIRST: the config is copied to a `.bak.<timestamp>` file
    before any edit. If anything goes wrong, the backup path is printed for rollback.

.PARAMETER Force
    Replace an existing `mcpServers.hands` entry without prompting.

.PARAMETER DryRun
    Print what would change without writing the config.

.EXAMPLE
    .\register-hands.ps1
    # Interactive (prompts on overwrite)

.EXAMPLE
    .\register-hands.ps1 -Force
    # Overwrite existing registration without prompt

.EXAMPLE
    .\register-hands.ps1 -DryRun
    # Show diff, write nothing
#>

[CmdletBinding()]
param(
    [switch]$Force,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'

# --- Locate config ---------------------------------------------------------

$configPath = Join-Path $env:APPDATA 'Claude\claude_desktop_config.json'

if (-not (Test-Path -LiteralPath $configPath)) {
    Write-Error @"
Claude Desktop config not found at:
    $configPath

Claude Desktop may not be installed, or has never been launched. Launch
Claude Desktop once so it creates the config file, then re-run this script.
If you need to seed the file manually, the minimal content is:
    { "mcpServers": {} }
"@
    exit 1
}

# --- Resolve hands.exe -----------------------------------------------------

$handsCmd = Get-Command hands -ErrorAction SilentlyContinue
if (-not $handsCmd) {
    Write-Error @"
`hands` executable not found on PATH.

If you installed via winget, the portable command shim is registered under:
    %LOCALAPPDATA%\Microsoft\WinGet\Links

That directory should be on your PATH. Open a new shell after install, or
add the directory to PATH manually, then re-run this script.
"@
    exit 1
}

$handsPath = $handsCmd.Source
Write-Host "Resolved hands binary: $handsPath"

# --- Backup ----------------------------------------------------------------

$timestamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$backupPath = "$configPath.bak.$timestamp"

if (-not $DryRun) {
    Copy-Item -LiteralPath $configPath -Destination $backupPath -Force
    Write-Host "Backup written: $backupPath"
} else {
    Write-Host "[DryRun] Would write backup to: $backupPath"
}

# --- Load + parse JSON -----------------------------------------------------

$raw = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8
if ([string]::IsNullOrWhiteSpace($raw)) {
    $config = [pscustomobject]@{}
} else {
    try {
        $config = $raw | ConvertFrom-Json -Depth 100
    } catch {
        Write-Error "Failed to parse $configPath as JSON: $_"
        exit 1
    }
}

# Ensure mcpServers exists (PSCustomObject, not hashtable, so use Add-Member).
if (-not ($config.PSObject.Properties.Name -contains 'mcpServers')) {
    Add-Member -InputObject $config -MemberType NoteProperty -Name 'mcpServers' -Value ([pscustomobject]@{})
} elseif ($null -eq $config.mcpServers) {
    $config.mcpServers = [pscustomobject]@{}
}

# --- Overwrite-existing prompt --------------------------------------------

$existing = $null
if ($config.mcpServers.PSObject.Properties.Name -contains 'hands') {
    $existing = $config.mcpServers.hands
}

if ($null -ne $existing -and -not $Force) {
    $existingCmd = if ($existing.PSObject.Properties.Name -contains 'command') { $existing.command } else { '<unknown>' }
    Write-Host ""
    Write-Host "AI-Hands is already registered:"
    Write-Host "    command: $existingCmd"
    Write-Host ""
    $reply = Read-Host "Replace? [y/N]"
    if ($reply -notmatch '^[Yy]') {
        Write-Host "Aborted by user. No changes written."
        if (-not $DryRun) {
            Remove-Item -LiteralPath $backupPath -ErrorAction SilentlyContinue
        }
        exit 0
    }
}

# --- Build new entry -------------------------------------------------------

$newEntry = [pscustomobject]@{
    command = $handsPath
    args    = @()
}

# Replace or add.
if ($config.mcpServers.PSObject.Properties.Name -contains 'hands') {
    $config.mcpServers.hands = $newEntry
} else {
    Add-Member -InputObject $config.mcpServers -MemberType NoteProperty -Name 'hands' -Value $newEntry
}

# --- Write back ------------------------------------------------------------

$newJson = $config | ConvertTo-Json -Depth 100

if ($DryRun) {
    Write-Host ""
    Write-Host "[DryRun] Resulting JSON would be:"
    Write-Host "----------------------------------"
    Write-Host $newJson
    Write-Host "----------------------------------"
    Write-Host "[DryRun] No file changes written."
    exit 0
}

# Write atomically: temp file + Move-Item.
$tmpPath = "$configPath.tmp.$timestamp"
$newJson | Set-Content -LiteralPath $tmpPath -Encoding UTF8 -NoNewline
Move-Item -LiteralPath $tmpPath -Destination $configPath -Force

Write-Host ""
Write-Host "Success."
Write-Host "    Backup        : $backupPath"
Write-Host "    Binary        : $handsPath"
Write-Host "    Config        : $configPath"
Write-Host ""
Write-Host "Restart Claude Desktop for the change to take effect."
