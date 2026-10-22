<p align="center">
  <img src="assets/branding/gamestrike-mark.svg" alt="GameStrike mark: two layered game cards with a control pad" width="128" height="128">
</p>

<h1 align="center">GameStrike</h1>

<p align="center"><strong>One console-style launcher for all your games, on Windows, Linux, and macOS.</strong><br>
GameStrike finds the games you have installed with Steam, Epic Games, the EA app, the Microsoft Store, GOG Galaxy, Ubisoft Connect, and Battle.net, lets you add any program and the HTML games you download, and shows them all in a framed game collection floating in a glowing 3D dome, for mouse, keyboard, or controller. It never signs in to a store. HTML games run offline in their own isolated window, with saves kept per game.</p>

<p align="center">
  <a href="#what-works-today">What works</a> ·
  <a href="#screenshots">Screenshots</a> ·
  <a href="#platforms">Platforms</a> ·
  <a href="#build-and-run">Build and run</a> ·
  <a href="#how-games-are-isolated">Isolation</a> ·
  <a href="#verification">Verification</a> ·
  <a href="#limitations">Limitations</a> ·
  <a href="#credits-and-license">Credits</a>
</p>

![GameStrike's default Collection style in PWDS orange: a thick bevelled orange frame with the PWDS logo in a round emblem and a GAME COLLECTION header tab, a sidebar of chamfered filter rows (Search games, All games selected, Recently played, Favorites, then one row per store with games: Steam, Epic, EA, MS Store, GOG, Ubisoft, Battle.net, Custom), a dense grid of synthetic store games and HTML games with generated covers, the selected one in a glowing orange frame, and a bottom bar with Synthetic Ember Tactics, Ready to play · Steam, and A Launch, B Back, X Filter, Y Sort, Details prompts, over a dark warped net with glossy tubes](docs/screenshots/library.png)

> [!IMPORTANT]
> **Early, unreleased software.** There is no installer or signed binary yet; you build it from source. Imported games are untrusted code. Offline mode blocks their network traffic, but the player is **not** an operating-system sandbox. Read [Limitations](#limitations) before running games you don't trust.

## What works today

Everything in this section is implemented in this repository and covered by the checks listed under [Verification](#verification).

**Store libraries**
- **Finds installed games** from Steam (Windows, Linux, macOS), Epic Games, GOG Galaxy, Ubisoft Connect, the EA app (best effort), Battle.net, and the Microsoft Store and Xbox app (Windows), by reading each launcher's own install records on this computer. No store accounts, logins, credentials, or network; nothing is written to the stores' folders. Steam's own cached library art becomes the cover: its portrait art when there is some, else a portrait GameStrike composes from Steam's wide hero art and logo, or its header.
- **Launches them** with A through the store's own launcher link (for example `steam://rungameid/...`) or program, from an allow-list, with arguments passed as a list and never through a shell. GameStrike counts its launches; it does not track play time.
- **Custom programs**: add any program, shortcut, app bundle, or `.desktop` entry, with optional arguments and a working folder.
- Rescans on start and on *Refresh store libraries*. Uninstalled games disappear, or stay marked *Not installed* if you favorited, renamed, played, hid, or gave them a cover; your data is never deleted by a rescan. Hide games, favorite them, set your own covers, rename them, and turn any store off in *Settings → Store libraries*.
- See the [support matrix](docs/LAUNCHER_UI.md#store-libraries) for exactly what is read and how each store's games start.

**HTML games**
- Add a game through the native file picker: a single `.html`/`.htm` file, or a `.zip` with `index.html` at its root or inside one wrapper folder. GameStrike copies the files into its own library; your original download is never modified.
- Re-adding the exact same file is recognized, and the game keeps its saves.
- **PWDS Collection** is the only dashboard: the orange frame, emblem, sidebar, dense cover grid and selected-game bottom bar. Earlier theme preferences migrate to this design without changing sound or library preferences.
- **The dome.** Behind the frame, a real-time WebGL scene (bundled three.js) puts you inside a large sphere whose wall carries an irregular, glowing net that curves around and behind you and fades into fog, lit from behind the emblem, with glossy glass tubes wrapping the frame and bloom on the bright parts. The frame drifts with the camera while the far net shows parallax, and moving to a game's details is a camera flight through the dome.
- **Game details** (A or Enter on a cover): the cover seated in a 3D cradle of translucent glass plates, claws, and cables; an info panel with the source, how it was imported, plays, last played, game data, and when it was added; and *Play offline*, *Play with network…*, favorite, cover, source, rename, and *Manage saves* (game data, clear saves, remove). A removal can keep the saves, so adding the same file again brings them back.
- **Covers.** A game with no art gets illustrated box art drawn by GameStrike: a scene picked from the title (a harbour with a lighthouse for *Pixel Harbor*, a forge for *Ember Forge*), a palette that suits it, and the full title as a cover logo. An image in the game's own files (for example `icon.png` or `thumbnail.png`) becomes its cover automatically, and you can set your own from a local image, then undo or reset it. Every cover image is untrusted: it is decoded by memory-safe Rust code with size limits and re-encoded as a new PNG inside the library.
- **Favorites and sources.** Mark favorites, and note where a game came from (1 to 32 characters); both become sidebar filters. There are no rows or logos for stores GameStrike does not support.
- Search by title, and sort by recently played, title, or date added.
- Problems are reported rather than hidden. One unreadable folder doesn't hide the rest of the library, and a game whose files have disappeared shows *Files missing* and won't launch.

**Playing**
- **Play offline** (the default) opens the game in a separate `gamestrike-player` window. The game's network requests are blocked. When the game closes, the launcher says how many requests the game itself attempted, so you can decide whether it needs network access.
- **Play with network…** asks for consent first; focus starts on *Go back*.
- Each game has its own browser origin and profile, so `localStorage`, IndexedDB, and cookies persist across launches and never mix between games.
- Only one game runs at a time. While it runs, the launcher shows *Playing*, pauses its 3D scene and sounds, and the game can't be removed or cleared.

**Input**
- Mouse, keyboard (arrows, Enter, Tab between the grid and sidebar, `S` sort, `N` network, `F` favorite, `I` details, `F2`, `Delete`, `/` or `Ctrl+F` search, `M` mute, `Esc`), and a standard-mapping controller: D-pad or stick to move, **A** to select (a cover opens its details), **B** back, **X** between the grid and the filters, **Y** sort (favorite in the details). Prompts show ringed A/B/X/Y buttons after controller input and keys after keyboard input. A held button never repeats, so it can't confirm a dialog that opens underneath it. Every control has a visible focus state and a screen-reader label.
- Original interface sounds (move, select, open, back, launch, error) and a quiet ambient hum, synthesized at run time from oscillators and filtered noise; no samples. Mute them with `M` or in *Settings*.
- Reduced motion turns the 3D scene into still frames with no camera moves, lays the panels flat, and makes every transition instant. High-contrast (forced colors) is respected.
- Without WebGL 2, or if the GPU context is lost, the same dome net is painted as a still image and the interface works the same.

**Command line** (`gamestrike`): `import`, `list`, `rename`, `reset-saves`, `remove [--keep-saves]`, `set-source`, `favorite`, `set-cover`, `undo-cover`, `reset-cover`, `add-program`, `refresh-stores --fixtures` (synthetic fixtures, for tests), and `stores`.

## Screenshots

Every launcher image on this page was rendered by the real `gamestrike-launcher` in GitHub Actions through the `--capture-preview` option. The top image, *First launch*, and the store library images come from [Cross-platform checks run 39416805857](https://github.com/Gerbzz/GameStrike/actions/runs/39416805857); the rest come from the earlier [run 39381032589](https://github.com/Gerbzz/GameStrike/actions/runs/39381032589), before store libraries were added. WebView2 renders them on Windows, WebKitGTK under Xvfb on Linux, and WKWebView on macOS. Each Collection capture's log reports that WebGL drew the dome at the "high" quality step on all three systems. Captures render still frames with the camera at its station (the *Camera move* capture freezes the flight halfway), so the drift, the slowly turning net, and the tube pulses are not shown. Frame rates were not measured; CI runners are virtual machines, and the macOS runner crops its captures to 1024 × 645. Every game shown is from a library marked **synthetic test data** (hence the badge), with box art GameStrike drew and test images it generated; no third-party game or art appears. The store games come from synthetic fixtures (`scripts/make-store-fixtures.cjs`): a fake Steam folder, Epic manifests, and a registry snapshot, every game named *Synthetic ...*, with generated stand-ins for Steam's cached art. No real store install was read. They are not mockups.

**Store libraries** (run 39416805857):

| | |
| --- | --- |
| ![The Steam row selected in the sidebar, with five synthetic Steam games, one per cache layout: three portrait covers, a header letterboxed over a blurred copy of itself, and a hero image with a stand-in logo on it](docs/screenshots/stores.png) **A store filter** (Linux, from run [39424987196](https://github.com/Gerbzz/GameStrike/actions/runs/39424987196), whose code differs from the final one only in formatting): each store with games gets a row with its own glyph and count. The fixture gives each Steam game a different cache layout; the third and fifth covers are composed from wide art. | ![Details of Synthetic Ember Tactics: the cover in the cradle and an info panel with Store Steam, the install folder in its real case, plays, last played, starts with Steam, and found; actions include Open install folder and Hide game](docs/screenshots/store-detail.png) **Store game details** (Windows): the store, install folder, and *Open install folder*. |
| ![Add to your collection dialog with Add a program or shortcut, Add an HTML game, Refresh store libraries, and Back](docs/screenshots/add-menu.png) **Add game** (Linux): a program, an HTML game, or a store rescan. | ![Store libraries settings on macOS: Steam on with 5 games, Custom on with 1 game, and the other stores greyed out as Windows only](docs/screenshots/store-settings.png) **Settings → Store libraries** (macOS): each store can be turned off; stores this OS does not support say so. |

| | |
| --- | --- |
| ![Collection style in green: the same frame, sidebar, and grid in green, with the GameStrike mark in the emblem](docs/screenshots/green.png) **Archived Collection: Green** (Linux); this alternative is no longer selectable. | ![The grid with keyboard prompts Enter Select, Esc Back, Tab Filter, S Sort in the bottom bar](docs/screenshots/focus.png) **Keyboard prompts** (Windows): after keyboard input the bar shows keys instead of A, B, X, Y. |
| ![Game details: the cover seated in a translucent orange glass cradle with claws, a collar ring, an arm, and cables, the actions as chamfered slabs on the left, and an info panel with source, imported as, plays, last played, game data, and added](docs/screenshots/detail.png) **Game details** (Linux): the cover in its 3D cradle, with the info panel and actions. | ![The same view halfway through the camera move from the grid: the grid and sidebar fading back while the cradle arrives](docs/screenshots/moving.png) **Camera move** (Windows), frozen halfway from the grid to the cradle. |
| ![Saves panel with game data, what is stored, Clear saves, Remove from library, and Back to game](docs/screenshots/saves.png) **Manage saves** (Linux). | ![The Game jam source filter selected in the sidebar, with its two games in the grid](docs/screenshots/filters.png) **A source filter** (Windows): sources the player wrote become sidebar rows. |
| ![Search results for orbit](docs/screenshots/search.png) **Search** (macOS). | ![Settings panel with Style: Collection: PWDS Orange, Sounds, About, and Back](docs/screenshots/settings.png) **Historical Settings** (Linux); current Settings has no theme selector. |
| ![Network consent dialog over the game's details](docs/screenshots/network-dialog.png) **Network consent** (Windows). Offline is the default. | ![Startup: the GameStrike mark in a glowing orange core inside rings, with GAMESTRIKE by PWDS](docs/screenshots/boot.png) **Startup** (Linux), near its end. |
| ![Empty library: No games found yet, a note that GameStrike looked for games from Steam, Epic Games, the EA app, the Microsoft Store, GOG Galaxy, Ubisoft Connect, and Battle.net, and an Add a game button](docs/screenshots/empty-library.png) **First launch** (Windows), with no store games found. | ![Library at the 880 by 620 minimum window size, five columns, with the status line in the bottom bar](docs/screenshots/minimum-880x620.png) **Minimum window size**, 880 × 620 (Linux). |
| ![About dialog with the GameStrike mark, the PWDS publisher logo, and the font and three.js credits](docs/screenshots/about.png) **About** (macOS). | ![Player window titled Save Counter (Synthetic Test), GameStrike, Offline, showing localStorage points 2 and IndexedDB opens 1](docs/screenshots/player-offline-save-counter.png) **A game in its own offline player window** (Windows, from the manual run at `c888e7e`; the player is unchanged). |

**The earlier styles**, unchanged and still selectable (same run, Linux):

| | |
| --- | --- |
| ![Blades style: channel names at the top left and a row of receding tiles](docs/screenshots/style-blades.png) **Blades**. | ![Original style: a glowing green orb with menu bars fanned beside it on a green wireframe sphere](docs/screenshots/style-original.png) **Original**, main menu. |
| ![Original style My Games sub-screen with ruled rows and an info box](docs/screenshots/style-original-games.png) **Original**, My Games. | ![PWDS Orange style: the blades layout in orange](docs/screenshots/style-pwds.png) **PWDS Orange**. |

Screenshots of earlier designs are kept in [docs/history](docs/history).

## Platforms

Only platforms that GitHub Actions has built **and** tested, including the real player isolation test, count as supported. See [docs/PLAYER_SECURITY.md](docs/PLAYER_SECURITY.md#per-platform-implementation) for how each guarantee is implemented per OS.

| Platform | Engine | Status | Evidence |
| --- | --- | --- | --- |
| Windows (CI: `windows-latest`, Windows Server 2025 image) | WebView2 | **Supported** | [CI run 39416805857, windows job](https://github.com/Gerbzz/GameStrike/actions/runs/39416805857/job/118607705853): fmt, clippy, Rust and JS tests, real-player isolation test, release build, player smoke test, launcher captures |
| Linux x64 (CI: `ubuntu-latest`, Ubuntu 24.04) | WebKitGTK 4.1 (2.52.6 in CI) | **Supported** | [same run, ubuntu job](https://github.com/Gerbzz/GameStrike/actions/runs/39416805857/job/118607706165): the same steps, with the player and launcher windows under Xvfb |
| macOS 14 or later (CI: `macos-latest`, macOS 26.6 on Apple silicon; 14 and 15 not tested) | WKWebView | **Supported** | [same run, macOS job](https://github.com/Gerbzz/GameStrike/actions/runs/39416805857/job/118607706357): the same steps |

Platform caveats that apply even when a platform is verified:
- **Linux and macOS:** WebRTC is switched off for games in both offline and network mode, because WebKit offers no way to limit its UDP to the proxy. Games that need WebRTC will not work there.
- **macOS:** turning WebRTC off uses WebKit's private peer-connection preference; if a future macOS removes it, the player refuses to start rather than run without it. Each game's saves live in a WebKit data store named after the game ID (outside the library folder), so the library's "game data" size shows 0, and copying a library folder to another Mac does not copy its saves. Needs macOS 14 for per-game data stores; older versions are refused.
- **macOS:** Clear saves deletes every website data type from the game's data store through WebKit's public API; the empty store folder stays. Launcher captures in CI come out at the runner's screen size (1024 × 645), not the requested 1250 × 800.
- **Windows:** in 3 of about 10 Windows CI runs on this branch, the second offline launch of a game failed its first same-origin `fetch()` (blocked by the offline proxy, so nothing leaked). The probe now retries that fetch and logs the attempts; games that fetch their own files may see the same occasional failure. The cause is not yet known.
- **Windows:** WebView2's helper processes can hold a game's profile files open for a moment after the player exits. Clearing saves or removing a game right after closing it retries the delete for up to five seconds (CI hit this twice before the retry was added).
- **All platforms:** the player is not an OS sandbox (no AppContainer, bubblewrap, or App Sandbox). Saves are kept in the engine's own format, so they do not move between operating systems.

## Build and run

Stable Rust with edition 2024 support, plus:

| OS | Prerequisites |
| --- | --- |
| Windows 10/11 | The [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/) (preinstalled on Windows 11) and the MSVC build tools. |
| Linux | WebKitGTK 4.1 and GTK 3 development packages. Debian/Ubuntu: `sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev`. Fedora: `webkit2gtk4.1-devel gtk3-devel libsoup3-devel`. |
| macOS 14+ | Xcode Command Line Tools (`xcode-select --install`). |

```sh
cargo build --locked --release --features launcher --bins
./target/release/gamestrike-launcher        # .\target\release\gamestrike-launcher.exe on Windows
```

The `launcher-windows` / `player-windows` feature names from earlier versions still work; `launcher-linux` and `launcher-macos` are aliases too. The launcher expects `gamestrike-player` in the same folder, which this build produces. Pass `--library DIRECTORY` to use another folder, for example a throwaway test library. Default library locations:

| OS | Default library |
| --- | --- |
| Windows | `%LOCALAPPDATA%\GameStrike\Library` (unchanged, so existing libraries and saves keep working) |
| Linux | `$XDG_DATA_HOME/gamestrike/library`, usually `~/.local/share/gamestrike/library` |
| macOS | `~/Library/Application Support/GameStrike/Library` (saves: WebKit data stores, see above) |

```text
gamestrike --library DIR import GAME.html|GAME.zip
gamestrike --library DIR list
gamestrike --library DIR rename GAME_UUID "New title"
gamestrike --library DIR reset-saves GAME_UUID
gamestrike --library DIR remove GAME_UUID [--keep-saves]
gamestrike-player --library DIR --id GAME_UUID --offline|--allow-network
```

**Tests**

```sh
cargo fmt --all -- --check
cargo clippy --locked --features launcher --all-targets -- -D warnings
cargo test --locked --features launcher
node --test ui/app.test.cjs
# Opens real player windows for about 30 seconds (prefix with `xvfb-run -a` on headless Linux):
cargo test --locked --features launcher --test player_isolation -- --ignored --nocapture
# Release launch and capture smoke test with synthetic games (Linux/macOS; Windows: scripts/hosted-library-capture.ps1):
bash scripts/hosted-library-capture.sh target/release /tmp/gamestrike-capture
```

## How games are isolated

| Layer | What it does |
| --- | --- |
| Separate process | Game HTML runs only in `gamestrike-player`, never in the launcher's WebView. The player registers no IPC handler or host objects, and disables WebView2 web messages and host objects before loading the game. A source test fails if any of that is added. |
| Per-game origin and profile | Each game is served from `https://gamestrike<uuid>.localhost` with its own WebView2 data folder. |
| Files | Only canonical files inside that game's content folder are served. `file://` fetches, frames, and navigation are blocked, as are other origins, popups, downloads, permission prompts, and external protocol launches. |
| Offline mode | A strict Content-Security-Policy (only the game's own origin plus `data:`/`blob:`) on every response, plus a proxy owned by the player that answers every HTTP, HTTPS, and WebSocket connection with 403, including loopback. WebRTC is restricted to proxied traffic, which blocks its UDP. |
| Network mode | Only plugins and base-URL changes are restricted. The game can reach the network, and you are told so first. |
| Launcher | Serves only its bundled files (the two style pages and their scripts and styles, the vendored three.js, fonts) under `default-src 'none'`, plus cover PNGs that GameStrike encoded, by game ID only. It accepts twenty-one JSON commands, and only from its own origin. The page never supplies a file path, link, or program; Rust validates every game ID. The 3D scene is decoration; the only game content it receives is the selected game's cover. |
| Store games | Trusted apps you installed with their store. GameStrike only reads the stores' install records (never credentials or tokens) and starts a game through its store's link, from an allow-list of schemes with URL-safe characters only, or through the store's or your own program with its arguments as a list, never a shell. They run as ordinary programs, outside GameStrike's HTML isolation. |
| Covers | Untrusted cover images are decoded only by the memory-safe Rust `image` crate (PNG, JPEG, GIF, WebP; 16 MiB per file, 8192 px per side, 256 MiB of decoder memory), scaled to fit 600 × 900, and re-encoded as a new PNG inside the library. SVG and HTML are refused. |
| Imports | Rejects path traversal (zip-slip), absolute and drive paths, Windows device names, links and special files, case-insensitive duplicates, ambiguous entry points, unsupported compression, zip bombs (expansion ratio and size limits), and corrupt archives. Extraction is staged and published atomically. |
| Integrity | The player refuses to open imported games from an elevated (high-integrity) process. |

The details and remaining open questions are in [docs/PLAYER_SECURITY.md](docs/PLAYER_SECURITY.md). The launcher's IPC and input model is in [docs/LAUNCHER_UI.md](docs/LAUNCHER_UI.md).

## Verification

Linux and macOS results come from CI only; see [Platforms](#platforms) and the per-OS tables in [docs/PLAYER_SECURITY.md](docs/PLAYER_SECURITY.md#linux-and-macos-measurements). The rest of this section is the Windows record.

Results as of 2026-10-17 on Windows 11 Pro, WebView2 runtime as installed on the dev PC and on GitHub's `windows-latest` runner.

**Automated, all passing in [cross-platform CI](https://github.com/Gerbzz/GameStrike/actions/workflows/cross-platform.yml) on Windows, Linux, and macOS:**
- Rust: 57 library tests (19 of them for the store scanners, the launch allow-list, and the store catalog), 14 player tests (15 on Linux and macOS), 9 launcher tests, and 2 player command-line tests, plus the real-player isolation test. Besides the areas below, they cover cover decoding and its limits (including a forged 9000 × 9000 PNG header), custom, automatic, undone, and reset covers, sources and favorites, the cover endpoint, that every bundled UI file is served and every module import resolves, that the earlier styles' capture views open their own page, and that the launcher refuses store commands carrying a link or a path. They cover zip-slip, drive paths, device names, corrupt and empty files, expansion limits, duplicate and legacy re-imports, removal with and without saves, restoring kept saves, clearing saves, the per-game lock, rename validation, tolerant scanning, missing files, the launcher's origin and command allow-list, and CSP headers.
- JavaScript: 25 tests of sorting, search, the sidebar filters built from library data (store rows only for stores with games, Hidden), store status lines, the menu button, the source rule, cover URLs, grid reflow and list movement, the style list and its cycle, the Collection scale and column count, the generated box art rules, the seeded random generator, the dome net, the earlier shelf and perspective helpers, controller edge detection and repeat, and formatting.
- **Real player isolation test** (`tests/player_isolation.rs`). It runs the real WebView2 player against a bundled probe page, with a harness listening on loopback over TCP and UDP.
  - Offline, twice: 0 TCP and 0 UDP connections reached the harness. Fetch, XHR, WebSocket, image, `file://`, other-origin requests, and popups were all blocked.
  - Network mode, as a control: the same probe reached the harness (8 TCP connections, 4 UDP packets), which shows the offline zeros mean "blocked", not "unobservable".
  - Saves: `localStorage` and IndexedDB counters reached 3 across relaunches. A second game started at 1, so the profiles are separate.
  - A second player for a running game is refused.

**Manual run of the real app** (computer use, build at `c888e7e`, before the console-style restyle; the Rust side is unchanged since apart from `bc6fd31`):
- Native picker: cancel left the library unchanged; an HTML import was added and selected.
- Offline launch, then the player closed back to the launcher with an exit message.
- Relaunch: the save counter kept its `localStorage` points and IndexedDB count, and play count went to 2.
- While a game ran, the launcher showed *Playing* and disabled Play.
- A wrapped ZIP imported, and its relative CSS and JS loaded offline.
- A zip-slip archive ("ZIP contains an unsafe path") and a corrupt archive were rejected with a message, leaving the library unchanged.

That run found two issues, both fixed in `bc6fd31`: the offline summary counted WebView2's own background requests as the game's, and the corrupt-ZIP error was too technical.

**The earlier styles are unchanged.** The baseline run [39336633107](https://github.com/Gerbzz/GameStrike/actions/runs/39336633107) captured Blades, Original, and PWDS Orange before this work, under the same file names the later runs use (`legacy-*.png`). Run to run, the Windows and macOS captures of those styles differ only in the profile clock and in where the animated bokeh and glow happen to be (under 1.2 % of pixels, inside the same regions), which is also how much two runs of the unchanged baseline differ from each other. The Linux captures vary much more between any two runs, baseline included, because Xvfb captures the Blades and Original views at different points of their CSS animations. To rule that out, the pages before and after were also rendered side by side in Chrome with a fixed clock and every animation frozen at the same instant: all nine views (Blades, System, Manage, Original, Original's My Games, PWDS Orange, and the three startups) were pixel-identical. `classic.css` is byte-identical to the old stylesheet.

**Not yet verified by hand:** the Collection style has been rendered and captured from the real launcher in CI on all three systems, and its keyboard flow was exercised in a browser preview with a stubbed IPC, but it has not been driven interactively in the real app. The manual run above used an earlier design. Rename, clear saves, remove, duplicate re-import, network launch, and a physical controller were checked only by automated tests.

## Limitations

- **Store detection is mostly tested against synthetic fixtures.** The parsers run in CI on all three OSes. Only Steam has been tried on a real install: on one Windows PC it found all 13 installed games, showed Steam's own art for all 13 (after `9be32d8`), and launched a Steam game through `steam://rungameid/`. Ubisoft Connect was installed there with no games, and none were reported. No Epic, EA, Microsoft Store, GOG, or Battle.net install, and no Ubisoft install with games, has been tried; the real registry reads and launch calls are not exercised in CI. EA detection is best effort (the EA app's own list is encrypted), Battle.net products outside a built-in table open Battle.net instead of the game, and Microsoft Store games without a `MicrosoftGame.config` are not found.
- **Not a sandbox.** The player is an ordinary user-level process. Offline mode blocks network paths at the browser level (CSP plus proxy plus WebRTC policy). It does not stop a browser engine exploit, and the probe doesn't cover every possible network path, such as DNS prefetch.
- WebView2 still defines `window.chrome.webview` in game pages. GameStrike registers nothing behind it, but the object exists.
- In network mode, the runtime's own requests for the game's origin outside the custom protocol resolve to `localhost` and could reach a local server on port 443.
- Controller support and network needs are shown as *not checked* for every game; GameStrike does not inspect games.
- Generated box art is decoration (a scene picked from a word in the title, or from the game ID, with the title on it), not the game's own artwork.
- The Collection style was checked only through CI captures on virtual machines and a browser preview. It has not been run by hand on real hardware, and its frame rate on integrated GPUs has not been measured; the automatic quality steps are untested there.
- The frame and panels are HTML, SVG, and CSS 3D in front of the WebGL scene, not meshes inside it. They glow with SVG filters rather than the scene's bloom, and the 3D tubes can only show around the frame, never in front of it as they do in the reference concept.
- See [Platforms](#platforms) for which operating systems are verified. No installer, code signing, auto-update, or app icon resource yet.

## Credits and license

GameStrike is published by [PWDS (ProWebDevSolutions)](https://prowebdevsolutions.com). The geometric GameStrike mark is original artwork for this project. The PWDS logos in `assets/branding` are the official PWDS assets and are not covered by any open-source license; the Collection: PWDS Orange style shows the PWDS logo in its emblem. The current Collection dashboard and archived earlier artwork are original recreations inspired by console dashboards; the Collection layout follows a concept by the project owner. They use no Microsoft logos, artwork, sounds, or fonts, and GameStrike is not affiliated with or endorsed by Microsoft; Xbox is a trademark of Microsoft Corporation. The colored A/B/X/Y glyphs follow the common controller color convention and are drawn in CSS.

The launcher bundles [three.js](https://threejs.org) r170 (MIT) and the fonts [Nova Square](https://fonts.google.com/specimen/Nova+Square), [Rajdhani](https://fonts.google.com/specimen/Rajdhani), and [Share Tech Mono](https://fonts.google.com/specimen/Share+Tech+Mono) (SIL Open Font License 1.1); see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for versions and license files.

Built with [Wry](https://github.com/tauri-apps/wry) and [Tao](https://github.com/tauri-apps/tao) (Apache-2.0/MIT), [webview2-com](https://github.com/wravery/webview2-rs) (MIT), [zip](https://github.com/zip-rs/zip2) (MIT), and [serde](https://serde.rs) (Apache-2.0/MIT). The README layout takes cues from [PhotoCraft](https://github.com/storytold/photocraft): a status callout, features kept separate from limitations, and command-first build steps. No code or assets were copied from it.

No license file has been chosen for GameStrike's own code yet; until one is added, all rights are reserved by the author. Other [PWDS Strike apps](https://github.com/Gerbzz?tab=repositories&q=Strike) follow the same suite design.
