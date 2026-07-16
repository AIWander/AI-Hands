[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$AppDir,

    [Parameter(Mandatory = $true)]
    [ValidateSet('ai-hands', 'ai-hands-skills')]
    [string]$PluginId,

    [Parameter(Mandatory = $true)]
    [ValidateSet(0, 1)]
    [int]$VoiceIncluded
)

$ErrorActionPreference = 'Stop'
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$appRoot = [System.IO.Path]::GetFullPath($AppDir)
$marketplaceRoot = Join-Path $appRoot 'marketplace'
$pluginRoot = Join-Path $marketplaceRoot (Join-Path 'plugins' $PluginId)
$handsExe = Join-Path $appRoot 'bin\hands.exe'
$mcpPath = Join-Path $pluginRoot '.mcp.json'
$pluginManifestPath = Join-Path $pluginRoot '.codex-plugin\plugin.json'
$guideTemplatePath = Join-Path $pluginRoot 'instructions\APPLY_TO_YOUR_AI.txt'
$instructionsPath = Join-Path $appRoot 'APPLY_TO_YOUR_AI.txt'
$resultPath = Join-Path $appRoot 'install-result.json'
$withVoice = $VoiceIncluded -eq 1

function Write-Utf8Json {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [object]$Value,
        [int]$Depth = 20
    )

    $parent = Split-Path -Parent $Path
    [System.IO.Directory]::CreateDirectory($parent) | Out-Null
    $json = $Value | ConvertTo-Json -Depth $Depth
    [System.IO.File]::WriteAllText($Path, $json + [Environment]::NewLine, $utf8NoBom)
}

try {
    foreach ($required in @($handsExe, $mcpPath, $pluginManifestPath, $guideTemplatePath)) {
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
            throw "Required installed file is missing: $required"
        }
    }

    $pluginManifest = Get-Content -LiteralPath $pluginManifestPath -Raw | ConvertFrom-Json
    if ([string]$pluginManifest.name -ne $PluginId) {
        throw "Plugin manifest name '$($pluginManifest.name)' does not match selected profile '$PluginId'."
    }

    $mcp = Get-Content -LiteralPath $mcpPath -Raw | ConvertFrom-Json
    if (-not $mcp.mcpServers.hands) {
        throw 'Installed .mcp.json does not contain mcpServers.hands.'
    }
    $mcp.mcpServers.hands.command = $handsExe
    Write-Utf8Json -Path $mcpPath -Value $mcp

    $hookTemplatesPresent = Test-Path -LiteralPath (Join-Path $pluginRoot 'hooks\opt-in') -PathType Container
    if ($PluginId -eq 'ai-hands' -and -not $hookTemplatesPresent) {
        throw 'Hook-capable profile is missing its opt-in hook templates.'
    }
    if ($PluginId -eq 'ai-hands-skills' -and $hookTemplatesPresent) {
        throw 'Skills-only profile unexpectedly contains hook templates.'
    }

    $agentPlugins = @(
        [ordered]@{
            name = $PluginId
            source = [ordered]@{
                source = 'local'
                path = "./plugins/$PluginId"
            }
            policy = [ordered]@{
                installation = 'AVAILABLE'
                authentication = 'ON_INSTALL'
            }
            category = 'Productivity'
        }
    )
    $claudePlugins = @(
        [ordered]@{
            name = $PluginId
            description = [string]$pluginManifest.description
            author = [ordered]@{ name = 'AIWander' }
            category = 'productivity'
            source = "./plugins/$PluginId"
            homepage = 'https://github.com/AIWander/AI-Hands'
        }
    )

    $voiceExe = $null
    $voicePluginRoot = $null
    if ($withVoice) {
        $voiceExe = Join-Path $appRoot 'bin\voice-mcp.exe'
        $voicePluginRoot = Join-Path $marketplaceRoot 'plugins\voice-command'
        $voiceMcpPath = Join-Path $voicePluginRoot '.mcp.json'
        $voiceManifestPath = Join-Path $voicePluginRoot '.codex-plugin\plugin.json'
        $voiceGuideTemplate = Join-Path $appRoot 'installer\VOICE_APPLY_TO_YOUR_AI.template.txt'
        foreach ($required in @($voiceExe, $voiceMcpPath, $voiceManifestPath, $voiceGuideTemplate)) {
            if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
                throw "Voice package payload is missing: $required"
            }
        }

        $voiceMcp = Get-Content -LiteralPath $voiceMcpPath -Raw | ConvertFrom-Json
        if (-not $voiceMcp.mcpServers.voice) {
            throw 'Installed Voice .mcp.json does not contain mcpServers.voice.'
        }
        $voiceMcp.mcpServers.voice.command = $voiceExe
        Write-Utf8Json -Path $voiceMcpPath -Value $voiceMcp

        $voiceManifest = Get-Content -LiteralPath $voiceManifestPath -Raw | ConvertFrom-Json
        $agentPlugins += [ordered]@{
            name = 'voice-command'
            source = [ordered]@{
                source = 'local'
                path = './plugins/voice-command'
            }
            policy = [ordered]@{
                installation = 'AVAILABLE'
                authentication = 'ON_INSTALL'
            }
            category = 'Productivity'
        }
        $claudePlugins += [ordered]@{
            name = 'voice-command'
            description = [string]$voiceManifest.description
            author = [ordered]@{ name = 'AIWander' }
            category = 'productivity'
            source = './plugins/voice-command'
            homepage = 'https://github.com/AIWander/Voice-Command'
        }
    }

    $agentMarketplace = [ordered]@{
        name = 'aiwander-ai-hands'
        interface = [ordered]@{ displayName = 'AIWander AI-Hands' }
        plugins = $agentPlugins
    }
    $claudeMarketplace = [ordered]@{
        '$schema' = 'https://anthropic.com/claude-code/marketplace.schema.json'
        name = 'aiwander-ai-hands'
        description = 'Selected AI-Hands profile and optional Voice-Command companion.'
        owner = [ordered]@{ name = 'AIWander' }
        plugins = $claudePlugins
    }
    Write-Utf8Json -Path (Join-Path $marketplaceRoot '.agents\plugins\marketplace.json') -Value $agentMarketplace
    Write-Utf8Json -Path (Join-Path $marketplaceRoot '.claude-plugin\marketplace.json') -Value $claudeMarketplace

    $guide = [System.IO.File]::ReadAllText($guideTemplatePath)
    $guide += [Environment]::NewLine + [Environment]::NewLine
    $guide += 'INSTALLED PATHS' + [Environment]::NewLine
    $guide += '---------------' + [Environment]::NewLine
    $guide += "Marketplace: $marketplaceRoot" + [Environment]::NewLine
    $guide += "Hands plugin: $pluginRoot" + [Environment]::NewLine
    $guide += "Hands Rust MCP: $handsExe" + [Environment]::NewLine

    if ($withVoice) {
        $voiceGuideTemplate = Join-Path $appRoot 'installer\VOICE_APPLY_TO_YOUR_AI.template.txt'
        $voiceGuide = [System.IO.File]::ReadAllText($voiceGuideTemplate)
        $voiceGuide = $voiceGuide.Replace('__MARKETPLACE_ROOT__', $marketplaceRoot)
        $voiceGuide = $voiceGuide.Replace('__PLUGIN_ROOT__', $voicePluginRoot)
        $voiceGuide = $voiceGuide.Replace('__VOICE_EXE__', $voiceExe)
        $voiceGuide = $voiceGuide.Replace('@aiwander-voice-command', '@aiwander-ai-hands')
        $guide += [Environment]::NewLine + [Environment]::NewLine
        $guide += 'VOICE-COMMAND COMPANION' + [Environment]::NewLine
        $guide += '=======================' + [Environment]::NewLine + [Environment]::NewLine
        $guide += $voiceGuide
        $guide += [Environment]::NewLine + [Environment]::NewLine
        $guide += 'This combined package includes the Voice-Command plugin and Rust MCP wrapper only. Start or install the full Voice App or Python listener separately before listening. The installer does not start the microphone.' + [Environment]::NewLine
    }
    [System.IO.File]::WriteAllText($instructionsPath, $guide, $utf8NoBom)

    $result = [ordered]@{
        schema = 'ai-hands-plugin-install-v2'
        success = $true
        app_root = $appRoot
        marketplace_root = $marketplaceRoot
        plugin_id = $PluginId
        profile_kind = if ($PluginId -eq 'ai-hands') { 'hook-capable' } else { 'skills-only' }
        plugin_root = $pluginRoot
        hands_exe = $handsExe
        hands_mcp_registration = $mcpPath
        tool_profile = [string]$mcp.mcpServers.hands.env.HANDS_TOOL_PROFILE
        hook_templates_present = [bool]$hookTemplatesPresent
        hooks_enabled = $false
        voice_included = [bool]$withVoice
        voice_plugin_root = $voicePluginRoot
        voice_exe = $voiceExe
        full_voice_runtime_bundled = $false
        instructions = $instructionsPath
        client_configs_changed = $false
        server_started = $false
        microphone_started = $false
    }
    Write-Utf8Json -Path $resultPath -Value $result -Depth 8
}
catch {
    $failure = [ordered]@{
        schema = 'ai-hands-plugin-install-v2'
        success = $false
        app_root = $appRoot
        plugin_id = $PluginId
        voice_included = [bool]$withVoice
        error = [string]$_.Exception.Message
        client_configs_changed = $false
        hooks_enabled = $false
        server_started = $false
        microphone_started = $false
    }
    Write-Utf8Json -Path $resultPath -Value $failure -Depth 8
    Write-Error 'AI-Hands plugin finalization failed. See install-result.json in the installation directory.'
    exit 1
}
