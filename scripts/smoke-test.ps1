[CmdletBinding()]
param(
    [string] $ExePath = ''
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot '..\dist\hand.exe'
}
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$manifestPath = Join-Path $PSScriptRoot '..\manifest\unified_hands_manifest.json'
$manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json

if ($manifest.checks.exact_cover -ne $true) {
    throw 'Manifest does not report an exact, duplicate-free ability cover'
}

$manifestNames = @($manifest.tools | ForEach-Object name)
$uniqueManifestNames = @($manifestNames | Sort-Object -Unique)
if ($manifestNames.Count -ne $uniqueManifestNames.Count) {
    throw 'Manifest contains duplicate tool rows'
}
if ($manifestNames.Count -ne [int]$manifest.counts.raw_union_unique) {
    throw 'Manifest raw count does not match its tool rows'
}

$collectionNames = @(
    $manifest.collections |
        ForEach-Object { $_.tools } |
        ForEach-Object { $_ }
)
if ($collectionNames.Count -ne @($collectionNames | Sort-Object -Unique).Count) {
    throw 'Manifest collections assign at least one tool more than once'
}
$collectionDiff = @(Compare-Object ($manifestNames | Sort-Object) ($collectionNames | Sort-Object))
if ($collectionDiff.Count -ne 0) {
    throw "Manifest collections are not an exact tool cover: $($collectionDiff | Out-String)"
}

$removedWorkflowFrontDoors = @(
    'hands_self_record_lookup',
    'hands_self_record_start',
    'hands_self_record_stop_and_optimize'
)
if (@($manifestNames | Where-Object { $_ -in $removedWorkflowFrontDoors }).Count -ne 0) {
    throw 'Manifest still exposes a Workflow-owned self-record front door'
}
$monitorRow = @($manifest.tools | Where-Object name -eq 'hands_monitor_scope')
if ($monitorRow.Count -ne 1 -or $monitorRow[0].collection -ne 'monitor-scope-and-topology') {
    throw 'Manifest must assign hands_monitor_scope exactly once to monitor-scope-and-topology'
}

function Invoke-HandRpc {
    param(
        [Parameter(Mandatory)]
        [string] $Profile,

        [Parameter(Mandatory)]
        [object[]] $Requests
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $ExePath
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.EnvironmentVariables['HANDS_TOOL_PROFILE'] = $Profile

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        throw "Could not start $ExePath"
    }

    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()

    foreach ($request in $Requests) {
        $process.StandardInput.WriteLine(($request | ConvertTo-Json -Depth 20 -Compress))
    }
    $process.StandardInput.Close()

    if (-not $process.WaitForExit(60000)) {
        $process.Kill()
        throw "Timed out waiting for profile '$Profile'"
    }

    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    if ($process.ExitCode -ne 0) {
        throw "Profile '$Profile' exited $($process.ExitCode): $stderr"
    }

    $responses = @(
        $stdout -split "`r?`n" |
            Where-Object { $_.Trim().StartsWith('{') } |
            ForEach-Object { $_ | ConvertFrom-Json }
    )

    [pscustomobject]@{
        Profile = $Profile
        Responses = $responses
        Stderr = $stderr
    }
}

$expected = [ordered]@{
    default = [int]$manifest.counts.default_tools_list
    full = [int]$manifest.counts.full_tools_list
    strict = [int]$manifest.counts.strict_tools_list
    compatibility = [int]$manifest.counts.compatibility_tools_list
}

$results = foreach ($profile in $expected.Keys) {
    $requests = @(
        [ordered]@{ jsonrpc='2.0'; id=1; method='initialize'; params=[ordered]@{} },
        [ordered]@{ jsonrpc='2.0'; id=2; method='tools/list'; params=[ordered]@{} },
        [ordered]@{
            jsonrpc='2.0'
            id=3
            method='tools/call'
            params=[ordered]@{
                name='hands_capability_catalog'
                arguments=[ordered]@{ include_tools=$false }
            }
        }
    )

    $hiddenDefault = @()
    if ($profile -eq 'default') {
        $requests += [ordered]@{
            jsonrpc='2.0'
            id=4
            method='tools/call'
            params=[ordered]@{ name='hands_health'; arguments=[ordered]@{} }
        }
        $requests += [ordered]@{
            jsonrpc='2.0'
            id=5
            method='tools/call'
            params=[ordered]@{
                name='vision_ocr'
                arguments=[ordered]@{ image_path='C:\this-path-must-not-exist\unified-hands-ocr.png' }
            }
        }

        $hiddenDefault = @(
            $manifest.tools |
                Where-Object { -not $_.exposed_default } |
                ForEach-Object name
        ) + @('hands_status', 'browser_accessibility_snapshot')

        $requestId = 100
        foreach ($hiddenName in $hiddenDefault) {
            $requests += [ordered]@{
                jsonrpc='2.0'
                id=$requestId
                method='tools/call'
                params=[ordered]@{ name=$hiddenName; arguments=[ordered]@{} }
            }
            $requestId++
        }
    }

    $run = Invoke-HandRpc -Profile $profile -Requests $requests
    $list = @($run.Responses | Where-Object id -eq 2)[0]
    $catalogResponse = @($run.Responses | Where-Object id -eq 3)[0]
    $definitions = @($list.result.tools)
    $names = @($definitions | ForEach-Object name)
    $uniqueCount = @($names | Sort-Object -Unique).Count
    $catalogText = $catalogResponse.result.content[0].text | ConvertFrom-Json
    $profileField = switch ($profile) {
        'default' { 'exposed_default' }
        'full' { 'exposed_full' }
        'strict' { 'exposed_strict' }
        'compatibility' { $null }
    }
    $expectedNames = @('hands_capability_catalog') + @(
        $manifest.tools |
            Where-Object { $null -eq $profileField -or $_.$profileField } |
            ForEach-Object name
    )

    if ($definitions.Count -ne $expected[$profile]) {
        throw "Profile '$profile' listed $($definitions.Count), expected $($expected[$profile])"
    }
    if ($uniqueCount -ne $definitions.Count) {
        throw "Profile '$profile' contains duplicate tool names"
    }
    $nameDiff = @(Compare-Object ($expectedNames | Sort-Object) ($names | Sort-Object))
    if ($nameDiff.Count -ne 0) {
        throw "Profile '$profile' differs from the manifest: $($nameDiff | Out-String)"
    }
    if ($catalogText.active_profile -ne $profile) {
        throw "Catalog reported profile '$($catalogText.active_profile)', expected '$profile'"
    }
    if ($catalogText.listed_tool_count -ne $definitions.Count) {
        throw "Catalog/list count mismatch for '$profile'"
    }
    if (@($definitions | Where-Object { -not $_.'x-hands-collection' }).Count -ne 0) {
        throw "Profile '$profile' contains an ungrouped tool definition"
    }

    if ($profile -eq 'default') {
        $initialize = @($run.Responses | Where-Object id -eq 1)[0]
        $healthResponse = @($run.Responses | Where-Object id -eq 4)[0]
        $health = $healthResponse.result.content[0].text | ConvertFrom-Json
        if ($health.version -ne $initialize.result.serverInfo.version -or $health.version -ne $catalogText.version) {
            throw 'Initialize, catalog, and health versions do not agree'
        }
        if ($health.tool_count -ne $definitions.Count -or $health.tool_surface.listed_tool_count -ne $definitions.Count) {
            throw 'Canonical health did not absorb the advertised tool counts'
        }
        if (@($health.tool_surface.collections).Count -ne @($manifest.collections).Count) {
            throw 'Canonical health did not absorb the purpose collections'
        }
        if (-not $health.categories -or -not $health.subsystems) {
            throw 'Canonical health did not absorb legacy status categories and subsystem summary fields'
        }

        $ocrResponse = @($run.Responses | Where-Object id -eq 5)[0]
        $ocrResult = $ocrResponse.result.content[0].text | ConvertFrom-Json
        if ($ocrResult.backend_metadata.cache_enabled -ne $true -or -not $ocrResult.backend_metadata.backend) {
            throw 'Canonical vision_ocr did not return absorbed cache/backend metadata'
        }

        $requestId = 100
        foreach ($hiddenName in $hiddenDefault) {
            $blockedResponse = @($run.Responses | Where-Object id -eq $requestId)[0]
            $blockedText = $blockedResponse.result.content[0].text | ConvertFrom-Json
            if ($blockedText.success -ne $false -or -not $blockedText.replacement) {
                throw "Default profile did not block hidden route '$hiddenName' with a replacement"
            }
            $requestId++
        }

        $snapshotDefinition = @($definitions | Where-Object name -eq 'browser_a11y_snapshot')[0]
        $snapshotProperties = @($snapshotDefinition.inputSchema.properties.PSObject.Properties.Name)
        foreach ($requiredProperty in @('root_selector','include_ignored','max_depth','max_nodes','incremental')) {
            if ($requiredProperty -notin $snapshotProperties) {
                throw "United a11y snapshot schema is missing '$requiredProperty'"
            }
        }

        $findDefinition = @($definitions | Where-Object name -eq 'browser_a11y_find')[0]
        $findProperties = @($findDefinition.inputSchema.properties.PSObject.Properties.Name)
        foreach ($requiredProperty in @('role','name','exact','refresh','max_nodes')) {
            if ($requiredProperty -notin $findProperties) {
                throw "United a11y find schema is missing '$requiredProperty'"
            }
        }

        $ocrDefinition = @($definitions | Where-Object name -eq 'vision_ocr')[0]
        if ($ocrDefinition.description -notmatch 'automatic result caching' -or $ocrDefinition.description -notmatch 'backend metadata') {
            throw 'Canonical vision_ocr does not advertise the absorbed cache/backend contract'
        }

        $attachDefinition = @($definitions | Where-Object name -eq 'browser_attach')[0]
        if ($attachDefinition.description -notmatch 'locking is acquired automatically') {
            throw 'browser_attach does not advertise automatic lock ownership'
        }

        $monitorDefinition = @($definitions | Where-Object name -eq 'hands_monitor_scope')[0]
        if (-not $monitorDefinition -or $monitorDefinition.'x-hands-collection' -ne 'monitor-scope-and-topology') {
            throw 'Default profile is missing the central monitor-scope front door or its ability group'
        }
        $monitorProperties = @($monitorDefinition.inputSchema.properties.PSObject.Properties.Name)
        foreach ($requiredProperty in @('action','mode','monitor','display_id','browser_window_title')) {
            if ($requiredProperty -notin $monitorProperties) {
                throw "hands_monitor_scope schema is missing '$requiredProperty'"
            }
        }
    }

    [pscustomobject]@{
        Profile = $profile
        Listed = $definitions.Count
        Unique = $uniqueCount
        Grouped = $true
        CatalogCountMatches = $true
        HiddenRoutesBlocked = if ($profile -eq 'default') { $hiddenDefault.Count } else { $null }
        AbsorbedContracts = if ($profile -eq 'default') { $true } else { $null }
    }
}

$results | Format-Table -AutoSize
