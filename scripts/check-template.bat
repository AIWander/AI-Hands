@echo off
setlocal

set "REPO=%~dp0.."
set "CARGO_TARGET_DIR=%TEMP%\rust-build-staged\ai-hands"

pushd "%REPO%"
rustfmt --edition 2021 --check src\monitor_scope.rs src\meta\script.rs
if errorlevel 1 (
  popd
  exit /b 1
)

cargo clippy --all-targets --locked -- -D warnings
set "RESULT=%ERRORLEVEL%"
popd

exit /b %RESULT%
