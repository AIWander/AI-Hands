# Hands MCP Server — Preflight Check
# Run: .\doctor.ps1
# Checks that your system is ready for Hands.

$ErrorActionPreference = "SilentlyContinue"
$pass = 0
$fail = 0

function Check($name, $condition, $hint) {
    if ($condition) {
        Write-Host "  PASS  $name" -ForegroundColor Green
        $script:pass++
    } else {
        Write-Host "  FAIL  $name" -ForegroundColor Red
        if ($hint) { Write-Host "        -> $hint" -ForegroundColor Yellow }
        $script:fail++
    }
}

Write-Host ""
Write-Host "Hands MCP Server — Preflight Check" -ForegroundColor Cyan
Write-Host "===================================" -ForegroundColor Cyan
Write-Host ""

# 1. Binary exists
$binaryPaths = @(
    "$env:LOCALAPPDATA\CPC\servers\hands.exe",
    "C:\MCP\servers\hands.exe",
    "$env:LOCALAPPDATA\hands\hands.exe"
)
$foundBinary = $false
foreach ($p in $binaryPaths) {
    if (Test-Path $p) {
        $foundBinary = $true
        Write-Host "  INFO  Found binary at: $p" -ForegroundColor Gray
        break
    }
}
Check "hands.exe binary found" $foundBinary "Place hands.exe in one of: $($binaryPaths -join ', ')"

# 2. Chrome installed
$chromePaths = @(
    "${env:ProgramFiles}\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
    "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe"
)
$foundChrome = $false
foreach ($p in $chromePaths) {
    if (Test-Path $p) {
        $foundChrome = $true
        break
    }
}
Check "Chrome installed" $foundChrome "Install Google Chrome for browser automation tools"

# 3. Edge installed (alternative to Chrome)
$edgePaths = @(
    "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
    "${env:ProgramFiles}\Microsoft\Edge\Application\msedge.exe"
)
$foundEdge = $false
foreach ($p in $edgePaths) {
    if (Test-Path $p) {
        $foundEdge = $true
        break
    }
}
Check "Edge installed (alternative)" $foundEdge "Chrome or Edge required for browser tools"

if (-not $foundChrome -and -not $foundEdge) {
    Write-Host "  WARN  No Chromium browser found. Browser tools will not work." -ForegroundColor Yellow
}

# 4. Screenshot capability (test that .NET assemblies load)
$screenshotOk = $false
try {
    Add-Type -AssemblyName System.Windows.Forms
    $screen = [System.Windows.Forms.Screen]::PrimaryScreen
    if ($screen) { $screenshotOk = $true }
} catch {}
Check "Screenshot capability" $screenshotOk "System.Windows.Forms not available — vision tools may fail"

# 5. Windows version
$osVersion = [System.Environment]::OSVersion.Version
$win10Plus = $osVersion.Major -ge 10
Check "Windows 10+ detected" $win10Plus "Hands requires Windows 10 or later for UIA support"

# 6. Architecture
$arch = $env:PROCESSOR_ARCHITECTURE
Write-Host "  INFO  Architecture: $arch" -ForegroundColor Gray

Write-Host ""
Write-Host "===================================" -ForegroundColor Cyan
Write-Host "  Results: $pass passed, $fail failed" -ForegroundColor $(if ($fail -eq 0) { "Green" } else { "Yellow" })
Write-Host ""

if ($fail -eq 0) {
    Write-Host "  All checks passed. Hands is ready." -ForegroundColor Green
} else {
    Write-Host "  Some checks failed. Review the FAIL items above." -ForegroundColor Yellow
}
