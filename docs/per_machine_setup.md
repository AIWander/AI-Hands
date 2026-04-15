# Hands — Per-Machine Setup

This guide covers everything you need to do on each machine where you want to run the `hands` MCP server.

## Per-Machine Checklist

| Item | Per-machine? | How to set up |
|---|---|---|
| MCP binary | Yes | Download from GitHub release → `C:\CPC\servers\hands.exe`. Pick right arch (`_arm64.exe` or `_x64.exe`). |
| Claude Desktop config | Yes | Edit `%APPDATA%\Claude\claude_desktop_config.json` — copy entry from `claude_desktop_config.example.json` in this repo into your `mcpServers` object. |
| Per-machine paths | Yes | Will be auto-detected by `cpc-paths` (forthcoming). For now, set env vars or hardcode in your config. See "Path Configuration" below. |
| User preferences | Yes | Open Claude Desktop → Settings → Profile → paste your preferences. (UI-only, can't script.) |
| Skills (optional) | Yes | If using CPC skills system, mirror from your Drive's `Volumes/skills/{skill}/` to `%LOCALAPPDATA%\claude-skills\{skill}\`. |
| Volumes / knowledge base | No (Drive-synced) | If Volumes is on Google Drive, just verify Drive is syncing on each machine. No copy needed. |

## Hands-Specific Notes

- **Chrome profile path:** `%LOCALAPPDATA%\CPC\chrome-debug-profile` — created on first `browser_debug_launch` call, persists logins across sessions. This path is per-machine; each machine builds its own profile independently.
- **Vision OCR:** Uses Windows OCR APIs built into Windows 10/11. Works out of the box — no additional install required.
- **UIA tools:** Windows UI Automation requires accessibility permissions to be enabled if you're accessing processes running at elevated privilege. If `uia_find` fails on a system dialog or elevated-privilege window, check that your Claude Desktop process has the right permissions or run as administrator for testing.
- **Playwright (browser tier):** Browser binaries are auto-managed by Playwright on first use. On first `browser_launch`, Playwright downloads Chromium to its local cache. Internet access is required the first time on each machine.

**Test post-install:** `hands:browser_status` should return a clean status with no errors.

## Path Configuration

**Coming in `cpc-paths` (next release):** automatic detection of Volumes path, install path, backups path. Auto-writes `.cpc-config.toml` on first run. Until then, paths are detected via env vars with fallbacks:

| Path | Env var | Default fallback |
|---|---|---|
| Volumes (knowledge base) | `CPC_VOLUMES_PATH` | `C:\My Drive\Volumes` (Windows) |
| Install (server binaries) | `CPC_INSTALL_PATH` | `C:\CPC\servers` (Windows) |
| Backups | `CPC_BACKUPS_PATH` | `%LOCALAPPDATA%\CPC\backups` (Windows) |

If you're on a different platform or your Drive is mounted elsewhere, set the env vars in your shell profile or system environment before launching Claude Desktop.

## Future: cpc-setup.exe (planned)

A single-binary helper that automates this entire per-machine setup is planned. It will:
- Detect platform + architecture
- Download the right MCP server binary from GitHub releases
- Auto-detect Volumes / install / backup paths and write `.cpc-config.toml`
- Mirror skills from your Drive (if using CPC skills system)
- Generate a `claude_desktop_config.json` snippet ready to paste

Until cpc-setup.exe ships, follow the manual steps above.
