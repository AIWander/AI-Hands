AI-Hands privacy note
=====================

Applies to hands.exe and the AI-Hands installer (HandsSetup-<version>-win.exe), version 1.1.0-unified.2.

In short: AI-Hands runs on your PC as a helper process started by your AI client. It has no telemetry, no
analytics, no account and no update check. It goes online only when your AI asks it to do something that
needs the network.


What it can see and do
----------------------

When your AI client calls a hands tool, hands acts on this PC with your user rights:

- Browser: it starts or attaches to Google Chrome through Chrome's DevTools port on 127.0.0.1 (9222 by
  default) and can read pages, click, type and fill forms in that browser, including sites where you are
  signed in.
- Desktop: it reads and operates windows through Windows UI Automation (buttons, text fields, menus).
- Screen: it takes screenshots and reads text from them with the OCR built into Windows.

Your AI client decides when to call these tools. Most clients ask you before running a tool; check your
client's tool-permission settings.


Network
-------

- No telemetry, analytics, crash reporting or update check. hands does not contact AIWander.
- Web pages load because the AI asked hands to open or fetch them; that traffic goes to those sites.
- vision_analyze is the one tool that sends data to a third party: if you set the ANTHROPIC_API_KEY
  environment variable and the AI calls that tool, the screenshot is sent to api.anthropic.com for
  analysis. Without that variable the tool returns an error and sends nothing.
- hands opens no listening port by default. An optional status endpoint on 127.0.0.1:9102 (loopback only)
  starts only if you set HANDS_ENABLE_DASHBOARD=1.


What it writes on your PC
-------------------------

- Activity log: one line per tool attempt in hands_meta.jsonl (rotated daily, kept 7 days) under
  %LOCALAPPDATA%\CPC\data\hands-data\ (or C:\CPC\logs if that folder already holds hands logs). Lines carry the
  tool name, timing and success; raw arguments, URLs, selectors, typed text, page content, headers,
  cookies and file paths are left out by design.
- Screenshots: temporary captures go to %TEMP%\hands-captures; captures the AI asks to keep go to
  %USERPROFILE%\Pictures\Screenshots.
- Browser profile data stays in Chrome's own profile.


What the installer touches
--------------------------

- Program files: hands.exe, Register-HandsClients.ps1, README.md, PRIVACY.md and an icon in
  %LOCALAPPDATA%\AIWander\Hands (or the folder you chose), plus register.log, the result of the last
  client registration.
- For each client you tick, one hands entry in that client's MCP config:
  Claude desktop  %APPDATA%\Claude\claude_desktop_config.json (or its Store-app copy)
  Codex CLI       %USERPROFILE%\.codex\config.toml
  Grok CLI        %USERPROFILE%\.grok\config.toml
  Before changing a file, the installer saves a copy in a backups folder beside it, named
  <file>.pre_hands_<date>_<time>. A client that already has a hands entry is left unchanged.
- An entry in Settings > Apps so you can uninstall it.

Uninstall removes the program folder and the hands entries that still point at it. It keeps the backups
folders, so you can always get your earlier config back.


Contact
-------

Questions or concerns: https://github.com/AIWander/AI-Hands/issues (security reports: see SECURITY.md).
