# Third-party notices

GameStrike bundles the following third-party files into the launcher. They are served only to the launcher's own window, from the binary, with no network access.

| Component | Version | Files | License |
| --- | --- | --- | --- |
| [three.js](https://threejs.org) | r170 (npm `three@0.170.0`) | `ui/vendor/three/three.module.min.js`; from `examples/jsm`: `postprocessing/EffectComposer.js`, `MaskPass.js`, `OutputPass.js`, `Pass.js`, `RenderPass.js`, `ShaderPass.js`, `UnrealBloomPass.js`, and `shaders/CopyShader.js`, `LuminosityHighPassShader.js`, `OutputShader.js` | MIT, see `ui/vendor/three/LICENSE` |
| [Orbitron](https://github.com/theleagueof/orbitron) | `@fontsource/orbitron@5.2.8` | `ui/fonts/orbitron-latin-700-normal.woff2`, `orbitron-latin-900-normal.woff2` | SIL Open Font License 1.1, see `ui/fonts/OFL-Orbitron.txt` |
| [Exo 2](https://github.com/googlefonts/Exo-2.0) | `@fontsource/exo-2@5.2.8` | `ui/fonts/exo-2-latin-400-normal.woff2`, `exo-2-latin-600-normal.woff2`, `exo-2-latin-700-normal.woff2` | SIL Open Font License 1.1, see `ui/fonts/OFL-Exo2.txt` |

The only change to the vendored three.js add-ons is their import specifier: `from 'three'` became `from '../three.module.min.js'`, because the launcher's Content-Security-Policy allows no inline import map. The packages were fetched from the npm registry and checked against the registry's published SHA-512 integrity values.

Rust crates are listed in `Cargo.lock`, each under its own license.

All interface sounds are synthesized at run time by `ui/app.js`; GameStrike ships no audio samples.
