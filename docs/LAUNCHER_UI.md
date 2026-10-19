# GameStrike launcher

`gamestrike-launcher` is a Tao/Wry window that shows only bundled files from `ui/`. Imported game HTML never loads in it.

## Trusted surface

The fixed `gamestrikeui` origin serves only the files in the `UI_FILES` table in `src/launcher.rs`: `index.html` (also at `/`), `app.js`, `core.js`, `scene.js`, `styles.css`, the GameStrike mark, the PWDS publisher logo, the vendored three.js modules under `/vendor/three/`, and the Orbitron and Exo 2 fonts under `/fonts/`. It also serves `/cover/<game uuid>.png`, but only for a valid game ID, and only as a regular PNG file of at most 8 MiB that GameStrike itself encoded into that game's library folder. Every response carries `default-src 'none'` with only `'self'` scripts, styles, images, and fonts, plus `connect-src 'none'`, `nosniff`, and `no-store`. Navigation is limited to this origin; new windows, downloads, devtools, and the default context menu are disabled. A test checks that every file in the table is served and that every ES module import resolves to a file in it.

The page can send only these IPC messages, as JSON of at most 1 KiB from the exact UI origin:

| Message | Rust action |
| --- | --- |
| `ready` | Send the library scan |
| `pick_and_import` | Open the native file picker; the page never supplies a path |
| `launch_game {id, network}` | Validate the UUID and start `gamestrike-player --offline` or `--allow-network`; refused while another game is open |
| `rename_game {id, title}` | Rename after trimming and length checks |
| `reset_saves {id}` | Delete that game's browser data |
| `remove_game {id, keep_saves}` | Remove the game's copy, optionally keeping saves |
| `pick_cover {id}` | Open the native image picker, then decode, scale, and store the cover (see below); the page never supplies a path |
| `undo_cover {id}` / `reset_cover {id}` | Bring back the previous custom cover, or return to the automatic one (the custom one is kept for Undo) |
| `set_favorite {id, favorite}` | Mark or unmark a favorite |
| `set_source {id, source}` | Set the player's note of where the game came from (1 to 32 characters; empty clears it) |
| `quit` | Close the launcher, refused while a game is open |

Unknown message types and unknown fields are rejected. Game titles and sources reach the page as JSON and are rendered with `textContent`.

## Covers

Imported HTML games rarely have cover art, and any image that does come with them is untrusted.

- **Decoding.** Cover images are decoded only by the memory-safe Rust `image` crate (PNG, JPEG, GIF, WebP), with limits of 16 MiB per file, 8192 px per side, and 256 MiB of decoder memory. The result is scaled to fit 600 × 900 and saved as a freshly encoded PNG; nothing from the original file except its pixels is kept. SVG, HTML, and anything else are refused.
- **Automatic cover.** When a game is imported (and once for games imported earlier), GameStrike looks among the game's own top-level files for a well-known name such as `cover.png`, `thumbnail.png`, `screenshot.png`, `icon.png`, or `favicon.png`, and runs the first readable one through the same decoder. The search runs once per game.
- **Custom cover.** *Set cover image…* in the detail panel opens the native image picker. *Undo cover change* swaps back to the previous custom cover; *Use automatic cover* puts the custom one aside (Undo restores it).
- **Generated cover.** A game with no stored cover gets one drawn from its ID and title: two hues, one of four patterns, and the title's initials. It is clearly generated decoration, not the game's art.
- Covers live in the game's own library folder (`cover.png`, `cover-previous.png`, `auto-cover.png`) and are deleted with the game.

## The 3D scene

`ui/scene.js` renders the backdrop with the bundled three.js (r170, MIT): a deep green void with a faint nebula, a pulsing core inside a glassy shell with three orbiting rings, five translucent energy conduits with travelling pulses, and up to 1600 drifting wisps, all through UnrealBloom post-processing and ACES tone mapping. It is decoration only: it receives no game data, never touches the player, and listens to the page for just two events, where the camera should go (`gamestrike:view`) and whether to pause (`gamestrike:pause`).

- **Camera.** The camera eases between stations for the startup, library, a game's detail, saves dialogs, settings, and About. The DOM stage moves with it in CSS 3D (the library pushes back and turns as the detail blade flies in from depth), and the pointer adds a slight parallax.
- **Quality.** Four steps (pixel ratio, wisp count, bloom on or off and its resolution). After every 90 frames, if frames averaged more than 20 ms, the scene drops a step, and on the lowest two steps the glass panels stop blurring what is behind them. Rendering stops while a game is running and while the window is hidden.
- **Reduced motion.** Still frames only: no camera flights, parallax, or animation, and every CSS transition is instant.
- **Fallback.** Without WebGL 2 (or if the context is lost), a static 2D painting of the same composition replaces the canvas. If `scene.js` cannot load at all, a CSS glow stands in. The UI is the same in all three cases. Launcher captures print which renderer was used.

## Layout

The look is an original homage to the 2001 console dashboard (a dark 3D space with a glowing core and conduits, green glass panels with chrome rims, panels that fly in with depth, wide techno headings) combined with a cover-grid game launcher. It uses no console maker's logos, fonts, sounds, or artwork; the GameStrike mark appears in the startup animation and About.

- **Typography.** Orbitron (SIL OFL) for headings, labels, and counts; Exo 2 (SIL OFL) for text. Both are bundled.
- **Sidebar** (left, flies in on start): search at the top, then filter rows with an icon, an uppercase label, and a count: *All games*, *Recently played*, *Favorites*; *Imported as* HTML files or ZIP archives (only the kinds present); and *Sources*, one row per source the player wrote. There are no rows for stores GameStrike does not support. *Add game*, *Settings*, and *Power* are pinned at the bottom. The sidebar collapses to icons with its button or `[`, and does so automatically below 760 px.
- **Grid**: a dense grid of 2:3 portrait covers that reflows with the window (`auto-fill`, 118 px tiles on narrow windows, 150 px normally, 180 px on very wide ones). Each tile shows its cover under a glass sheen, its title, and its source or kind with last-played time, plus badges for *Favorite*, *Playing*, and *Files missing*. The focused tile lifts toward the camera in CSS 3D with a chrome and green-glow rim and a repeating light sweep; its row and column neighbours recede slightly and the rest of the wall a little more.
- **Header**: the current filter or search, the number of games, and a sort chip (Recent, Title A–Z, Title Z–A, Newest; Recently played is always newest first).
- **Detail panel** (I, Y, or right-click): flies in from depth on the right with the cover, facts (imported as, source, last played, plays, game data, added, cover kind, and that controller and network needs are not checked), and actions: Play offline, Play with network…, favorite, Set cover image…, Undo cover change, Use automatic cover, Set source…, Rename, Clear saves, Remove from library, and Back.
- **Dialogs** reuse the same flows as before: network consent (focus starts on *Go back*), rename, source, clear saves, remove (with *Keep saves* checked by default), settings (sounds, unreadable folders, About), and About.
- **Sounds**: short sounds synthesized at run time from oscillators and filtered noise (move, change zone, select, open, back, launch, error). No samples. On by default, but the audio context starts only after the first key, click, or button press; M or Settings mutes them, and the choice is remembered.
- **Captures**: `--capture-view` accepts only a fixed list of names: library, focus, long, launch, manage (the detail panel), about, favorites, collapsed, search, and boot. Captures use default preferences, put the camera at its station, render still frames, and never save preferences.

## Input

| Action | Keyboard | Controller (standard mapping) |
| --- | --- | --- |
| Move in the grid | Arrows, Home, End | D-pad or left stick (repeats while held) |
| Move between the grid and the sidebar | Left from the first column; Right from the sidebar | Same |
| Play offline, or choose a filter or sidebar item | Enter or Space; click the focused tile; double-click a game | A |
| Play with network | N | X, then move to *Allow* and press A |
| Details | I, the context-menu key, or right-click | Y |
| Favorite | F | In the detail panel |
| Rename, remove | F2, Delete | In the detail panel |
| Search | / or Ctrl+F; Enter or Down returns to the grid | Search row in the sidebar |
| Collapse the sidebar | [ | Collapse button |
| Mute sounds | M | Settings |
| Back (closes the panel, leaves the sidebar, clears the search, then the filter) | Esc or Backspace | B |

Controller buttons fire once per press, so a held A cannot confirm a dialog that opens under it. Pad input is ignored while a game is open or the launcher is not focused. Every tile, filter, and button has a visible focus ring and a screen-reader label; launcher navigation does not mean a game supports controllers.

## Logic tests

`ui/core.js` holds the pure parts: filtering and sorting, the sidebar filters built from library data, the source rule (shared with Rust), cover URLs, grid and list movement, controller edge detection and repeat, generated covers, and formatting. `node --test ui/app.test.cjs` covers them. The DOM behaviour in `ui/app.js` and the 3D scene are checked by the launcher captures in CI; see the README.
