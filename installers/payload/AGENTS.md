# AI-Hands agent notes

This file ships in the Hands home (`HANDS_HOME`, otherwise `CPC_ROOT`, otherwise `%LOCALAPPDATA%\AIWander\Hands`).

1. Call `hands_setup_status` before any other Hands tool.
2. Use the install location, host list, Tailscale state, and MagicDNS name from that response. Do not guess paths, hostnames, or IP addresses.
3. Do not invent a Tailscale auth key. The user signs in, or passes their own key to `Initialize-HandsOnboarding.ps1` for that invocation. Hands does not store keys.
4. If a host has no hands entry, tell the user to run `Register-HandsClients.ps1`. Do not duplicate an existing hands entry.
5. ChatGPT desktop needs a remote HTTPS MCP endpoint. Follow the authenticated tunnel plan. Do not enable Tailscale Funnel or an unauthenticated public link.
6. Keep `HANDS_TOOL_PROFILE=default` unless the user asks for another safe profile.
7. When a sign-in, captcha, UAC prompt, or unexpected dialog blocks you and the owner is away, call `live_screen_request_help`. Do not type passwords or other secrets for the user.

Read `skills/hands-getting-started/SKILL.md` in this folder when choosing a setup or routing step.
