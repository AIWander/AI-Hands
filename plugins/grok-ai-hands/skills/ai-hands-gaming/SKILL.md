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
   `browser_type` by selector. Check with `browser_get_clickables` — if the board
   elements appear, stay in the DOM path.
2. **Canvas browser games** (one `<canvas>`, no per-cell DOM): vision path inside
   the browser window — `hands_capture` (window) or `browser_screenshot`, find the
   target with `vision_find_template` / `vision_ocr`, then click coordinates with
   `uia_click(x, y)` on the browser window. `browser_evaluate` can read JS game
   state when exposed (window.game, localStorage) — cheaper than pixels.
3. **Windows desktop games**: try `uia_find` once; most DirectX/fullscreen games
   are invisible to UIA. Then use the vision path: `hands_capture` with OCR,
   template match, `uia_click(x, y)`, `uia_key_press` / `uia_hold_key` for
   movement keys, `drag` for camera or piece movement.
4. **Run the game in windowed or borderless-windowed mode**, never exclusive
   fullscreen: screenshots stay reliable, coordinates stay stable, and the game
   keeps rendering when focus flickers.

## The loop

Repeat: capture (`hands_capture` / `browser_screenshot`) -> read state (OCR /
template / DOM / JS) -> decide one move -> act (single click or key) -> verify the
board changed (`vision_diff`, `wait_for_visual`, or a re-read). Never fire blind
action bursts; batch only verified-stable sequences with `hands_script` /
`uia_batch`.

## Enforcement-hook interplay (sanctioned game sessions)

The enforcement hook is active during games; three gates matter:

- **Destructive-word gate**: game buttons labeled Buy, Sell, Confirm, Cancel,
  Reset, Discard trip the PreToolUse deny. When the user has sanctioned the game
  session and the click stays inside the game, include `allow_destructive: true`
  in the tool input. Never blanket-apply it outside the game window; real-money
  purchase flows still require the human.
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
   windowed/borderless.
3. Take one capture and prove you can read the board state; say what you see
   before acting.
4. State the input path chosen (DOM / canvas-vision / desktop-vision) so failures
   are diagnosable.
