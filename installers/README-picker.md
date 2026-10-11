# AI-Hands for Windows - install guide

AI-Hands (`hands.exe`) is a local MCP server that lets AI assistants automate your browser (Chrome),
Windows desktop apps (UI Automation) and the screen (screenshots and OCR). `HandsSetup-<version>-win.exe`
installs it and connects it to the AI clients you pick: the Claude desktop app, Codex CLI and Grok CLI.

- Windows 10 1809 (build 17763) or later, arm64 or x64. One installer covers both.
- Per-user install by default: no administrator rights needed.
- Browser tools need Google Chrome. OCR uses the text recognition built into Windows.
- Privacy: see [PRIVACY.md](https://github.com/AIWander/AI-Hands/blob/main/PRIVACY.md). No telemetry or update check; it goes online only when your AI asks.

## Install

1. Download `HandsSetup-<version>-win.exe` from the
   [releases page](https://github.com/AIWander/AI-Hands/releases) and check its SHA-256 against
   `SHA256SUMS.txt` from the same release (`Get-FileHash .\HandsSetup-*-win.exe`).
2. Run it. If Windows shows "Windows protected your PC", see [Unsigned builds](#unsigned-builds-and-smartscreen).
3. Welcome, license, then an optional "install for me only / for all users" choice. Keep "for me only"
   unless you know you need otherwise.
4. Pick the clients (next section), install, read the privacy note, and check the result on the last page.

### What gets installed

| Item | Where |
|---|---|
| `hands.exe` (the build for your CPU) | `%LOCALAPPDATA%\AIWander\Hands\` (all-users install: `C:\Program Files\AIWander\Hands\`) |
| `Register-HandsClients.ps1` (the config writer), `README.md`, `PRIVACY.md`, icon | same folder |
| `register.log` (result of the last client registration) | same folder |
| Settings > Apps entry "AI-Hands" | per-user (or machine for an all-users install) |

Nothing is added to PATH, no service or scheduled task is created, and nothing starts on its own. Your AI
client starts `hands.exe` when it needs it.

## The client picker

Each client is listed. A client found on this PC is ticked; one that is not found stays visible, unticked
and labelled "(not detected)". You can tick or untick any of them. Upgrades reuse your previous choice.

| Client | Counted as present when | Config written |
|---|---|---|
| Claude desktop app | `%LOCALAPPDATA%\AnthropicClaude\claude.exe` or a Store (MSIX) Claude package exists, and its config folder exists | `%APPDATA%\Claude\claude_desktop_config.json`, or the Store copy under `%LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming\Claude\` |
| Codex CLI | `%USERPROFILE%\.codex\config.toml` exists, or `codex` is on PATH | `%USERPROFILE%\.codex\config.toml` |
| Grok CLI | `%USERPROFILE%\.grok\bin\grok(.exe)` or `%USERPROFILE%\.grok\config.toml` exists | `%USERPROFILE%\.grok\config.toml` |

## What each config change looks like

Claude desktop (JSON; every other key is kept; saved as UTF-8 without BOM):

```json
"mcpServers": {
  "hands": { "command": "C:\\Users\\you\\AppData\\Local\\AIWander\\Hands\\hands.exe", "args": [] }
}
```

Codex CLI and Grok CLI (TOML, appended after a blank line):

```toml
[mcp_servers.hands]
command = 'C:\Users\you\AppData\Local\AIWander\Hands\hands.exe'
```

For Grok CLI the writer also takes `"hands"` out of the top-level `disabled_mcp_servers = [...]` list.

If a client already has a `hands` entry, it is left alone and reported as `skipped existing`; the
installer never adds a second one. To replace an existing entry, run the writer yourself with `-Force`.

Afterwards, fully quit Claude desktop from its tray icon (Quit, not just closing the window) and start it
again. Codex and Grok CLI pick hands up the next time they start.

## Backups

Before any change, the writer copies the config to
`<config folder>\backups\<file name>.pre_hands_<yyyyMMdd_HHmmss>`, for example
`%APPDATA%\Claude\backups\claude_desktop_config.json.pre_hands_20261011_101500`. It writes a temp file,
swaps it in and parses the result back; if that check fails, it restores the backup and reports an error.
Backups are never deleted, not even by uninstall. To go back, copy a backup over the config file.

## Upgrade

Run the newer `HandsSetup-<version>-win.exe` over the old one. It closes any running `hands.exe` (quit your
AI clients first if you prefer), replaces the program files, and runs the writer again with the clients you
picked last time. Entries that already point at the installed `hands.exe` report `skipped existing`. Your
configs are not touched again.

## Uninstall

Settings > Apps > AI-Hands > Uninstall (or `unins000.exe` in the install folder; add `/SILENT` for no
prompts). It:

- removes `hands` from the three clients, but only where the entry still points at the installed
  `hands.exe`; an entry you pointed somewhere else is left in place and reported as `skipped foreign`;
- deletes the install folder and the Settings > Apps entry;
- keeps the `backups` folders next to each config. Nothing else is restored.

Not undone: taking `"hands"` out of Grok's `disabled_mcp_servers`.

## Running the writer yourself

```powershell
cd $env:LOCALAPPDATA\AIWander\Hands
.\Register-HandsClients.ps1 -Detect                                  # which clients are present
.\Register-HandsClients.ps1 -Clients ClaudeDesktop,Codex -DryRun     # what would change
.\Register-HandsClients.ps1 -Clients GrokCli -Force                  # replace an existing hands entry
.\Register-HandsClients.ps1 -Uninstall                               # remove hands entries
```

Result lines read `<Client>: written | skipped existing | not detected | removed | skipped foreign | error`
(plus `would write` / `would remove` with `-DryRun`). Exit codes: 0 = changed, 2 = nothing to do, 1 = error.

## Portable fallback (no installer)

`HandsSetup-portable.zip` holds `arm64\hands.exe`, `x64\hands.exe`, `Register-HandsClients.ps1` and this
guide. Copy the right `hands.exe` to a folder you choose, then:

```powershell
.\Register-HandsClients.ps1 -HandsExe C:\path\to\hands.exe -Clients ClaudeDesktop,Codex,GrokCli
```

For Claude desktop only, `hands-<version>-win-arm64.mcpb` is a double-click alternative on arm64 PCs:
Claude desktop installs it as an extension and no config file is edited.

## Unsigned builds and SmartScreen

Draft and preview builds may be unsigned. Windows SmartScreen then shows "Windows protected your PC" on the
first run. Check the SHA-256 against `SHA256SUMS.txt` from the release, then choose More info > Run anyway.
Signed releases show the publisher "Joseph Wander". To see the state of a file:
`Get-AuthenticodeSignature .\HandsSetup-*-win.exe`.

## Troubleshooting

| Symptom | What to do |
|---|---|
| Last page says `skipped existing` | The client already had a `hands` entry. Keep it, or run the writer with `-Force` to point it at this install. |
| Last page says `error` | Read `register.log` in the install folder. A config that is not valid JSON or TOML is never changed; fix it, then run the writer again. |
| Claude desktop does not show the hands tools | Quit Claude desktop completely from the tray icon and start it again. Check that `%APPDATA%\Claude\claude_desktop_config.json` has the `hands` entry. |
| A client shows "(not detected)" but is installed | Tick it anyway; the writer creates the config where that client expects it. Or run `-Detect` to see which paths were checked. |
| Browser tools fail | Install Google Chrome. hands attaches to Chrome's DevTools port on 127.0.0.1 (9222 by default). |
| Setup says hands.exe is in use | Quit the AI clients that use hands, then choose Retry. |
| Antivirus quarantines hands.exe | Automation tools are sometimes flagged. Verify the hash from `SHA256SUMS.txt`, then report it to us. |

## Known limits

- Grok Bot (`%LOCALAPPDATA%\Programs\Grok Bot`) is not supported. It uses a different config
  (`%USERPROFILE%\.grokbot\settings.json`, `mcpBoxServers`).
- TOML is edited line by line, not with a full parser. A `hands` entry written as an inline table or with
  dotted keys is reported but not rewritten; edit it by hand.
- An all-users install writes the client configs of the user who started setup. Each other user can run
  `Register-HandsClients.ps1` from the install folder for their own profile.

## Support

Questions and bug reports: <https://github.com/AIWander/AI-Hands/issues>. Security reports: see
[SECURITY.md](https://github.com/AIWander/AI-Hands/blob/main/SECURITY.md). Changes per version: [CHANGELOG.md](https://github.com/AIWander/AI-Hands/blob/main/CHANGELOG.md).
