<#
.SYNOPSIS
    Generates winget v1.6 manifests for a published AI-Hands release.

.DESCRIPTION
    Calls `gh release view v<Version>` to pull asset metadata for the x64 and
    aarch64 binaries, then renders the three required winget manifest files
    (version, installer, locale.en-US) into
    installers/winget/manifests/a/AIWander/AI-Hands/<Version>/.

    Assumes the release assets follow the canonical naming:
        hands-v<Version>-x64.exe
        hands-v<Version>-aarch64.exe

.PARAMETER Version
    Semver string without leading `v`, e.g. `1.1.0`.

.PARAMETER ReleaseDate
    ISO date the release was published (yyyy-MM-dd). Defaults to today.

.PARAMETER DryRun
    Print what would be written; do not touch the filesystem.

.EXAMPLE
    .\generate-winget-manifests.ps1 -Version 1.1.0

.EXAMPLE
    .\generate-winget-manifests.ps1 -Version 1.1.0 -DryRun
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [Parameter(Mandatory = $false)]
    [string]$ReleaseDate = (Get-Date -Format 'yyyy-MM-dd'),

    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'

# --- Sanity check Version --------------------------------------------------

if ($Version -notmatch '^\d+\.\d+\.\d+(-[\w\.]+)?$') {
    Write-Error "Version must look like 1.2.3 or 1.2.3-rc1 (got: $Version)"
    exit 1
}

# --- Locate repo root + output dir -----------------------------------------

# Script lives at installers/scripts/, repo root is two up.
$scriptDir = Split-Path -LiteralPath $PSCommandPath -Parent
$repoRoot  = Split-Path -LiteralPath (Split-Path -LiteralPath $scriptDir -Parent) -Parent
$outDir    = Join-Path $repoRoot "installers/winget/manifests/a/AIWander/AI-Hands/$Version"

Write-Host "Repo root  : $repoRoot"
Write-Host "Output dir : $outDir"

# --- Pull release metadata via gh ------------------------------------------

$ghCmd = Get-Command gh -ErrorAction SilentlyContinue
if (-not $ghCmd) {
    Write-Error "GitHub CLI (gh) not found. Install from https://cli.github.com/ or download the release-asset metadata manually."
    exit 1
}

Write-Host "Fetching release metadata for v$Version ..."
$ghJson = & gh release view "v$Version" -R AIWander/AI-Hands --json assets -q '.assets' 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Error "gh release view failed for v${Version}: $ghJson"
    exit 1
}

try {
    $assets = $ghJson | ConvertFrom-Json -Depth 100
} catch {
    Write-Error "Failed to parse gh JSON output: $_"
    exit 1
}

# --- Pick out the two binaries we care about -------------------------------

$wantX64    = "hands-v$Version-x64.exe"
$wantArm64  = "hands-v$Version-aarch64.exe"

$x64Asset   = $assets | Where-Object { $_.name -eq $wantX64 }   | Select-Object -First 1
$arm64Asset = $assets | Where-Object { $_.name -eq $wantArm64 } | Select-Object -First 1

if (-not $x64Asset) {
    Write-Error "x64 asset '$wantX64' not found on release v$Version. Found: $($assets.name -join ', ')"
    exit 1
}
if (-not $arm64Asset) {
    Write-Error "arm64 asset '$wantArm64' not found on release v$Version. Found: $($assets.name -join ', ')"
    exit 1
}

# gh emits `digest` as e.g. "sha256:88408a98..." — strip the prefix.
function Get-Sha256FromAsset {
    param([Parameter(Mandatory = $true)][object]$Asset)
    $rawDigest = $Asset.digest
    if ([string]::IsNullOrEmpty($rawDigest)) {
        Write-Error "Asset '$($Asset.name)' has no `digest` field; gh CLI may be too old (need >= 2.40)."
        exit 1
    }
    if ($rawDigest -match '^sha256:([0-9a-fA-F]{64})$') {
        return $Matches[1].ToLower()
    }
    if ($rawDigest -match '^([0-9a-fA-F]{64})$') {
        return $rawDigest.ToLower()
    }
    Write-Error "Asset '$($Asset.name)' digest is not a sha256: $rawDigest"
    exit 1
}

$x64Sha    = Get-Sha256FromAsset -Asset $x64Asset
$arm64Sha  = Get-Sha256FromAsset -Asset $arm64Asset

$x64Url    = "https://github.com/AIWander/AI-Hands/releases/download/v$Version/$wantX64"
$arm64Url  = "https://github.com/AIWander/AI-Hands/releases/download/v$Version/$wantArm64"

Write-Host ""
Write-Host "x64    sha256: $x64Sha"
Write-Host "arm64  sha256: $arm64Sha"
Write-Host ""

# --- Render manifests ------------------------------------------------------

$versionManifest = @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.version.1.6.0.schema.json
PackageIdentifier: AIWander.AI-Hands
PackageVersion: $Version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.6.0
"@

$installerManifest = @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.installer.1.6.0.schema.json
PackageIdentifier: AIWander.AI-Hands
PackageVersion: $Version
InstallerType: portable
Commands:
  - hands
ReleaseDate: $ReleaseDate
Installers:
  - Architecture: x64
    InstallerUrl: $x64Url
    InstallerSha256: $x64Sha
    InstallerType: portable
    Commands:
      - hands
  - Architecture: arm64
    InstallerUrl: $arm64Url
    InstallerSha256: $arm64Sha
    InstallerType: portable
    Commands:
      - hands
ManifestType: installer
ManifestVersion: 1.6.0
"@

$localeManifest = @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.defaultLocale.1.6.0.schema.json
PackageIdentifier: AIWander.AI-Hands
PackageVersion: $Version
PackageLocale: en-US
Publisher: AIWander
PublisherUrl: https://github.com/AIWander
PublisherSupportUrl: https://github.com/AIWander/AI-Hands/issues
PackageName: AI-Hands
PackageUrl: https://github.com/AIWander/AI-Hands
License: Apache-2.0
LicenseUrl: https://github.com/AIWander/AI-Hands/blob/main/LICENSE
ShortDescription: Single-binary MCP server for browser + Windows UI + vision/OCR automation
Description: |
  AI-Hands is a single Rust binary MCP (Model Context Protocol) server that exposes
  three primitive subsystems plus a meta-orchestration layer for agentic browser and
  desktop automation:

  - Browser via chromiumoxide CDP (attach to existing Chrome, accessibility-first)
  - Windows UI Automation via native UIA
  - Vision (screenshot + OCR via Tesseract)
  - Meta-tools: hands_click, hands_navigate, hands_find, hands_verify

  Lean by design: ~184 MB per session vs Playwright MCP's ~320 MB. BYOM
  (bring-your-own-model) — works with Claude, Codex, Gemini, GPT, or any
  MCP-compatible client.

  Post-install, run installers/scripts/register-hands.ps1 to wire AI-Hands
  into Claude Desktop's MCP config (with backup-first).

Moniker: ai-hands
Tags:
  - mcp
  - automation
  - browser-automation
  - rust
  - claude
  - ai
  - ocr
  - accessibility
  - uia
ReleaseNotesUrl: https://github.com/AIWander/AI-Hands/releases/tag/v$Version
Documentation:
  - DocumentLabel: README
    DocumentUrl: https://github.com/AIWander/AI-Hands#readme
  - DocumentLabel: CHANGELOG
    DocumentUrl: https://github.com/AIWander/AI-Hands/blob/main/CHANGELOG.md
ManifestType: defaultLocale
ManifestVersion: 1.6.0
"@

# --- Write -----------------------------------------------------------------

$versionPath   = Join-Path $outDir 'AIWander.AI-Hands.yaml'
$installerPath = Join-Path $outDir 'AIWander.AI-Hands.installer.yaml'
$localePath    = Join-Path $outDir 'AIWander.AI-Hands.locale.en-US.yaml'

if ($DryRun) {
    Write-Host "[DryRun] Would create directory: $outDir"
    Write-Host "[DryRun] Would write: $versionPath"
    Write-Host "[DryRun] Would write: $installerPath"
    Write-Host "[DryRun] Would write: $localePath"
    Write-Host ""
    Write-Host "[DryRun] --- $versionPath ---"
    Write-Host $versionManifest
    Write-Host "[DryRun] --- $installerPath ---"
    Write-Host $installerManifest
    Write-Host "[DryRun] --- $localePath ---"
    Write-Host $localeManifest
    exit 0
}

if (-not (Test-Path -LiteralPath $outDir)) {
    New-Item -ItemType Directory -Path $outDir -Force | Out-Null
}

$versionManifest   | Set-Content -LiteralPath $versionPath   -Encoding UTF8 -NoNewline
$installerManifest | Set-Content -LiteralPath $installerPath -Encoding UTF8 -NoNewline
$localeManifest    | Set-Content -LiteralPath $localePath    -Encoding UTF8 -NoNewline

Write-Host "Generated 3 manifests at $outDir. Submit via:"
Write-Host "    1) Fork microsoft/winget-pkgs"
Write-Host "    2) Copy the generated $Version/ directory to manifests/a/AIWander/AI-Hands/$Version/ in your fork"
Write-Host "    3) gh pr create --repo microsoft/winget-pkgs --title `"New version: AIWander.AI-Hands v$Version`" --body `"Adds AI-Hands $Version manifests.`""
Write-Host "    OR use wingetcreate:"
Write-Host "    wingetcreate submit --token <PAT> --manifest $outDir"
