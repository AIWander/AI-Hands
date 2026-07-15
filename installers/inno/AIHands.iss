#define MyAppName "AI-Hands"
#define MyAppVersion "1.1.0-unified.1"
#define MyAppPublisher "AIWander"
#define MyAppURL "https://github.com/AIWander/AI-Hands"

#ifndef BinaryPath
  #define BinaryPath "..\..\dist\hand.exe"
#endif

#ifndef BinaryArch
  #define BinaryArch "arm64"
#endif

[Setup]
AppId={{A2D39077-909B-4D5E-BA3A-67C12FA2A84B}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={localappdata}\AIWander\AIHands
DefaultGroupName=AIWander
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
MinVersion=10.0.17763
ArchitecturesAllowed={#BinaryArch}
ArchitecturesInstallIn64BitMode={#BinaryArch}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ChangesEnvironment=yes
OutputDir=..\..\dist
OutputBaseFilename=AIHands-Setup-{#MyAppVersion}-{#BinaryArch}
UninstallDisplayIcon={app}\bin\hands.exe
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "addtopath"; Description: "Add AI-Hands to the current user's PATH"; Flags: checkedonce

[Files]
Source: "{#BinaryPath}"; DestDir: "{app}\bin"; DestName: "hands.exe"; Flags: ignoreversion
Source: "..\..\scripts\js_extract.js"; DestDir: "{app}\bin\helpers"; Flags: ignoreversion
Source: "..\..\plugins\ai-hands\*"; DestDir: "{app}\marketplace\plugins\ai-hands"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\..\.agents\plugins\marketplace.json"; DestDir: "{app}\marketplace\.agents\plugins"; Flags: ignoreversion
Source: "..\..\.claude-plugin\marketplace.json"; DestDir: "{app}\marketplace\.claude-plugin"; Flags: ignoreversion
Source: "PREREQUISITES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{code:UpdatedUserPath}"; Flags: preservestringtype; Tasks: addtopath

[Icons]
Name: "{group}\AI-Hands application guide"; Filename: "{app}\marketplace\plugins\ai-hands\instructions\APPLY_TO_YOUR_AI.txt"
Name: "{group}\AI-Hands prerequisites"; Filename: "{app}\PREREQUISITES.txt"

[Run]
Filename: "notepad.exe"; Parameters: """{app}\marketplace\plugins\ai-hands\instructions\APPLY_TO_YOUR_AI.txt"""; Description: "Open the per-AI application guide"; Flags: postinstall nowait skipifsilent unchecked

[Code]
const
  CF_UNICODETEXT = 13;
  GMEM_MOVEABLE = $0002;

function OpenClipboard(hWndNewOwner: HWND): Boolean;
external 'OpenClipboard@user32.dll stdcall';

function EmptyClipboard(): Boolean;
external 'EmptyClipboard@user32.dll stdcall';

function CloseClipboard(): Boolean;
external 'CloseClipboard@user32.dll stdcall';

function SetClipboardData(uFormat: Cardinal; hMem: HWND): HWND;
external 'SetClipboardData@user32.dll stdcall';

function GlobalAlloc(uFlags: Cardinal; dwBytes: Cardinal): HWND;
external 'GlobalAlloc@kernel32.dll stdcall';

function GlobalLock(hMem: HWND): HWND;
external 'GlobalLock@kernel32.dll stdcall';

function GlobalUnlock(hMem: HWND): Boolean;
external 'GlobalUnlock@kernel32.dll stdcall';

function GlobalFree(hMem: HWND): HWND;
external 'GlobalFree@kernel32.dll stdcall';

function LStrCopy(Destination: HWND; Source: String): HWND;
external 'lstrcpyW@kernel32.dll stdcall';

function GetLastErrorCode(): Cardinal;
external 'GetLastError@kernel32.dll stdcall';

var
  ClipboardSucceeded: Boolean;

function PathContains(const Haystack, Needle: String): Boolean;
begin
  Result := Pos(';' + Lowercase(Needle) + ';', ';' + Lowercase(Haystack) + ';') > 0;
end;

function UpdatedUserPath(Param: String): String;
var
  ExistingPath: String;
  BinPath: String;
begin
  BinPath := ExpandConstant('{app}\bin');
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', ExistingPath) then
    ExistingPath := '';

  if PathContains(ExistingPath, BinPath) then
    Result := ExistingPath
  else if ExistingPath = '' then
    Result := BinPath
  else
    Result := ExistingPath + ';' + BinPath;
end;

function SupportedBrowserFound(): Boolean;
begin
  Result :=
    FileExists(ExpandConstant('{localappdata}\Google\Chrome\Application\chrome.exe')) or
    FileExists(ExpandConstant('{commonpf64}\Google\Chrome\Application\chrome.exe')) or
    FileExists(ExpandConstant('{pf}\Google\Chrome\Application\chrome.exe')) or
    FileExists(ExpandConstant('{pf32}\Google\Chrome\Application\chrome.exe')) or
    FileExists(ExpandConstant('{localappdata}\Chromium\Application\chrome.exe')) or
    FileExists(ExpandConstant('{commonpf64}\Chromium\Application\chrome.exe')) or
    FileExists(ExpandConstant('{pf}\Chromium\Application\chrome.exe')) or
    FileExists(ExpandConstant('{pf32}\Chromium\Application\chrome.exe'));
end;

function TrySetClipboardText(const Text: String): Boolean;
var
  MemoryHandle: HWND;
  MemoryPointer: HWND;
  ClipboardHandle: HWND;
begin
  Result := False;
  MemoryHandle := GlobalAlloc(GMEM_MOVEABLE, (Length(Text) + 1) * 2);
  if MemoryHandle = 0 then
  begin
    Log(Format('Clipboard copy: GlobalAlloc failed with code %d.', [GetLastErrorCode()]));
    Exit;
  end;

  MemoryPointer := GlobalLock(MemoryHandle);
  if MemoryPointer = 0 then
  begin
    Log(Format('Clipboard copy: GlobalLock failed with code %d.', [GetLastErrorCode()]));
    GlobalFree(MemoryHandle);
    Exit;
  end;

  LStrCopy(MemoryPointer, Text);
  GlobalUnlock(MemoryHandle);

  if not OpenClipboard(WizardForm.Handle) then
  begin
    Log(Format('Clipboard copy: OpenClipboard failed with code %d.', [GetLastErrorCode()]));
    GlobalFree(MemoryHandle);
    Exit;
  end;

  try
    if not EmptyClipboard() then
    begin
      Log(Format('Clipboard copy: EmptyClipboard failed with code %d.', [GetLastErrorCode()]));
      Exit;
    end;

    ClipboardHandle := SetClipboardData(CF_UNICODETEXT, MemoryHandle);
    if ClipboardHandle = 0 then
    begin
      Log(Format('Clipboard copy: SetClipboardData failed with code %d.', [GetLastErrorCode()]));
      Exit;
    end;

    MemoryHandle := 0;
    Result := True;
    Log('Clipboard copy: native Unicode clipboard write succeeded.');
  finally
    CloseClipboard();
    if MemoryHandle <> 0 then
      GlobalFree(MemoryHandle);
  end;
end;

procedure CopyGuideToClipboard();
var
  GuidePath: String;
  GuideBytes: AnsiString;
  GuideText: String;
  PwshPath: String;
  Params: String;
  Attempt: Integer;
  ResultCode: Integer;
begin
  GuidePath := ExpandConstant('{app}\marketplace\plugins\ai-hands\instructions\APPLY_TO_YOUR_AI.txt');
  ClipboardSucceeded := LoadStringFromFile(GuidePath, GuideBytes);
  if not ClipboardSucceeded then
  begin
    Log('Clipboard copy: installed guide could not be read.');
    Exit;
  end;
  GuideText := GuideBytes;

  ClipboardSucceeded := False;
  for Attempt := 1 to 3 do
  begin
    if TrySetClipboardText(GuideText) then
    begin
      ClipboardSucceeded := True;
      Exit;
    end;
    Sleep(250);
  end;

  Params :=
    '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command ' +
    '"$content = Get-Content -Raw -LiteralPath ''' + GuidePath + '''; ' +
    'Set-Clipboard -Value $content -ErrorAction Stop; ' +
    'if ((Get-Clipboard -Raw) -ceq $content) { exit 0 } else { exit 1 }"';
  PwshPath := ExpandConstant('{pf}\PowerShell\7\pwsh.exe');
  if FileExists(PwshPath) then
  begin
    for Attempt := 1 to 3 do
    begin
      if ExecAsOriginalUser(
        PwshPath,
        Params,
        '',
        SW_HIDE,
        ewWaitUntilTerminated,
        ResultCode
      ) and (ResultCode = 0) then
      begin
        ClipboardSucceeded := True;
        Log('Clipboard copy: original-user PowerShell 7 fallback succeeded.');
        Exit;
      end;
      Sleep(250);
    end;
  end;

  for Attempt := 1 to 3 do
  begin
    if ExecAsOriginalUser(
      ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
      Params,
      '',
      SW_HIDE,
      ewWaitUntilTerminated,
      ResultCode
    ) and (ResultCode = 0) then
    begin
      ClipboardSucceeded := True;
      Log('Clipboard copy: original-user Windows PowerShell fallback succeeded.');
      Exit;
    end;
    Sleep(250);
  end;
  Log(Format('Clipboard copy: all methods failed; last helper result was %d.', [ResultCode]));
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  MessageText: String;
begin
  if CurStep = ssPostInstall then
  begin
    ClipboardSucceeded := False;
    if not WizardSilent then
      CopyGuideToClipboard();

    MessageText :=
      'AI-Hands, its optional plugin, and its skills are staged locally.' + #13#10 + #13#10 +
      'No Codex, Claude, Grok, ChatGPT, MCP, or hook configuration was edited.' + #13#10 +
      'Use the per-AI guide to apply the plugin through each host''s supported UI or CLI.';

    if ClipboardSucceeded then
      MessageText := MessageText + #13#10 + #13#10 +
        'The complete per-AI guide is now on your clipboard.'
    else
      MessageText := MessageText + #13#10 + #13#10 +
        'Clipboard copy was unavailable. Open the installed application guide instead.';

    if not SupportedBrowserFound() then
      MessageText := MessageText + #13#10 + #13#10 +
        'Chrome or Chromium was not detected. UIA and vision abilities may still work, but browser automation requires a supported browser installed separately.';

    if not WizardSilent then
      MsgBox(MessageText, mbInformation, MB_OK);
  end;
end;
