# Screenshot provenance

These PNGs are captures of the running Windows `gamestrike-launcher.exe` WebView2 surface, produced with its explicit `--capture-preview` option. They are not HTML mockups, browser recreations, composited images, or retouched renders.

- Source: commit `aa1d86b699687198ad59983d44f7547c24a1871f`
- Build: [Windows source checks run 38650007764](https://github.com/Gerbzz/GameStrike/actions/runs/38650007764), artifact `gamestrike-windows-debug` (`11934187624`), ZIP SHA-256 `cec7bc83071575831ff48a095e5f97e21933901ad86f600d59410f9cee87466b`
- Host and date: Office PC, 2026-10-14 UTC
- `empty-library.png`: a newly created, empty isolated library at 1250 × 800
- `local-library.png`: an isolated library containing only the bundled benign `fixtures/save-counter.html` at 1250 × 800
- `local-library-minimum.png`: the same fixture library at the 880 × 620 minimum window size

The fixture was imported through the existing GameStrike CLI before the captures. The native launcher's own file picker, import flow, and consent dialog still require interaction testing. No third-party games or user files appear in these screenshots.
