<#
.SYNOPSIS
    Back-compat entry point for AI-Hands host registration.

.DESCRIPTION
    Forwards to Register-HandsClients.ps1, which keeps the original backup-first
    behavior and skips hosts that already have a hands entry unless -Force is set.
#>

[CmdletBinding()]
param(
    [string]$HandsExe,
    [string]$HandsHome,
    [switch]$Force,
    [switch]$DryRun
)

$target = Join-Path $PSScriptRoot 'Register-HandsClients.ps1'
if (-not (Test-Path -LiteralPath $target)) {
    throw "Register-HandsClients.ps1 is missing next to register-hands.ps1."
}
& $target @PSBoundParameters
exit $LASTEXITCODE
