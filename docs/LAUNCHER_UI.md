# GameStrike launcher

`gamestrike-launcher` is a Tao/Wry window that shows only bundled files from `ui/`. Imported game HTML never loads in it. Besides HTML games, it lists the games installed with other store launchers and programs the player adds, and starts them through their own launchers (see [Store libraries](#store-libraries)).

## Trusted surface

The fixed `gamestrikeui` origin serves only the files in the `UI_FILES` table in `src/launcher.rs`: `index.html` (also at `/`), `style-gate.js`, `app.js`, `core.js`, `frame.js`, `covers.js`, `scene.js`, `styles.css`, `classic.html`, `classic.js`, `classic.css`, the GameStrike mark, the PWDS publisher logo, the vendored three.js modules under `/vendor/three/`, and the Nova Square, Rajdhani, and Share Tech Mono fonts under `/fonts/`. It also serves `/cover/<game uuid>.png`, but only for a valid game ID, and only as a regular PNG file of at most 8 MiB that GameStrike itself encoded into that game's library folder (`games/<id>/` for HTML games, `stores/<id>/` for store games). Every response carries `default-src 'none'` with only `'self'` scripts, styles, images, and fonts, plus `connect-src 'none'`, `nosniff`, and `no-store`. Navigation is limited to this origin; new windows, downloads, devtools, and the default context menu are disabled. A test checks that every file in the table is served and that every ES module import resolves to a file in it.

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
| `refresh_stores` | Rescan the store libraries in the background (ignored while a rescan runs) |
| `launch_store_game {id}` | Start a store game or custom program by ID; what to start comes only from GameStrike's own catalog and is checked again (see below) |
| `open_install_folder {id}` | Open the game's install folder, as recorded by the scan, in the system file manager |
| `add_program` | Open the native picker for a program, shortcut, or app; the page never supplies a path |
| `set_program {id, args}` | Set a custom program's arguments (at most 600 characters, split into a list by GameStrike) |
| `pick_working_folder {id}` | Open the native folder picker for a custom program's working folder |
| `remove_program {id}` | Forget a custom program (the program itself is not touched) |
| `set_hidden {id, hidden}` | Hide or show a store game |
| `set_store_enabled {store, enabled}` | Turn a store's scanning and listing on or off |
| `quit` | Close the launcher, refused while a game is open |

`rename_game`, `pick_cover`, `undo_cover`, `reset_cover`, and `set_favorite` also work for store games; a rename there only changes GameStrike's title, and an empty name goes back to the store's.

Unknown message types and unknown fields are rejected. Game titles and sources reach the page as JSON and are rendered with `textContent`.

## Store libraries

GameStrike finds the games other launchers have installed on this computer and lists them beside HTML games. Detection is **read-only and local**: it reads each launcher's own install records, never signs in, never reads store credentials, tokens, or cookies, never uses the network, and never writes to a store's folders or registry keys.

| Store | Where GameStrike looks | How the game starts | Platforms |
| --- | --- | --- | --- |
| Steam | The Steam folder (registry `SteamPath` on Windows; `~/.steam/steam`, `~/.local/share/Steam`, or the Flatpak folder on Linux; `~/Library/Application Support/Steam` on macOS), every library in `steamapps/libraryfolders.vdf` (current and older formats), and each `appmanifest_*.acf` with the installed flag (`StateFlags` bit 4). Redistributables, runtimes, and Proton are skipped. Cover: Steam's own `appcache/librarycache` art (flat `<appid>_library_600x900.jpg` or the newer per-app folders). | `steam://rungameid/<appid>` | Windows, Linux, macOS |
| Epic Games | `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests\*.item` (JSON). Add-ons, engines, and incomplete installs are skipped. | `com.epicgames.launcher://apps/<namespace>%3A<catalogItemId>%3A<AppName>?action=launch&silent=true` (the form Playnite's Epic plugin uses), or the AppName form when the catalog IDs are missing | Windows |
| GOG Galaxy | `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\<id>` (`gameName`, `path`, `exe`); add-ons (`dependsOn`) are skipped | `GalaxyClient.exe /command=runGame /gameId=<id> /path=<folder>` when Galaxy is installed, else the game's own program with its `launchParam` and working folder | Windows |
| Ubisoft Connect | `HKLM\SOFTWARE\WOW6432Node\Ubisoft\Launcher\Installs\<id>` (`InstallDir`); the title from the `Uplay Install <id>` uninstall entry, else the folder name | `uplay://launch/<id>/0` | Windows |
| EA app | **Best effort.** The EA app keeps its install list encrypted, so GameStrike reads the registry keys EA games write (`EA Games`, `Electronic Arts`, and uninstall entries published by Electronic Arts) and each game's `__Installer\installerdata.xml` for its content IDs. Games without a content ID are skipped. | `origin2://game/launch?offerIds=<ids>&autoDownload=1`, which the EA app handles | Windows |
| Battle.net | Uninstall entries published by Blizzard Entertainment; the product comes from `--uid=` in the uninstall command | `Battle.net.exe --exec="launch <code>"` for products in a known table (WoW, Diablo III and IV, Overwatch, StarCraft, Hearthstone, Call of Duty titles, and others), `battlenet://<code>` if Battle.net's program is not found, or Battle.net itself for unknown products | Windows |
| Microsoft Store and Xbox app | Packages registered for the current user (`HKCU\...\AppModel\Repository\Packages`) whose folder has a `MicrosoftGame.config` (games built with Microsoft's game kit); the title from that file, the app ID from `AppxManifest.xml`. Older store games without that file are not found. No administrator rights are needed. | `shell:AppsFolder\<PackageFamilyName>!<AppId>` | Windows |
| Custom | A program, shortcut (`.lnk`, `.url`), app bundle, `.desktop` entry, or executable the player picks, with optional arguments and working folder | Shortcuts and `.desktop` entries open through the system; programs start directly | Windows, Linux, macOS |

- **Catalog.** Found games are kept as references (store, store ID, title, install folder, how to start, cover source) in `LIBRARY/stores/catalog.json`. The player's own data (favorite, hidden, title, custom covers, and GameStrike's launch count) lives in `LIBRARY/stores/<game id>/` and is never touched by a rescan. A game that is no longer found is marked *Not installed* and stays listed only if the player favorited, renamed, played, hid, or gave it a cover; otherwise it disappears. The launcher rescans when it starts and on *Refresh store libraries*.
- **IDs.** A store game's ID is derived from its store and store ID (SHA-256), so it stays the same across rescans and never collides with an HTML game's random ID in practice.
- **Covers.** Store art is untrusted like any cover: decoded with the same limits, scaled, re-encoded as PNG, and cached with the source's path, size, and time, so it is decoded again only when it changes.
- **Launching.** The page names a game by ID only. What to start comes from GameStrike's catalog and is checked right before use: links must use one of the store schemes above and contain only URL-safe characters; programs must have absolute paths, and their arguments (at most 32, 1024 characters in all, no NUL) are passed as a list, never through a shell. Links open through `ShellExecuteW` on Windows, `xdg-open` on Linux, and `open` on macOS. GameStrike counts its own launches but does not track store games while they run. HTML games keep their isolated player, unchanged.
- **Settings.** *Settings → Store libraries* turns each store off (it is then neither scanned nor shown) and shows which stores this OS supports.
- **Tests.** Every scanner is unit tested on all three OSes against synthetic fixtures: VDF parsing (escapes, comments, nesting limits), Steam libraries in both formats, tools and partial installs, both cover layouts, Epic manifests and skipped add-ons, a registry snapshot for GOG, Ubisoft, EA, Battle.net, and Microsoft Store packages, the link allow-list, argument splitting, stable IDs, catalog merging with uninstalls and player data, turned-off stores, custom programs, and cover caching. The real Windows registry and the real launch calls are not exercised in CI, and no scanner has been tried against a real store install yet.

## Styles

The launcher has five dashboard styles over the same library and actions, chosen in *Settings* (Collection) or in the *System* channel (the earlier styles), and stored in the WebView profile at `LIBRARY/launcher-ui`:

| Style | Page | What it is |
| --- | --- | --- |
| **Collection: PWDS Orange** (default) | `index.html` | The framed game collection described below, in PWDS orange, with the PWDS logo in the emblem. |
| **Collection: Green** | `index.html` | The same frame in green, with the GameStrike mark in the emblem. Only colour tokens differ. |
| **Blades** | `classic.html` | The Xbox 360-inspired blades dashboard from earlier versions. |
| **Original** | `classic.html` | The 2001-inspired orb and menu style from earlier versions, with PR #9's transitions. |
| **PWDS Orange** | `classic.html` | The blades layout in PWDS orange. |

`style-gate.js` runs first on `index.html` and opens `classic.html` when one of the last three is saved; choosing a Collection style there opens `index.html` again, without replaying a startup sequence. `classic.css` is byte-identical to the stylesheet before this change, `classic.html` differs only in its file references, and `classic.js` only in the style cycle (which now ends at Collection), the capture view names, and one description string. A library opened for the first time uses Collection: PWDS Orange; a library that had saved Blades, Original, or PWDS Orange keeps it.

## The Collection style

The look follows a reference concept the project owner supplied: one thick, bevelled glass-and-metal frame filling the window, inside a glowing dome, styled after the 2001 console dashboard (hard chamfered panels with double edges, a wide square typeface, orange or lime selection, translucent panels, and a warped net on the inside of a sphere). It uses no console maker's logos, fonts, sounds, or artwork; the green variant puts the GameStrike mark in the emblem where the concept had a console logo.

- **Frame** (`frame.js`): drawn as one SVG to the exact window size around the regions the stylesheet lays out. The outer edge, the header tab, the sidebar panel, the grid's well, and the two bottom panels each have a bright outer edge with a glow and an inset second edge; every corner is a 45-degree chamfer, never a radius. Glass inlays on the top bar and the sides, bracket ears and rivets on the sidebar panel, rivets on the right column, and gloss streaks finish it. It reads untransformed layout offsets, so the frame's own 3D drift never distorts it.
- **Emblem and header tab**: a round orb set into the top-left corner (the PWDS logo or the GameStrike mark), then a raised tab with an arrow-cut end carrying the view title (*Game Collection*, a filter, a search, or the game in the detail view) in the square display face, and a small count (*21 games · Recent*, or *2 of 21*), like the reference list's item count.
- **Sidebar**: *Search games* (an inline search field), then one chamfered slab per filter with GameStrike's own line icon and an uppercase label: *All games*, *Recently played*, *Favorites*, one row per store **that has games** (*Steam*, *Epic*, *EA*, *MS Store*, *GOG*, *Ubisoft*, *Battle.net*, *Custom*, in that order, each with a count and an original monochrome glyph: vapour, a shield, a diamond, a bag, an orbit, a hexagon, an arrow, a window; no store logos), *HTML games*, each source the player wrote for an HTML game, and *Hidden* when anything is hidden. Stores without games have no row. The selected filter is a solid accent slab with dark text and a play arrow; keyboard or controller focus is an outlined slab. *Add game*, *Settings*, and *Power* sit in their own panel at the bottom. Arrows mark when the list scrolls.
- **Grid**: a dense grid of portrait covers (176 × 236 frame proportions), seven columns at the reference size and fewer in narrow windows, scrolling vertically. Each cover sits in its own dark, bevelled, chamfered frame with a thin highlight; the selected cover gets a thick, glowing accent frame and lifts toward the camera in CSS 3D, and the others dim slightly. Badges mark favorites, the game that is playing, and missing files.
- **Bottom bar**: on the left, the selected game's title in large letterspaced capitals and its status (*Ready to play · Steam*, *Not installed · Epic Games*, *Files missing · HTML game*, *Playing now*, or *Another game is open*, with the play count) beside the store's glyph; on the right, the prompts for the current view (*A Select, B Back, X Filter, Y Sort* in the grid; *X Network, Y Favorite* in a game's detail) as ringed button glyphs drawn in CSS (green A, red B, blue X, yellow Y), or as keys (Enter, Esc, Tab, S, N, F) after keyboard input. The prompts are clickable. The status lives here, so it is never hidden behind the grid, even at 880 × 620.
- **Launching**: A, Enter, or a click on the selected cover starts the game: a store game through its launcher or program, an HTML game offline in its player.
- **Detail view** (the menu button, I, or right-click): the camera flies forward through the dome, the grid and sidebar recede, and the game's cover appears seated in a 3D cradle: a card with a lit chamfered edge held by corner claws, a collar ring, a long bevelled arm crossing behind it, a strut, and cables trailing out of view, all translucent glass with lit edges. The actions are slabs in the sidebar's panel: *Play offline*, *Play with network…*, favorite, *Set cover image…*, *Undo cover change*, *Use automatic cover*, *Set source…*, *Rename*, *Manage saves*, and *Back*. A dark chamfered info panel at the lower right gives the title in large white capitals and accent labels with white values (source, imported as, plays, last played, game data, added) and what the cover is. The net dims, as the reference's list screen is dimmer than its main menu. B returns to the grid.
- **Saves** (*Manage saves*): the info panel becomes a saves panel with the game data size, what is stored, and *Clear saves* and *Remove from library*; the camera moves a little further. Clearing and removing still ask first.
- **Dialogs**: network consent (focus starts on *Go back*), rename, source, clear saves, remove (with *Keep saves* checked by default), Settings (style, sounds, unreadable folders, About), and About, all as chamfered panels that fly in with depth.
- **Generated box art** (`covers.js`): a game with no stored cover gets an illustrated cover drawn on a canvas. A word in the title picks one of twelve scenes (a harbour with a lighthouse, a forge, a reef, a frozen ridge, a castle, a neon grid, a city, dunes, a ringed planet in space, a forest, mountain ridges, or an emblem); otherwise the game ID does. A palette that suits the scene, a title treatment (block, wide, slanted, or stencil), and its placement come from the ID, and the source or kind is set as a small kicker. The detail view says the cover was drawn by GameStrike from the title. No third-party art is used.
- **Synthetic test data**: a library with a `SYNTHETIC-TEST-LIBRARY` file in its root shows a "Synthetic test library" badge in the header tab. The CI capture scripts mark their libraries this way.
- **Sounds**: short sounds synthesized at run time from oscillators and filtered noise (move, change zone, select, open, back, launch, error), plus a very quiet ambient hum (two low detuned tones and filtered noise under a slow swell) that starts with the first sound and stops while a game runs or the window is hidden. No samples. M or *Settings* mutes them, and the choice is remembered.

### The dome

`scene.js` renders, with the bundled three.js (r170, MIT), the inside of a sphere of radius 30 whose centre sits ahead of and above the camera, so the near wall is close and the far wall recedes. The net is computed per pixel in a shader: 34 latitude rings and 68 meridians around a pole that leans toward the light at the upper left, bent by a smooth two-frequency warp so the cells are irregular, quad-ish, and slightly wavy (the same rule as `core.domeNet`, which the tests cover and the fallback uses). Lines are about 1.3 px with a soft halo near the camera and thinner far away, brighter toward the light, and fade into fog with distance; crowded lines near the pole fade out instead of shimmering. A soft light sits behind the emblem. Glossy tubes (dark glass, saturated rims, collars, white highlights, and a pulsing energy core) wrap over the top right, down the left edge, around the bottom left, and into the right edge. UnrealBloom lights only the bright parts. Rendering stops while a game runs and while the window is hidden; quality steps down automatically when frames take more than 22 ms.

**One camera for both layers.** The CSS perspective is set from the camera's 40-degree field of view, and the frame, the tubes, and the cradle ride with the camera, tilted together by the same slow drift (about ±0.55° of yaw and ±0.35° of pitch). The net, much further away, moves against them, so the frame floats inside the dome. Flights between the library, the detail view, saves, Settings, and About are camera moves: the net slides and turns while the panels recede or swing in on their own depth. The dome also turns very slowly.

**Fallbacks.** Without WebGL 2 (or if the context is lost), the same net is projected from `core.domeNet` onto a 2D canvas with the tubes and the light, the cradle becomes a CSS card, and the frame and panels lie flat. With reduced motion, the scene draws still frames, every transition is instant, and the panels lie flat. The UI works the same in every case. Launcher captures print which renderer drew the backdrop.

## Reference study

The Collection style was measured from frames the project owner supplied for this work: a still of the 2001 console dashboard's main menu, a still of its Music Collection screen, and the "Game Collection" concept (in green, and in PWDS orange, which is the primary design). They were used for measurement only and are not in the repository. Colours below are sampled from those frames; sizes are in each frame's own pixels.

- **Concept, 1672 × 941**: frame margins 20 px left and 22 px right; header tab 50 to 100 px with a cap height of 25 px (2.7 % of the height) and about 0.15 em letter spacing; emblem centre (125, 80), radius about 75; sidebar 48 to 292 px, rows 47 px high on a 54.5 px pitch; grid 305 to 1597 by 112 to 830, seven columns by three rows, 184.6 × 244 px pitch, cover frames 176 × 236, gutters about 9 px; the selected frame about 10 px thick and scaled by about 1.08; bottom bar 835 to 915, with a 22 px title cap height at 0.16 em and a 13 px status line; prompt rings 38 px. Orange: selected fill #fd8e16 to #fe7411 with near-black ink, header #f58c39, row edges #c1703c, interior #090301, frame body #26150d, title #ffcc7e, background #080201 to #120401 with net lines up to #34130a. These are the `--u` = 1 values in `styles.css`; `--u` scales them to the window, with a floor on type size.
- **Main menu, 1112 × 1652 crop**: void #0a1c04 to #152d09; net lines peaking at #243e1a to #375922, about 1.5 px with a halo, cells larger near the viewer and compressing toward the orb; menu slabs 72 px tall on a 200 px pitch with 10 to 12 px chamfers and a raised 13 px tab; selected #6d7c23 to #8b9c30 with #0d2b01 text; unselected fill #193c0c to #1f4410 with a #6f934e to #84ac61 double outline and #a0c164 text; square letterforms with a cap height of 38 px, strokes about a quarter of the cap height, and an m-shaped M. Nova Square, thickened with a 0.045 em stroke, was the closest open-licensed match of about forty fonts compared side by side; Rajdhani matches the concept's labels, and Share Tech Mono the list and info text.
- **Music Collection, 640 × 480**: background #020801 to #051303 (much darker than the main menu, so the net dims per screen), selected row #c1f639, rows #33611e, header #1e5610, info panel #205e0c with #457d2b labels; eight rows of about 30 px.
- **Motion**: camera flights ease exponentially (63 % of the way every 0.42 s, settled in about 1.5 s); panels move over 0.7 to 0.8 s with `cubic-bezier(.16, 1, .3, 1)`; the cradle flies in from deep in the dome over about a second.

## Input

| Action | Keyboard | Controller (standard mapping) |
| --- | --- | --- |
| Move in the grid | Arrows, Home, End | D-pad or left stick (repeats while held) |
| Move between the grid and the sidebar | Left from the first column, Right from the sidebar, or Tab | X |
| Launch the selected game (store games through their launcher, HTML games offline), or choose a filter or sidebar item | Enter or Space; click the selected cover; double-click a cover | A |
| Open a game's details | I, the context-menu key, or right-click | Menu (Start) |
| Play with network (HTML games) | N | X in the detail view, then move to *Allow* and press A |
| Open the install folder (store games) | O in the detail view | X in the detail view |
| Sort (Recent, Title A–Z, Title Z–A, Newest) | S | Y |
| Favorite | F | Y in the detail view |
| Rename, remove | F2, Delete | In the detail view |
| Search | / or Ctrl+F; Enter returns to the grid | *Search games* row |
| Mute sounds | M | Settings |
| Back (leaves saves, the detail view, or the sidebar, then clears the search, then the filter) | Esc or Backspace | B |

Controller buttons fire once per press, so a held A cannot confirm a dialog that opens under it. Pad input is ignored while a game is open or the launcher is not focused. Every cover, row, and button has a visible focus state and a screen-reader label; launcher navigation does not mean a game supports controllers.

## Covers

Imported HTML games rarely have cover art, and any image that does come with them is untrusted.

- **Decoding.** Cover images are decoded only by the memory-safe Rust `image` crate (PNG, JPEG, GIF, WebP), with limits of 16 MiB per file, 8192 px per side, and 256 MiB of decoder memory. The result is scaled to fit 600 × 900 and saved as a freshly encoded PNG; nothing from the original file except its pixels is kept. SVG, HTML, and anything else are refused.
- **Automatic cover.** When a game is imported (and once for games imported earlier), GameStrike looks among the game's own top-level files for a well-known name such as `cover.png`, `thumbnail.png`, `screenshot.png`, `icon.png`, or `favicon.png`, and runs the first readable one through the same decoder.
- **Custom cover.** *Set cover image…* opens the native image picker. *Undo cover change* swaps back to the previous custom cover; *Use automatic cover* puts the custom one aside (Undo restores it).
- Covers live in the game's own library folder (`cover.png`, `cover-previous.png`, `auto-cover.png`) and are deleted with the game.

## Captures

`--capture-view` accepts only a fixed list of names. Collection views: library (the default), green, focus, controller, filters, search, favorites, long, detail, saves, moving (a still frame halfway through the flight from the grid to the cradle), settings, launch, about, and boot. Earlier styles, for which the launcher opens `classic.html`: blades, blades-long, blades-launch, blades-manage, blades-about, system, original, original-games, pwds, boot-blades, boot-original, and boot-pwds. Captures use default preferences and never save them. A Collection capture waits until its fonts, the covers in view, and the 3D scene have drawn (checked every 250 ms, for up to 10 s); `classic.html` has no such check, so the earlier styles are captured exactly as before.

## Logic tests

`ui/core.js` holds the pure parts: filtering and sorting, the sidebar filters built from library data, the source rule (shared with Rust), cover URLs, grid and list movement, controller edge detection and repeat, the style list and its cycle, the Collection scale and column count, the generated box art rules, the seeded random generator, the dome net, and formatting. `node --test ui/app.test.cjs` covers them. The DOM behaviour, the frame, the covers, and the 3D scene are checked by the launcher captures in CI; see the README.
