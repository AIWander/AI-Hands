# AI-Hands installer with a client picker

`HandsSetup-1.1.0-unified.2.exe` installs the standalone AI-Hands MCP server and registers it with the
AI clients you choose. It is a per-user install: no administrator rights, nothing outside your profile.

## What it does

1. Copies `hands.exe` (the build matching your CPU, arm64 or x64) to
   `%LOCALAPPDATA%\AIWander\Hands\hands.exe`, along with `Register-HandsClients.ps1` and this README.
2. Shows a picker with three clients. A client found on this PC is ticked; one that is not found
   stays visible, unticked, and labelled "(not detected)". You can tick or untick any of them.
3. Runs `Register-HandsClients.ps1` for the ticked clients and shows its result on the last page.

| Client | Counted as present when | Config written |
|---|---|---|
| Claude desktop app | `%LOCALAPPDATA%\AnthropicClaude\claude.exe` or a Store (MSIX) Claude package exists, and its config folder exists | `%APPDATA%\Claude\claude_desktop_config.json`, or the MSIX copy under `%LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming\Claude\` |
| Codex CLI | `%USERPROFILE%\.codex\config.toml` exists, or `codex` is on PATH | `%USERPROFILE%\.codex\config.toml` |
| Grok CLI | `%USERPROFILE%\.grok\bin\grok(.exe)` or `%USERPROFILE%\.grok\config.toml` exists | `%USERPROFILE%\.grok\config.toml` |

## What each write looks like

Claude desktop (JSON, every other key kept, saved as UTF-8 without BOM):

```json
"mcpServers": {
  "hands": { "command": "C:\\Users\\you\\AppData\\Local\\AIWander\\Hands\\hands.exe", "args": [] }
}
```

Codex and Grok CLI (TOML, appended after a blank line, or replaced in place with `-Force`):

```toml
[mcp_servers.hands]
command = 'C:\Users\you\AppData\Local\AIWander\Hands\hands.exe'
```

For Grok CLI the writer also takes `"hands"` out of the top-level `disabled_mcp_servers = [...]` list.
Because the table is written fresh, it never carries an `enabled = false`.

If a client already has a `hands` entry, the installer leaves it alone and reports `skipped existing`. It
never adds a second one. To replace an existing entry, run the script yourself with `-Force`.

After installing, fully quit Claude desktop from its tray icon (Quit, not just closing the window) and start
it again. Codex and Grok CLI pick hands up the next time they start.

## Backups

Before any change, the writer copies the config to
`<config folder>\backups\<file name>.pre_hands_<yyyyMMdd_HHmmss>`, for example
`%APPDATA%\Claude\backups\claude_desktop_config.json.pre_hands_20261011_101500`. It writes a temp file,
swaps it in, and parses the result back. If the check fails, it puts the backup back and reports an error.

## Running the writer yourself

```powershell
cd $env:LOCALAPPDATA\AIWander\Hands
.\Register-HandsClients.ps1 -Detect                                  # which clients are present
.\Register-HandsClients.ps1 -Clients ClaudeDesktop,Codex -DryRun     # what would change
.\Register-HandsClients.ps1 -Clients GrokCli -Force                  # replace an existing hands entry
.\Register-HandsClients.ps1 -Uninstall                               # remove hands entries
```

Summary lines read `<Client>: written | skipped existing | not detected | removed | skipped foreign | error`.
Exit codes: 0 = changed, 2 = nothing to do, 1 = error.

## Uninstall

Settings > Apps > AI-Hands > Uninstall. This removes `hands` from all three clients, but only where the entry
still points at the installed `hands.exe`. An entry you pointed somewhere else is left in place and reported
as `skipped foreign`. Nothing else is changed or restored. The backups stay where they are.

## Portable fallback

`HandsSetup-portable.zip` holds `arm64\hands.exe` (and `x64\hands.exe` when that build exists),
`Register-HandsClients.ps1` and this README. Copy the right `hands.exe` to a folder you choose, then run:

```powershell
.\Register-HandsClients.ps1 -HandsExe C:\path\to\hands.exe -Clients ClaudeDesktop,Codex,GrokCli
```

For Claude desktop only, `hands-1.1.0-unified.2-win-arm64.mcpb` is the double-click alternative. Claude
desktop installs it as an extension, and no config file is edited.

## Known limits

- Grok Bot (`%LOCALAPPDATA%\Programs\Grok Bot`) is not supported. It uses a different config
  (`%USERPROFILE%\.grokbot\settings.json`, `mcpBoxServers`), and it is not known whether it accepts a
  local stdio server.
- x64: the installer only carries an x64 `hands.exe` when the x64 cross-build succeeded on the build
  machine. Otherwise it is arm64-only and refuses to install on x64. `dist\x64-build-failure.txt` records why.
- TOML is edited line by line, not with a full parser. The writer handles `[mcp_servers.hands]` tables and
  their own sub-tables, and repairs duplicate tables when run with `-Force`. A `hands` entry written as an
  inline table or with dotted keys is reported but not rewritten; edit it by hand.
- Removing `"hands"` from Grok's `disabled_mcp_servers` is not undone on uninstall.
- Unsigned builds trigger a SmartScreen prompt on other PCs until they are signed (`build-hands-setup.ps1 -Sign`).
- hands needs Google Chrome for browser tools, and uses the OCR built into Windows.
