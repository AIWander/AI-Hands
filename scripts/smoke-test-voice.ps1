[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ExePath
)

$ErrorActionPreference = 'Stop'
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path

$startInfo = [System.Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $ExePath
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.RedirectStandardInput = $true
$startInfo.RedirectStandardOutput = $true
$startInfo.RedirectStandardError = $true

$process = [System.Diagnostics.Process]::new()
$process.StartInfo = $startInfo
if (-not $process.Start()) {
    throw "Could not start $ExePath"
}

$stdoutTask = $process.StandardOutput.ReadToEndAsync()
$stderrTask = $process.StandardError.ReadToEndAsync()
$requests = @(
    [ordered]@{ jsonrpc = '2.0'; id = 1; method = 'initialize'; params = [ordered]@{} },
    [ordered]@{ jsonrpc = '2.0'; id = 2; method = 'tools/list'; params = [ordered]@{} }
)
foreach ($request in $requests) {
    $process.StandardInput.WriteLine(($request | ConvertTo-Json -Depth 10 -Compress))
}
$process.StandardInput.Close()

if (-not $process.WaitForExit(30000)) {
    $process.Kill()
    throw 'Voice MCP smoke test timed out.'
}
$stdout = $stdoutTask.GetAwaiter().GetResult()
$stderr = $stderrTask.GetAwaiter().GetResult()
if ($process.ExitCode -ne 0) {
    throw "Voice MCP exited $($process.ExitCode): $stderr"
}

$responses = @(
    $stdout -split "`r?`n" |
        Where-Object { $_.Trim().StartsWith('{') } |
        ForEach-Object { $_ | ConvertFrom-Json }
)
$initialize = @($responses | Where-Object id -eq 1)[0]
$list = @($responses | Where-Object id -eq 2)[0]
if (-not $initialize.result.serverInfo) {
    throw 'Voice MCP initialize response is missing serverInfo.'
}
$names = @($list.result.tools | ForEach-Object name)
$expected = @(
    'speak',
    'listen_for_speech',
    'start_voice_mode',
    'voice_checkpoint',
    'voice_load_checkpoint',
    'voice_get_transcript',
    'voice_add_note',
    'list_voices',
    'playback_control',
    'get_config'
)
if ($names.Count -ne 10 -or @($names | Sort-Object -Unique).Count -ne 10) {
    throw "Voice MCP exposed $($names.Count) tools; expected ten unique tools."
}
$difference = @(Compare-Object ($expected | Sort-Object) ($names | Sort-Object))
if ($difference.Count -ne 0) {
    throw "Voice MCP tool contract drifted: $($difference | Out-String)"
}

[pscustomobject]@{
    Executable = $ExePath
    Server = $initialize.result.serverInfo.name
    Version = $initialize.result.serverInfo.version
    Tools = $names.Count
}
