<p align="center">
  <img src="assets/branding/gamestrike-mark.svg" alt="GameStrike mark: two layered game cards with a control pad" width="128" height="128">
</p>

<h1 align="center">GameStrike</h1>

<p align="center"><strong>A console-style home on Windows for the HTML games you download.</strong><br>
Add a game from an <code>.html</code> file or a <code>.zip</code>, pick it from a glossy, console-style dashboard with a mouse, keyboard, or controller, and play it in its own window. Offline by default, with saves kept per game. No account, no uploads.</p>

<p align="center">
  <a href="#what-works-today">What works</a> ·
  <a href="#screenshots">Screenshots</a> ·
  <a href="#build-and-run">Build and run</a> ·
  <a href="#how-games-are-isolated">Isolation</a> ·
  <a href="#verification">Verification</a> ·
  <a href="#limitations">Limitations</a> ·
  <a href="#credits-and-license">Credits</a>
</p>

![GameStrike dashboard in the Lime theme: the My Games channel with the focused game tile large on the left and the other games receding in perspective, channel names stacked above, and A/X/Y/B button hints at the bottom](docs/screenshots/dashboard.png)

> [!IMPORTANT]
> **Early, unreleased software.** There is no installer or signed binary yet; you build it from source. Imported games are untrusted code. Offline mode blocks their network traffic, but the player is **not** an operating-system sandbox. Read [Limitations](#limitations) before running games you don't trust.

## What works today

Everything in this section is implemented in this repository and covered by the checks listed under [Verification](#verification).

**Library**
- Add a game through the Windows file picker: a single `.html`/`.htm` file, or a `.zip` with `index.html` at its root or inside one wrapper folder. GameStrike copies the files into its own library; your original download is never modified.
- Re-adding the exact same file is recognized, and the game keeps its saves.
- Browse a console-style dashboard inspired by the classic 2008-era console dashboards. Channel names stack at the top left, and Up/Down switches channel: *Jump Back In*, *Add & Find*, *My Games*, *System*. Within a channel, the focused tile sits large on the left and the rest recede in perspective, with glossy reflections and an *N of M* counter. You can search, and sort by recently played, title, or date added.
- Five color themes (Lime, Ocean, Sunset, Grape, Graphite), chosen from the *System* channel and remembered.
- Each game shows its source type, when it was added, when it was last played and how often, and how much data it has stored.
- **Manage** a game: rename it, clear its saves, or remove it. A removal can keep the saves, so adding the same file again brings them back.
- Problems are reported rather than hidden. One unreadable folder doesn't hide the rest of the library, and a game whose files have disappeared shows *Files missing* and won't launch.

**Playing**
- **Play offline** (the default) opens the game in a separate `gamestrike-player` window. The game's network requests are blocked. When the game closes, the launcher says how many requests the game itself attempted, so you can decide whether it needs network access.
- **Play with network…** asks for consent first; focus starts on *Go back*.
- Each game has its own browser origin and profile, so `localStorage`, IndexedDB, and cookies persist across launches and never mix between games.
- Only one game runs at a time. While it runs, the launcher shows *Playing*, and the game can't be removed or cleared.

**Input**
- Mouse, keyboard (arrows, Enter, `N`, `F2`, `Delete`, `Esc`, `Ctrl+F`), and a standard-mapping controller: D-pad or stick left/right to move along a channel and up/down to switch channels, **A** to play or select, **X** for network, **Y** to manage, **B** to go back. A held button never repeats, so it can't confirm a dialog that opens underneath it.
- Interface sounds, synthesized locally (nothing is downloaded). Toggle them in the *System* channel.
- Reduced motion and high-contrast (forced colors) are respected.

**Command line** (`gamestrike`): `import`, `list`, `rename`, `reset-saves`, and `remove [--keep-saves]`.

## Screenshots

All images come from the running Windows build of this branch, on 2026-10-17. Launcher views are from the final UI in this PR; the player capture is from the manual run at `c888e7e`. They are not mockups. Every game shown is a labeled **synthetic test fixture** created for testing; no third-party game appears.

- **Launcher views** were rendered by the real `gamestrike-launcher.exe` through WebView2 `CapturePreview` (the `--capture-preview` option). They show the window's content area at 1250 × 800 unless noted.
- **The player** was captured from the screen during a manual run of the real app and cropped to the player window.

| | |
| --- | --- |
| ![Network consent dialog with Go back and Allow network & play](docs/screenshots/network-dialog.png) **Network consent.** Offline is the default; network needs this extra step. | ![Manage dialog with Rename, Clear saves, Remove from library, Back](docs/screenshots/manage-dialog.png) **Manage** (Y on a controller). |
| ![Empty library showing a single Add your first game tile](docs/screenshots/empty-library.png) **First launch** with an empty library. | ![About dialog with the PWDS publisher logo](docs/screenshots/about.png) **About**, with the publisher credit. |
| ![System channel with Sounds, Theme, and About tiles](docs/screenshots/system-channel.png) **System channel**: sounds, color theme, and About. | ![Dashboard at the 880 by 620 minimum window size](docs/screenshots/minimum-880x620.png) **Minimum window size**, 880 × 620. |
| ![Player window titled Save Counter (Synthetic Test), GameStrike, Offline, showing localStorage points 2 and IndexedDB opens 1](docs/screenshots/player-offline-save-counter.png) **A game in its own offline player window.** | |

Screenshots of the earlier design are kept in [docs/history](docs/history).

## Build and run

You need Windows 10 or 11 with the [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/) (preinstalled on Windows 11) and stable Rust with edition 2024 support.

```powershell
cargo build --locked --features launcher-windows --bins
.\target\debug\gamestrike-launcher.exe
```

The launcher expects `gamestrike-player.exe` in the same folder, which this build produces. By default the library lives in `%LOCALAPPDATA%\GameStrike\Library`; pass `--library DIRECTORY` to use another folder, for example a throwaway test library.

```text
gamestrike --library DIR import GAME.html|GAME.zip
gamestrike --library DIR list
gamestrike --library DIR rename GAME_UUID "New title"
gamestrike --library DIR reset-saves GAME_UUID
gamestrike --library DIR remove GAME_UUID [--keep-saves]
gamestrike-player --library DIR --id GAME_UUID --offline|--allow-network
```

**Tests**

```powershell
cargo fmt --all -- --check
cargo clippy --locked --features launcher-windows --all-targets -- -D warnings
cargo test --locked --features launcher-windows
node --test ui/app.test.cjs
# Opens real player windows for about 20 seconds:
cargo test --locked --features launcher-windows --test player_isolation -- --ignored --nocapture
```

## How games are isolated

| Layer | What it does |
| --- | --- |
| Separate process | Game HTML runs only in `gamestrike-player`, never in the launcher's WebView. The player registers no IPC handler or host objects, and disables WebView2 web messages and host objects before loading the game. A source test fails if any of that is added. |
| Per-game origin and profile | Each game is served from `https://gamestrike<uuid>.localhost` with its own WebView2 data folder. |
| Files | Only canonical files inside that game's content folder are served. `file://` fetches, frames, and navigation are blocked, as are other origins, popups, downloads, permission prompts, and external protocol launches. |
| Offline mode | A strict Content-Security-Policy (only the game's own origin plus `data:`/`blob:`) on every response, plus a proxy owned by the player that answers every HTTP, HTTPS, and WebSocket connection with 403, including loopback. WebRTC is restricted to proxied traffic, which blocks its UDP. |
| Network mode | Only plugins and base-URL changes are restricted. The game can reach the network, and you are told so first. |
| Launcher | Serves six bundled files under `default-src 'none'`. It accepts six JSON commands, and only from its own origin. The page never supplies a file path; Rust validates every game ID. |
| Imports | Rejects path traversal (zip-slip), absolute and drive paths, Windows device names, links and special files, case-insensitive duplicates, ambiguous entry points, unsupported compression, zip bombs (expansion ratio and size limits), and corrupt archives. Extraction is staged and published atomically. |
| Integrity | The player refuses to open imported games from an elevated (high-integrity) process. |

The details and remaining open questions are in [docs/PLAYER_SECURITY.md](docs/PLAYER_SECURITY.md). The launcher's IPC and input model is in [docs/LAUNCHER_UI.md](docs/LAUNCHER_UI.md).

## Verification

Results as of 2026-10-17 on Windows 11 Pro, WebView2 runtime as installed on the dev PC and on GitHub's `windows-latest` runner.

**Automated, all passing locally and in [Windows CI](https://github.com/Gerbzz/GameStrike/actions/workflows/windows-source.yml):**
- Rust: 26 library tests, 12 player tests, 3 launcher tests, and 1 player command-line test. They cover zip-slip, drive paths, device names, corrupt and empty files, expansion limits, duplicate and legacy re-imports, removal with and without saves, restoring kept saves, clearing saves, the per-game lock, rename validation, tolerant scanning, missing files, the launcher's origin and command allow-list, and CSP headers.
- JavaScript: 11 tests of sorting, search, grid and shelf movement, the perspective tile layout, controller edge detection and repeat, generated covers, and formatting.
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

**Not yet verified by hand:** the console-style dashboard has been rendered and captured from the real launcher, but not driven interactively. Rename, clear saves, remove, duplicate re-import, network launch, and a physical controller were checked only by automated tests.

## Limitations

- **Not a sandbox.** The player is an ordinary user-level process. Offline mode blocks network paths at the browser level (CSP plus proxy plus WebRTC policy). It does not stop a browser engine exploit, and the probe doesn't cover every possible network path, such as DNS prefetch.
- WebView2 still defines `window.chrome.webview` in game pages. GameStrike registers nothing behind it, but the object exists.
- In network mode, the runtime's own requests for the game's origin outside the custom protocol resolve to `localhost` and could reach a local server on port 443.
- Controller support and network needs are shown as *not checked* for every game; GameStrike does not inspect games.
- Covers are generated decorations (initials and a pattern from the game ID), not game artwork.
- Windows only. No installer, code signing, auto-update, or app icon resource yet.

## Credits and license

GameStrike is published by [PWDS (ProWebDevSolutions)](https://prowebdevsolutions.com). The geometric GameStrike mark is original artwork for this project. The PWDS logos in `assets/branding` are the official PWDS assets and are not covered by any open-source license. The colored A/B/X/Y glyphs follow the common controller color convention and are drawn in CSS; no console maker's assets or trademarks are used.

Built with [Wry](https://github.com/tauri-apps/wry) and [Tao](https://github.com/tauri-apps/tao) (Apache-2.0/MIT), [webview2-com](https://github.com/wravery/webview2-rs) (MIT), [zip](https://github.com/zip-rs/zip2) (MIT), and [serde](https://serde.rs) (Apache-2.0/MIT). The README layout takes cues from [PhotoCraft](https://github.com/storytold/photocraft): a status callout, features kept separate from limitations, and command-first build steps. No code or assets were copied from it.

No license file has been chosen for GameStrike's own code yet; until one is added, all rights are reserved by the author. Other [PWDS Strike apps](https://github.com/Gerbzz?tab=repositories&q=Strike) follow the same suite design.
