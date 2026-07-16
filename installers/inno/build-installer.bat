@echo off
setlocal EnableExtensions

set "REPO=%~dp0..\.."
set "ISCC=C:\Program Files (x86)\Inno Setup 6\ISCC.exe"
set "SOURCE_BINARY=%~1"
set "BINARY_ARCH=%~2"
set "PROFILE=%~3"
set "VOICE_MODE=%~4"
set "VOICE_ROOT=%~5"
set "VOICE_BINARY=%~6"

if "%SOURCE_BINARY%"=="" set "SOURCE_BINARY=%REPO%\dist\hand.exe"
if "%BINARY_ARCH%"=="" set "BINARY_ARCH=arm64"
if "%PROFILE%"=="" set "PROFILE=hooks"
if "%VOICE_MODE%"=="" set "VOICE_MODE=no-voice"
for %%I in ("%SOURCE_BINARY%") do set "SOURCE_BINARY=%%~fI"

if exist "%ISCC%" goto iscc_ok
echo Inno Setup compiler was not found.
exit /b 2
:iscc_ok
if not exist "%SOURCE_BINARY%" (
  echo Rust binary not found: %SOURCE_BINARY%
  exit /b 3
)
if /I not "%BINARY_ARCH%"=="arm64" if /I not "%BINARY_ARCH%"=="x64compatible" (
  echo Binary architecture must be arm64 or x64compatible.
  exit /b 4
)
if /I "%BINARY_ARCH%"=="arm64" (set "OUTPUT_ARCH=arm64") else (set "OUTPUT_ARCH=x64")

if /I "%PROFILE%"=="hooks" (
  set "PLUGIN_ID=ai-hands"
  set "PLUGIN_PATH=%REPO%\plugins\ai-hands"
) else if /I "%PROFILE%"=="skills" (
  set "PLUGIN_ID=ai-hands-skills"
  set "PLUGIN_PATH=%REPO%\plugins\ai-hands-skills"
) else (
  echo Plugin profile must be hooks or skills.
  exit /b 5
)

if not exist "%PLUGIN_PATH%\.codex-plugin\plugin.json" (
  echo Plugin manifest not found: %PLUGIN_PATH%\.codex-plugin\plugin.json
  exit /b 6
)

set "INCLUDE_VOICE=0"
set "VOICE_DEFINES="
if /I "%VOICE_MODE%"=="voice" (
  if "%VOICE_ROOT%"=="" (
    echo Voice root is required for a voice package.
    exit /b 7
  )
  if "%VOICE_BINARY%"=="" (
    echo Voice Rust binary is required for a voice package.
    exit /b 8
  )
  for %%I in ("%VOICE_ROOT%") do set "VOICE_ROOT=%%~fI"
  for %%I in ("%VOICE_BINARY%") do set "VOICE_BINARY=%%~fI"
  if not exist "%VOICE_ROOT%\plugins\voice-command\.codex-plugin\plugin.json" (
    echo Voice plugin manifest not found under: %VOICE_ROOT%
    exit /b 9
  )
  if not exist "%VOICE_ROOT%\installer\APPLY_TO_YOUR_AI.txt" (
    echo Voice activation guide not found under: %VOICE_ROOT%
    exit /b 10
  )
  if not exist "%VOICE_BINARY%" (
    echo Voice Rust binary not found: %VOICE_BINARY%
    exit /b 11
  )
  set "INCLUDE_VOICE=1"
  set "VOICE_DEFINES=/DVoicePluginRoot="%VOICE_ROOT%" /DVoiceBinaryPath="%VOICE_BINARY%""
) else if /I not "%VOICE_MODE%"=="no-voice" (
  echo Voice mode must be voice or no-voice.
  exit /b 12
)

pushd "%REPO%\installers\inno"
"%ISCC%" /DBinaryPath="%SOURCE_BINARY%" /DBinaryArch="%BINARY_ARCH%" /DOutputArch="%OUTPUT_ARCH%" /DPluginId="%PLUGIN_ID%" /DPluginPath="%PLUGIN_PATH%" /DPackageFlavor="%PROFILE%" /DIncludeVoice=%INCLUDE_VOICE% %VOICE_DEFINES% AIHands.iss
set "RESULT=%ERRORLEVEL%"
popd

exit /b %RESULT%
