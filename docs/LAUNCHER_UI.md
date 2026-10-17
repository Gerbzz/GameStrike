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

- **Splash** with the GameStrike mark and PWDS credit while the library loads (at least 650 ms). The publisher logo appears again only in About.
- **Top bar**: compact mark and name, search, sort (recently played, title A–Z or Z–A, recently added), Add game, About.
- **Grid** of game cards. Covers are generated from each game's ID: a decorative gradient and pattern with the title's initials, not the game's own art. Badges show *Playing* and *Files missing*.
- **Details panel**: source, date added, last played and play count, size of saved data, controller and network support (both shown as *Not checked*), Play offline, Play with network…, Rename, Clear saves, and Remove.
- **Status bar**: keyboard and controller hints, plus the game currently playing.
- **Dialogs**: network consent (focus starts on *Go back*), Manage, Rename, Clear saves, Remove (with *Keep saves* checked by default), unreadable library folders, and About.

The empty library shows a welcome panel with a single Add button. Layouts were checked at 1250 × 800 and at the 880 × 620 minimum, where the Play button stays visible. Reduced-motion and forced-colors preferences are respected.

## Input

| Action | Keyboard | Controller (standard mapping) |
| --- | --- | --- |
| Move through the grid | Arrow keys, Home, End | D-pad or left stick (repeats while held) |
| Play offline | Enter, or double-click a card | A |
| Play with network | N | X, then move to *Allow* and press A |
| Manage | F2 (rename), Delete (remove) | Y |
| Back or close | Esc | B |
| Search | Ctrl+F or / | — |

Controller buttons fire once per press, so a held A cannot confirm a dialog that opens under it. Pad input is ignored while a game is open or the launcher is not focused. Launcher navigation does not mean a game supports controllers.

## Logic tests

`ui/core.js` holds the pure parts: filtering and sorting, grid movement, controller edge detection and repeat, generated covers, and formatting. `node --test ui/app.test.cjs` covers them. The DOM behaviour in `ui/app.js` is checked by running the real launcher; see the README.
