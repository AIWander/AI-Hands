<#
.SYNOPSIS
    First-run onboarding for AI-Hands: guidance, stdio registration, Tailscale.

.DESCRIPTION
    Copies install guidance and registers stdio hosts by calling
    Register-HandsClients.ps1. Then offers to install Tailscale
    (winget id Tailscale.Tailscale, or an MSI you supply via msiexec /quiet).
    The user signs in. This script never creates or stores an auth key.

    If Tailscale is declined, it prints an authenticated Cloudflare Tunnel or
    ngrok flow. It does not start a public tunnel and does not enable
    Tailscale Funnel.

.PARAMETER AuthKey
    Optional Tailscale auth key the user already has. Passed only to
    `tailscale up` for this invocation. It is not written to a file or log.
    Command history on the machine may still retain process arguments; prefer
    the login URL from `tailscale up`.

.PARAMETER TailscaleMsi
    Installer MSI to use when winget is unavailable. Installed with one UAC
    prompt: msiexec /i <msi> /quiet /norestart.

.PARAMETER Tailscale
    Ask, Yes, or No. No skips install and prints the tunnel guide.

.PARAMETER NonInteractive
    Do not prompt. Tailscale defaults to No unless -Tailscale Yes.

.PARAMETER DryRun
    Print actions without installing, registering, or starting Tailscale.
#>

[CmdletBinding()]
param(
    [string]$AuthKey,
    [string]$TailscaleMsi,
    [ValidateSet('Ask', 'Yes', 'No')]
    [string]$Tailscale = 'Ask',
    [switch]$NonInteractive,
    [switch]$DryRun,
    [switch]$SkipRegister,
    [string]$HandsExe,
    [string]$HandsHome
)

$ErrorActionPreference = 'Stop'

function Write-TunnelGuide {
    Write-Host ""
    Write-Host "Authenticated remote access (no public default)"
    Write-Host "Hands stays a stdio process. A remote host such as ChatGPT desktop needs an HTTPS bridge on 127.0.0.1 with a bearer token you choose, then a tunnel that requires its own login."
    Write-Host "Do not use Tailscale Funnel. Do not run 'cloudflared tunnel --url'. Do not run 'ngrok http' without an auth flag."
    Write-Host ""
    Write-Host "Cloudflare Tunnel"
    Write-Host "  1. Install cloudflared and run: cloudflared tunnel login"
    Write-Host "  2. cloudflared tunnel create hands"
    Write-Host "  3. Route a hostname you control to http://127.0.0.1:<bridge-port> only."
    Write-Host "  4. Put Cloudflare Access in front of that hostname (one-time PIN or a service token)."
    Write-Host "  5. Give the remote client the tunnel URL plus the bridge bearer token."
    Write-Host ""
    Write-Host "ngrok"
    Write-Host "  ngrok http 127.0.0.1:<bridge-port> --oauth=google"
    Write-Host "  or: ngrok http 127.0.0.1:<bridge-port> --basic-auth 'user:password'"
    Write-Host "  The bridge still requires its own bearer token. An open ngrok URL is not a substitute."
    Write-Host ""
    Write-Host "Details: docs/chatgpt-desktop-mcp.md and docs/remote-access.md"
}

function Find-TailscaleExe {
    $command = Get-Command tailscale -ErrorAction SilentlyContinue
    if ($command -and $command.Source) { return $command.Source }
    foreach ($rootName in @('ProgramFiles', 'ProgramW6432')) {
        $root = [Environment]::GetEnvironmentVariable($rootName)
        if (-not $root) { continue }
        $candidate = Join-Path (Join-Path $root 'Tailscale') 'tailscale.exe'
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }
    return $null
}

function Get-TailscaleState {
    $exe = Find-TailscaleExe
    if (-not $exe) { return $null }
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $raw = & $exe status --json 2>$null
    $ErrorActionPreference = $previous
    if (-not $raw) { return [pscustomobject]@{ Installed = $true; LoggedIn = $false; BackendState = ''; MagicDns = $null } }
    try {
        $status = ($raw | Out-String) | ConvertFrom-Json
    } catch {
        return [pscustomobject]@{ Installed = $true; LoggedIn = $false; BackendState = ''; MagicDns = $null }
    }
    $backend = [string]$status.BackendState
    $dns = $null
    if ($backend -eq 'Running' -and $status.Self -and $status.Self.DNSName) {
        $dns = ([string]$status.Self.DNSName).Trim().TrimEnd('.')
    }
    # Do not print TailscaleIPs, peer names, or the raw document.
    return [pscustomobject]@{
        Installed = $true
        LoggedIn = ($backend -eq 'Running')
        BackendState = $backend
        MagicDns = $dns
    }
}

function Install-TailscalePackage {
    if ($DryRun) {
        Write-Host "[DryRun] Would install Tailscale (winget id Tailscale.Tailscale, or msiexec /quiet)."
        return
    }
    if (Get-Command winget -ErrorAction SilentlyContinue) {
        Write-Host "Installing Tailscale with winget. Expect one UAC prompt."
        & winget install --id Tailscale.Tailscale -e --accept-package-agreements --accept-source-agreements --disable-interactivity
        if ($LASTEXITCODE -ne 0) {
            & winget install --id Tailscale.Tailscale -e --accept-package-agreements --accept-source-agreements
        }
        if ($LASTEXITCODE -eq 0) { return }
        Write-Warning "winget install exited $LASTEXITCODE."
    }
    if ($TailscaleMsi) {
        if (-not (Test-Path -LiteralPath $TailscaleMsi)) {
            throw "Tailscale MSI not found: $TailscaleMsi"
        }
        Write-Host "Installing Tailscale MSI with one UAC prompt."
        $process = Start-Process -FilePath msiexec.exe -ArgumentList @('/i', $TailscaleMsi, '/quiet', '/norestart') -Verb RunAs -Wait -PassThru
        if ($process.ExitCode -ne 0) {
            throw "msiexec exited $($process.ExitCode)"
        }
        return
    }
    throw "Tailscale is not installed and winget is unavailable. Re-run with -TailscaleMsi <path-to-official-msi> or install from https://tailscale.com/download/windows"
}

$register = Join-Path $PSScriptRoot 'Register-HandsClients.ps1'
if (-not $SkipRegister) {
    if (-not (Test-Path -LiteralPath $register)) {
        throw "Register-HandsClients.ps1 not found next to this script."
    }
    $registerArgs = @{}
    if ($HandsExe) { $registerArgs.HandsExe = $HandsExe }
    if ($HandsHome) { $registerArgs.HandsHome = $HandsHome }
    if ($DryRun) { $registerArgs.DryRun = $true }
    & $register @registerArgs
    if ($LASTEXITCODE -and $LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

$choice = $Tailscale
if ($NonInteractive -and $choice -eq 'Ask') { $choice = 'No' }
$state = Get-TailscaleState
if ($state -and $state.LoggedIn) {
    Write-Host "Tailscale is logged in."
    if ($state.MagicDns) { Write-Host "MagicDNS (live, not stored): $($state.MagicDns)" }
    $choice = 'No'
} elseif ($state) {
    Write-Host "Tailscale is installed (backend: $($state.BackendState)). Sign-in is still required."
}

if ($choice -eq 'Ask') {
    $reply = Read-Host "Install or use Tailscale so your other devices can reach this machine? [y/N]"
    if ($reply -match '^[Yy]') { $choice = 'Yes' } else { $choice = 'No' }
}

if ($choice -eq 'No') {
    if (-not ($state -and $state.LoggedIn)) { Write-TunnelGuide }
    if ($AuthKey) { $AuthKey = $null }
    exit 0
}

if (-not (Find-TailscaleExe)) {
    Install-TailscalePackage
}

$exe = Find-TailscaleExe
if (-not $exe) {
    throw "Tailscale install finished but tailscale.exe was not found. Open a new shell and run: tailscale up"
}

Write-Host "Sign in to Tailscale yourself. Hands will not create or store a key."
if ($DryRun) {
    Write-Host "[DryRun] Would run tailscale up. An auth key, if supplied, would not be printed or saved."
    if ($AuthKey) { $AuthKey = $null }
    exit 0
}

if ($AuthKey) {
    Write-Host "Using the auth key you passed for this invocation only."
    & $exe up --auth-key $AuthKey --reset=false
    $AuthKey = $null
} else {
    Write-Host "Running tailscale up. Open the login URL it prints."
    & $exe up
}

$after = Get-TailscaleState
if ($after -and $after.LoggedIn) {
    Write-Host "Tailscale login succeeded."
    if ($after.MagicDns) { Write-Host "MagicDNS (live, not stored): $($after.MagicDns)" }
    exit 0
}
Write-Warning "Tailscale is not in the Running state yet. Finish login with: tailscale up"
exit 1
