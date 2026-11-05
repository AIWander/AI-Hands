# ChatGPT desktop and AI-Hands

Checked against OpenAI's current MCP documentation and the ChatGPT desktop connector model (November 2026).

## What works today

Codex CLI and the Codex IDE launch a local stdio server. `Register-HandsClients.ps1` writes that entry into `%USERPROFILE%\.codex\config.toml`:

```toml
[mcp_servers.hands]
command = "C:\\Users\\<you>\\AppData\\Local\\AIWander\\Hands\\bin\\hands.exe"
args = []

[mcp_servers.hands.env]
HANDS_TOOL_PROFILE = "default"
```

The command value is the absolute path of the installed `hands.exe` on that machine. The example above is only a shape.

ChatGPT desktop (the chat app, including the Windows app) does not launch a local stdio process. Its connector model is a remote MCP server over HTTPS. A hands entry in a stdio config is not read by that app. Some desktop builds have been reported to discover a local server during experiments; that is not the supported configuration, and this installer does not write one.

## Plan: stdio-to-HTTP bridge over the user's tunnel

Hands itself stays a stdio process. A small loopback bridge, not shipped as a default listener, would:

1. Spawn `hands.exe` and speak MCP over stdin/stdout.
2. Expose streamable HTTP only on `127.0.0.1`, never `0.0.0.0`.
3. Require a bearer token from `HANDS_HTTP_BEARER` (or an equivalent user-supplied secret). Refuse to listen when the token is empty.
4. Avoid writing the token, MagicDNS name, or any Tailscale address to logs.

The user then publishes that loopback port through a tunnel they authenticate:

- Tailscale: the bridge is reachable only on the tailnet, at the machine's MagicDNS name read live from `tailscale status --json`. Tailscale Funnel stays off.
- Cloudflare Tunnel: a named tunnel plus Cloudflare Access (one-time PIN or a service token). Not `cloudflared tunnel --url`, which publishes a public quick-tunnel URL.
- ngrok: `ngrok http 127.0.0.1:<port> --oauth=google` or `--basic-auth`. The bridge bearer token is still required. A plain `ngrok http` command is not acceptable.

ChatGPT developer mode then receives the tunnel HTTPS URL and the bearer token. The user creates both. Hands does not.

`Initialize-HandsOnboarding.ps1` prints this choice. It does not start the bridge, install a public tunnel, or store a credential.

## Still open

The loopback bridge is a plan, not a listener in this build. Until it exists, ChatGPT desktop cannot call Hands. Codex on the same machine can.
