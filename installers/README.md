# installers/

Packaging artifacts and post-install helpers for AI-Hands.

## Layout

| Path | Purpose |
|------|---------|
| `winget/manifests/a/AIWander/AI-Hands/<version>/` | Per-version winget v1.6 manifests pending submission to `microsoft/winget-pkgs`. Source of truth until merged. |
| `scoop/ai-hands.json` | Scoop manifest for users on the Scoop ecosystem. Can be installed directly from the URL or via a bucket. |
| `scripts/Register-HandsClients.ps1` | Post-install helper: writes the absolute `hands.exe` path into Claude Desktop (classic and MSIX), Claude Code, Codex, Gemini CLI, Grok CLI, and Cursor. Backs up first. Skips a host that already has a hands entry unless `-Force`. |
| `scripts/register-hands.ps1` | Forwards to `Register-HandsClients.ps1`. |
| `scripts/Initialize-HandsOnboarding.ps1` | Optional Tailscale install and user sign-in. Declining it prints an authenticated Cloudflare Tunnel or ngrok flow. No public URL by default. |
| `payload/` | `README.md`, `AGENTS.md`, and `skills/hands-getting-started/SKILL.md` copied into the Hands home. |
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

After winget/Scoop drops `hands.exe` onto PATH, register it with every installed stdio host:

```powershell
# Skip hosts that already have a hands entry
.\installers\scripts\Register-HandsClients.ps1

# Replace existing entries
.\installers\scripts\Register-HandsClients.ps1 -Force

# Show actions without writing
.\installers\scripts\Register-HandsClients.ps1 -DryRun
```

`register-hands.ps1` forwards to the same script. A `.bak.<yyyyMMdd-HHmmss>` file is written beside each config before that config is changed. ChatGPT desktop is not registered; see [`docs/chatgpt-desktop-mcp.md`](../docs/chatgpt-desktop-mcp.md).

Optional Tailscale setup, with the user signing in themselves:

```powershell
.\installers\scripts\Initialize-HandsOnboarding.ps1
```
