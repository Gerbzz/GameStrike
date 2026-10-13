# Trusted library launcher: source design

This is the next UI slice, not an implemented native launcher. Use the cached Tauri 2 stack for the trusted library window and keep the existing Wry game player as a separate executable. The Tauri window must never navigate to imported HTML. Its capability set should be limited to the library window and the small commands below; imported game JavaScript gets none of those commands.

## First screen

The first screen has an original `GAMESTRIKE` text mark, with `by PWDS` as a separate publisher mark. It shows a large focused game card, a horizontal library row, and a simple `Add game` action. Empty, importing, rejected archive, ready, and player-exit states need clear messages. A game card shows `Controller support: unknown` and `Network requirement: unknown` until the game is verified; the shell's controller support must not be presented as gamepad support in the game.

Keyboard arrows and a controller D-pad move focus between cards. Enter / controller A activates the focused action. Escape / controller B returns to the previous launcher panel. Focus must remain visible and not depend on hover. The launcher can remember the last focused game ID. Player windows have their own input handling; launcher navigation does not inject controls into imported games.

## Narrow native command surface

The trusted UI needs only these Rust operations for the first integrated version:

| Command | Input | Result |
| --- | --- | --- |
| `list_games` | none | Generated manifest summaries: ID, title, controller/network unknown fields |
| `pick_and_import` | user-selected local HTML/ZIP file | Game summary or bounded import error |
| `launch_game` | validated game UUID and explicit network choice | Separate player process started or actionable error |

The frontend must render game titles as text, never as HTML. `launch_game` must resolve content and profile paths from the library's generated UUID, not accept a path from web content. The current player has no strict offline mode, so the first trusted UI must not show an `Offline play` button. Network-enabled preview launch should require a deliberate choice and plainly state that the game may contact local or remote services. Until the player boundary checks in [PLAYER_SECURITY.md](PLAYER_SECURITY.md) are complete, arbitrary downloaded games remain an experimental input, not a general-safe catalog.

## Integration gate

Do not label this a native launcher until the Tauri window, capabilities, import dialog, library list, player launch, controller focus, and error flows are built and exercised on Windows. The existing CLI and player fixture checks are separate evidence. The next native build should inspect Tauri capability defaults, preserve a distinct Wry player executable, and repeat the save-counter reopen test through a click or controller action in the trusted launcher.
