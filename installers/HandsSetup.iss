; HandsSetup.iss - one AI-Hands installer with a client picker (Claude desktop app, Codex CLI, Grok CLI).
;
; Installs the standalone stdio hands.exe (arch-matched) to {localappdata}\AIWander\Hands\hands.exe and
; runs Register-HandsClients.ps1 for the clients ticked on the Components page. The writer backs up every
; config it touches to <config dir>\backups\<file>.pre_hands_<stamp> and never adds a second hands entry.
;
; Build with installers\build-hands-setup.ps1, which stages the payload and passes the defines below.
; Grok Bot is not supported (different config schema). Signing: see installers\SIGNING.md.

#define MyAppName "AI-Hands"
#define MyAppPublisher "AIWander"
#define MyAppURL "https://github.com/AIWander/AI-Hands"

#ifndef MyAppVersion
  #define MyAppVersion "1.1.0-unified.2"
#endif

; Windows version resources need four numbers: 1.1.0-unified.2 -> 1.1.0.2 (the build script computes it).
#ifndef NumericVersion
  #define NumericVersion "1.1.0.2"
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
; AppId identifies this product to Windows for upgrades and uninstall. Generated once on 2026-10-11 for the
; picker installer. Do not change it: a new AppId makes the next version install side by side instead of
; upgrading. (installers\inno\AIHands.iss, the older plugin package, uses a different AppId on purpose.)
AppId={{6E0B2C47-3F1D-4C55-9B8E-A1D4F0C2B7E3}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={code:DefaultInstallDir}
DisableDirPage=auto
DisableProgramGroupPage=yes
DisableWelcomePage=no
LicenseFile=..\LICENSE
InfoAfterFile=..\PRIVACY.md
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
MinVersion=10.0.17763
CloseApplications=yes
CloseApplicationsFilter=hands.exe
RestartApplications=no
#if HaveX64
ArchitecturesAllowed=x64compatible arm64
ArchitecturesInstallIn64BitMode=x64compatible arm64
#else
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#endif
UsePreviousSetupType=no
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
OutputDir={#OutputDir}
OutputBaseFilename=HandsSetup-{#MyAppVersion}-win
SetupIconFile=assets\hands-icon.ico
UninstallDisplayIcon={app}\hands-icon.ico
UninstallDisplayName={#MyAppName} {#MyAppVersion}
VersionInfoVersion={#NumericVersion}
VersionInfoProductVersion={#NumericVersion}
VersionInfoTextVersion={#MyAppVersion}
VersionInfoProductTextVersion={#MyAppVersion}
VersionInfoCompany={#MyAppPublisher}
VersionInfoProductName={#MyAppName}
VersionInfoDescription={#MyAppName} {#MyAppVersion} Setup
VersionInfoCopyright=Copyright 2026 Joseph Wander / AIWander. Apache-2.0.
SetupLogging=yes
#ifdef UseSignTool
; build-hands-setup.ps1 -Sign passes /DUseSignTool=1 and /Scpcsign=...; Inno then signs the uninstaller it
; embeds and the finished setup exe with the same Azure Trusted Signing profile.
SignTool=cpcsign
SignedUninstaller=yes
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Messages]
WelcomeLabel2=This will install [name/ver] on your computer.%n%nAI-Hands is a local MCP server that lets AI assistants automate your browser (Chrome), Windows desktop apps (UI Automation) and the screen (screenshots and OCR). It runs on this PC, has no telemetry or update check, and goes online only when your AI asks it to (for example to load a web page). PRIVACY.md, shown after install, has the details.%n%nOn the next pages you choose which AI clients (Claude desktop app, Codex CLI, Grok CLI) should use it. Setup adds a hands entry to each chosen client's MCP config file, after saving a backup copy of that file.
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
Source: "..\PRIVACY.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "assets\hands-icon.ico"; DestDir: "{app}"; Flags: ignoreversion

[InstallDelete]
; fresh log per install, so the finish page shows only this run
Type: files; Name: "{app}\register.log"

[Run]
; runasoriginaluser: in an all-users (elevated) install the configs written are still the signed-in user's.
Filename: "powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\Register-HandsClients.ps1"" -HandsExe ""{app}\hands.exe"" -Clients {code:SelectedClients} -LogPath ""{app}\register.log"""; StatusMsg: "Registering hands with the selected clients..."; Check: AnyClientSelected; Flags: runhidden waituntilterminated runasoriginaluser

[UninstallRun]
; Removes only a hands entry whose command is this {app}\hands.exe; entries the user pointed elsewhere stay.
Filename: "powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\Register-HandsClients.ps1"" -HandsExe ""{app}\hands.exe"" -Clients ClaudeDesktop,Codex,GrokCli -Uninstall"; RunOnceId: "HandsUnregister"; Flags: runhidden waituntilterminated

[UninstallDelete]
Type: files; Name: "{app}\register.log"
Type: dirifempty; Name: "{app}"

[Code]
var
  HasClaude, HasCodex, HasGrok: Boolean;

function DefaultInstallDir(Param: String): String;
begin
  { Per-user by default; an all-users install (chosen in the privileges dialog) goes to Program Files. }
  if IsAdminInstallMode() then
    Result := ExpandConstant('{autopf}\AIWander\Hands')
  else
    Result := ExpandConstant('{localappdata}\AIWander\Hands');
end;

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

procedure SelectOnly(Name: String; On: Boolean);
begin
  if On then
    WizardSelectComponents(Name)
  else
    WizardSelectComponents('!' + Name);
end;

function ComponentsOnCommandLine(): Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if Pos('/COMPONENTS=', Uppercase(ParamStr(I))) = 1 then
      Result := True;
end;

procedure InitializeWizard();
var
  Prev: String;
begin
  { An explicit /COMPONENTS= wins. Otherwise an upgrade repeats the previous run's client choice exactly
    (stored by RegisterPreviousData), and a first install pre-ticks only detected clients; undetected ones
    stay visible and selectable. }
  if ComponentsOnCommandLine() then
    exit;
  Prev := GetPreviousData('Clients', '#none#');
  if Prev <> '#none#' then
  begin
    Prev := ',' + Prev + ',';
    SelectOnly('claude_desktop', Pos(',ClaudeDesktop,', Prev) > 0);
    SelectOnly('codex', Pos(',Codex,', Prev) > 0);
    SelectOnly('grokcli', Pos(',GrokCli,', Prev) > 0);
  end
  else
  begin
    SelectOnly('claude_desktop', HasClaude);
    SelectOnly('codex', HasCodex);
    SelectOnly('grokcli', HasGrok);
  end;
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

procedure RegisterPreviousData(PreviousDataKey: Integer);
begin
  SetPreviousData(PreviousDataKey, 'Clients', SelectedClients(''));
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
  Note := Note + 'Codex and Grok CLI pick hands up on their next launch.' + #13#10#13#10 +
    'Config backups are in the backups folder next to each config file. Uninstalling (Settings > Apps > AI-Hands) ' +
    'removes this program folder and the hands entries that point at it, and keeps those backups folders.';
  WizardForm.FinishedLabel.Caption := Note;
end;
