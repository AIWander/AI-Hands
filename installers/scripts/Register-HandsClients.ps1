<#
.SYNOPSIS
    Registers (or removes) the standalone AI-Hands stdio server in Claude desktop, Codex and Grok CLI.

.DESCRIPTION
    Backup-first config writer used by HandsSetup (Inno Setup) and usable on its own.

      ClaudeDesktop  JSON  mcpServers.hands = {"command": "<HandsExe>", "args": []}
                           in claude_desktop_config.json (Squirrel %APPDATA%\Claude and/or the
                           MSIX %LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming\Claude copy)
      Codex          TOML  [mcp_servers.hands] command = '<HandsExe>'  in %USERPROFILE%\.codex\config.toml
      GrokCli        TOML  same table in %USERPROFILE%\.grok\config.toml, plus "hands" removed from the
                           top-level disabled_mcp_servers list and no enabled = false in the table

    Rules:
      - An existing hands entry is never doubled. Without -Force it is reported "skipped existing".
      - Every write backs the file up first to <dir>\backups\<file>.pre_hands_<yyyyMMdd_HHmmss>,
        writes a temp file, swaps it in, then parses the result back. A failed parse-back restores
        the backup and exits 1.
      - JSON is read tolerant of a UTF-8 BOM and written as UTF-8 without BOM. All other keys are kept.
      - TOML is edited line by line; only the hands table (and its own sub-tables) is touched.
      - -Uninstall removes only a hands entry that points at -HandsExe (any hands entry with -Force).
        Nothing else is restored or changed.

    Runs on Windows PowerShell 5.1 and PowerShell 7+.

    Exit codes: 0 = something changed (or -Detect), 2 = nothing to do, 1 = error.

.PARAMETER HandsExe
    Path written into each client config. Default %LOCALAPPDATA%\AIWander\Hands\hands.exe.

.PARAMETER Clients
    Any of ClaudeDesktop, Codex, GrokCli. Comma-separated strings are accepted (powershell -File
    passes "A,B" as one string). Default: all three; undetected clients report "not detected".

.PARAMETER Detect
    Print which clients are present, with evidence, and exit without writing.

.PARAMETER DryRun
    Report what would change; write nothing (no backups either).

.PARAMETER Uninstall
    Remove the hands entry/table instead of adding it.

.PARAMETER Force
    Replace an existing hands entry (install), or remove a hands entry that is not ours (uninstall).

.PARAMETER ConfigRoot
    Test hook. When set, this one directory stands in for %APPDATA%, %LOCALAPPDATA% and %USERPROFILE%,
    and the machine-wide probes (Get-AppxPackage, PATH) are skipped, so detection is hermetic:
      <root>\AnthropicClaude\claude.exe          Claude binary marker
      <root>\Claude\claude_desktop_config.json   Claude config (Squirrel layout)
      <root>\Packages\Claude_*\LocalCache\Roaming\Claude\   Claude config (MSIX layout)
      <root>\.codex\config.toml, <root>\npm\codex.cmd
      <root>\.grok\config.toml,  <root>\.grok\bin\grok(.exe)

.PARAMETER LogPath
    Optional file the summary lines are appended to (the installer passes {app}\register.log).

.EXAMPLE
    .\Register-HandsClients.ps1 -Detect
.EXAMPLE
    .\Register-HandsClients.ps1 -Clients ClaudeDesktop,Codex -DryRun
.EXAMPLE
    .\Register-HandsClients.ps1 -Clients GrokCli -Force
.EXAMPLE
    .\Register-HandsClients.ps1 -Uninstall
#>

[CmdletBinding()]
param(
    [string]$HandsExe,
    [string[]]$Clients,
    [switch]$Detect,
    [switch]$DryRun,
    [switch]$Uninstall,
    [switch]$Force,
    [string]$ConfigRoot,
    [string]$LogPath
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

$AllClients = @('ClaudeDesktop', 'Codex', 'GrokCli')
$Utf8NoBom = New-Object System.Text.UTF8Encoding $false
$Stamp = Get-Date -Format 'yyyyMMdd_HHmmss'

# --- roots ------------------------------------------------------------------

if ($ConfigRoot) {
    $ConfigRoot = [System.IO.Path]::GetFullPath($ConfigRoot)
    $AppDataRoot = $ConfigRoot
    $LocalRoot = $ConfigRoot
    $ProfileRoot = $ConfigRoot
    $Hermetic = $true
} else {
    $AppDataRoot = $env:APPDATA
    $LocalRoot = $env:LOCALAPPDATA
    $ProfileRoot = $env:USERPROFILE
    $Hermetic = $false
}

if (-not $HandsExe) {
    $HandsExe = Join-Path $env:LOCALAPPDATA 'AIWander\Hands\hands.exe'
}

# --- output -----------------------------------------------------------------

$script:Lines = New-Object System.Collections.Generic.List[string]

function Write-Summary([string]$Text) {
    $script:Lines.Add($Text)
    Write-Output $Text
}

function Save-Log {
    if (-not $LogPath) { return }
    try {
        $dir = Split-Path -Parent $LogPath
        if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
        $head = "--- $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') Register-HandsClients mode=$(if ($Uninstall) {'uninstall'} elseif ($Detect) {'detect'} else {'install'}) dryrun=$([bool]$DryRun) force=$([bool]$Force)"
        $text = $head + [Environment]::NewLine + ($script:Lines -join [Environment]::NewLine) + [Environment]::NewLine
        [System.IO.File]::AppendAllText($LogPath, $text, $Utf8NoBom)
    } catch {
        Write-Warning "could not write log ${LogPath}: $($_.Exception.Message)"
    }
}

# --- detection --------------------------------------------------------------

function Get-ClaudeConfigPaths {
    $paths = @()
    $squirrelDir = Join-Path $AppDataRoot 'Claude'
    if (Test-Path -LiteralPath $squirrelDir -PathType Container) {
        $paths += (Join-Path $squirrelDir 'claude_desktop_config.json')
    }
    $pkgRoot = Join-Path $LocalRoot 'Packages'
    if (Test-Path -LiteralPath $pkgRoot -PathType Container) {
        foreach ($pkg in @(Get-ChildItem -LiteralPath $pkgRoot -Directory -Filter 'Claude_*' -ErrorAction SilentlyContinue)) {
            $msixDir = Join-Path $pkg.FullName 'LocalCache\Roaming\Claude'
            if (Test-Path -LiteralPath $msixDir -PathType Container) {
                $paths += (Join-Path $msixDir 'claude_desktop_config.json')
            }
        }
    }
    return ,$paths
}

function Test-ClaudeAppx {
    if ($Hermetic) { return $false }
    try {
        $pkgs = @(Get-AppxPackage -Name '*Claude*' -ErrorAction Stop)
        return ($pkgs.Count -gt 0)
    } catch {
        return $false
    }
}

function Get-Detection([string]$Client) {
    $evidence = @()
    $configs = @()
    $present = $false
    switch ($Client) {
        'ClaudeDesktop' {
            $exe = Join-Path $LocalRoot 'AnthropicClaude\claude.exe'
            $bin = $false
            if (Test-Path -LiteralPath $exe) { $bin = $true; $evidence += $exe }
            if (-not $bin -and (Test-ClaudeAppx)) { $bin = $true; $evidence += 'Get-AppxPackage *Claude*' }
            $configs = Get-ClaudeConfigPaths
            if ($configs.Count -gt 0) { $evidence += ($configs | ForEach-Object { Split-Path -Parent $_ }) }
            $present = ($bin -and $configs.Count -gt 0)
        }
        'Codex' {
            $cfg = Join-Path $ProfileRoot '.codex\config.toml'
            $configs = @($cfg)
            if (Test-Path -LiteralPath $cfg) { $evidence += $cfg }
            $shim = Join-Path $AppDataRoot 'npm\codex.cmd'
            if (Test-Path -LiteralPath $shim) { $evidence += $shim }
            elseif (-not $Hermetic) {
                $cmd = Get-Command codex -ErrorAction SilentlyContinue | Select-Object -First 1
                if ($cmd) { $evidence += $cmd.Source }
            }
            $present = ($evidence.Count -gt 0)
        }
        'GrokCli' {
            $cfg = Join-Path $ProfileRoot '.grok\config.toml'
            $configs = @($cfg)
            foreach ($p in @((Join-Path $ProfileRoot '.grok\bin\grok.exe'), (Join-Path $ProfileRoot '.grok\bin\grok'), $cfg)) {
                if (Test-Path -LiteralPath $p) { $evidence += $p }
            }
            $present = ($evidence.Count -gt 0)
        }
    }
    return [pscustomobject]@{ Client = $Client; Present = $present; Evidence = $evidence; Configs = $configs }
}

# --- file plumbing ----------------------------------------------------------

function Read-TextNoBom([string]$Path) {
    $text = [System.IO.File]::ReadAllText($Path, $Utf8NoBom)
    if ($text.Length -gt 0 -and $text[0] -eq [char]0xFEFF) { $text = $text.Substring(1) }
    return $text
}

function Backup-File([string]$Path) {
    $dir = Join-Path (Split-Path -Parent $Path) 'backups'
    if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    $leaf = Split-Path -Leaf $Path
    $dest = Join-Path $dir "$leaf.pre_hands_$Stamp"
    $n = 2
    while (Test-Path -LiteralPath $dest) { $dest = Join-Path $dir "$leaf.pre_hands_${Stamp}_$n"; $n++ }
    Copy-Item -LiteralPath $Path -Destination $dest
    if ((Get-Item -LiteralPath $dest).Length -ne (Get-Item -LiteralPath $Path).Length) {
        throw "backup size mismatch for $Path"
    }
    return $dest
}

# Temp-file + swap. File.Replace is an atomic rename on NTFS when the target exists.
function Write-Swap([string]$Path, [string]$Text) {
    $dir = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    $tmp = Join-Path $dir ("." + (Split-Path -Leaf $Path) + ".hands_tmp_" + [guid]::NewGuid().ToString('N'))
    [System.IO.File]::WriteAllText($tmp, $Text, $Utf8NoBom)
    if (Test-Path -LiteralPath $Path) {
        [System.IO.File]::Replace($tmp, $Path, [NullString]::Value)
    } else {
        [System.IO.File]::Move($tmp, $Path)
    }
}

function Restore-Backup([string]$Path, [string]$Backup) {
    if ($Backup) {
        Copy-Item -LiteralPath $Backup -Destination $Path -Force
    } elseif (Test-Path -LiteralPath $Path) {
        Remove-Item -LiteralPath $Path -Force
    }
}

# StrictMode-safe property test (".PSObject.Properties.Name" throws on an empty object in PS 5.1)
function Test-Prop($Obj, [string]$Name) {
    if ($null -eq $Obj) { return $false }
    return ($null -ne $Obj.PSObject.Properties[$Name])
}

function Test-SamePath([string]$A, [string]$B) {
    if (-not $A -or -not $B) { return $false }
    $na = $A.Trim().Replace('/', '\').TrimEnd('\')
    $nb = $B.Trim().Replace('/', '\').TrimEnd('\')
    return [string]::Equals($na, $nb, [System.StringComparison]::OrdinalIgnoreCase)
}

# --- JSON -------------------------------------------------------------------

$script:FromJsonHasDateKind = (Get-Command ConvertFrom-Json).Parameters.ContainsKey('DateKind')

function ConvertFrom-JsonText([string]$Text) {
    if ($script:FromJsonHasDateKind) { return ($Text | ConvertFrom-Json -DateKind String) }
    return ($Text | ConvertFrom-Json)
}

function ConvertTo-JsonString([string]$s) {
    $sb = New-Object System.Text.StringBuilder
    [void]$sb.Append('"')
    foreach ($ch in $s.ToCharArray()) {
        switch ($ch) {
            '"' { [void]$sb.Append('\"'); continue }
            '\' { [void]$sb.Append('\\'); continue }
            "`n" { [void]$sb.Append('\n'); continue }
            "`r" { [void]$sb.Append('\r'); continue }
            "`t" { [void]$sb.Append('\t'); continue }
            "`b" { [void]$sb.Append('\b'); continue }
            "`f" { [void]$sb.Append('\f'); continue }
            default {
                if ([int]$ch -lt 0x20) { [void]$sb.Append(('\u{0:x4}' -f [int]$ch)) }
                else { [void]$sb.Append($ch) }
            }
        }
    }
    [void]$sb.Append('"')
    return $sb.ToString()
}

# Own serializer: identical, readable output on PS 5.1 and 7 (5.1's ConvertTo-Json indents
# oddly and escapes < > ' as < etc.).
function ConvertTo-JsonText($Value, [int]$Level = 0) {
    $pad = '  ' * ($Level + 1)
    $end = '  ' * $Level
    if ($null -eq $Value) { return 'null' }
    if ($Value -is [string]) { return (ConvertTo-JsonString $Value) }
    if ($Value -is [bool]) { if ($Value) { return 'true' } else { return 'false' } }
    if ($Value -is [double] -or $Value -is [single]) { return $Value.ToString('R', [System.Globalization.CultureInfo]::InvariantCulture) }
    if ($Value -is [int] -or $Value -is [long] -or $Value -is [decimal] -or $Value -is [int16] -or $Value -is [byte] -or
        $Value -is [uint32] -or $Value -is [uint64] -or $Value.GetType().FullName -eq 'System.Numerics.BigInteger') {
        return $Value.ToString([System.Globalization.CultureInfo]::InvariantCulture)
    }
    if ($Value -is [datetime]) { return (ConvertTo-JsonString $Value.ToString('o')) }
    if ($Value -is [System.Collections.IDictionary]) {
        if ($Value.Count -eq 0) { return '{}' }
        $parts = foreach ($k in $Value.Keys) { $pad + (ConvertTo-JsonString ([string]$k)) + ': ' + (ConvertTo-JsonText $Value[$k] ($Level + 1)) }
        return "{`n" + ($parts -join ",`n") + "`n$end}"
    }
    if ($Value -is [System.Management.Automation.PSCustomObject]) {
        $props = @($Value.PSObject.Properties | Where-Object { $_.MemberType -eq 'NoteProperty' })
        if ($props.Count -eq 0) { return '{}' }
        $parts = foreach ($p in $props) { $pad + (ConvertTo-JsonString $p.Name) + ': ' + (ConvertTo-JsonText $p.Value ($Level + 1)) }
        return "{`n" + ($parts -join ",`n") + "`n$end}"
    }
    if ($Value -is [System.Collections.IEnumerable]) {
        $items = @($Value)
        if ($items.Count -eq 0) { return '[]' }
        $parts = foreach ($i in $items) { $pad + (ConvertTo-JsonText $i ($Level + 1)) }
        return "[`n" + ($parts -join ",`n") + "`n$end]"
    }
    return (ConvertTo-JsonString $Value.ToString())
}

function Invoke-ClaudeConfig([string]$Path) {
    $exists = Test-Path -LiteralPath $Path
    if ($exists) {
        $raw = Read-TextNoBom $Path
        if ([string]::IsNullOrWhiteSpace($raw)) { $root = New-Object psobject }
        else {
            try { $root = ConvertFrom-JsonText $raw }
            catch { throw "cannot parse $Path as JSON: $($_.Exception.Message)" }
        }
        if (-not ($root -is [System.Management.Automation.PSCustomObject])) { throw "$Path does not hold a JSON object" }
    } else {
        if ($Uninstall) { return 'nothing to remove' }
        $root = ConvertFrom-JsonText '{"mcpServers":{}}'
    }

    $hasServers = (Test-Prop $root 'mcpServers')
    if ($hasServers -and $null -ne $root.mcpServers -and -not ($root.mcpServers -is [System.Management.Automation.PSCustomObject])) {
        throw "$Path has an mcpServers value that is not an object"
    }
    $servers = if ($hasServers) { $root.mcpServers } else { $null }
    $existing = $null
    if ($null -ne $servers -and (Test-Prop $servers 'hands')) { $existing = $servers.hands }

    if ($Uninstall) {
        if ($null -eq $existing) { return 'nothing to remove' }
        $cmd = $null
        if ($existing -is [System.Management.Automation.PSCustomObject] -and (Test-Prop $existing 'command')) { $cmd = [string]$existing.command }
        if (-not $Force -and -not (Test-SamePath $cmd $HandsExe)) { return 'skipped foreign' }
        if ($DryRun) { return 'would remove' }
        $servers.PSObject.Properties.Remove('hands')
    } else {
        if ($null -ne $existing -and -not $Force) { return 'skipped existing' }
        if ($DryRun) { return 'would write' }
        if (-not $hasServers -or $null -eq $root.mcpServers) {
            if ($hasServers) { $root.PSObject.Properties.Remove('mcpServers') }
            $root | Add-Member -MemberType NoteProperty -Name 'mcpServers' -Value (New-Object psobject)
            $servers = $root.mcpServers
        }
        $entry = [pscustomobject][ordered]@{ command = $HandsExe; args = @() }
        if ($null -ne $existing) { $servers.hands = $entry }
        else { $servers | Add-Member -MemberType NoteProperty -Name 'hands' -Value $entry }
    }

    $text = (ConvertTo-JsonText $root) + "`n"
    $backup = $null
    if ($exists) { $backup = Backup-File $Path }
    Write-Swap $Path $text

    try {
        $check = ConvertFrom-JsonText (Read-TextNoBom $Path)
        $after = $null
        if ((Test-Prop $check 'mcpServers') -and (Test-Prop $check.mcpServers 'hands')) { $after = $check.mcpServers.hands }
        if ($Uninstall) {
            if ($null -ne $after) { throw 'hands still present after removal' }
        } elseif ($null -eq $after -or -not (Test-SamePath ([string]$after.command) $HandsExe)) {
            throw 'hands entry missing or wrong after write'
        }
        $bytes = [System.IO.File]::ReadAllBytes($Path)
        if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) { throw 'BOM present after write' }
    } catch {
        Restore-Backup $Path $backup
        throw "parse-back failed for $Path ($($_.Exception.Message)); restored previous file"
    }
    if ($backup) { Write-Verbose "backup: $backup" }
    if ($Uninstall) { return 'removed' } else { return 'written' }
}

# --- TOML (guarded line-based) ----------------------------------------------

$HandsHeaderRx = '^\s*\[\s*(mcp_servers|"mcp_servers")\s*\.\s*(hands|"hands")\s*\]\s*(#.*)?$'
$HandsSubRx = '^\s*\[\[?\s*(mcp_servers|"mcp_servers")\s*\.\s*(hands|"hands")\s*\.'
$McpServersHeaderRx = '^\s*\[\s*(mcp_servers|"mcp_servers")\s*\]\s*(#.*)?$'

function Test-Header([string]$Line) {
    # A table header is "[name...]" or "[[name...]]" on its own line; array continuation lines like
    # "  "a", "b"," never start with "[" followed by a bare/quoted key.
    return ($Line -match '^\s*\[\[?\s*("[^"]*"|''[^'']*''|[A-Za-z0-9_\-]+)(\s*\.\s*("[^"]*"|''[^'']*''|[A-Za-z0-9_\-]+))*\s*\]\]?\s*(#.*)?$')
}

# Returns a list of [start,end] index pairs, one per hands table (with its sub-tables).
function Find-HandsBlocks([string[]]$L) {
    $blocks = @()
    $i = 0
    while ($i -lt $L.Count) {
        if ($L[$i] -match $HandsHeaderRx -or $L[$i] -match $HandsSubRx) {
            $start = $i
            $j = $i + 1
            while ($j -lt $L.Count) {
                if ((Test-Header $L[$j]) -and -not ($L[$j] -match $HandsSubRx)) { break }
                $j++
            }
            # leave trailing blank/comment lines in place; they usually belong to the next table
            $end = $j - 1
            while ($end -gt $start -and ($L[$end] -match '^\s*$' -or $L[$end] -match '^\s*#')) { $end-- }
            $blocks += , @($start, $end)
            $i = $j
        } else {
            $i++
        }
    }
    return , $blocks
}

# hands defined in a form we do not rewrite: [mcp_servers] hands = {...}, or dotted mcp_servers.hands.x = ...
function Test-HandsInlineForm([string[]]$L) {
    $inMcp = $false
    $topLevel = $true
    foreach ($line in $L) {
        if (Test-Header $line) { $topLevel = $false; $inMcp = ($line -match $McpServersHeaderRx); continue }
        if ($inMcp -and $line -match '^\s*("?hands"?)\s*(\.|=)') { return $true }
        if ($topLevel -and $line -match '^\s*("?mcp_servers"?)\s*\.\s*("?hands"?)\s*(\.|=)') { return $true }
    }
    return $false
}

function Format-TomlString([string]$s) {
    if ($s -notmatch "'" -and $s -notmatch "[\r\n]") { return "'" + $s + "'" }
    return '"' + $s.Replace('\', '\\').Replace('"', '\"') + '"'
}

function Get-FirstHeaderIndex([string[]]$L) {
    for ($i = 0; $i -lt $L.Count; $i++) { if (Test-Header $L[$i]) { return $i } }
    return $L.Count
}

# Removes "hands" from a top-level disabled_mcp_servers = [...] (single- or multi-line).
# Returns the new line array, or $null when there was nothing to change.
function Remove-HandsFromDisabled([string[]]$L) {
    $limit = Get-FirstHeaderIndex $L
    for ($i = 0; $i -lt $limit; $i++) {
        if ($L[$i] -notmatch '^\s*disabled_mcp_servers\s*=\s*\[') { continue }
        $depth = 0
        $j = $i
        while ($j -lt $L.Count) {
            $bare = ($L[$j] -replace '"(?:[^"\\]|\\.)*"', '""' -replace "'[^']*'", "''") -replace '#.*$', ''
            $depth += ([regex]::Matches($bare, '\[')).Count - ([regex]::Matches($bare, '\]')).Count
            if ($depth -le 0) { break }
            $j++
        }
        if ($j -ge $L.Count) { throw 'unterminated disabled_mcp_servers array' }
        $span = ($L[$i..$j]) -join "`n"
        $new = $span
        $item = '(?:"hands"|''hands'')'
        if ($new -match "$item\s*,") { $new = ([regex]"$item\s*,[ \t]*(\n[ \t]*)?").Replace($new, '', 1) }
        elseif ($new -match ",\s*$item") { $new = ([regex]",\s*$item").Replace($new, '', 1) }
        elseif ($new -match $item) { $new = ([regex]$item).Replace($new, '', 1) }
        if ($new -eq $span) { return $null }
        $before = @(); if ($i -gt 0) { $before = $L[0..($i - 1)] }
        $after = @(); if ($j + 1 -lt $L.Count) { $after = $L[($j + 1)..($L.Count - 1)] }
        # a removed "hands", line on its own leaves an empty line; drop it
        $mid = @($new -split "`n" | Where-Object { $_ -notmatch '^\s*$' })
        return , ([string[]]($before + $mid + $after))
    }
    return $null
}

function Invoke-TomlConfig([string]$Client, [string]$Path) {
    $exists = Test-Path -LiteralPath $Path
    $raw = ''
    if ($exists) { $raw = Read-TextNoBom $Path }
    elseif ($Uninstall) { return 'nothing to remove' }
    $nl = if ($raw -match "`r`n") { "`r`n" } else { "`n" }
    $endsWithNl = ($raw.Length -eq 0) -or $raw.EndsWith("`n")
    [string[]]$L = @()
    if ($raw.Length -gt 0) {
        $L = $raw -split "`r?`n"
        if ($endsWithNl) { $L = $L[0..($L.Count - 2)] }
    }

    $blocks = Find-HandsBlocks $L
    $inline = Test-HandsInlineForm $L
    $hasHands = ($blocks.Count -gt 0) -or $inline

    $newBlock = @('[mcp_servers.hands]', ('command = ' + (Format-TomlString $HandsExe)))

    if ($Uninstall) {
        if (-not $hasHands) { return 'nothing to remove' }
        if ($inline) { throw "$Path defines hands inline or with dotted keys; remove it by hand" }
        if (-not $Force) {
            foreach ($b in $blocks) {
                $cmdLine = @($L[$b[0]..$b[1]] | Where-Object { $_ -match '^\s*command\s*=' } | Select-Object -First 1)
                $val = $null
                if ($cmdLine.Count -gt 0 -and $cmdLine[0] -match '^\s*command\s*=\s*(''([^'']*)''|"((?:[^"\\]|\\.)*)")') {
                    $val = if ($Matches[2]) { $Matches[2] } else { $Matches[3] -replace '\\\\', '\' }
                }
                if (-not (Test-SamePath $val $HandsExe)) { return 'skipped foreign' }
            }
        }
        if ($DryRun) { return 'would remove' }
        $out = New-Object System.Collections.Generic.List[string]
        $skip = @{}
        foreach ($b in $blocks) { for ($k = $b[0]; $k -le $b[1]; $k++) { $skip[$k] = $true } }
        for ($k = 0; $k -lt $L.Count; $k++) { if (-not $skip.ContainsKey($k)) { $out.Add($L[$k]) } }
        $L2 = [string[]]$out.ToArray()
    } else {
        if ($hasHands -and -not $Force) { return 'skipped existing' }
        if ($inline) { throw "$Path defines hands inline or with dotted keys; -Force cannot safely replace it, edit by hand" }
        if ($DryRun) { return 'would write' }
        $out = New-Object System.Collections.Generic.List[string]
        if ($blocks.Count -gt 0) {
            # replace the first block in place, drop any duplicates
            $skip = @{}
            foreach ($b in $blocks) { for ($k = $b[0]; $k -le $b[1]; $k++) { $skip[$k] = $true } }
            for ($k = 0; $k -lt $L.Count; $k++) {
                if ($k -eq $blocks[0][0]) { foreach ($n in $newBlock) { $out.Add($n) } }
                if (-not $skip.ContainsKey($k)) { $out.Add($L[$k]) }
            }
        } else {
            foreach ($x in $L) { $out.Add($x) }
            while ($out.Count -gt 0 -and $out[$out.Count - 1] -match '^\s*$') { $out.RemoveAt($out.Count - 1) }
            if ($out.Count -gt 0) { $out.Add('') }
            foreach ($n in $newBlock) { $out.Add($n) }
        }
        $L2 = [string[]]$out.ToArray()
        if ($Client -eq 'GrokCli') {
            $undisabled = Remove-HandsFromDisabled $L2
            if ($null -ne $undisabled) { $L2 = $undisabled }
        }
    }

    # collapse runs of blank lines a removal may have produced (only where we removed something)
    $collapsed = New-Object System.Collections.Generic.List[string]
    $prevBlank = $false
    foreach ($x in $L2) {
        $blank = ($x -match '^\s*$')
        if ($blank -and $prevBlank) { continue }
        $collapsed.Add($x); $prevBlank = $blank
    }
    while ($collapsed.Count -gt 0 -and $collapsed[$collapsed.Count - 1] -match '^\s*$') { $collapsed.RemoveAt($collapsed.Count - 1) }
    $text = ($collapsed.ToArray() -join $nl)
    if ($collapsed.Count -gt 0) { $text += $nl }

    $backup = $null
    if ($exists) { $backup = Backup-File $Path }
    Write-Swap $Path $text

    try {
        [string[]]$C = @((Read-TextNoBom $Path) -split "`r?`n")
        $headers = @($C | Where-Object { $_ -match $HandsHeaderRx }).Count
        if ($Uninstall) {
            if ($headers -ne 0) { throw 'hands table still present after removal' }
        } else {
            if ($headers -ne 1) { throw "expected exactly one [mcp_servers.hands], found $headers" }
            $cb = Find-HandsBlocks $C
            $body = @($C[$cb[0][0]..$cb[0][1]])
            if (-not ($body -contains $newBlock[1])) { throw 'command line missing after write' }
            if (@($body | Where-Object { $_ -match '^\s*enabled\s*=\s*false' }).Count -gt 0) { throw 'enabled = false still in hands table' }
            if ($Client -eq 'GrokCli' -and $null -ne (Remove-HandsFromDisabled $C)) { throw 'hands still in disabled_mcp_servers' }
        }
        # every header line must still be unique
        $dups = @($C | Where-Object { Test-Header $_ } | Where-Object { $_ -notmatch '^\s*\[\[' } | ForEach-Object { ($_ -replace '#.*$', '').Trim() } | Group-Object | Where-Object { $_.Count -gt 1 })
        if ($dups.Count -gt 0 -and -not (@($L | Where-Object { Test-Header $_ } | Where-Object { $_ -notmatch '^\s*\[\[' } | ForEach-Object { ($_ -replace '#.*$', '').Trim() } | Group-Object | Where-Object { $_.Count -gt 1 }).Count -gt 0)) {
            throw "duplicate table header introduced: $($dups[0].Name)"
        }
    } catch {
        Restore-Backup $Path $backup
        throw "parse-back failed for $Path ($($_.Exception.Message)); restored previous file"
    }
    if ($backup) { Write-Verbose "backup: $backup" }
    if ($Uninstall) { return 'removed' } else { return 'written' }
}

# --- main -------------------------------------------------------------------

$selected = @()
if ($Clients) {
    foreach ($c in $Clients) {
        foreach ($part in ($c -split '[,;\s]+')) {
            if (-not $part) { continue }
            $match = $AllClients | Where-Object { $_ -ieq $part }
            if (-not $match) {
                Write-Summary "error: unknown client '$part' (expected ClaudeDesktop, Codex, GrokCli)"
                Save-Log
                exit 1
            }
            if ($selected -notcontains $match) { $selected += $match }
        }
    }
} else {
    $selected = $AllClients
}

if ($Detect) {
    foreach ($c in $AllClients) {
        $d = Get-Detection $c
        if ($d.Present) { Write-Summary ("{0}: present ({1})" -f $c, ($d.Evidence -join '; ')) }
        else { Write-Summary ("{0}: not detected" -f $c) }
    }
    Save-Log
    exit 0
}

if (-not $Uninstall -and -not (Test-Path -LiteralPath $HandsExe)) {
    Write-Warning "HandsExe does not exist yet: $HandsExe (writing the path anyway)"
}

$changed = $false
$failed = $false
foreach ($c in $selected) {
    $d = Get-Detection $c
    if (-not $d.Present) {
        Write-Summary "${c}: not detected"
        continue
    }
    $targets = @($d.Configs)
    if ($c -eq 'ClaudeDesktop' -and $targets.Count -gt 1) {
        # write the configs that exist; if none exist, create only the first (Squirrel layout first)
        $existingCfg = @($targets | Where-Object { Test-Path -LiteralPath $_ })
        if ($existingCfg.Count -gt 0) { $targets = $existingCfg } else { $targets = @($targets[0]) }
    }
    foreach ($t in $targets) {
        try {
            if ($c -eq 'ClaudeDesktop') { $status = Invoke-ClaudeConfig $t }
            else { $status = Invoke-TomlConfig $c $t }
            if ($status -in @('written', 'removed', 'would write', 'would remove')) { $changed = $true }
            Write-Summary "${c}: $status ($t)"
        } catch {
            $failed = $true
            Write-Summary "${c}: error ($t): $($_.Exception.Message)"
        }
    }
}

if (-not $Uninstall -and $changed -and -not $DryRun -and $selected -contains 'ClaudeDesktop') {
    Write-Summary 'Note: fully quit Claude desktop (tray icon > Quit) and start it again to load hands. Codex and Grok CLI pick it up on their next launch.'
}

Save-Log
if ($failed) { exit 1 }
if ($changed) { exit 0 }
exit 2
