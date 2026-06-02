# Hands — Per-Machine Setup

This guide covers everything you need to do on each machine where you want to run the `hands` MCP server.

## Per-Machine Checklist

| Item | Per-machine? | How to set up |
|---|---|---|
| MCP binary | Yes | Download from GitHub release → `%LOCALAPPDATA%\CPC\servers\hands.exe`. Pick right arch (`_arm64.exe` or `_x64.exe`). |
| Claude Desktop config | Yes | Edit `%APPDATA%\Claude\claude_desktop_config.json` — copy entry from `claude_desktop_config.example.json` in this repo into your `mcpServers` object. |
| Per-machine paths | Yes | Will be auto-detected by `cpc-paths` (forthcoming). For now, set env vars or hardcode in your config. See "Path Configuration" below. |
| User preferences | Yes | Open Claude Desktop → Settings → Profile → paste your preferences. (UI-only, can't script.) |
| Skills (optional) | Yes | If using CPC skills system, mirror from your Drive's `Volumes/skills/{skill}/` to `%LOCALAPPDATA%\claude-skills\{skill}\`. |
| Volumes / knowledge base | No (Drive-synced) | If Volumes is on Google Drive, just verify Drive is syncing on each machine. No copy needed. |

## Hands-Specific Notes

- **Chrome profile path:** `%LOCALAPPDATA%\CPC\chrome-debug-profile` — created on first `browser_debug_launch` call, persists logins across sessions. This path is per-machine; each machine builds its own profile independently.
- **Vision OCR:** Uses Windows OCR APIs built into Windows 10/11. Works out of the box — no additional install required.
- **UIA tools:** Windows UI Automation requires accessibility permissions to be enabled if you're accessing processes running at elevated privilege. If `uia_find` fails on a system dialog or elevated-privilege window, check that your Claude Desktop process has the right permissions or run as administrator for testing.
- **Chrome (browser tier):** Hands connects to Chrome over CDP (Chrome DevTools Protocol) via chromiumoxide. Chrome must be installed normally on each machine. Use `browser_debug_launch` to start Chrome with the debug port, or `browser_attach` to connect to an already-running instance with `--remote-debugging-port=9222`. No browser binaries are downloaded or managed by Hands.

**Test post-install:** `hands:browser_status` should return a clean status with no errors.

## Path Configuration

**Coming in `cpc-paths` (next release):** automatic detection of Volumes path, install path, backups path. Auto-writes `.cpc-config.toml` on first run. Until then, paths are detected via env vars with fallbacks:

| Path | Env var | Default fallback |
|---|---|---|
| Volumes (knowledge base) | `CPC_VOLUMES_PATH` | Auto-detected by `cpc-paths`; set explicitly if your Drive root is custom |
| Install (server binaries) | `CPC_INSTALL_PATH` | `%LOCALAPPDATA%\CPC\servers` (Windows) |
| Backups | `CPC_BACKUPS_PATH` | `%LOCALAPPDATA%\CPC\backups` (Windows) |

If you're on a different platform or your Drive is mounted elsewhere, set the env vars in your shell profile or system environment before launching Claude Desktop.

## Two-Tier Storage

Every CPC server writes data to exactly one of two roots. Keeping them separate is what makes multi-machine setups safe.

### Tier 1 — Volumes (Drive-synced, cross-machine)

- **Resolver:** `cpc_paths::volumes_path()`
- **Default:** resolved by `cpc-paths`; set `CPC_VOLUMES_PATH` if your Drive root is custom
- **Override:** `CPC_VOLUMES_PATH` env var or `.cpc-config.toml`

What lives here: knowledge base (Operating files, CATALOG.md, skills/), completed breadcrumb archives, handoffs, transcripts, behavioral patterns, shared reference docs.

**Rule:** write-once or write-rarely. Cloud sync's last-write-wins doesn't corrupt because writes are sequential, not concurrent.

### Tier 2 — Local data (per-machine, never sync)

- **Resolver:** `cpc_paths::data_path("hands")`
- **Default:** `%LOCALAPPDATA%\CPC\data\hands-data\` (Windows)
- **Override:** `CPC_HANDS_DATA_DIR` env var or `.cpc-config.toml`

What lives here: active breadcrumb state, Chrome debug profile, logs, per-machine config, cache/index files.

**Rule:** anything with concurrent writes, OS file locks, executable code, or per-machine identity belongs here.

### What NOT to sync

| Do NOT sync | Reason |
|-------------|--------|
| Chrome debug profile (`chrome-debug-profile/`) | Chrome holds a file lock while running — syncing corrupts the profile DB |
| Active breadcrumb state (`state/breadcrumbs/active.index.json`) | FLOCK-sensitive; concurrent servers write here |
| Logs | High churn, per-machine context only |
| MCP binaries | Per-arch executables — wrong arch binary on wrong machine breaks silently |

Breadcrumb *archives* (completed breadcrumbs) DO live in Volumes — they're write-once and safe to sync.

### Legacy paths (existing installs)

If you have `C:\CPC\logs\`, `C:\CPC\state\`, etc. from a legacy install, they continue to work. The server detects the legacy location at startup and uses it automatically. No migration required — new installs use the `%LOCALAPPDATA%\CPC\data\` default; existing installs stay put.

### Setting up a second machine

1. Verify Google Drive is syncing on the new machine — Volumes is already there.
2. Download and install the MCP binaries (right arch) to `%LOCALAPPDATA%\CPC\servers\` or wherever you configure `CPC_INSTALL_PATH`.
3. Copy the `mcpServers` entry from `claude_desktop_config.example.json` into your Claude Desktop config on the new machine.
4. Re-enter credentials per machine via `workflow:credential_store` — secrets do NOT sync.
5. Active session state starts fresh on each machine. That's intentional.

## Future: cpc-setup.exe (planned)

A single-binary helper that automates this entire per-machine setup is planned. It will:
- Detect platform + architecture
- Download the right MCP server binary from GitHub releases
- Auto-detect Volumes / install / backup paths and write `.cpc-config.toml`
- Mirror skills from your Drive (if using CPC skills system)
- Generate a `claude_desktop_config.json` snippet ready to paste

Until cpc-setup.exe ships, follow the manual steps above.
