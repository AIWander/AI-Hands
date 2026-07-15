@echo off
setlocal

set "REPO=%~dp0.."
set "CARGO_TARGET_DIR=%TEMP%\rust-build-staged\ai-hands"

pushd "%REPO%"
cargo check
set "RESULT=%ERRORLEVEL%"
popd

exit /b %RESULT%
