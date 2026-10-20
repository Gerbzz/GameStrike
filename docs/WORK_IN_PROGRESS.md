# Work in progress: 3D chrome and cinematic startup (branch `gamestrike-chrome-3d`)

Draft PR: https://github.com/Gerbzz/GameStrike/pull/14 (base `gamestrike-store-library`, PR #13, which is stacked on #12).
This file is for picking the work up cold. Delete it when the work is finished and the PR description holds the final state.

## Goal

Make the Collection style match `GOAL-pwds-orange-game-collection.webp` (kept outside the repo, never committed) element by element at 1672 × 941:
thick lit 3D frame, big glossy tubes that pass in front of the frame at the top right, left side, and bottom left, a large glossy emblem disc, a raised bevelled header tab with larger text, thick lit sidebar slabs, thick bevelled tile frames, a raised bottom plate, and glossy A/B/X/Y rings.
Then replace the Collection style's startup with a cinematic WebGL boot (about 5.5 s), as specified in the request (original assets only: no Microsoft logo, X, footage, or sounds).

Hard rules: no local cargo or rustc (all Rust builds and tests run in GitHub Actions), no force-push, no merging or closing PRs, no Microsoft assets, commit as Neil Gerbe with the Co-Authored-By line, legacy styles (`classic.*`, `style-gate.js`) unchanged.

## Done

- `ui/chrome.js` (new ES module): extrudes the outlines from `frame.js` `measure()` into slabs (ExtrudeGeometry, rounded bevels), MeshPhysicalMaterial faces (near-black clearcoat plastic) and amber rims, light pipes (thin rings above 1.0 so they bloom), emblem disc and ring (LatheGeometry), rivets, a procedural half-float HDR environment (PMREM). Palettes for orange and green (`PALETTES`; `setPalette` overrides material colours, so tune colours there).
- `ui/scene.js`: builds the chrome in the camera rig at the frame plane (one layout pixel = one pixel of the plane at `FRAME_DEPTH`), rebuilds it on `gamestrike:layout` and resize, fades the interior in the detail view, new `TUBES` with `front` flags, a second WebGL renderer on `#scene-front` (above the page, `pointer-events: none`) for front tubes, shader tubes (amber glass with a studio reflection), collars. The static fallback paints front tubes on the front canvas.
- `ui/frame.js`: `measure()` split out of `draw()`; the SVG frame is now only the fallback (`body[data-chrome="3d"]` hides it). The bottom title plate is one long plate.
- `ui/styles.css`: thicker lit rows and tiles, glossy rings, header title 1.25 times larger.
- `src/launcher.rs`: serves `/chrome.js`; launcher webview `with_autoplay(true)` for the startup sounds.
- Commit `68868cb` (3D chrome), pushed; its Cross-platform run was cancelled in favour of `149ef57` (same code plus this note): Cross-platform checks https://github.com/Gerbzz/GameStrike/actions/runs/39488904943, Windows source checks https://github.com/Gerbzz/GameStrike/actions/runs/39488905136 (passed).
- Commit `95a5bef` (WIP startup): `ui/boot.js` (particles, tendrils, core, flash, shockwave, emblem with the real PWDS logo, light sweep, flight to the seat), driven from `scene.js` (`gamestrike:boot` play / skip / freeze, `poseBoot(t)`, `extendTubes(p)`, `chrome.setAssembly`, `chrome.seat()`); page side in `app.js` (`startBoot`, `skipBoot`, `finishBoot({ assemble })`, `stagger()`, startup sounds on a bus that a skip fades out) and `styles.css` (wordmark, `body.assembling` slide and flip animations, `--seek` and `--aseek` for captures). Capture views `boot` (3.5 s), `boot-0.5`, `boot-2`, `boot-4.5`, `boot-5.5`. CI step "Record the startup (Linux, Xvfb)" runs `scripts/record-startup.sh` (ffmpeg; mp4, gif, stills in the Linux capture artifact).

## In progress

- CI for `95a5bef`: check the boot captures and the Linux recording on all three systems, and the chrome captures from run 39488904943.

## Next steps

1. Fix anything the CI captures show (each OS renders with a different GL stack).
2. (Done as WIP in `95a5bef`; tune after CI.) Startup: a boot timeline in `scene.js` (black, a point of light, particles and tendrils swirling in, an orb, camera pull-back with the dome net fading in, flash and shockwave, the PWDS emblem disc with the real `PWDS-logo-orange.svg`, the GAMESTRIKE wordmark with a light sweep, then the emblem flying to its seat while the chrome assembles via `chrome.setAssembly(t)`, tubes extend, and the DOM header, rows, and tiles slide and flip in). Synthesized sounds in `app.js` (swell, shimmer, impact, clunks, hiss), muted when sounds are off. Any key, click, or button skips to the end; reduced motion is a short fade. The library loads in parallel.
3. (Done in `95a5bef`.) Capture views for the boot at about 0.5, 2, 3.5, 4.5, and 5.5 s (add names to `CAPTURE_VIEWS` in `src/launcher.rs` and update its test; add them to `scripts/hosted-library-capture.sh` and `.ps1`), plus an animated PNG from a frame sequence on one runner if feasible.
4. Side-by-sides against the goal, the PR description, and the report.

## How to preview locally (no cargo)

`node <work>/tools/serve.cjs <port> GameStrike-chrome/ui <work>` serves the UI with a mock IPC library; headless Chrome with `--enable-unsafe-swiftshader --use-angle=swiftshader` captures `index.html?view=library` and the other capture views. The work folder (`gamestrike-reference/dome-work`) is outside the repo.

## Comparison notes (local preview vs goal, 1672 × 941)

- Frame: dark glossy faces and glowing double edges now read like the goal; the goal's outer left rim is a bit thicker and brighter.
- Tubes: top right is close; the bottom left tubes are darker than the goal's.
- Emblem, header tab, rows, tiles, rings: close; rows and tiles could still be a touch heavier.
