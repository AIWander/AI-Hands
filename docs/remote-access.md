# Remote access

Hands is a stdio MCP server. Other programs on the same machine talk to it through the host config written by `Register-HandsClients.ps1`. Reaching it from another device is optional.

## Tailscale

`Initialize-HandsOnboarding.ps1` can install Tailscale:

- `winget install --id Tailscale.Tailscale` when winget is available
- otherwise `msiexec /i <msi-you-supply> /quiet /norestart`

Either path is one UAC prompt. The script does not download an MSI on its own.

Sign-in is yours:

- Run `tailscale up` and open the URL it prints, or
- Pass an auth key you already created: `Initialize-HandsOnboarding.ps1 -AuthKey <key>`

The key is an argument for that process only. It is not written to a file, a log, or the Hands home. Prefer the URL, because shell history can retain process arguments.

The MagicDNS name is read from `tailscale status --json` when a tool or the script needs it. It is not saved. Peer names and Tailscale addresses are not recorded.

Tailscale Funnel is not turned on. A tailnet-only machine is the default.

## If you decline Tailscale

The onboarding script prints one of these authenticated alternatives. It does not run them.

### Cloudflare Tunnel

1. `cloudflared tunnel login`
2. `cloudflared tunnel create hands`
3. Point a hostname you control at `http://127.0.0.1:<port>` only.
4. Require Cloudflare Access on that hostname.

Do not use `cloudflared tunnel --url`. That command allocates a public quick-tunnel URL.

### ngrok

```text
ngrok http 127.0.0.1:<port> --oauth=google
```

or `--basic-auth`. Do not run `ngrok http` with no auth flag.

A future stdio-to-HTTP bridge must still require its own bearer token. Tunnel auth is a second layer, not a replacement. See [chatgpt-desktop-mcp.md](chatgpt-desktop-mcp.md).
