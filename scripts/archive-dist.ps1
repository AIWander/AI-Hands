[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $Path,
    [string] $BackupRoot = ''
)

$ErrorActionPreference = 'Stop'

function Get-Sha256 {
    param(
        [Parameter(Mandatory)]
        [string] $LiteralPath
    )

    $stream = [System.IO.File]::OpenRead($LiteralPath)
    try {
        $sha256 = [System.Security.Cryptography.SHA256]::Create()
        try {
            return (($sha256.ComputeHash($stream) | ForEach-Object {
                $_.ToString('X2')
            }) -join '')
        }
        finally {
            $sha256.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

if (-not (Test-Path -LiteralPath $Path)) {
    return
}

if ([string]::IsNullOrWhiteSpace($BackupRoot)) {
    $BackupRoot = Join-Path $env:LOCALAPPDATA 'AIWander\AIHands\backups'
}
$backupRoot = [System.IO.Path]::GetFullPath($BackupRoot)
$archiveRoot = [System.IO.Path]::GetFullPath((Join-Path $backupRoot 'archive'))
$null = New-Item -ItemType Directory -Path $backupRoot -Force
$source = (Resolve-Path -LiteralPath $Path).Path
$stamp = Get-Date -Format 'yyyyMMdd_HHmmss'
$prefix = 'unified-hands-template__hand.exe.pre_rebuild_'
$backup = Join-Path $backupRoot "$prefix$stamp"

Copy-Item -LiteralPath $source -Destination $backup
(Get-Item -LiteralPath $backup).LastWriteTime = Get-Date

$sourceHash = Get-Sha256 -LiteralPath $source
$backupHash = Get-Sha256 -LiteralPath $backup
if ($sourceHash -ne $backupHash) {
    throw "Archive hash mismatch for $source"
}

$activeBackups = @(
    Get-ChildItem -LiteralPath $backupRoot -File -Filter "$prefix*" |
        Sort-Object LastWriteTime -Descending
)

if ($activeBackups.Count -gt 3) {
    if (-not (Test-Path -LiteralPath $archiveRoot)) {
        New-Item -ItemType Directory -Path $archiveRoot | Out-Null
    }

    foreach ($oldBackup in $activeBackups | Select-Object -Skip 3) {
        $sourceFull = [System.IO.Path]::GetFullPath($oldBackup.FullName)
        $destinationFull = [System.IO.Path]::GetFullPath(
            (Join-Path $archiveRoot $oldBackup.Name)
        )
        if (-not $sourceFull.StartsWith($backupRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to rotate backup outside $backupRoot"
        }
        if (-not $destinationFull.StartsWith($archiveRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to rotate backup outside $archiveRoot"
        }
        Move-Item -LiteralPath $sourceFull -Destination $destinationFull
    }
}

[pscustomobject]@{
    Archived = $source
    Backup = $backup
    SHA256 = $backupHash
}
