# Hands — Per-Machine Setup

This guide covers everything you need to do on each machine where you want to run the `hands` MCP server.

## Per-Machine Checklist

| Item | Per-machine? | How to set up |
|---|---|---|
| MCP binary | Yes | Download from GitHub release → `%LOCALAPPDATA%\AIWander\Hands\bin\hands.exe`, or set `HANDS_HOME`. Pick the right arch (`_arm64.exe` or `_x64.exe`). |
| Host config | Yes | Run `installers/scripts/Register-HandsClients.ps1`. It writes the absolute path into each installed stdio host and skips a host that already has a hands entry. |
| Per-machine paths | Yes | Resolved at runtime. `HANDS_HOME` overrides. `CPC_ROOT` is only a back-compat alias for that same directory. |
| User preferences | Yes | Open Claude Desktop → Settings → Profile → paste your preferences. (UI-only, can't script.) |
| Skills (optional) | Yes | If using CPC skills system, mirror from your Drive's `Volumes/skills/{skill}/` to `%LOCALAPPDATA%\claude-skills\{skill}\`. |
| Volumes / knowledge base | No (Drive-synced) | If Volumes is on Google Drive, just verify Drive is syncing on each machine. No copy needed. |

## Hands-Specific Notes

- **Chrome profile path:** `%LOCALAPPDATA%\AIWander\Hands\chrome-debug-profile` (or `<HANDS_HOME>\chrome-debug-profile`) — created on first `browser_debug_launch` call, persists logins across sessions. This path is per-machine; each machine builds its own profile independently.
- **Vision OCR:** Uses Windows OCR APIs built into Windows 10/11. Works out of the box — no additional install required.
- **UIA tools:** Windows UI Automation requires accessibility permissions to be enabled if you're accessing processes running at elevated privilege. If `uia_find` fails on a system dialog or elevated-privilege window, check that your Claude Desktop process has the right permissions or run as administrator for testing.
- **Chrome (browser tier):** Hands connects to Chrome over CDP (Chrome DevTools Protocol) via chromiumoxide. Chrome must be installed normally on each machine. Use `browser_debug_launch` to start Chrome with the debug port, or `browser_attach` to connect to an already-running instance with `--remote-debugging-port=9222`. No browser binaries are downloaded or managed by Hands.

**Test post-install:** `hands:browser_status` should return a clean status with no errors.

## Path Configuration

Hands resolves its own folder when the process starts. Nothing in the binary is a particular machine's path.

| Path | Env var | Default |
|---|---|---|
| Hands home | `HANDS_HOME` | `%LOCALAPPDATA%\AIWander\Hands` |
| Same folder, older name | `CPC_ROOT` | Used only when `HANDS_HOME` is unset |
| Breadcrumb archive root | `CPC_VOLUMES_PATH` | `{Hands home}\data\breadcrumbs` when unset |
| Logs | (under the home) | `{Hands home}\logs` |
| Local data | (under the home) | `{Hands home}\data` |

`CPC_ROOT` is an alias for the Hands home, not a separate tree. A new install does not read a hard-coded `C:\CPC` path unless you set that variable.

## Two-Tier Storage

Every CPC server writes data to exactly one of two roots. Keeping them separate is what makes multi-machine setups safe.

### Tier 1 — Volumes (Drive-synced, cross-machine)

- **Resolver:** `cpc_paths::volumes_path()`
- **Default:** resolved by `cpc-paths`; set `CPC_VOLUMES_PATH` if your Drive root is custom
- **Override:** `CPC_VOLUMES_PATH` env var or `.cpc-config.toml`

What lives here: knowledge base (Operating files, CATALOG.md, skills/), completed breadcrumb archives, handoffs, transcripts, behavioral patterns, shared reference docs.

**Rule:** write-once or write-rarely. Cloud sync's last-write-wins doesn't corrupt because writes are sequential, not concurrent.

### Tier 2 — Local data (per-machine, never sync)

- **Resolver:** the Hands home above
- **Default:** `%LOCALAPPDATA%\AIWander\Hands\data\`
- **Override:** `HANDS_HOME` (or `CPC_ROOT` when `HANDS_HOME` is unset)

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

### Older installs

Point `CPC_ROOT` at the directory you already use. Logs are `{CPC_ROOT}\logs` and data is `{CPC_ROOT}\data`. A new machine that does not set `HANDS_HOME` or `CPC_ROOT` uses `%LOCALAPPDATA%\AIWander\Hands` and does not read another user's folders.

### Setting up a second machine

1. Verify Google Drive is syncing on the new machine — Volumes is already there.
2. Download and install the MCP binary (right arch) to `%LOCALAPPDATA%\AIWander\Hands\bin\` or the `HANDS_HOME` you choose.
3. Run `Register-HandsClients.ps1` on the new machine so each host gets that machine's absolute path.
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
