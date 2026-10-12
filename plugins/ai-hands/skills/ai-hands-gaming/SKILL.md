---
name: ai-hands-gaming
description: >
  Play Windows desktop games and browser/online games with AI-Hands: game-genre
  selection, input paths that work when UIA cannot see the game, the
  see-decide-act-verify loop, and how to work with the enforcement hook's
  destructive-word and window-list gates during a sanctioned game session. Use when
  the user asks to play, demo, or automate a game, or says /ai-hands-gaming.
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

## Enforcement-hook interplay (sanctioned game sessions)

The enforcement hook is active during games; three gates matter:

- **Commerce-word gate**: everyday UI words (Confirm, Cancel, Reset, Discard) do
  not trip the hook. Commerce words do: buttons labeled Buy, Sell, Purchase, Pay,
  Transfer are denied unless the input carries `allow_destructive: true`. Use the
  marker only for in-game purchases with play currency, inside the game window.
  Real-money flows stay with the human, and payment-information entry (card/bank
  numbers, CVV, expiry, billing fields) is hard-blocked with no override — hand
  control to the user for that step.
- **`uia_list_window` cooldown** (45 s): find the game window once with
  `uia_focus_window(title=...)` and keep acting inside it; do not re-enumerate
  windows each turn. If a session genuinely needs rapid re-lists, set
  `HANDS_ALLOW_UIA_LIST=1`.
- **Verify streak**: verification reads (screenshots, `vision_diff`, DOM reads)
  clear it automatically — the loop above satisfies it for free.

## Session preflight (do once, before the first move)

1. Confirm the hands server is connected (Grok CLI: `/mcps`, retry with `r` if
   hands is missing — server startup is nondeterministic).
2. Focus or launch the game (`uia_app_launch` / `browser_navigate`), switch it to
   windowed/borderless, and keep its window on the primary display at 100% scaling
   when clicks go through `uia_click(x, y)`.
3. Take one capture and prove you can read the board state; say what you see
   before acting.
4. State the input path chosen (DOM / canvas-vision / desktop-vision) so failures
   are diagnosable.
