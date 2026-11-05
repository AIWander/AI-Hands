# Live screen

Live screen is a tailnet-only page the owner opens on a phone when an agent is stuck. The agent keeps using normal Hands tools. The owner watches a live JPEG of the machine and can tap, scroll, or type.

Use it for a sign-in, captcha, UAC prompt, or any dialog the agent should not guess through. The agent must not type passwords, one-time codes, or other secrets. `live_screen_request_help` starts the page if it is not already running and returns a short message the agent can relay.

## Tools

| Tool | What it does |
|---|---|
| `live_screen_request_help` | Start the page if needed and return the owner message. Pass a short `reason` with no secrets. |
| `live_screen_start` | Open the page. Optional `port` (default `8744`) and `ttl_minutes` (`1` through `1440`). |
| `live_screen_status` | Report the URL, whether a viewer polled recently, and whether the on-screen banner is showing. |
| `live_screen_stop` | Close the listener, invalidate the session token, and hide the banner. |

The URL uses the MagicDNS name read from `tailscale status --json` on that call. The response also includes an IP fallback on the same port. Neither value is compiled into Hands. The session token is random, is part of the URL, and stops working when the session stops.

## What the phone page does

- Refreshes a JPEG about every 1.5 seconds. Quality is about 65. The phone sends its viewport width and the server downscales, capped at 1280 pixels wide.
- Unchanged frames come back as a tiny `unchanged` body or HTTP 304, using an ETag.
- Tap is a left click. The page also has right-click and double-click modes, a batched scroll, and a password-style box for typed text. Typed text is not written to the log.
- Buttons send Enter, Tab, Esc, Backspace, and Ctrl+L. A combo box sends other shortcuts. Win+L and similar lock, run, close, and secure-attention combinations are rejected.
- Clicks are mapped from the JPEG back onto the real desktop, including more than one monitor and per-monitor DPI, and land on pixel centres.
- Only one phone action runs at a time. Typing stops if the foreground window changes mid-string.

## Tailscale

The listener binds this machine's Tailscale IPv4, discovered at runtime. It refuses to start when Tailscale is missing, logged out, or has no address in `100.64.0.0/10`. It never binds `0.0.0.0`, and it does not open Tailscale Funnel or any public URL.

Clients must come from `100.64.0.0/10`. The Host header must be this machine's tailnet address or MagicDNS name. Action posts must be JSON, must send `X-Hands-Live-Action: 1`, and must send an Origin of `http://` plus that host. Responses send `X-Frame-Options: DENY` and `Content-Security-Policy: frame-ancestors 'none'`. Bodies larger than 64 KB are rejected.

A minimal tailnet grant, with your own device names filled in:

```json
{"src":["<owner phone>"],"dst":["<this machine>"],"ip":["tcp:8744"]}
```

Change the port if `live_screen_start` used a different one.

## Windows firewall

Tailscale interfaces are on the Private profile. The program rule for `hands.exe` must allow inbound TCP on that profile. Do not switch the rule to Public, and do not open the port on every profile.

```powershell
$path = (Get-Command hands.exe).Source
New-NetFirewallRule -DisplayName "Hands live screen" -Direction Inbound -Program $path -Protocol TCP -LocalPort 8744 -Profile Private -Action Allow
```

Use the port you actually started. If `hands.exe` is not on `PATH`, pass the executable path `hands_setup_status` reported.

## Banner

While a viewer has polled in the last few seconds, or a phone action just ran, the machine shows a click-through, topmost banner that does not take focus and does not add a taskbar button. The default text is:

```text
Assistant is taking over
Please stand by

I said please
```

`HANDS_LIVE_SCREEN_AI_NAME` replaces `Assistant`. `HANDS_LIVE_SCREEN_BANNER_TEXT` replaces the whole message (`\n` starts a new line). `HANDS_LIVE_SCREEN_DISABLE_BANNER=1` hides it. The window is marked with `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`, so it is not part of the JPEG.

The banner, the capture, and the injected input are Windows-only. The request checks, click mapping, and blocked-combo rules are ordinary Rust and have unit tests.
