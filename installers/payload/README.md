# AI-Hands

AI-Hands is a local stdio MCP server. It gives an AI host browser, Windows UI Automation, and vision/OCR tools on this machine.

This folder is the Hands home. It is resolved at runtime:

1. `HANDS_HOME`, if set
2. `CPC_ROOT`, only as a back-compat alias for the same directory
3. `%LOCALAPPDATA%\AIWander\Hands` on Windows

Nothing in this install is tied to a particular computer name, user name, or IP address.

## Register AI hosts

From the repository or the installed scripts directory:

```powershell
.\Register-HandsClients.ps1
```

The script writes the absolute path of `hands.exe` into each host that is installed and does not already have a hands entry. It backs up a config before changing it. Hosts:

- Claude Desktop, including the Microsoft Store / MSIX config path
- Claude Code (`%USERPROFILE%\.claude.json`, or `%CLAUDE_CONFIG_DIR%\.claude.json`)
- Codex CLI and Codex IDE (`%USERPROFILE%\.codex\config.toml`)
- Gemini CLI (`%USERPROFILE%\.gemini\settings.json`)
- Grok CLI (`%USERPROFILE%\.grok\config.toml`, or `%GROK_HOME%\config.toml`)
- Cursor (`%USERPROFILE%\.cursor\mcp.json`)

ChatGPT desktop is not registered here. It does not launch a local stdio server. See `docs/chatgpt-desktop-mcp.md` in the AI-Hands repo.

## First session

The server's MCP `instructions` tell the model to call `hands_setup_status` first. That tool reports this folder, which hosts are configured, whether Tailscale is installed and logged in, and this machine's MagicDNS name read live from `tailscale status --json`. It does not store that name.

For Tailscale install and sign-in, or an authenticated Cloudflare Tunnel / ngrok fallback, run:

```powershell
.\Initialize-HandsOnboarding.ps1
```

You sign in to Tailscale yourself. The script never creates or stores an auth key. It does not open a public URL.

## Guidance for the model

- `AGENTS.md` in this folder
- `skills/hands-getting-started/SKILL.md`
