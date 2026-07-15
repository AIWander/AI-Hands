@echo off
setlocal

set "REPO=%~dp0..\.."
set "ISCC=C:\Program Files (x86)\Inno Setup 6\ISCC.exe"
set "SOURCE_BINARY=%~1"
set "BINARY_ARCH=%~2"

if "%SOURCE_BINARY%"=="" set "SOURCE_BINARY=%REPO%\dist\hand.exe"
if "%BINARY_ARCH%"=="" set "BINARY_ARCH=arm64"

if exist "%ISCC%" goto iscc_ok
echo Inno Setup compiler not found: %ISCC%
exit /b 2
:iscc_ok

if exist "%SOURCE_BINARY%" goto binary_ok
echo Rust binary not found: %SOURCE_BINARY%
exit /b 3
:binary_ok

if /I not "%BINARY_ARCH%"=="arm64" if /I not "%BINARY_ARCH%"=="x64compatible" (
  echo Binary architecture must be arm64 or x64compatible.
  exit /b 4
)

pushd "%REPO%\installers\inno"
"%ISCC%" /DBinaryPath="%SOURCE_BINARY%" /DBinaryArch="%BINARY_ARCH%" AIHands.iss
set "RESULT=%ERRORLEVEL%"
popd

exit /b %RESULT%
