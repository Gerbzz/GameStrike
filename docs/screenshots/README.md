# Screenshot provenance

These PNGs are captures of the running Windows `gamestrike-launcher.exe` WebView2 surface, produced with its explicit `--capture-preview` option. They are not HTML mockups, browser recreations, composited images, or retouched renders.

- Source: commit `5cf79aef85c633533bfaf318599f6cbd0a25b50d`
- Build: [Windows source checks run 38651373781](https://github.com/Gerbzz/GameStrike/actions/runs/38651373781), artifact `gamestrike-windows-debug` (`11933939090`), ZIP SHA-256 `7336523490f94d525633da3640f636abe1d60a16a938a4bd770a7174ee1cdfd3`
- Host and date: Office PC, 2026-10-14 UTC
- `empty-library.png`: a newly created, empty isolated library at 1250 × 800
- `local-library.png`: five benign synthetic HTML files imported into an isolated test library at 1250 × 800
- `local-library-minimum.png`: the same test library at the 880 × 620 minimum window size
- `long-title.png`: the fifth synthetic title selected and scrolled into view; it is visible in the shelf and selected-game panel
- `launch-warning.png`: the launch dialog for the bundled benign `fixtures/save-counter.html`, before consenting or launching a game
- `about-publisher.png`: the About dialog with the unchanged official PWDS display logo

The files were imported through the existing GameStrike CLI before capture. The synthetic HTML contains no scripts or remote assets. The explicit `--capture-view` option opened the bundled launch and About dialogs before WebView2 captured them; it did not approve network access or execute a game. The native launcher's file picker, import flow, physical controller focus, and player return still require interaction testing. No third-party games or user files appear in these screenshots.
