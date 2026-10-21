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

- CI for `8023ed3` (run 39490904364, green on all three) showed: the startup plays correctly in the Linux recording (`startup.mp4` / `startup.gif` in the Linux capture artifact), but on Windows and macOS the boot captures did not hold the page animations (rAF paused in hidden capture windows), the 3.5 s macOS capture lacked the logo, the Linux 5.5 s capture was black, and the flying emblem was hidden behind the covers. Fixed in `115d8ca` (emblem and flash on the front layer, timer-based holds, logo readiness, 5.5 s = library at rest). `bb38b94` (run 39491181344, green) brightened the tubes and thickened the outer rim.

## In progress

- CI for `115d8ca` (Cross-platform run 39492482644): check boot-0.5 / boot-2 / boot (3.5) / boot-4.5 / boot-5.5 on all three systems and the Linux recording.

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

## Codex takeover checkpoint - 2026-10-21

Baseline: `d5c80ce2e45074631f39b1fd6bf83a81e8f4b226`, draft PR #14 over #13. Checkout was clean. No Git lock, cargo/rustc, GameStrike process, or Claude CLI worker was found; Claude desktop itself was open. No AGENTS.md found in the ancestors or PWDS-Strike tree. All other worktrees were preserved.

Recovered real-PC failure: `../gamestrike-reference/review-images/chrome-d5c80ce-library.png` shows the real 13-game Steam library with no chrome/tubes and a large startup emblem over the covers. Cause: normal capture calls finishBoot() without skipping the 3D timeline, so fast machines capture at ~3.5 seconds while the scene still hides the chrome. This is a capture/startup lifecycle defect; natural startup in Chrome completes. Separate current-CI macOS blank-DOM evidence was supplied by the parent reviewer.

Focused takeover changes (source verified; native acceptance remains pending):
- app.js: non-boot captures skip the scene boot; only GS CSS animations are held, never opacity transitions; readiness rejects an invisible stage.
- scene.js: context failure restores the CSS startup instead of retaining cinema-only CSS.
- launcher.rs: missing/throwing readiness, evaluation errors and readiness timeout fail capture instead of silently taking a screenshot.
- ui/startup.test.cjs: five behavioral regression tests, observed failing before their fixes.
- Cross-platform workflow: check scene/chrome/boot syntax, run regression tests, fail on startup recording failure.

Verified locally: `node --test ui/app.test.cjs ui/startup.test.cjs` = 30/30 pass; syntax for app/scene/chrome/boot passes; diff whitespace check passes. No local cargo/rustc invoked. Browser QA used the existing mock preview server on loopback (port 8766), Room PC Chrome at 1250x800: early library capture has full chrome/no central emblem, Steam filter changes 17 games to 5, boot-5.5 shows complete DOM and chrome. No console errors/warnings. Screenshots are outside the repo in `Documents/Codex/2026-10-20/task-2/gamestrike-fixed-library-chrome.jpg` and `gamestrike-fixed-boot-5.5-chrome.jpg`. These are Chrome synthetic-library evidence, NOT native WebView2 acceptance.

Pending: parent reports a user approval request for artifact/runtime work; no binary was downloaded or executed here. Existing baseline binary is `ci-build-39492893114/gamestrike-launcher.exe`. No local operation was denied by auto-review. Source review is complete; the focused patch is being pushed to the existing draft branch for fresh CI. All five regression tests also fail against an isolated copy of d5c80ce2, confirming the negative controls. Fresh run results will be recorded in the parent handoff and PR description. Native current-UI/real-library launch acceptance remains behind the pending approval. Fresh Windows/macOS/Linux CI captures, early wordmark timing and final visual comparison are being checked independently. Keep this WIP note until those checks pass; preserve draft status, branch chain, security settings and all reference assets outside Git.

## Source review follow-up - 2026-10-21

The takeover now has seven startup regression cases (32 JS tests total). The static-renderer then capture sequencing case and the late-boot-capture visibility case were each observed failing before their fixes. A recording-helper negative-control harness using fake programs verifies missing encoder, capture error, empty video and conversion error all fail; the original helper returned success for the forced encoder error. No GameStrike binary was run by these checks.

CI at ab12ac3331ec31f62838ffb918f5a1718cf00597 passed Windows/Linux/macOS and Windows source checks (runs 39528496101 and 39528496127). Pixel review still found premature wordmarks in reduced-motion boot captures. Reproduced in Room-PC Chrome: an explicit frozen 0.5-second capture with reduced motion enabled had wordmark opacity 1 and animation none. Capture-only CSS now allows the paused pose (opacity 0, all three animations paused at time 0). A normal reduced-motion page still goes to the library with animations disabled. This does not change user accessibility preferences.

The follow-up commit requires its own fresh CI/capture review, recorded in the parent HANDOFF.md and PR description. Reference art is unchanged and stays outside Git. Native Room-PC current-artifact and actual-library launch acceptance remain pending the parent's approval gate.

## Renderer recovery follow-up - 2026-10-21

A bounded readiness review found that losing the main WebGL context switched to the static scene but left the old WebGL event subscriptions active. A subsequent resize could mark the scene WebGL again. The foreground canvas had no loss handler, so its tubes and startup emblem could disappear without a fallback. An in-flight cover image could also upload/render through the abandoned renderer.

The focused fix stops both canvas paths, removes the WebGL subscriptions before handing over to startStatic(), and ignores stale render/image/logo callbacks. It keeps the established chrome/startup design and all classic.* / style-gate.js files intact. Four renderer-boundary tests cover the three failures plus healthy image rendering; the three negative cases fail against unchanged 8f630c7. The workflow now runs this test file. Full local JavaScript verification: 36/36 pass; scene syntax and git diff whitespace checks pass. No cargo/rustc or native GameStrike execution was used.

Rendered verification used existing Playwright because the Browser plugin was unavailable; no dependencies installed. One isolated headless Chromium instance tested the real source against the existing synthetic preview at 1250x800, then 1180x780. For both main and foreground context loss, the fix stays static after resize/palette/visibility events, keeps the stage visible, filters 17 games to five Steam games, and returns ready=true. Fixed cases have no console warnings/errors or page errors. Screenshots and JSON evidence are outside Git under readiness-review-20261021/browser-evidence. Baseline foreground-loss screenshot is blank despite a populated DOM and ready=true; fixed screenshots show the complete static fallback. This is browser/synthetic-fixture evidence, not native Room-PC acceptance.

Fresh exact-head CI/capture verification for this follow-up must be recorded in the root HANDOFF.md and PR description. Native startup, skip/reduced-motion, actual library and Steam-launch acceptance remain incomplete after the authorized 8f630c7 executable was rejected by Windows Application Control. That rejection is not a pending permission request. No denied launch was retried, no security or trust metadata changed, and no Microsoft contact/sample upload or signing purchase occurred. Keep this WIP file until native acceptance is resolved.
