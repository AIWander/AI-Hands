@echo off
setlocal EnableExtensions

set "HANDS_BINARY=%~1"
set "BINARY_ARCH=%~2"
set "VOICE_ROOT=%~3"
set "VOICE_BINARY=%~4"

if "%HANDS_BINARY%"=="" (
  echo Usage: %~nx0 HANDS_BINARY [arm64^|x64compatible] VOICE_COMMAND_ROOT VOICE_BINARY
  exit /b 2
)
if "%BINARY_ARCH%"=="" set "BINARY_ARCH=arm64"
if "%VOICE_ROOT%"=="" (
  echo Voice-Command root is required to build all four variants.
  exit /b 3
)
if "%VOICE_BINARY%"=="" (
  echo Voice-Command Rust binary is required to build all four variants.
  exit /b 4
)

call "%~dp0build-installer.bat" "%HANDS_BINARY%" "%BINARY_ARCH%" hooks no-voice
if errorlevel 1 exit /b %errorlevel%
call "%~dp0build-installer.bat" "%HANDS_BINARY%" "%BINARY_ARCH%" hooks voice "%VOICE_ROOT%" "%VOICE_BINARY%"
if errorlevel 1 exit /b %errorlevel%
call "%~dp0build-installer.bat" "%HANDS_BINARY%" "%BINARY_ARCH%" skills no-voice
if errorlevel 1 exit /b %errorlevel%
call "%~dp0build-installer.bat" "%HANDS_BINARY%" "%BINARY_ARCH%" skills voice "%VOICE_ROOT%" "%VOICE_BINARY%"
if errorlevel 1 exit /b %errorlevel%

echo Built hooks, hooks-plus-Voice, skills, and skills-plus-Voice packages.
exit /b 0
