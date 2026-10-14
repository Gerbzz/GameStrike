# GameStrike trusted library

## Implemented source

`gamestrike-launcher` is a Windows Tao/Wry window built from bundled `ui/` assets. It uses the already pinned Wry stack, without adding a second browser framework. The fixed `gamestrikeui` custom protocol serves only five bundled paths: HTML, CSS, JavaScript, the original GameStrike fox, and the official PWDS display logo. PWDS appears in About as publisher attribution; the working library uses GameStrike branding. Navigation is constrained to this origin, new windows and downloads are denied, and the response includes a restrictive Content Security Policy. Game titles enter the UI as JSON data and are rendered as text.

The empty library has a welcome panel with the original fox. A populated library puts title-led manifest cards first, without fabricated cover art, followed by a compact selected-game panel. It also has visible focus, a native HTML/ZIP file picker, import progress and errors, an explicit network launch warning, and player start/exit messages. Arrow keys and a connected controller's D-pad/left stick move between game cards or dialog choices. Enter reviews a card's launch warning. Controller A activates the focused dialog choice, initially **Go back**; a directional move must focus **Allow network & launch** before a fresh A approves it. Escape or controller B closes the launch dialog. Launcher input is never injected into an imported game.

The launcher's only IPC messages are `ready`, `pick_and_import`, and `launch_game` with a UUID and explicit `allow_network` flag. The file picker supplies the import path; the web UI never supplies one. Rust validates the UUID against generated manifests, resolves paths via `Library::player_paths`, then starts the sibling `gamestrike-player.exe` with the library root and game ID. Imported HTML runs in that separate player and has no launcher IPC handler. A missing player, rejected archive, or nonzero player exit produces a visible message.

## Truthful limits

Controller support and network requirement are shown as unknown for every imported game. A local import is not a safety verdict. The player does not provide strict offline mode, so the launcher only offers a deliberate network-enabled preview. Imported game JavaScript may reach local or remote services. The separate process is not a Windows sandbox; read [PLAYER_SECURITY.md](PLAYER_SECURITY.md) for open checks.

## Verification still needed

The launcher passed formatting and JavaScript syntax checks, 8 importer tests, 1 launcher origin test, 5 player tests, and 5 controller input regressions in [Windows CI](https://github.com/Gerbzz/GameStrike/actions/runs/38650007764). The CI-built launcher rendered and captured the actual empty and populated WebView2 surfaces on the Office PC at 1250 × 800, plus the populated surface at the 880 × 620 minimum. Its matching player launched the bundled save fixture at standard Windows integrity. The image files and source provenance are in [screenshots](screenshots/README.md). The native file picker, successful and rejected imports through the launcher, dialog focus with a physical controller, and player return still need interactive checks. Repeat the player browser-boundary fixture checks before expanding beyond controlled games.
