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

The launcher is a console-style dashboard.

- **Splash**: the GameStrike mark in a glowing ring with the PWDS credit, shown while the library loads (at least 0.9 s). The publisher logo appears again only in About.
- **Ambient background**: slow drifting light and a sweeping highlight, in pure CSS. It is turned off for reduced-motion and forced-colors users.
- **Top bar**: compact mark and name, search, sort (recently played, title A–Z or Z–A, recently added), Add game, interface-sound toggle, About, and a clock.
- **Hero panel** for the selected game: a large generated cover, the title, detail chips (source, plays, game-data size, and controller and network support as *not checked*), and actions with controller glyphs: **A** Play offline, **X** Play with network…, **Y** Manage.
- **Shelves** of tiles that scroll sideways. The selected tile springs forward with a glow. A *Jump back in* shelf appears above *Your library* for libraries of more than six games. Covers are generated from each game's ID (gradient, pattern, and initials), not the game's own art. Badges show *Playing* and *Files missing*.
- **Status bar**: A/B/X/Y and keyboard hints, plus the game currently playing.
- **Dialogs**: network consent (focus starts on *Go back*), Manage, Rename, Clear saves, Remove (with *Keep saves* checked by default), unreadable library folders, and About. Focused dialog buttons always show a ring, because controller users never trigger the browser's focus-visible heuristics.
- **Sounds**: short tones synthesized with Web Audio (nothing is fetched), on by default, toggled from the top bar and remembered.

The empty library shows a welcome panel with a single Add button. Layouts were captured from the real launcher at 1250 × 800 and at the 880 × 620 minimum; see the README.

## Input

| Action | Keyboard | Controller (standard mapping) |
| --- | --- | --- |
| Move along a shelf, or between shelves | Arrow keys, Home, End | D-pad or left stick (repeats while held) |
| Play offline | Enter, or double-click a card | A |
| Play with network | N | X, then move to *Allow* and press A |
| Manage | F2 (rename), Delete (remove) | Y |
| Back or close | Esc | B |
| Search | Ctrl+F or / | — |

Controller buttons fire once per press, so a held A cannot confirm a dialog that opens under it. Pad input is ignored while a game is open or the launcher is not focused. Launcher navigation does not mean a game supports controllers.

## Logic tests

`ui/core.js` holds the pure parts: filtering and sorting, grid and shelf movement, controller edge detection and repeat, generated covers, and formatting. `node --test ui/app.test.cjs` covers them. The DOM behaviour in `ui/app.js` is checked by running the real launcher; see the README.
