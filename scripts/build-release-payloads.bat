@echo off
setlocal EnableExtensions

set "TARGET=%~1"
set "VOICE_ROOT=%~2"
set "OUTPUT_ROOT=%~3"
for %%I in ("%~dp0..") do set "REPO=%%~fI"

if "%TARGET%"=="" (
  echo Usage: %~nx0 RUST_TARGET VOICE_COMMAND_ROOT [OUTPUT_ROOT]
  exit /b 2
)
if "%VOICE_ROOT%"=="" (
  echo Voice-Command root is required.
  exit /b 3
)
for %%I in ("%VOICE_ROOT%") do set "VOICE_ROOT=%%~fI"
if "%OUTPUT_ROOT%"=="" set "OUTPUT_ROOT=%TEMP%\aihands-release-payloads\%TARGET%"
for %%I in ("%OUTPUT_ROOT%") do set "OUTPUT_ROOT=%%~fI"

if not exist "%VOICE_ROOT%\voice-mcp\Cargo.toml" (
  echo Voice-Command Cargo manifest not found: %VOICE_ROOT%\voice-mcp\Cargo.toml
  exit /b 4
)

set "HANDS_TARGET_DIR=%OUTPUT_ROOT%\hands"
set "VOICE_TARGET_DIR=%OUTPUT_ROOT%\voice"

pushd "%REPO%"
cargo build --release --locked --target "%TARGET%" --target-dir "%HANDS_TARGET_DIR%"
if errorlevel 1 (
  popd
  exit /b 5
)
popd

pushd "%VOICE_ROOT%"
cargo build --release --locked --manifest-path "%VOICE_ROOT%\voice-mcp\Cargo.toml" --target "%TARGET%" --target-dir "%VOICE_TARGET_DIR%"
if errorlevel 1 (
  popd
  exit /b 6
)
popd

if not exist "%HANDS_TARGET_DIR%\%TARGET%\release\hands.exe" (
  echo Hands output missing after build.
  exit /b 7
)
if not exist "%VOICE_TARGET_DIR%\%TARGET%\release\voice-mcp.exe" (
  echo Voice output missing after build.
  exit /b 8
)

echo Hands=%HANDS_TARGET_DIR%\%TARGET%\release\hands.exe
echo Voice=%VOICE_TARGET_DIR%\%TARGET%\release\voice-mcp.exe
exit /b 0
