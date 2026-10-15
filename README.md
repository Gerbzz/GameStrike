<p align="center">
  <img src="assets/branding/gamestrike-fox-icon.png" alt="Original GameStrike fox logo" width="160" height="160">
</p>

<h1 align="center">GameStrike</h1>

<p align="center">
  <a href="https://prowebdevsolutions.com"><picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/branding/PWDS-logo-orange.svg">
    <img src="assets/branding/PWDS-logo.png" alt="PWDS company logo" width="120">
  </picture></a>
</p>

**A local home for HTML games you choose to keep.** The default branch contains a command-line importer and a separate Windows player for controlled fixtures. A working visual library is under review in [draft PR #1](https://github.com/Gerbzz/GameStrike/pull/1); the captures below show that development branch, not the current default-branch binary.

[Development previews](#development-previews) · [Import and launch](#try-the-current-cli) · [Player security and open checks](docs/PLAYER_SECURITY.md) · [Original artwork](assets/branding)

## Development previews

These are unedited WebView2 captures of the running Windows launcher built from draft-branch commit [`175016d`](https://github.com/Gerbzz/GameStrike/commit/175016d86c62460e17f1342af687958b772c96c0). The [Windows build and tests passed](https://github.com/Gerbzz/GameStrike/actions/runs/38665296183). The library contains five small synthetic local HTML fixtures; no downloaded third-party game appears. **The visual launcher and these workflows are not in `main` yet.**

**Five-game local library**

![Development preview of the running GameStrike library showing five local synthetic games, search, sort, and selected-game details](docs/screenshots/local-library.png)

**First launch, before adding a game**

![Development preview of the empty GameStrike library with the original fox artwork](docs/screenshots/empty-library.png)

The [minimum-window view](docs/screenshots/local-library-minimum.png), [long-title selection](docs/screenshots/long-title.png), [network launch warning](docs/screenshots/launch-warning.png), and [About panel with the authentic PWDS logo](docs/screenshots/about-publisher.png) are also current captures from that tested build. [Capture provenance and testing limits](docs/screenshots/README.md) explain exactly how they were made.

## What the current foundation does

### Import a game without running package scripts

Import one HTML file or a ZIP with a root `index.html`. GameStrike copies bounded content into `LIBRARY/games/<id>/content`, assigns an ID, and writes its own `game.json`. It rejects traversal, absolute and device paths, links and special files, case-insensitive duplicates, unsupported compression, and archives that exceed its size or entry limits. Formats with a nested or nonstandard entry point are not supported yet.

### Keep each title's browser data separate

The Windows player opens one imported ID at a time. Each game gets its own UUID-derived HTTPS-mapped origin and persistent WebView profile under `LIBRARY/profiles/<id>`. A first-party save fixture reopened with its state intact, while a second ID used separate storage. A separate process is an application boundary, **not an OS sandbox**.

### Build a controller-friendly library

The trusted visual launcher is being developed in [draft PR #1](https://github.com/Gerbzz/GameStrike/pull/1) and is **not implemented on `main`**. Its controller navigation does not make an imported game's own JavaScript gamepad-aware. Network and controller requirements remain **unknown** until each game is checked.

## Try the current CLI

These commands describe the source preview, not a finished console interface:

```text
gamestrike --library LIBRARY import GAME.html
gamestrike --library LIBRARY import GAME.zip
gamestrike --library LIBRARY list
gamestrike-player --library LIBRARY --id GAME_UUID --allow-network
```

Normal player launch requires the explicit `--allow-network` choice. **Strict offline play is unavailable.** Game JavaScript may still contact local or remote services, and some titles need remote assets to work. Do not treat arbitrary downloaded games as safe under this checkpoint; use only controlled fixtures until the [browser boundary checks](docs/PLAYER_SECURITY.md) are complete. No third-party games or assets are bundled.

## Build from source

On Windows, use a Rust toolchain that supports edition 2024 and an available WebView2 runtime. From the repository root:

```powershell
cargo build --locked --bin gamestrike
cargo build --locked --features player-windows --bin gamestrike-player
```

These commands describe how to build the source; the latest player source has not been validated as a newly built runnable executable. Check disk headroom before building.

## Current status

The default branch remains an **early CLI and player foundation**. Focused offline library tests passed 8/8, focused Windows player tests passed 5/5, and its source passed `cargo check`. An earlier guarded player build launched and reopened the bundled first-party save fixture with isolated storage. The newer frame and external-URI handlers on this branch compiled and passed focused tests but have **not** been exercised in a freshly built player. There is no visual launcher on `main`, general-game safety validation, strict offline mode, or release installer.

The separate Wry/system-WebView player is designed to have no Tauri commands, launcher credentials, game-to-host message bridge, or unrestricted file access. It limits local assets to the selected game's content root and registers navigation, download, window, and permission restrictions. Those checks still need controlled runtime validation across frames, protocols, workers, and network paths. Read [PLAYER_SECURITY.md](docs/PLAYER_SECURITY.md) before widening use; [LAUNCHER_UI.md](docs/LAUNCHER_UI.md) describes the planned trusted library.

The original [fox artwork](assets/branding) is included. Native app-icon and packaging work remain on `main`. Next steps are to finish the browser boundary tests with benign fixtures, review the [draft library UI](https://github.com/Gerbzz/GameStrike/pull/1), and validate real game compatibility before distribution.

Explore the other [PWDS Strike repositories](https://github.com/Gerbzz?tab=repositories&q=Strike).

## PWDS brand artwork

The company mark in this masthead is the existing [official PWDS logo](https://github.com/Gerbzz/PWDS-Website/blob/main/public/prowebdevsolutions-logo.png), with the [official orange display asset](https://prowebdevsolutions.com/images/prowebdevsolutions-logo-orange.svg) for dark backgrounds. Any app logo above it remains the original Strike suite artwork.
