# GameStrike trusted library

## Implemented source

`gamestrike-launcher` is a Windows Tao/Wry window built from bundled `ui/` assets. It uses the already pinned Wry stack, without adding a second browser framework. The fixed `gamestrikeui` custom protocol serves only five bundled paths: HTML, CSS, JavaScript, the original GameStrike fox, and the official PWDS display logo. Navigation is constrained to this origin, new windows and downloads are denied, and the response includes a restrictive Content Security Policy. Game titles enter the UI as JSON data and are rendered as text.

The screen has an empty shelf, generated cards for real manifests, visible focus, a native HTML/ZIP file picker, import progress and errors, an explicit network launch warning, and player start/exit messages. Arrow keys and a connected controller's D-pad/left stick move between game cards. Enter or controller A selects; Escape or controller B closes the launch dialog. Launcher input is never injected into an imported game.

The launcher's only IPC messages are `ready`, `pick_and_import`, and `launch_game` with a UUID and explicit `allow_network` flag. The file picker supplies the import path; the web UI never supplies one. Rust validates the UUID against generated manifests, resolves paths via `Library::player_paths`, then starts the sibling `gamestrike-player.exe` with the library root and game ID. Imported HTML runs in that separate player and has no launcher IPC handler. A missing player, rejected archive, or nonzero player exit produces a visible message.

## Truthful limits

Controller support and network requirement are shown as unknown for every imported game. A local import is not a safety verdict. The player does not provide strict offline mode, so the launcher only offers a deliberate network-enabled preview. Imported game JavaScript may reach local or remote services. The separate process is not a Windows sandbox; read [PLAYER_SECURITY.md](PLAYER_SECURITY.md) for open checks.

## Verification still needed

The new launcher source passed formatting and JavaScript syntax checks, 8 importer tests and 1 launcher origin test, and a Windows CI build of both binaries. It has not yet been launched or captured on the Office PC. Before presenting screenshots or claiming a working native window, exercise: empty library, file picker cancellation, successful benign HTML import, rejected ZIP, keyboard and controller focus, network choice, player reopen and exit. Then capture the actual window. Repeat the player browser-boundary fixture checks before expanding beyond controlled games.
