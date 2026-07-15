@echo off
setlocal

set "REPO=%~dp0.."
set "CARGO_TARGET_DIR=%TEMP%\rust-build-staged\ai-hands"
set "RUSTFLAGS=%RUSTFLAGS% --remap-path-prefix=%USERPROFILE%=C:\BuildUser"
set "DIST=%REPO%\dist"

if not exist "%DIST%" mkdir "%DIST%"

pushd "%REPO%"
if /I "%~1"=="--release-only" goto release_build
cargo test --locked -- --test-threads=1
if errorlevel 1 (
  popd
  exit /b 1
)

:release_build
cargo build --release --locked
if errorlevel 1 (
  popd
  exit /b 1
)
popd

if exist "%DIST%\hand.exe" (
  powershell -NoProfile -ExecutionPolicy Bypass -File "%REPO%\scripts\archive-dist.ps1" -Path "%DIST%\hand.exe"
  if errorlevel 1 exit /b 1
)

copy /y "%CARGO_TARGET_DIR%\release\hands.exe" "%DIST%\hand.exe" >nul
if errorlevel 1 exit /b 1

echo Built "%DIST%\hand.exe"
exit /b 0
