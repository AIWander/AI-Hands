---
name: ai-hands-gaming
description: >
  Play Windows desktop games and browser/online games with AI-Hands: game-genre
  selection, input paths that work when UIA cannot see the game, the
  see-decide-act-verify loop, and the action boundaries to hold yourself during a
  sanctioned game session in a hookless host. Use when the user asks to play, demo,
  or automate a game, or says /ai-hands-gaming.
metadata:
  short-description: "Game playing with hands"
  product: "AI-Hands"
---

# Playing games with AI-Hands

## Pick a winnable game first

Tool-call latency is 1-5 s per action. Genres that work well: turn-based (chess,
card games, Wordle-likes), puzzle (Minesweeper, Solitaire, 2048), menu-driven and
idle games, board-style browser games. Genres that will not work: twitch/reflex
games (platformers, shooters, rhythm). For a first demo, prefer a turn-based game
with large, high-contrast UI.

## Input paths — decide before the first click

1. **DOM browser games** (buttons/divs move the game): normal `browser_click` /
   `browser_type` by selector. Check with `hands_find` or `browser_get_text` — if
   the board elements appear, stay in the DOM path.
2. **Canvas browser games** (one `<canvas>`, no per-cell DOM): `browser_click(x, y)`
   does not reach the game, because it calls `click()` on `elementFromPoint(x, y)`,
   a synthetic event with no pointer position. Use the desktop vision path on the
   browser window instead (item 3). Never pass `browser_screenshot` coordinates to
   `uia_click`: they are relative to the page viewport, not the screen. Reading JS
   game state (window.game, localStorage) needs `browser_eval`, which only the
   compatibility profile exposes (`HANDS_TOOL_PROFILE=compatibility` plus the raw
   and direct-fetch gates); on the default profile, read the board from pixels or
   `browser_get_text`.
3. **Windows desktop games**: try `uia_find` once; most DirectX/fullscreen games
   are invisible to UIA. Then use the vision path: read the board with
   `hands_capture` and OCR, click visible text with `hands_click(target=...)`, and
   click shapes with `vision_find_template` plus `uia_click(x, y)` at the match
   center. Template matches and captures report pixels on the captured monitor,
   while `uia_click` takes screen coordinates; the two agree only on the primary
   display at 100% scaling, so keep the game there (`hands_find` and
   `hands_click` convert for you). Use `uia_key_press` / `uia_hold_key` for
   movement keys and `drag` for camera or piece movement.
4. **Run the game in windowed or borderless-windowed mode**, never exclusive
   fullscreen: screenshots stay reliable, coordinates stay stable, and the game
   keeps rendering when focus flickers.

## The loop

Repeat: capture (`hands_capture` / `browser_screenshot`) -> read state (OCR /
template / DOM / JS) -> decide one move -> act (single click or key) -> verify the
board changed (`vision_diff`, `wait_for_visual`, or a re-read). Never fire blind
action bursts; batch only verified-stable sequences with `browser_batch` /
`uia_batch`.

## Action boundaries (sanctioned game sessions)

This profile contains no hook code, so nothing intercepts the calls below. These
are boundaries you hold yourself; load `ai-hands-safety` at each one.

- **Commerce actions**: everyday UI words (Confirm, Cancel, Reset, Discard) are
  ordinary game chrome and need no ceremony. Commerce words do: before clicking a
  button labeled Buy, Sell, Purchase, Pay, or Transfer, get action-scoped
  confirmation from the user, even for in-game purchases with play currency.
  `allow_destructive: true` is a tool boolean, not user consent — setting it
  authorizes nothing. Real-money flows stay with the human, and
  payment-information entry (card/bank numbers, CVV, expiry, billing fields) is
  human-only with no marker that makes it yours — hand control to the user for
  that step.
- **Window enumeration**: find the game window once with
  `uia_focus_window(title=...)` and keep acting inside it; do not re-enumerate
  windows each turn. The hook-capable profile rate-limits `uia_list_window` for
  45 s; here that discipline is yours, and repeated enumeration is the classic
  stuck loop.
- **Verification cadence**: nothing here counts your unverified mutations. Keep
  the cadence yourself — verification reads (screenshots, `vision_diff`, DOM
  reads) after each short burst, which the loop above already supplies.

This skill is behavioral guidance and cannot block a tool call. Native host
permissions and the Rust monitor fence remain authoritative.

## Session preflight (do once, before the first move)

1. Confirm the `hands` MCP server is connected in the host's tool list and start a
   fresh session if it is missing — server startup is nondeterministic.
2. Focus or launch the game (`uia_app_launch` / `browser_navigate`), switch it to
   windowed/borderless, and keep its window on the primary display at 100% scaling
   when clicks go through `uia_click(x, y)`.
3. Take one capture and prove you can read the board state; say what you see
   before acting.
4. State the input path chosen (DOM / canvas-vision / desktop-vision) so failures
   are diagnosable.
