<#
.SYNOPSIS
    Renders the winget and Scoop manifests for the current Cargo.toml version from dist\SHA256SUMS.txt.

.DESCRIPTION
    Reads the version from Cargo.toml and the SHA-256 of these release assets from dist\SHA256SUMS.txt
    (written by installers\build-hands-setup.ps1):
        HandsSetup-<version>-win.exe   -> winget (InstallerType inno, one installer for x64 and arm64)
        HandsSetup-portable.zip        -> Scoop (x64\hands.exe or arm64\hands.exe from the zip)
    and writes, overwriting what is there:
        installers\winget\manifests\a\AIWander\AI-Hands\<version>\AIWander.AI-Hands{,.installer,.locale.en-US}.yaml
        installers\scoop\ai-hands.json
    Download URLs point at the GitHub release tag v<version>.

    Run it again after every rebuild and after signing: signing rewrites the files, so every hash changes.
    Older version folders under installers\winget\manifests are left alone.

.PARAMETER SumsFile
    Checksum file to read. Default: dist\SHA256SUMS.txt.
.PARAMETER ReleaseDate
    yyyy-MM-dd for the winget ReleaseDate field. Default: today.
.PARAMETER Check
    Do not write; exit 1 if any manifest on disk differs from what would be written.
#>
[CmdletBinding()]
param(
    [string]$SumsFile,
    [string]$ReleaseDate = (Get-Date -Format 'yyyy-MM-dd'),
    [switch]$Check
)

$ErrorActionPreference = 'Stop'
$Repo = Split-Path -Parent $PSScriptRoot
if (-not $SumsFile) { $SumsFile = Join-Path $Repo 'dist\SHA256SUMS.txt' }
if (-not (Test-Path $SumsFile)) { throw "missing $SumsFile; run installers\build-hands-setup.ps1 first" }

$Version = (Select-String -Path (Join-Path $Repo 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$Base = "https://github.com/AIWander/AI-Hands/releases/download/v$Version"
$SetupName = "HandsSetup-$Version-win.exe"
$ZipName = 'HandsSetup-portable.zip'

$sums = @{}
foreach ($line in Get-Content $SumsFile) {
    if ($line -match '^([0-9a-fA-F]{64})\s+\*?(.+)$') { $sums[$Matches[2].Trim()] = $Matches[1].ToLowerInvariant() }
}
foreach ($n in $SetupName, $ZipName) {
    if (-not $sums.ContainsKey($n)) { throw "$n is not listed in $SumsFile" }
}
$SetupSha = $sums[$SetupName]
$ZipSha = $sums[$ZipName]

$wingetDir = Join-Path $Repo "installers\winget\manifests\a\AIWander\AI-Hands\$Version"
$files = [ordered]@{}

$files[(Join-Path $wingetDir 'AIWander.AI-Hands.yaml')] = @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.version.1.6.0.schema.json
PackageIdentifier: AIWander.AI-Hands
PackageVersion: $Version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.6.0
"@

$files[(Join-Path $wingetDir 'AIWander.AI-Hands.installer.yaml')] = @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.installer.1.6.0.schema.json
PackageIdentifier: AIWander.AI-Hands
PackageVersion: $Version
InstallerType: inno
Scope: user
InstallModes:
  - interactive
  - silent
  - silentWithProgress
UpgradeBehavior: install
MinimumOSVersion: 10.0.17763.0
ProductCode: '{6E0B2C47-3F1D-4C55-9B8E-A1D4F0C2B7E3}_is1'
ReleaseDate: $ReleaseDate
Installers:
  - Architecture: x64
    InstallerUrl: $Base/$SetupName
    InstallerSha256: $($SetupSha.ToUpperInvariant())
  - Architecture: arm64
    InstallerUrl: $Base/$SetupName
    InstallerSha256: $($SetupSha.ToUpperInvariant())
ManifestType: installer
ManifestVersion: 1.6.0
"@

$files[(Join-Path $wingetDir 'AIWander.AI-Hands.locale.en-US.yaml')] = @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.defaultLocale.1.6.0.schema.json
PackageIdentifier: AIWander.AI-Hands
PackageVersion: $Version
PackageLocale: en-US
Publisher: AIWander
PublisherUrl: https://github.com/AIWander
PublisherSupportUrl: https://github.com/AIWander/AI-Hands/issues
PrivacyUrl: https://github.com/AIWander/AI-Hands/blob/main/PRIVACY.md
Author: Joseph Wander
PackageName: AI-Hands
PackageUrl: https://github.com/AIWander/AI-Hands
License: Apache-2.0
LicenseUrl: https://github.com/AIWander/AI-Hands/blob/main/LICENSE
Copyright: Copyright 2026 Joseph Wander / AIWander
ShortDescription: Local MCP server that lets AI assistants automate the browser, Windows apps and the screen
Description: |
  AI-Hands is a single Rust binary MCP (Model Context Protocol) server for browser automation
  (Chrome over CDP), Windows UI Automation, and screenshots with OCR. It runs locally, has no
  telemetry, and works with any MCP client.

  The installer is per-user and offers a client picker (Claude desktop app, Codex CLI, Grok CLI).
  In a silent install (winget's default) it registers hands with the clients it detects. Each
  config is backed up first, and an existing hands entry is never changed.
Moniker: ai-hands
Tags:
  - mcp
  - automation
  - browser-automation
  - claude
  - codex
  - ocr
  - accessibility
  - uia
ReleaseNotesUrl: https://github.com/AIWander/AI-Hands/releases/tag/v$Version
Documentations:
  - DocumentLabel: Install guide
    DocumentUrl: https://github.com/AIWander/AI-Hands/blob/main/installers/README-picker.md
  - DocumentLabel: CHANGELOG
    DocumentUrl: https://github.com/AIWander/AI-Hands/blob/main/CHANGELOG.md
ManifestType: defaultLocale
ManifestVersion: 1.6.0
"@

$files[(Join-Path $Repo 'installers\scoop\ai-hands.json')] = @"
{
    "version": "$Version",
    "description": "Local MCP server that lets AI assistants automate the browser, Windows apps and the screen",
    "homepage": "https://github.com/AIWander/AI-Hands",
    "license": "Apache-2.0",
    "notes": [
        "Register hands with your AI clients (backs up each config first):",
        "  & \"`$dir\\Register-HandsClients.ps1\" -HandsExe \"`$dir\\<x64 or arm64>\\hands.exe\" -Clients ClaudeDesktop,Codex,GrokCli",
        "Use the real hands.exe path above, not the Scoop shim. Privacy: https://github.com/AIWander/AI-Hands/blob/main/PRIVACY.md"
    ],
    "architecture": {
        "64bit": {
            "url": "$Base/$ZipName",
            "hash": "$ZipSha",
            "bin": [["x64\\hands.exe", "hands"]]
        },
        "arm64": {
            "url": "$Base/$ZipName",
            "hash": "$ZipSha",
            "bin": [["arm64\\hands.exe", "hands"]]
        }
    },
    "checkver": "github",
    "autoupdate": {
        "architecture": {
            "64bit": {
                "url": "https://github.com/AIWander/AI-Hands/releases/download/v`$version/$ZipName"
            },
            "arm64": {
                "url": "https://github.com/AIWander/AI-Hands/releases/download/v`$version/$ZipName"
            }
        }
    }
}
"@

$utf8 = New-Object System.Text.UTF8Encoding $false
$drift = 0
foreach ($path in $files.Keys) {
    $text = ($files[$path] -replace "`r`n", "`n").TrimEnd("`n") + "`n"
    $current = if (Test-Path $path) { [System.IO.File]::ReadAllText($path) -replace "`r`n", "`n" } else { $null }
    if ($Check) {
        if ($current -ne $text) { Write-Host "differs: $path"; $drift++ }
        continue
    }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $path) | Out-Null
    [System.IO.File]::WriteAllText($path, $text, $utf8)
    Write-Host "wrote $path"
}
Write-Host "version $Version  setup $SetupSha  zip $ZipSha"
if ($Check -and $drift) { exit 1 }
