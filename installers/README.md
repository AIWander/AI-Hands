# installers/

Packaging artifacts and post-install helpers for AI-Hands.

## Layout

| Path | Purpose |
|------|---------|
| `winget/manifests/a/AIWander/AI-Hands/<version>/` | Per-version winget v1.6 manifests pending submission to `microsoft/winget-pkgs`. Source of truth until merged. |
| `scoop/ai-hands.json` | Scoop manifest for users on the Scoop ecosystem. Can be installed directly from the URL or via a bucket. |
| `scripts/register-hands.ps1` | Post-install helper: wires `hands` into Claude Desktop's `claude_desktop_config.json` (ARCHIVE-FIRST backup, idempotent, supports `-Force` and `-DryRun`). |
| `scripts/generate-winget-manifests.ps1` | Version-bump helper: pulls release-asset metadata via `gh release view` and renders the three winget YAMLs for a new version. |

## Submitting to winget (`microsoft/winget-pkgs`)

The community winget index is a separate repo. When a new AI-Hands release goes out:

1. **Generate manifests** for the new version:
   ```powershell
   .\installers\scripts\generate-winget-manifests.ps1 -Version 1.1.0
   ```
2. **Fork** [`microsoft/winget-pkgs`](https://github.com/microsoft/winget-pkgs).
3. **Copy** the generated directory (`installers/winget/manifests/a/AIWander/AI-Hands/1.1.0/`) into your fork at the same relative path (`manifests/a/AIWander/AI-Hands/1.1.0/`).
4. **Open a PR** against `microsoft/winget-pkgs:master`. Microsoft's bot validates within minutes; human review typically lands within days to weeks.
5. **After merge**: any Windows machine with winget can install AI-Hands with:
   ```powershell
   winget install AIWander.AI-Hands
   ```

Until the PR lands, point users at the manifest URL directly:
```powershell
winget install --manifest https://raw.githubusercontent.com/AIWander/AI-Hands/main/installers/winget/manifests/a/AIWander/AI-Hands/1.0.1/AIWander.AI-Hands.installer.yaml
```
(or fall back to the manual download flow in the root README.)

## Submitting to Scoop

Scoop manifests can be installed from any URL with `scoop install`. The lowest-friction options:

- **Direct URL** (no bucket required):
  ```powershell
  scoop install https://raw.githubusercontent.com/AIWander/AI-Hands/main/installers/scoop/ai-hands.json
  ```
- **Bucket** (better UX, gets autoupdate via Scoop's nightly `checkver`): publish an `AIWander/scoop-bucket` repo and have users run:
  ```powershell
  scoop bucket add aiwander https://github.com/AIWander/scoop-bucket
  scoop install ai-hands
  ```

## Post-install registration

After winget/Scoop drops `hands.exe` onto PATH, register it with Claude Desktop:

```powershell
# Interactive (prompts if already registered)
.\installers\scripts\register-hands.ps1

# Replace existing entry without prompting
.\installers\scripts\register-hands.ps1 -Force

# Show the diff without writing
.\installers\scripts\register-hands.ps1 -DryRun
```

The script writes a `.bak.<yyyyMMdd-HHmmss>` of `claude_desktop_config.json` before any edit. The backup path is always printed; revert with:
```powershell
Move-Item $env:APPDATA\Claude\claude_desktop_config.json.bak.<timestamp> `
          $env:APPDATA\Claude\claude_desktop_config.json -Force
```
