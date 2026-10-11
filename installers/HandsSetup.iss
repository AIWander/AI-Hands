; HandsSetup.iss - one AI-Hands installer with a client picker (Claude desktop app, Codex CLI, Grok CLI).
;
; Installs the standalone stdio hands.exe (arch-matched) to {localappdata}\AIWander\Hands\hands.exe and
; runs Register-HandsClients.ps1 for the clients ticked on the Components page. The writer backs up every
; config it touches to <config dir>\backups\<file>.pre_hands_<stamp> and never adds a second hands entry.
;
; Build with installers\build-hands-setup.ps1, which stages the payload and passes the defines below.
; Grok Bot is not supported (different config schema).

#define MyAppName "AI-Hands"
#define MyAppPublisher "AIWander"
#define MyAppURL "https://github.com/AIWander/AI-Hands"

#ifndef MyAppVersion
  #define MyAppVersion "1.1.0-unified.2"
#endif

#ifndef PayloadDir
  #define PayloadDir "..\dist\payload"
#endif

#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

; x64 is optional: build-hands-setup.ps1 only stages hands-x64.exe when the cross-build succeeded.
#ifexist AddBackslash(PayloadDir) + "hands-x64.exe"
  #define HaveX64 1
#else
  #define HaveX64 0
#endif

[Setup]
AppId={{6E0B2C47-3F1D-4C55-9B8E-A1D4F0C2B7E3}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={localappdata}\AIWander\Hands
DisableDirPage=auto
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
MinVersion=10.0.17763
#if HaveX64
ArchitecturesAllowed=x64compatible arm64
ArchitecturesInstallIn64BitMode=x64compatible arm64
#else
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#endif
UsePreviousSetupType=no
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
OutputDir={#OutputDir}
OutputBaseFilename=HandsSetup-{#MyAppVersion}
UninstallDisplayIcon={app}\hands.exe
UninstallDisplayName={#MyAppName} {#MyAppVersion}
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Messages]
SelectComponentsLabel2=Choose which AI clients should get the hands MCP server. Clients found on this PC are ticked. A client that already has a hands entry is left as it is (reported as "skipped existing").

[Types]
Name: "custom"; Description: "Choose clients"; Flags: iscustom

[Components]
Name: "claude_desktop"; Description: "{code:ClientDesc|claude_desktop}"
Name: "codex"; Description: "{code:ClientDesc|codex}"
Name: "grokcli"; Description: "{code:ClientDesc|grokcli}"

[Files]
Source: "{#PayloadDir}\hands-arm64.exe"; DestDir: "{app}"; DestName: "hands.exe"; Check: IsArm64; Flags: ignoreversion
#if HaveX64
Source: "{#PayloadDir}\hands-x64.exe"; DestDir: "{app}"; DestName: "hands.exe"; Check: not IsArm64; Flags: ignoreversion
#endif
Source: "{#PayloadDir}\Register-HandsClients.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion

[InstallDelete]
; fresh log per install, so the finish page shows only this run
Type: files; Name: "{app}\register.log"

[Run]
Filename: "powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\Register-HandsClients.ps1"" -HandsExe ""{app}\hands.exe"" -Clients {code:SelectedClients} -LogPath ""{app}\register.log"""; StatusMsg: "Registering hands with the selected clients..."; Check: AnyClientSelected; Flags: runhidden waituntilterminated

[UninstallRun]
; Removes only a hands entry whose command is this {app}\hands.exe; entries the user pointed elsewhere stay.
Filename: "powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\Register-HandsClients.ps1"" -HandsExe ""{app}\hands.exe"" -Clients ClaudeDesktop,Codex,GrokCli -Uninstall"; RunOnceId: "HandsUnregister"; Flags: runhidden waituntilterminated

[UninstallDelete]
Type: files; Name: "{app}\register.log"

[Code]
var
  HasClaude, HasCodex, HasGrok: Boolean;

function ClaudeMsixConfigDirExists(): Boolean;
var
  FindRec: TFindRec;
  Root: String;
begin
  Result := False;
  Root := ExpandConstant('{localappdata}\Packages\');
  if FindFirst(Root + 'Claude_*', FindRec) then
  begin
    try
      repeat
        if (FindRec.Attributes and FILE_ATTRIBUTE_DIRECTORY <> 0) and
           DirExists(Root + FindRec.Name + '\LocalCache\Roaming\Claude') then
          Result := True;
      until Result or not FindNext(FindRec);
    finally
      FindClose(FindRec);
    end;
  end;
end;

function ClaudeAppxInstalled(): Boolean;
var
  FindRec: TFindRec;
begin
  { Per-user MSIX installs always own a %LOCALAPPDATA%\Packages\Claude_* folder; this stands in for
    Get-AppxPackage *Claude* so the wizard does not have to start PowerShell to draw the page. }
  Result := FindFirst(ExpandConstant('{localappdata}\Packages\Claude_*'), FindRec);
  if Result then
    FindClose(FindRec);
end;

function DetectClaude(): Boolean;
begin
  Result := (FileExists(ExpandConstant('{localappdata}\AnthropicClaude\claude.exe')) or ClaudeAppxInstalled()) and
            (DirExists(ExpandConstant('{userappdata}\Claude')) or ClaudeMsixConfigDirExists());
end;

function DetectCodex(): Boolean;
begin
  Result := FileExists(ExpandConstant('{%USERPROFILE}\.codex\config.toml')) or
            FileExists(ExpandConstant('{userappdata}\npm\codex.cmd')) or
            (FileSearch('codex.cmd', GetEnv('PATH')) <> '') or
            (FileSearch('codex.exe', GetEnv('PATH')) <> '');
end;

function DetectGrok(): Boolean;
begin
  Result := FileExists(ExpandConstant('{%USERPROFILE}\.grok\bin\grok.exe')) or
            FileExists(ExpandConstant('{%USERPROFILE}\.grok\bin\grok')) or
            FileExists(ExpandConstant('{%USERPROFILE}\.grok\config.toml'));
end;

function InitializeSetup(): Boolean;
begin
  HasClaude := DetectClaude();
  HasCodex := DetectCodex();
  HasGrok := DetectGrok();
  Result := True;
end;

function ClientDesc(Param: String): String;
var
  Found: Boolean;
begin
  if Param = 'claude_desktop' then
  begin
    Result := 'Claude desktop app';
    Found := HasClaude;
  end
  else if Param = 'codex' then
  begin
    Result := 'Codex CLI';
    Found := HasCodex;
  end
  else
  begin
    Result := 'Grok CLI';
    Found := HasGrok;
  end;
  if not Found then
    Result := Result + ' (not detected)';
end;

procedure InitializeWizard();
begin
  { Pre-tick only detected clients; undetected ones stay visible and selectable. }
  if HasClaude then WizardSelectComponents('claude_desktop');
  if HasCodex then WizardSelectComponents('codex');
  if HasGrok then WizardSelectComponents('grokcli');
end;

function SelectedClients(Param: String): String;
begin
  Result := '';
  if WizardIsComponentSelected('claude_desktop') then Result := Result + ',ClaudeDesktop';
  if WizardIsComponentSelected('codex') then Result := Result + ',Codex';
  if WizardIsComponentSelected('grokcli') then Result := Result + ',GrokCli';
  if Result <> '' then
    Delete(Result, 1, 1);
end;

function AnyClientSelected(): Boolean;
begin
  Result := SelectedClients('') <> '';
end;

procedure CurPageChanged(CurPageID: Integer);
var
  Log: AnsiString;
  Note: String;
begin
  if CurPageID <> wpFinished then
    exit;
  Note := 'hands.exe is installed in ' + ExpandConstant('{app}') + '.' + #13#10#13#10;
  if AnyClientSelected() then
  begin
    if LoadStringFromFile(ExpandConstant('{app}\register.log'), Log) then
      Note := Note + 'Client registration:' + #13#10 + String(Log) + #13#10
    else
      Note := Note + 'Client registration did not leave a log; run Register-HandsClients.ps1 -Detect from the install folder to check.' + #13#10#13#10;
  end
  else
    Note := Note + 'No client was selected, so no config was changed.' + #13#10#13#10;
  if WizardIsComponentSelected('claude_desktop') then
    Note := Note + 'Claude desktop: fully quit it from the tray icon (Quit, not just closing the window) and start it again. ';
  Note := Note + 'Codex and Grok CLI pick hands up on their next launch. Config backups are in the backups folder next to each config file.';
  WizardForm.FinishedLabel.Caption := Note;
end;
