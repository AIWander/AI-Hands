[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$AppDir
)

$ErrorActionPreference = 'Stop'

$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$appRoot = [System.IO.Path]::GetFullPath($AppDir)
$resultPath = Join-Path $appRoot 'install-result.json'

try {
    $handsExe = Join-Path $appRoot 'bin\hands.exe'
    $pluginRoot = Join-Path $appRoot 'marketplace\plugins\ai-hands'
    $mcpPath = Join-Path $pluginRoot '.mcp.json'
    $instructionsPath = Join-Path $pluginRoot 'instructions\APPLY_TO_YOUR_AI.txt'

    foreach ($required in @($handsExe, $mcpPath, $instructionsPath)) {
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
            throw "Required installed file is missing: $required"
        }
    }

    $mcp = Get-Content -LiteralPath $mcpPath -Raw | ConvertFrom-Json
    if (-not $mcp.mcpServers.hands) {
        throw 'Installed .mcp.json does not contain mcpServers.hands.'
    }

    $mcp.mcpServers.hands.command = $handsExe
    $mcpJson = $mcp | ConvertTo-Json -Depth 20
    [System.IO.File]::WriteAllText($mcpPath, $mcpJson + [Environment]::NewLine, $utf8NoBom)

    $result = [ordered]@{
        schema = 'ai-hands-plugin-install-v1'
        success = $true
        app_root = $appRoot
        plugin_root = $pluginRoot
        hands_exe = $handsExe
        mcp_registration = $mcpPath
        instructions = $instructionsPath
        tool_profile = [string]$mcp.mcpServers.hands.env.HANDS_TOOL_PROFILE
        client_configs_changed = $false
        hooks_enabled = $false
        server_started = $false
    }
    $resultJson = $result | ConvertTo-Json -Depth 5
    [System.IO.File]::WriteAllText(
        $resultPath,
        $resultJson + [Environment]::NewLine,
        $utf8NoBom
    )
}
catch {
    $failure = [ordered]@{
        schema = 'ai-hands-plugin-install-v1'
        success = $false
        app_root = $appRoot
        error = [string]$_.Exception.Message
        client_configs_changed = $false
        hooks_enabled = $false
        server_started = $false
    }
    $failureJson = $failure | ConvertTo-Json -Depth 5
    [System.IO.File]::WriteAllText(
        $resultPath,
        $failureJson + [Environment]::NewLine,
        $utf8NoBom
    )
    Write-Error 'AI-Hands plugin finalization failed. See install-result.json in the installation directory.'
    exit 1
}
