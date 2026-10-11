<#
.SYNOPSIS
    Builds the one-installer AI-Hands package: hands.exe (arm64, x64 when the cross-build works),
    HandsSetup-<version>-win.exe (Inno Setup, client picker), the desktop-only .mcpb, and SHA256SUMS.txt.

.DESCRIPTION
    1. cargo build --release                                   -> target\release\hands.exe (host arch)
    2. rustup target add x86_64-pc-windows-msvc
       cargo build --release --target x86_64-pc-windows-msvc   -> optional; on failure the reason is written
                                                                  to dist\x64-build-failure.txt and the
                                                                  installer is built arm64-only
    3. stage dist\payload (hands-arm64.exe, hands-x64.exe, Register-HandsClients.ps1, README.md)
    4. ISCC installers\HandsSetup.iss                          -> dist\HandsSetup-<version>-win.exe
       (no ISCC: dist\HandsSetup-portable.zip instead; -Portable builds the zip as well)
    5. mcpb pack mcpb dist\hands-<version>-win-arm64.mcpb      (arm64 hands.exe copied to mcpb\server\)
    6. dist\SHA256SUMS.txt

    Signing is opt-in (-Sign): C:\CPC\scripts\sign-cpc.ps1 (Azure Trusted Signing, the same script and
    profile as C:\CPC\installers\shared\build-cpc-installer.ps1) signs the payload binaries and the ps1
    before they are packed, then Inno's SignTool directive calls it for the embedded uninstaller and the
    setup exe. Run installers\update-manifests.ps1 afterwards: signing changes every hash.
    Unsigned builds trigger SmartScreen on other machines. See installers\SIGNING.md.

.PARAMETER SkipBuild
    Reuse existing target\ outputs instead of running cargo.
.PARAMETER SkipX64
    Do not attempt the x64 cross-build.
.PARAMETER Portable
    Also build dist\HandsSetup-portable.zip when ISCC is present.
.PARAMETER SkipMcpb
    Do not pack the .mcpb.
.PARAMETER Sign
    Sign binaries and the installer with sign-cpc.ps1.
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$SkipX64,
    [switch]$Portable,
    [switch]$SkipMcpb,
    [switch]$Sign
)

$ErrorActionPreference = 'Stop'
$Repo = Split-Path -Parent $PSScriptRoot
$Dist = Join-Path $Repo 'dist'
$Payload = Join-Path $Dist 'payload'
$SignScript = 'C:\CPC\scripts\sign-cpc.ps1'
$X64Target = 'x86_64-pc-windows-msvc'

$Version = (Select-String -Path (Join-Path $Repo 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
# Windows version resources need four numbers: 1.1.0-unified.2 -> 1.1.0.2 (build.rs uses the same rule).
$core, $pre = $Version -split '-', 2
$nums = @(@($core.Split('.') | ForEach-Object { [int]$_ }) + @(0, 0, 0) | Select-Object -First 3)
$last = if ($pre -and ($pre.Split('.')[-1] -match '^\d+$')) { [int]$pre.Split('.')[-1] } else { 0 }
$NumericVersion = '{0}.{1}.{2}.{3}' -f $nums[0], $nums[1], $nums[2], $last
Write-Host "AI-Hands $Version ($NumericVersion)  repo $Repo"

function Get-PeMachine([string]$Path) {
    $b = [System.IO.File]::ReadAllBytes($Path)
    $pe = [BitConverter]::ToInt32($b, 0x3C)
    switch ([BitConverter]::ToUInt16($b, $pe + 4)) {
        0xAA64 { return 'arm64' }
        0x8664 { return 'x64' }
        0x014C { return 'x86' }
        default { return 'unknown' }
    }
}

function Invoke-Native([string]$Exe, [string[]]$Arguments, [string]$LogFile) {
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & $Exe @Arguments *> $LogFile
    $code = $LASTEXITCODE
    $ErrorActionPreference = $prev
    return $code
}

New-Item -ItemType Directory -Force -Path $Dist | Out-Null
if (Test-Path $Payload) { Remove-Item -Recurse -Force $Payload }
New-Item -ItemType Directory -Force -Path $Payload | Out-Null
$Notes = New-Object System.Collections.Generic.List[string]

# --- 1. host (arm64) build ------------------------------------------------------
Push-Location $Repo
try {
    if (-not $SkipBuild) {
        Write-Host 'cargo build --release'
        $code = Invoke-Native 'cargo' @('build', '--release') (Join-Path $Dist 'build-arm64.log')
        if ($code -ne 0) { throw "arm64 build failed (exit $code); see dist\build-arm64.log" }
    }
    $arm = Join-Path $Repo 'target\release\hands.exe'
    if (-not (Test-Path $arm)) { throw "missing $arm" }
    $armMachine = Get-PeMachine $arm
    if ($armMachine -ne 'arm64') { throw "target\release\hands.exe is $armMachine, expected arm64 (build on an arm64 host)" }
    Copy-Item $arm (Join-Path $Payload 'hands-arm64.exe')

    # --- 2. x64 cross-build (optional) -------------------------------------------
    $x64Ok = $false
    $failFile = Join-Path $Dist 'x64-build-failure.txt'
    if (Test-Path $failFile) { Remove-Item $failFile }
    if ($SkipX64) {
        $Notes.Add('x64: skipped (-SkipX64); installer is arm64-only')
    } else {
        $x64Log = Join-Path $Dist 'build-x64.log'
        $code = Invoke-Native 'rustup' @('target', 'add', $X64Target) (Join-Path $Dist 'rustup-x64.log')
        if ($code -eq 0 -and -not $SkipBuild) {
            Write-Host "cargo build --release --target $X64Target"
            $code = Invoke-Native 'cargo' @('build', '--release', '--target', $X64Target) $x64Log
        }
        $x64 = Join-Path $Repo "target\$X64Target\release\hands.exe"
        if ($code -eq 0 -and (Test-Path $x64) -and (Get-PeMachine $x64) -eq 'x64') {
            Copy-Item $x64 (Join-Path $Payload 'hands-x64.exe')
            $x64Ok = $true
            $Notes.Add('x64: built')
        } else {
            $tail = if (Test-Path $x64Log) { (Get-Content $x64Log -Tail 40) -join "`r`n" } else { 'no build log' }
            $reason = "x64 cross-build failed (exit $code) at $(Get-Date -Format s). Installer built arm64-only.`r`n`r`nLast lines of dist\build-x64.log:`r`n$tail`r`n"
            [System.IO.File]::WriteAllText($failFile, $reason, (New-Object System.Text.UTF8Encoding $false))
            $Notes.Add('x64: FAILED, see dist\x64-build-failure.txt; installer is arm64-only')
        }
    }
} finally {
    Pop-Location
}

# --- 3. stage the rest of the payload -------------------------------------------
Copy-Item (Join-Path $PSScriptRoot 'scripts\Register-HandsClients.ps1') $Payload
Copy-Item (Join-Path $PSScriptRoot 'README-picker.md') (Join-Path $Payload 'README.md')

if ($Sign) {
    if (-not (Test-Path $SignScript)) { throw "-Sign given but $SignScript is missing" }
    pwsh -NoProfile -File $SignScript -Path $Payload -Filter '*.exe'
    pwsh -NoProfile -File $SignScript -Path (Join-Path $Payload 'Register-HandsClients.ps1') -Filter '*.ps1'
    $Notes.Add('signing: payload signed with sign-cpc.ps1')
} else {
    $Notes.Add('signing: not signed (run with -Sign)')
}

# --- 4. installer -----------------------------------------------------------------
$iscc = @(
    'C:\Program Files (x86)\Inno Setup 6\ISCC.exe',
    'C:\Program Files\Inno Setup 6\ISCC.exe',
    (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe')
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) {
    $cmd = Get-Command iscc -ErrorAction SilentlyContinue
    if ($cmd) { $iscc = $cmd.Source }
}

$setupExe = Join-Path $Dist "HandsSetup-$Version-win.exe"
# drop setup exes from earlier builds, including the older HandsSetup-<version>.exe name
Get-ChildItem $Dist -Filter 'HandsSetup-*.exe' -File -ErrorAction SilentlyContinue | Remove-Item
if ($iscc) {
    Write-Host "ISCC: $iscc"
    $issLog = Join-Path $Dist 'iscc.log'
    $isccArgs = @("/DMyAppVersion=$Version", "/DNumericVersion=$NumericVersion", "/DPayloadDir=$Payload", "/DOutputDir=$Dist")
    if ($Sign) {
        # Inno replaces $f with the quoted path of each file it signs: the embedded uninstaller, then the setup exe.
        $isccArgs += @('/DUseSignTool=1', "/Scpcsign=pwsh -NoProfile -File $SignScript -Path `$f")
    }
    $isccArgs += (Join-Path $PSScriptRoot 'HandsSetup.iss')
    $code = Invoke-Native $iscc $isccArgs $issLog
    if ($code -ne 0 -or -not (Test-Path $setupExe)) { throw "ISCC failed (exit $code); see dist\iscc.log" }
    $Notes.Add("installer: $(Split-Path -Leaf $setupExe)$(if (-not $x64Ok) { ' (arm64-only)' })")
} else {
    $Notes.Add('installer: ISCC not found (looked in Program Files (x86)\Inno Setup 6, Program Files\Inno Setup 6, PATH); portable zip built instead')
    $Portable = $true
}

if ($Portable) {
    $zipStage = Join-Path $Dist 'portable'
    if (Test-Path $zipStage) { Remove-Item -Recurse -Force $zipStage }
    New-Item -ItemType Directory -Path (Join-Path $zipStage 'arm64') | Out-Null
    Copy-Item (Join-Path $Payload 'hands-arm64.exe') (Join-Path $zipStage 'arm64\hands.exe')
    if ($x64Ok) {
        New-Item -ItemType Directory -Path (Join-Path $zipStage 'x64') | Out-Null
        Copy-Item (Join-Path $Payload 'hands-x64.exe') (Join-Path $zipStage 'x64\hands.exe')
    }
    Copy-Item (Join-Path $Payload 'Register-HandsClients.ps1') $zipStage
    Copy-Item (Join-Path $Payload 'README.md') $zipStage
    $zip = Join-Path $Dist 'HandsSetup-portable.zip'
    if (Test-Path $zip) { Remove-Item $zip }
    Compress-Archive -Path (Join-Path $zipStage '*') -DestinationPath $zip
    Remove-Item -Recurse -Force $zipStage
    $Notes.Add('portable: HandsSetup-portable.zip')
}

# --- 5. desktop-only MCPB -------------------------------------------------------------
if (-not $SkipMcpb) {
    $mcpb = Get-Command mcpb -ErrorAction SilentlyContinue
    if (-not $mcpb) {
        Write-Host 'installing @anthropic-ai/mcpb globally'
        $null = Invoke-Native 'npm' @('install', '-g', '@anthropic-ai/mcpb') (Join-Path $Dist 'npm-mcpb.log')
        $mcpb = Get-Command mcpb -ErrorAction SilentlyContinue
    }
    if ($mcpb) {
        $server = Join-Path $Repo 'mcpb\server'
        New-Item -ItemType Directory -Force -Path $server | Out-Null
        Copy-Item (Join-Path $Payload 'hands-arm64.exe') (Join-Path $server 'hands.exe') -Force
        $out = Join-Path $Dist "hands-$Version-win-arm64.mcpb"
        if (Test-Path $out) { Remove-Item $out }
        Push-Location $Repo
        try { $code = Invoke-Native $mcpb.Source @('pack', 'mcpb', $out) (Join-Path $Dist 'mcpb-pack.log') } finally { Pop-Location }
        if ($code -eq 0 -and (Test-Path $out)) { $Notes.Add("mcpb: $(Split-Path -Leaf $out)") }
        else { $Notes.Add("mcpb: pack FAILED (exit $code), see dist\mcpb-pack.log") }
    } else {
        $Notes.Add('mcpb: mcpb CLI unavailable, not packed')
    }
}

# --- 6. checksums -------------------------------------------------------------------------
$sumTargets = @(Get-ChildItem $Dist -File | Where-Object { $_.Extension -in '.exe', '.zip', '.mcpb' }) +
              @(Get-ChildItem $Payload -File)
$lines = foreach ($f in $sumTargets) {
    $rel = $f.FullName.Substring($Dist.Length + 1).Replace('\', '/')
    '{0}  {1}' -f (Get-FileHash $f.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $rel
}
[System.IO.File]::WriteAllText((Join-Path $Dist 'SHA256SUMS.txt'), (($lines -join "`n") + "`n"), (New-Object System.Text.UTF8Encoding $false))

Write-Host ''
Write-Host 'Build summary'
$Notes | ForEach-Object { Write-Host "  $_" }
Get-Content (Join-Path $Dist 'SHA256SUMS.txt') | ForEach-Object { Write-Host "  $_" }
