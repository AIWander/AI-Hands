[CmdletBinding()]
param(
    [string] $ExePath = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot '..\dist\hand.exe'
}
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$sentinel = 'PAGE_SECRET_SENTINEL_847291'
$html = '<html><body><h1 id="ok">Browser security smoke</h1></body></html>'
$url = 'data:text/html,' + [Uri]::EscapeDataString($html)
$javascript = @'
(() => {
  const hostile = [{
    name: 'https://safe.example/api?token=__SENTINEL__',
    initiatorType: 'fetch',
    duration: 12.34,
    transferSize: 44,
    startTime: 1.25,
    headers: {Authorization: 'Bearer __SENTINEL__'},
    body: '__SENTINEL__',
    extra: '__SENTINEL__'
  }];
  Object.defineProperty(window.performance, 'getEntriesByType', {
    configurable: true,
    value: (kind) => kind === 'resource' ? hostile : []
  });
  Object.defineProperty(window.performance, 'clearResourceTimings', {
    configurable: true,
    value: () => {}
  });
  return 'patched';
})()
'@.Replace('__SENTINEL__', $sentinel)

function Tool-Request {
    param([int] $Id, [string] $Name, [hashtable] $Arguments = @{})
    [ordered]@{
        jsonrpc = '2.0'
        id = $Id
        method = 'tools/call'
        params = [ordered]@{ name = $Name; arguments = $Arguments }
    }
}

function Tool-Payload {
    param([object[]] $Responses, [int] $Id)
    $matches = @($Responses | Where-Object id -eq $Id)
    if ($matches.Count -ne 1) {
        throw "Expected one response for id $Id; found $($matches.Count)"
    }
    if ($null -ne $matches[0].PSObject.Properties['error']) {
        throw "MCP protocol error for id ${Id}: $($matches[0].error | ConvertTo-Json -Compress)"
    }
    $text = @(
        $matches[0].result.content |
            Where-Object type -eq 'text' |
            Select-Object -First 1
    ).text
    if ([string]::IsNullOrWhiteSpace($text)) {
        throw "Missing MCP text payload for response id $Id"
    }
    $text | ConvertFrom-Json
}

function Test-PayloadFailed {
    param($Payload)
    $errorProperty = $Payload.PSObject.Properties['error']
    if ($null -ne $errorProperty -and -not [string]::IsNullOrWhiteSpace([string] $errorProperty.Value)) {
        return $true
    }
    $successProperty = $Payload.PSObject.Properties['success']
    $null -ne $successProperty -and $successProperty.Value -eq $false
}

$startInfo = [System.Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $ExePath
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.RedirectStandardInput = $true
$startInfo.RedirectStandardOutput = $true
$startInfo.RedirectStandardError = $true
$startInfo.EnvironmentVariables['HANDS_TOOL_PROFILE'] = 'compatibility'
$startInfo.EnvironmentVariables['HANDS_ALLOW_UNSAFE_RAW_TOOLS'] = '1'
$startInfo.EnvironmentVariables['HANDS_ALLOW_UNSAFE_DIRECT_FETCH'] = '1'

$process = [System.Diagnostics.Process]::new()
$process.StartInfo = $startInfo
try {
    if (-not $process.Start()) {
        throw "Could not start $ExePath"
    }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $requests = @(
        [ordered]@{
            jsonrpc = '2.0'
            id = 9000
            method = 'initialize'
            params = [ordered]@{
                protocolVersion = '2024-11-05'
                capabilities = @{}
                clientInfo = [ordered]@{ name = 'browser-security-smoke'; version = '1.0.0' }
            }
        },
        [ordered]@{ jsonrpc = '2.0'; method = 'notifications/initialized'; params = @{} },
        (Tool-Request 1 'browser_launch' @{ headless = $true }),
        (Tool-Request 2 'browser_navigate' @{ url = $url }),
        (Tool-Request 3 'browser_eval' @{
            script = $javascript
            allow_unsafe_fetch = $true
            allow_unsafe_raw = $true
        }),
        (Tool-Request 4 'browser_get_performance_log' @{ max_entries = 10 }),
        (Tool-Request 5 'browser_close' @{})
    )
    foreach ($request in $requests) {
        $process.StandardInput.WriteLine(($request | ConvertTo-Json -Depth 30 -Compress))
    }
    $process.StandardInput.Close()

    if (-not $process.WaitForExit(60000)) {
        $process.Kill()
        throw 'Browser security smoke timed out'
    }
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    if ($process.ExitCode -ne 0) {
        throw "hand.exe exited $($process.ExitCode): $stderr"
    }

    $responses = @(
        $stdout -split "`r?`n" |
            Where-Object { $_.Trim().StartsWith('{') } |
            ForEach-Object { $_ | ConvertFrom-Json }
    )
    $launch = Tool-Payload $responses 1
    $navigate = Tool-Payload $responses 2
    $evaluate = Tool-Payload $responses 3
    $performance = Tool-Payload $responses 4
    $close = Tool-Payload $responses 5
    $performanceJson = $performance | ConvertTo-Json -Depth 20 -Compress

    if ((Test-PayloadFailed $launch) -or
        (Test-PayloadFailed $navigate) -or
        (Test-PayloadFailed $close)) {
        throw 'Launch, navigate, or close did not report success'
    }
    if (Test-PayloadFailed $evaluate) {
        throw "Browser eval did not execute: $($evaluate | ConvertTo-Json -Depth 10 -Compress)"
    }
    if ([int] $performance.count -ne 1 -or @($performance.entries).Count -ne 1) {
        throw "Expected one projected performance row: $performanceJson"
    }
    if ($performanceJson.Contains($sentinel)) {
        throw 'Performance projection leaked the hostile sentinel'
    }
    foreach ($forbidden in @('headers', 'body', 'extra', 'Authorization')) {
        if ($performanceJson -match ('"' + [regex]::Escape($forbidden) + '"')) {
            throw "Performance projection retained forbidden field '$forbidden'"
        }
    }

    [pscustomobject]@{
        Status = 'PASS'
        ExecutableSha256 = (Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash
        Launch = $true
        Navigate = $true
        EvalExecuted = $true
        ProjectedRows = [int] $performance.count
        SentinelAbsent = $true
        ForbiddenFieldsAbsent = $true
        Close = $true
    }
}
finally {
    if ($null -ne $process -and -not $process.HasExited) {
        $process.Kill()
        [void] $process.WaitForExit(5000)
    }
    $process.Dispose()
}
