[CmdletBinding()]
param(
    [string] $ExePath = '',
    [ValidateRange(1, 64)] [int] $MinimumMonitorCount = 4,
    [ValidateRange(2, 32)] [int] $ConcurrentCaptures = 4,
    [ValidateRange(2, 32)] [int] $SameProcessCaptures = 6,
    [switch] $KeepCaptures
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot '..\dist\hand.exe'
}
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path

$resultDirectory = Join-Path $PSScriptRoot '..\test-results'
New-Item -ItemType Directory -Force -Path $resultDirectory | Out-Null
$runId = '{0}_{1}' -f (Get-Date -Format 'yyyyMMdd_HHmmss_fff'), $PID
$reportPath = Join-Path $resultDirectory "monitor-smoke-$runId.json"
$latestPath = Join-Path $resultDirectory 'monitor-smoke-latest.json'
$scriptHash = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash
$exeHash = (Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash

$handles = [System.Collections.Generic.List[object]]::new()
$generatedPaths = [System.Collections.Generic.List[string]]::new()
$monitors = @()
$fixedResults = @()
$sameProcessResults = @()
$parallelResults = @()
$failure = $null
$status = 'FAIL'
$fullFourMonitorAcceptance = $false
$tests = [ordered]@{
    monitor_inventory_unique = $false
    locked_without_scope_failed_closed = $false
    fixed_stable_id_locked_each_monitor = $false
    mismatched_monitor_failed_closed = $false
    resolved_alias_browser_binding_enforced = $false
    qr_browser_binding_enforced = $false
    native_plugins_blocked_under_scope = $false
    rejected_uia_batch_aggregated_failure = $false
    same_process_paths_unique = $false
    same_second_paths_unique = $false
    cross_process_paths_unique = $false
    locked_scope_mutation_blocked = $false
    png_decode_and_dimensions_valid = $false
}

Add-Type -AssemblyName System.Drawing

function Start-HandProcess {
    param([hashtable] $Environment = @{})

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $ExePath
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.EnvironmentVariables['HANDS_TOOL_PROFILE'] = 'default'
    foreach ($name in @('HANDS_MONITOR_SCOPE', 'HANDS_MONITOR_SCOPE_LOCKED', 'HANDS_MONITOR_BROWSER_TITLE')) {
        [void] $startInfo.EnvironmentVariables.Remove($name)
    }
    foreach ($entry in $Environment.GetEnumerator()) {
        $startInfo.EnvironmentVariables[$entry.Key] = [string] $entry.Value
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        throw "Could not start $ExePath"
    }
    $handle = [pscustomobject]@{
        Process = $process
        Stdout = $process.StandardOutput.ReadToEndAsync()
        Stderr = $process.StandardError.ReadToEndAsync()
        Completed = $false
    }
    [void] $handles.Add($handle)
    $handle
}

function Complete-HandProcess {
    param(
        [Parameter(Mandatory)] $Handle,
        [int] $TimeoutMs = 120000
    )

    $Handle.Process.StandardInput.Close()
    if (-not $Handle.Process.WaitForExit($TimeoutMs)) {
        $Handle.Process.Kill()
        [void] $Handle.Process.WaitForExit(5000)
        throw 'Timed out waiting for Hands monitor smoke process'
    }
    $stdout = $Handle.Stdout.GetAwaiter().GetResult()
    $stderr = $Handle.Stderr.GetAwaiter().GetResult()
    $Handle.Completed = $true
    if ($Handle.Process.ExitCode -ne 0) {
        throw "Hands exited $($Handle.Process.ExitCode): $stderr"
    }

    $responses = @()
    foreach ($line in @($stdout -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })) {
        try {
            $responses += $line | ConvertFrom-Json
        } catch {
            throw "Hands emitted non-JSON stdout: $line"
        }
    }
    $responses
}

function Send-McpRequests {
    param(
        [Parameter(Mandatory)] $Handle,
        [Parameter(Mandatory)] [object[]] $Requests
    )

    $initialize = [ordered]@{
        jsonrpc = '2.0'
        id = 9000
        method = 'initialize'
        params = [ordered]@{
            protocolVersion = '2024-11-05'
            capabilities = @{}
            clientInfo = [ordered]@{ name = 'hands-monitor-smoke'; version = '1.0.0' }
        }
    }
    $initialized = [ordered]@{
        jsonrpc = '2.0'
        method = 'notifications/initialized'
        params = @{}
    }
    foreach ($request in @($initialize, $initialized) + @($Requests)) {
        $Handle.Process.StandardInput.WriteLine(($request | ConvertTo-Json -Depth 30 -Compress))
    }
}

function Tool-Request {
    param([int] $Id, [string] $Name, [hashtable] $Arguments = @{})
    [ordered]@{
        jsonrpc = '2.0'
        id = $Id
        method = 'tools/call'
        params = [ordered]@{ name = $Name; arguments = $Arguments }
    }
}

function Response-ById {
    param([object[]] $Responses, [int] $Id)
    $matches = @($Responses | Where-Object { $_.id -eq $Id })
    if ($matches.Count -ne 1) {
        throw "Expected one MCP response for id $Id; found $($matches.Count)"
    }
    $errorProperty = $matches[0].PSObject.Properties['error']
    if ($null -ne $errorProperty -and $null -ne $errorProperty.Value) {
        throw "MCP protocol error for id ${Id}: $($errorProperty.Value | ConvertTo-Json -Compress)"
    }
    $matches[0]
}

function Tool-Payload {
    param($Response)
    $text = @($Response.result.content | Where-Object { $_.type -eq 'text' } | Select-Object -First 1).text
    if ([string]::IsNullOrWhiteSpace($text)) {
        throw "Missing MCP text payload for response id $($Response.id)"
    }
    $text | ConvertFrom-Json
}

function Payload-HasCode {
    param($Payload, [string] $Code)
    ($Payload | ConvertTo-Json -Depth 30 -Compress) -match ('"code":"{0}"' -f [regex]::Escape($Code))
}

function Capture-Proof {
    param($Payload)
    $path = [string] $Payload.path
    if ([string]::IsNullOrWhiteSpace($path) -or -not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Capture path does not exist: $path"
    }
    [void] $generatedPaths.Add([System.IO.Path]::GetFullPath($path))
    $item = Get-Item -LiteralPath $path
    if ($item.Length -le 0) {
        throw "Capture is empty: $path"
    }

    $image = [System.Drawing.Image]::FromFile($item.FullName)
    try {
        $width = [int] $image.Width
        $height = [int] $image.Height
        if ($width -le 0 -or $height -le 0) {
            throw "Capture has invalid decoded dimensions: $path"
        }
        $payloadWidth = $Payload.PSObject.Properties['width']
        $payloadHeight = $Payload.PSObject.Properties['height']
        if ($null -ne $payloadWidth -and [int] $payloadWidth.Value -ne $width) {
            throw "Capture width mismatch for ${path}: payload=$($payloadWidth.Value), PNG=$width"
        }
        if ($null -ne $payloadHeight -and [int] $payloadHeight.Value -ne $height) {
            throw "Capture height mismatch for ${path}: payload=$($payloadHeight.Value), PNG=$height"
        }
    } finally {
        $image.Dispose()
    }

    $secondBucket = $null
    if ($item.Name -match '^screenshot_(\d{8}_\d{6})_') {
        $secondBucket = $Matches[1]
    }
    [pscustomobject]@{
        path = $item.FullName
        bytes = $item.Length
        width = $width
        height = $height
        sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash
        second_bucket = $secondBucket
    }
}

function Test-DescendantPath {
    param([string] $Root, [string] $Candidate)
    $separator = [System.IO.Path]::DirectorySeparatorChar
    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/') + $separator
    $candidateFull = [System.IO.Path]::GetFullPath($Candidate)
    $candidateFull.StartsWith($rootFull, [System.StringComparison]::OrdinalIgnoreCase)
}

try {
    $listHandle = Start-HandProcess
    Send-McpRequests $listHandle @(
        (Tool-Request 1 'hands_monitor_scope' @{ action = 'list' })
    )
    $listResponses = Complete-HandProcess $listHandle
    $listPayload = Tool-Payload (Response-ById $listResponses 1)
    $monitors = @($listPayload.monitors)
    if ($monitors.Count -lt $MinimumMonitorCount) {
        throw "Found $($monitors.Count) monitors, expected at least $MinimumMonitorCount"
    }
    if (@($monitors.stable_id | Sort-Object -Unique).Count -ne $monitors.Count) {
        throw 'Monitor stable IDs are not unique'
    }
    if (@($monitors | Where-Object { -not $_.stable_physical }).Count -ne 0) {
        throw 'At least one monitor lacks a physical stable identity required for fixed mode'
    }
    $tests.monitor_inventory_unique = $true

    $lockedOffHandle = Start-HandProcess @{ HANDS_MONITOR_SCOPE_LOCKED = '1' }
    Send-McpRequests $lockedOffHandle @(
        (Tool-Request 20 'hands_monitor_scope' @{ action = 'get' }),
        (Tool-Request 21 'vision_screenshot' @{})
    )
    $lockedOffResponses = Complete-HandProcess $lockedOffHandle
    $lockedOffGet = Tool-Payload (Response-ById $lockedOffResponses 20)
    $lockedOffCapture = Tool-Payload (Response-ById $lockedOffResponses 21)
    if ($lockedOffGet.success -ne $false -or
        $lockedOffCapture.success -ne $false -or
        -not (Payload-HasCode $lockedOffGet 'configuration_error') -or
        -not (Payload-HasCode $lockedOffCapture 'configuration_error')) {
        throw 'Locked-without-scope startup did not fail closed'
    }
    $tests.locked_without_scope_failed_closed = $true

    $fixedResults = @(
        foreach ($monitor in $monitors) {
            $other = @($monitors | Where-Object { $_.index -ne $monitor.index } | Select-Object -First 1)
            $mismatchIndex = if ($other.Count -gt 0) {
                [int] $other[0].index
            } else {
                [int] $monitor.index + 10000
            }
            $handle = Start-HandProcess @{
                HANDS_MONITOR_SCOPE = "stable:$($monitor.stable_id)"
                HANDS_MONITOR_SCOPE_LOCKED = '1'
            }
            Send-McpRequests $handle @(
                (Tool-Request 30 'hands_monitor_scope' @{ action = 'get' }),
                (Tool-Request 31 'vision_screenshot' @{}),
                (Tool-Request 32 'vision_screenshot' @{ monitor = $mismatchIndex }),
                (Tool-Request 33 'hands_monitor_scope' @{ action = 'set'; mode = 'primary' }),
                (Tool-Request 34 'hands_monitor_scope' @{ action = 'clear' })
            )
            $responses = Complete-HandProcess $handle
            $scope = Tool-Payload (Response-ById $responses 30)
            $capture = Tool-Payload (Response-ById $responses 31)
            $mismatch = Tool-Payload (Response-ById $responses 32)
            $setAttempt = Tool-Payload (Response-ById $responses 33)
            $clearAttempt = Tool-Payload (Response-ById $responses 34)
            if (-not $scope.success -or -not $capture.success) {
                throw "Fixed locked monitor test failed for monitor $($monitor.index)"
            }
            if ([string] $scope.scope.resolved_monitor.stable_id -ne [string] $monitor.stable_id -or
                [string] $capture.stable_id -ne [string] $monitor.stable_id -or
                [int] $capture.monitor -ne [int] $monitor.index) {
                throw "Fixed stable identity mismatch for monitor $($monitor.index)"
            }
            if ($mismatch.success -ne $false -or -not (Payload-HasCode $mismatch 'monitor_mismatch')) {
                throw "Monitor mismatch did not fail closed for monitor $($monitor.index)"
            }
            if ($setAttempt.success -ne $false -or $clearAttempt.success -ne $false -or
                -not (Payload-HasCode $setAttempt 'scope_locked') -or
                -not (Payload-HasCode $clearAttempt 'scope_locked')) {
                throw "Locked fixed scope allowed mutation for monitor $($monitor.index)"
            }
            [pscustomobject]@{
                index = [int] $monitor.index
                display_id = [uint32] $monitor.display_id
                stable_id = [string] $monitor.stable_id
                bounds = $monitor.logical_bounds
                scale_factor = [double] $monitor.scale_factor
                mismatch_index = $mismatchIndex
                capture = Capture-Proof $capture
            }
        }
    )
    $tests.fixed_stable_id_locked_each_monitor = $true
    $tests.mismatched_monitor_failed_closed = $true
    $tests.locked_scope_mutation_blocked = $true

    $policyHandle = Start-HandProcess @{
        HANDS_MONITOR_SCOPE = 'primary'
        HANDS_MONITOR_SCOPE_LOCKED = '1'
    }
    Send-McpRequests $policyHandle @(
        (Tool-Request 40 'hands_script' @{ steps = @(@{ tool = 'navigate'; args = @{ url = 'about:blank' } }) }),
        (Tool-Request 41 'hands_scan_qr' @{ source = 'browser' }),
        (Tool-Request 42 'hands_plugin_load' @{ path = 'C:\definitely-not-a-plugin.dll' }),
        (Tool-Request 43 'hands_plugin_call' @{ plugin = 'missing'; tool = 'missing'; arguments = @{} }),
        (Tool-Request 44 'uia_batch' @{ actions = @(@{ type = 'read_value'; params = @{ name = 'anything' } }) })
    )
    $policyResponses = Complete-HandProcess $policyHandle
    $scriptAlias = Tool-Payload (Response-ById $policyResponses 40)
    $qrBrowser = Tool-Payload (Response-ById $policyResponses 41)
    $pluginLoad = Tool-Payload (Response-ById $policyResponses 42)
    $pluginCall = Tool-Payload (Response-ById $policyResponses 43)
    $uiaBatch = Tool-Payload (Response-ById $policyResponses 44)
    if ($scriptAlias.success -ne $false -or -not (Payload-HasCode $scriptAlias 'browser_unbound')) {
        throw 'hands_script resolved browser alias bypassed browser binding'
    }
    if ($qrBrowser.success -ne $false -or -not (Payload-HasCode $qrBrowser 'browser_unbound')) {
        throw 'hands_scan_qr browser mode bypassed browser binding'
    }
    if ($pluginLoad.success -ne $false -or $pluginCall.success -ne $false -or
        -not (Payload-HasCode $pluginLoad 'unscoped_plugin_ability') -or
        -not (Payload-HasCode $pluginCall 'unscoped_plugin_ability')) {
        throw 'Native plugin execution was not blocked under strict scope'
    }
    if ($uiaBatch.success -ne $false -or
        [int] $uiaBatch.failures -lt 1 -or
        -not (Payload-HasCode $uiaBatch 'unscopable_global_ability')) {
        throw 'Rejected UIA batch action did not propagate aggregate failure'
    }
    $tests.resolved_alias_browser_binding_enforced = $true
    $tests.qr_browser_binding_enforced = $true
    $tests.native_plugins_blocked_under_scope = $true
    $tests.rejected_uia_batch_aggregated_failure = $true

    $burstHandle = Start-HandProcess @{
        HANDS_MONITOR_SCOPE = 'primary'
        HANDS_MONITOR_SCOPE_LOCKED = '1'
    }
    $burstRequests = @(
        for ($i = 0; $i -lt $SameProcessCaptures; $i++) {
            Tool-Request (100 + $i) 'vision_screenshot' @{}
        }
    )
    Send-McpRequests $burstHandle $burstRequests
    $burstResponses = Complete-HandProcess $burstHandle
    $sameProcessResults = @(
        for ($i = 0; $i -lt $SameProcessCaptures; $i++) {
            $payload = Tool-Payload (Response-ById $burstResponses (100 + $i))
            if (-not $payload.success) {
                throw "Same-process automatic screenshot $i failed"
            }
            Capture-Proof $payload
        }
    )
    if (@($sameProcessResults.path | Sort-Object -Unique).Count -ne $sameProcessResults.Count) {
        throw 'Automatic screenshot filenames collided inside one Hands process'
    }
    $sameSecondGroups = @($sameProcessResults | Where-Object { $_.second_bucket } | Group-Object second_bucket)
    if (@($sameSecondGroups | Where-Object { $_.Count -ge 2 }).Count -eq 0) {
        throw 'Same-process burst produced no two captures in the same wall-clock second'
    }
    $tests.same_process_paths_unique = $true
    $tests.same_second_paths_unique = $true

    $parallel = @()
    for ($i = 0; $i -lt $ConcurrentCaptures; $i++) {
        $handle = Start-HandProcess @{
            HANDS_MONITOR_SCOPE = 'primary'
            HANDS_MONITOR_SCOPE_LOCKED = '1'
        }
        Send-McpRequests $handle @(
            (Tool-Request 200 'vision_screenshot' @{}),
            (Tool-Request 201 'hands_monitor_scope' @{ action = 'clear' })
        )
        $parallel += $handle
    }
    $parallelResults = @(
        foreach ($handle in $parallel) {
            $responses = Complete-HandProcess $handle
            $capture = Tool-Payload (Response-ById $responses 200)
            $clear = Tool-Payload (Response-ById $responses 201)
            if (-not $capture.success) {
                throw 'Concurrent automatic screenshot failed'
            }
            if ($clear.success -ne $false -or -not (Payload-HasCode $clear 'scope_locked')) {
                throw 'Policy-locked monitor scope allowed itself to be cleared'
            }
            Capture-Proof $capture
        }
    )

    $allProofs = @($fixedResults.capture) + @($sameProcessResults) + @($parallelResults)
    if (@($allProofs.path | Sort-Object -Unique).Count -ne $allProofs.Count) {
        throw 'Automatic screenshot filenames collided across the full test run'
    }
    $tests.cross_process_paths_unique = $true
    $tests.png_decode_and_dimensions_valid = $true

    $fullFourMonitorAcceptance = $MinimumMonitorCount -ge 4 -and $monitors.Count -ge $MinimumMonitorCount
    $status = if ($fullFourMonitorAcceptance) { 'PASS' } else { 'PASS_DIAGNOSTIC' }
} catch {
    $failure = $_ | Out-String
    $status = 'FAIL'
} finally {
    foreach ($handle in $handles) {
        try {
            if (-not $handle.Process.HasExited) {
                $handle.Process.Kill()
                [void] $handle.Process.WaitForExit(5000)
            }
            $handle.Process.Dispose()
        } catch {
            # Preserve the original test failure; process cleanup is best effort.
        }
    }

    $report = [ordered]@{
        run_id = $runId
        timestamp = (Get-Date).ToString('o')
        status = $status
        error = $failure
        script = $PSCommandPath
        script_sha256 = $scriptHash
        executable = $ExePath
        executable_sha256 = $exeHash
        invocation = [ordered]@{
            minimum_monitor_count = $MinimumMonitorCount
            concurrent_captures = $ConcurrentCaptures
            same_process_captures = $SameProcessCaptures
            keep_captures = [bool] $KeepCaptures
        }
        monitor_count = $monitors.Count
        full_four_monitor_acceptance = $fullFourMonitorAcceptance
        test_tier = if ($fullFourMonitorAcceptance) { 'four-monitor-acceptance' } else { 'diagnostic-only' }
        monitors = $monitors
        fixed_monitor_results = @($fixedResults)
        same_process_results = @($sameProcessResults)
        concurrent_results = @($parallelResults)
        tests = $tests
    }
    $report | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $reportPath -Encoding utf8
    Copy-Item -LiteralPath $reportPath -Destination $latestPath -Force

    if (-not $KeepCaptures) {
        $allowedRoots = @(
            (Join-Path $env:USERPROFILE 'Pictures\Screenshots'),
            (Join-Path ([Environment]::GetFolderPath('MyPictures')) 'Screenshots')
        ) | Sort-Object -Unique
        foreach ($path in @($generatedPaths | Sort-Object -Unique)) {
            $name = [System.IO.Path]::GetFileName($path)
            $allowed = @($allowedRoots | Where-Object { Test-DescendantPath $_ $path }).Count -gt 0
            if ($allowed -and
                $name.StartsWith('screenshot_', [StringComparison]::OrdinalIgnoreCase) -and
                [System.IO.Path]::GetExtension($name).Equals('.png', [StringComparison]::OrdinalIgnoreCase)) {
                Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
            }
        }
    }
}

if ($failure) {
    throw $failure
}

[pscustomobject]@{
    Status = $status
    TestTier = if ($fullFourMonitorAcceptance) { 'four-monitor-acceptance' } else { 'diagnostic-only' }
    Monitors = $monitors.Count
    FixedCaptures = @($fixedResults).Count
    SameProcessCaptures = @($sameProcessResults).Count
    ConcurrentCaptures = @($parallelResults).Count
    Report = (Resolve-Path -LiteralPath $reportPath).Path
}
