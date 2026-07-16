[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerDir,

    [Parameter(Mandatory = $true)]
    [ValidateSet('arm64', 'x64')]
    [string]$Architecture,

    [Parameter(Mandatory = $true)]
    [string]$HandsPayload,

    [Parameter(Mandatory = $true)]
    [string]$VoicePayload,

    [string]$TestRoot = ''
)

$ErrorActionPreference = 'Stop'
$installerRoot = (Resolve-Path -LiteralPath $InstallerDir).Path
$handsPayload = (Resolve-Path -LiteralPath $HandsPayload).Path
$voicePayload = (Resolve-Path -LiteralPath $VoicePayload).Path
if ([string]::IsNullOrWhiteSpace($TestRoot)) {
    $TestRoot = Join-Path $env:TEMP "aihands-cold-test-$Architecture-$PID"
}
$testRootFull = [System.IO.Path]::GetFullPath($TestRoot)
[System.IO.Directory]::CreateDirectory($testRootFull) | Out-Null

function Assert-ValidSignature {
    param([Parameter(Mandatory = $true)][string]$Path)
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne 'Valid' -or
        $signature.SignerCertificate.Subject -notmatch 'CN=Joseph Wander') {
        throw "Authenticode verification failed for ${Path}: $($signature.Status) / $($signature.SignerCertificate.Subject)"
    }
}

Assert-ValidSignature -Path $handsPayload
Assert-ValidSignature -Path $voicePayload
$handsHash = (Get-FileHash -LiteralPath $handsPayload -Algorithm SHA256).Hash
$voiceHash = (Get-FileHash -LiteralPath $voicePayload -Algorithm SHA256).Hash
$installers = @(
    Get-ChildItem -LiteralPath $installerRoot -File -Filter "AIHands-Setup-1.1.0-unified.2-*-*-$Architecture.exe" |
        Sort-Object Name
)
if ($installers.Count -ne 4) {
    throw "Expected four $Architecture installer variants, found $($installers.Count)."
}

$results = foreach ($installer in $installers) {
    Assert-ValidSignature -Path $installer.FullName
    if ($installer.BaseName -notmatch "-(hooks|skills)-(voice|no-voice)-$Architecture$") {
        throw "Installer flavor could not be parsed: $($installer.Name)"
    }
    $profile = $Matches[1]
    $voiceMode = $Matches[2]
    $expectedPlugin = if ($profile -eq 'hooks') { 'ai-hands' } else { 'ai-hands-skills' }
    $expectedVoice = $voiceMode -eq 'voice'
    $caseRoot = Join-Path $testRootFull "$profile-$voiceMode"
    if (Test-Path -LiteralPath $caseRoot) {
        throw "Cold-test case root already exists: $caseRoot"
    }

    $install = Start-Process -FilePath $installer.FullName -ArgumentList @(
        '/VERYSILENT',
        '/SUPPRESSMSGBOXES',
        '/NORESTART',
        "/DIR=$caseRoot",
        '/MERGETASKS=!addtopath'
    ) -WindowStyle Hidden -Wait -PassThru
    if ($install.ExitCode -ne 0) {
        throw "$($installer.Name) exited $($install.ExitCode)."
    }

    $resultPath = Join-Path $caseRoot 'install-result.json'
    $result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
    if ($result.success -ne $true -or
        $result.plugin_id -ne $expectedPlugin -or
        $result.voice_included -ne $expectedVoice -or
        $result.client_configs_changed -ne $false -or
        $result.hooks_enabled -ne $false -or
        $result.server_started -ne $false -or
        $result.microphone_started -ne $false -or
        $result.full_voice_runtime_bundled -ne $false) {
        throw "Unsafe or incorrect install truth: $($result | ConvertTo-Json -Compress)"
    }

    $installedHands = (Resolve-Path -LiteralPath (Join-Path $caseRoot 'bin\hands.exe')).Path
    Assert-ValidSignature -Path $installedHands
    if ((Get-FileHash -LiteralPath $installedHands -Algorithm SHA256).Hash -ne $handsHash) {
        throw 'Installed Hands binary hash differs from the signed build.'
    }

    $pluginRoot = Join-Path $caseRoot "marketplace\plugins\$expectedPlugin"
    $manifest = Get-Content -LiteralPath (Join-Path $pluginRoot '.codex-plugin\plugin.json') -Raw | ConvertFrom-Json
    if ($manifest.name -ne $expectedPlugin) {
        throw 'Installed plugin manifest does not match the selected profile.'
    }
    foreach ($skill in 'ai-hands', 'ai-hands-getting-started', 'ai-hands-safety', 'ai-hands-workflows') {
        if (-not (Test-Path -LiteralPath (Join-Path $pluginRoot "skills\$skill\SKILL.md") -PathType Leaf)) {
            throw "Missing installed skill: $skill"
        }
    }
    $hooksPresent = Test-Path -LiteralPath (Join-Path $pluginRoot 'hooks\opt-in') -PathType Container
    if ($hooksPresent -ne ($profile -eq 'hooks')) {
        throw 'Installed hook profile boundary is wrong.'
    }
    if ($profile -eq 'hooks') {
        foreach ($fragmentName in 'codex-hooks.fragment.json', 'claude-grok-hooks.fragment.json') {
            $fragment = Get-Content -LiteralPath (Join-Path $pluginRoot "hooks\opt-in\$fragmentName") -Raw | ConvertFrom-Json
            foreach ($eventName in 'SessionStart', 'UserPromptSubmit', 'PreToolUse', 'PostToolUse', 'PostToolUseFailure') {
                if (-not $fragment.hooks.$eventName) {
                    throw "$fragmentName is missing $eventName."
                }
            }
        }
    }
    else {
        $guide = Get-Content -LiteralPath (Join-Path $caseRoot 'APPLY_TO_YOUR_AI.txt') -Raw
        foreach ($phrase in 'At the start of a task', 'Before any Hands mutation', 'behavioral guidance, not hard enforcement') {
            if ($guide -notlike "*$phrase*") {
                throw "Skills-only installed guide is missing: $phrase"
            }
        }
    }

    $agentMarketplace = Get-Content -LiteralPath (Join-Path $caseRoot 'marketplace\.agents\plugins\marketplace.json') -Raw | ConvertFrom-Json
    $expectedCount = if ($expectedVoice) { 2 } else { 1 }
    if (@($agentMarketplace.plugins).Count -ne $expectedCount) {
        throw 'Installed marketplace plugin count does not match the package flavor.'
    }

    $installedVoice = Join-Path $caseRoot 'bin\voice-mcp.exe'
    if ((Test-Path -LiteralPath $installedVoice -PathType Leaf) -ne $expectedVoice) {
        throw 'Voice executable presence does not match the package flavor.'
    }
    if ($expectedVoice) {
        Assert-ValidSignature -Path $installedVoice
        if ((Get-FileHash -LiteralPath $installedVoice -Algorithm SHA256).Hash -ne $voiceHash) {
            throw 'Installed Voice binary hash differs from the signed build.'
        }
        $guide = Get-Content -LiteralPath (Join-Path $caseRoot 'APPLY_TO_YOUR_AI.txt') -Raw
        foreach ($phrase in 'VOICE-COMMAND COMPANION', 'edge-tts', 'does not start the microphone', 'full Voice App or Python listener separately') {
            if ($guide -notlike "*$phrase*") {
                throw "Voice installed guide is missing: $phrase"
            }
        }
    }

    $uninstaller = (Resolve-Path -LiteralPath (Join-Path $caseRoot 'unins000.exe')).Path
    $uninstall = Start-Process -FilePath $uninstaller -ArgumentList @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'
    ) -WindowStyle Hidden -Wait -PassThru
    if ($uninstall.ExitCode -ne 0) {
        throw "Uninstaller exited $($uninstall.ExitCode)."
    }
    for ($attempt = 0; $attempt -lt 20 -and (Test-Path -LiteralPath $caseRoot); $attempt++) {
        Start-Sleep -Milliseconds 250
    }
    if (Test-Path -LiteralPath $caseRoot) {
        throw "Uninstaller left the isolated installation directory behind: $caseRoot"
    }

    [pscustomobject]@{
        Architecture = $Architecture
        Profile = $profile
        Voice = $expectedVoice
        Signature = 'Valid'
        ColdInstall = 'Passed'
        Uninstall = 'Passed'
    }
}

$transitionSource = @(
    $installers | Where-Object BaseName -Match "-hooks-voice-$Architecture$"
)
$transitionTarget = @(
    $installers | Where-Object BaseName -Match "-skills-no-voice-$Architecture$"
)
if ($transitionSource.Count -ne 1 -or $transitionTarget.Count -ne 1) {
    throw 'Could not select the Voice-to-no-Voice transition packages.'
}

$transitionRoot = Join-Path $testRootFull 'transition-hooks-voice-to-skills-no-voice'
if (Test-Path -LiteralPath $transitionRoot) {
    throw "Transition-test root already exists: $transitionRoot"
}

$firstInstall = Start-Process -FilePath $transitionSource[0].FullName -ArgumentList @(
    '/VERYSILENT',
    '/SUPPRESSMSGBOXES',
    '/NORESTART',
    "/DIR=$transitionRoot",
    '/MERGETASKS=!addtopath'
) -WindowStyle Hidden -Wait -PassThru
if ($firstInstall.ExitCode -ne 0) {
    throw "Voice transition source exited $($firstInstall.ExitCode)."
}
foreach ($required in @(
    'bin\voice-mcp.exe',
    'marketplace\plugins\ai-hands\hooks\opt-in',
    'marketplace\plugins\voice-command',
    'installer\VOICE_APPLY_TO_YOUR_AI.template.txt'
)) {
    if (-not (Test-Path -LiteralPath (Join-Path $transitionRoot $required))) {
        throw "Voice transition source omitted: $required"
    }
}

$secondInstall = Start-Process -FilePath $transitionTarget[0].FullName -ArgumentList @(
    '/VERYSILENT',
    '/SUPPRESSMSGBOXES',
    '/NORESTART',
    "/DIR=$transitionRoot",
    '/MERGETASKS=!addtopath'
) -WindowStyle Hidden -Wait -PassThru
if ($secondInstall.ExitCode -ne 0) {
    throw "No-Voice transition target exited $($secondInstall.ExitCode)."
}

$transitionResult = Get-Content -LiteralPath (
    Join-Path $transitionRoot 'install-result.json'
) -Raw | ConvertFrom-Json
if ($transitionResult.plugin_id -ne 'ai-hands-skills' -or
    $transitionResult.voice_included -ne $false -or
    $transitionResult.hooks_enabled -ne $false -or
    $transitionResult.microphone_started -ne $false) {
    throw "Transition truth record is incorrect: $($transitionResult | ConvertTo-Json -Compress)"
}
foreach ($stale in @(
    'bin\voice-mcp.exe',
    'marketplace\plugins\ai-hands',
    'marketplace\plugins\voice-command',
    'installer\VOICE_APPLY_TO_YOUR_AI.template.txt'
)) {
    if (Test-Path -LiteralPath (Join-Path $transitionRoot $stale)) {
        throw "Flavor transition left stale payload: $stale"
    }
}
$transitionPlugin = Join-Path $transitionRoot 'marketplace\plugins\ai-hands-skills'
if (-not (Test-Path -LiteralPath $transitionPlugin -PathType Container) -or
    (Test-Path -LiteralPath (Join-Path $transitionPlugin 'hooks') -PathType Container)) {
    throw 'Flavor transition did not land the clean skills-only profile.'
}
$transitionHands = Join-Path $transitionRoot 'bin\hands.exe'
Assert-ValidSignature -Path $transitionHands
if ((Get-FileHash -LiteralPath $transitionHands -Algorithm SHA256).Hash -ne $handsHash) {
    throw 'Flavor transition changed the signed Hands payload.'
}

$transitionUninstaller = (Resolve-Path -LiteralPath (
    Join-Path $transitionRoot 'unins000.exe'
)).Path
$transitionUninstall = Start-Process -FilePath $transitionUninstaller -ArgumentList @(
    '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'
) -WindowStyle Hidden -Wait -PassThru
if ($transitionUninstall.ExitCode -ne 0) {
    throw "Transition uninstaller exited $($transitionUninstall.ExitCode)."
}
for ($attempt = 0; $attempt -lt 20 -and (Test-Path -LiteralPath $transitionRoot); $attempt++) {
    Start-Sleep -Milliseconds 250
}
if (Test-Path -LiteralPath $transitionRoot) {
    throw "Transition uninstall left its directory behind: $transitionRoot"
}

$results += [pscustomobject]@{
    Architecture = $Architecture
    Profile = 'hooks -> skills'
    Voice = 'True -> False'
    Signature = 'Valid'
    ColdInstall = 'Passed'
    Uninstall = 'Passed'
}

$results | Format-Table -AutoSize
