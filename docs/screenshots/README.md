# Screenshot provenance

These development previews document the tested source commit below. They are app-window captures, not a binary release.

These PNGs are captures of the running Windows `gamestrike-launcher.exe` WebView2 surface, produced with its explicit `--capture-preview` option. They are not HTML mockups, browser recreations, composited images, or retouched renders.

- Source: commit `175016d86c62460e17f1342af687958b772c96c0`
- Build: [Windows source checks run 38665296183](https://github.com/Gerbzz/GameStrike/actions/runs/38665296183), artifact `gamestrike-windows-debug` (`11940304204`)
- Extracted launcher SHA-256: `46c5ca6e7a7cb208e97db07f941e8044886b580a300c3b04e346483d3c019cd1`
- Extracted player SHA-256: `0a5289f9406baf8e1ff5dccbc2aee95cdb0b221695350729aeb6cb42c1390be4`
- Host and date: Office PC, 2026-10-15 UTC
- `empty-library.png`: a newly created, empty isolated library at 1250 × 800
- `local-library.png`: five benign synthetic HTML files imported into an isolated test library at 1250 × 800
- `local-library-minimum.png`: the same test library at the 880 × 620 minimum window size
- `long-title.png`: the fifth synthetic title selected and scrolled into view; its full title and the footer remain visible
- `launch-warning.png`: the launch dialog for synthetic local game Orbit Garden, before consenting or launching a game
- `about-publisher.png`: the About dialog with the unchanged official PWDS display logo

The five files were imported through the existing GameStrike CLI before capture. Their synthetic HTML contains no scripts or remote assets. The explicit `--capture-view` option opened the bundled launch and About dialogs before WebView2 captured them; it did not approve network access or execute a game. The native launcher's file picker, import flow, physical controller focus, and player return still require interaction testing. The supported Windows computer-use helper could not initialize on this host (`HRESULT 0x80070003`), so these captures must not be taken as proof of those interactions. No third-party games or user files appear in these screenshots.
