# Third-party notices

GameStrike bundles the following third-party files into the launcher. They are served only to the launcher's own window, from the binary, with no network access.

| Component | Version | Files | License |
| --- | --- | --- | --- |
| [three.js](https://threejs.org) | r170 (npm `three@0.170.0`) | `ui/vendor/three/three.module.min.js`; from `examples/jsm`: `postprocessing/EffectComposer.js`, `MaskPass.js`, `OutputPass.js`, `Pass.js`, `RenderPass.js`, `ShaderPass.js`, `UnrealBloomPass.js`, and `shaders/CopyShader.js`, `LuminosityHighPassShader.js`, `OutputShader.js` | MIT, see `ui/vendor/three/LICENSE` |
| [Nova Square](https://fonts.google.com/specimen/Nova+Square) by wmk69 | `@fontsource/nova-square@5.3.0` | `ui/fonts/nova-square-latin-400-normal.woff2` | SIL Open Font License 1.1, see `ui/fonts/OFL-NovaSquare.txt` |
| [Rajdhani](https://fonts.google.com/specimen/Rajdhani) by Indian Type Foundry | `@fontsource/rajdhani@5.3.0` | `ui/fonts/rajdhani-latin-500-normal.woff2`, `rajdhani-latin-600-normal.woff2`, `rajdhani-latin-700-normal.woff2` | SIL Open Font License 1.1, see `ui/fonts/OFL-Rajdhani.txt` |
| [Share Tech Mono](https://fonts.google.com/specimen/Share+Tech+Mono) by Carrois Type Design | `@fontsource/share-tech-mono@5.3.0` | `ui/fonts/share-tech-mono-latin-400-normal.woff2` | SIL Open Font License 1.1, see `ui/fonts/OFL-ShareTechMono.txt` |

The font files are the unmodified Latin subsets published by Fontsource. Nova Square and Share Tech Mono have Reserved Font Names, so the launcher's stylesheet refers to the files by its own family names ("GS Square", "GS Sans", "GS Mono") rather than presenting any file as a modified version under the reserved name. The licenses were taken from the google/fonts repository.

The only change to the vendored three.js add-ons is their import specifier: `from 'three'` became `from '../three.module.min.js'`, because the launcher's Content-Security-Policy allows no inline import map. The packages were fetched from the npm registry and checked against the registry's published SHA-512 integrity values.

Rust crates are listed in `Cargo.lock`, each under its own license.

All interface sounds are synthesized at run time by `ui/app.js` and `ui/classic.js`; GameStrike ships no audio samples. Generated box art (`ui/covers.js`), the frame (`ui/frame.js`), and the dome (`ui/scene.js`) are drawn by GameStrike's own code; no third-party artwork, logos, or console maker's assets are bundled.
