# GameStrike launcher

`gamestrike-launcher` is a Tao/Wry window that shows only bundled files from `ui/`. Imported game HTML never loads in it.

## Trusted surface

The fixed `gamestrikeui` origin serves exactly six bundled files: `index.html` (also at `/`), `app.js`, `core.js`, `styles.css`, the GameStrike mark, and the PWDS publisher logo. Every response carries `default-src 'none'` with only `'self'` scripts, styles, and images, plus `connect-src 'none'` and `nosniff`. Navigation is limited to this origin; new windows, downloads, devtools, and the default context menu are disabled.

The page can send only these IPC messages, as JSON of at most 1 KiB from the exact UI origin:

| Message | Rust action |
| --- | --- |
| `ready` | Send the library scan |
| `pick_and_import` | Open the native file picker; the page never supplies a path |
| `launch_game {id, network}` | Validate the UUID and start `gamestrike-player --offline` or `--allow-network`; refused while another game is open |
| `rename_game {id, title}` | Rename after trimming and length checks |
| `reset_saves {id}` | Delete that game's browser profile |
| `remove_game {id, keep_saves}` | Remove the game's copy, optionally keeping saves |

Unknown message types and unknown fields are rejected. Game titles reach the page as JSON and are rendered with `textContent`.

## Layout

The launcher follows the classic console-dashboard pattern: channels stacked vertically, and a row of tiles per channel.

- **Splash**: the GameStrike mark with the PWDS credit while the library loads (at least 1.1 s). The publisher logo appears again only in About.
- **Backdrop**: a themed sky fading into light haze, slow bokeh circles, and a curved glossy floor, all in CSS. Five themes (Lime by default, Ocean, Sunset, Grape, Graphite) only change custom properties.
- **Channels**, top left: *Jump Back In* (only after something has been played), *Add & Find*, *My Games* (where the launcher opens), and *System*. The current channel is large, with a square marker; earlier channels fade upward above it. Up/Down or a click switches channel.
- **Tiles**: the focused tile is full size on the left, and later tiles shrink and overlap toward the right (`core.tileLayout`), with spring transitions and a soft reflection. Game tiles use a generated color and the title's initials, which are decoration, not the game's art, plus the title, source, last-played time, and *Playing* or *Files missing* badges. Other tiles carry an icon disc: Add a game, Find a game, Sort, Show all games, Sounds, Theme, About, and unreadable folders when there are any.
- **Under the row**: an *N of M* counter, and for games, plays, game-data size, and date added.
- **Profile corner**, top right: the GameStrike mark, the game count or the game now playing, and the time. The mark repeats bottom right with a ripple.
- **Hints**, bottom left: A/X/Y/B glyphs that change with the focused tile (*Play offline* or *Select*), plus keyboard shortcuts on wide windows.
- **Dialogs** are light glossy blades: Search, network consent (focus starts on *Go back*), Manage (which also states that controller and network needs are not checked), Rename, Clear saves, Remove (with *Keep saves* checked by default), unreadable folders, and About. Focused dialog buttons always show a ring.
- **Sounds**: short tones synthesized with Web Audio (nothing is fetched), on by default, toggled in *System*, and remembered.

Captures from the real launcher at 1250 × 800 and at the 880 × 620 minimum are in the README.

## Input

| Action | Keyboard | Controller (standard mapping) |
| --- | --- | --- |
| Move along a channel | Left/Right, Home, End, mouse wheel | D-pad or left stick left/right (repeats while held) |
| Switch channel | Up/Down | D-pad or left stick up/down |
| Play offline, or select a tile | Enter or Space; click the focused tile; double-click a game | A |
| Play with network | N | X, then move to *Allow* and press A |
| Manage | F2 (rename), Delete (remove) | Y |
| Back or close (returns to *My Games*, then clears a search) | Esc or Backspace | B |
| Search | Ctrl+F or / | *Find a game* tile |

Controller buttons fire once per press, so a held A cannot confirm a dialog that opens under it. Pad input is ignored while a game is open or the launcher is not focused. Launcher navigation does not mean a game supports controllers.

## Logic tests

`ui/core.js` holds the pure parts: filtering and sorting, grid and shelf movement, the perspective tile layout, controller edge detection and repeat, generated covers, and formatting. `node --test ui/app.test.cjs` covers them. The DOM behaviour in `ui/app.js` is checked by running the real launcher; see the README.
