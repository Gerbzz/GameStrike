# Native Rust desktop slice

This engineering slice is undergoing hosted validation; it is not a finished or verified replacement launcher.
The existing launcher/player and artwork remain intact. The new binary is
`gamestrike-desktop`; its `native-ui` feature is independent of `player` and Wry.

The source contains native egui collection/navigation controls, Rust meshes and a
retained wgpu/WGSL PWDS backdrop/tube pass, a deterministic startup clock, typed
worker-backed library/import/favorite/launch services, and a synthetic capture path.
Imported HTML games remain in the optional separate `gamestrike-player` process.
Core application UI is never rendered by that player or a browser.

## Contextual navigation follow-up

See [Native contextual sidebar](NATIVE_CONTEXT_SIDEBAR.md) for provider filters,
visible local HTML import, and route-specific Music/Soundboard/Voice context.
Its validation is separate from the historical diagnostic runs below.

## Evidence status

The first hosted diagnostic run is [39700483311](https://github.com/Gerbzz/GameStrike/actions/runs/39700483311), source `2e33f69d7758788a027636141cf43db73f922261`. The standalone synthetic audio core passes all 53 contracts. The retained Windows launcher job also passes 15 application, seven capture and four catalog contracts, four provisional cue-policy/PCM contracts, and its player-isolation test. These do not exercise the native renderer.

That run compiled a real native executable and passed all nine native framebuffer cases on hosted Windows DX12 / Microsoft Basic Render Driver, including the 1672x941 target, frozen startup, skip and reduced motion. Artifact [12393581357](https://github.com/Gerbzz/GameStrike/actions/runs/39700483311/artifacts/12393581357) records source identity and hashes; executable SHA256 is `72a8598c1bb030920e1fa0b8dbaa551648013bc4a1336e2da4b1b7b491dcf94f`. All 54 artifact file hashes were independently checked. This is hosted synthetic-library evidence, not a release or general hardware acceptance.

The run remains failed: five of 16 native visual contracts discarded unapplied texture updates in their no-renderer fixtures, Clippy reported seven native-source lint errors, formatting failed, and the intentionally enforced candidate-lock gate rejected an uncommitted lock. Test disposal now uses egui's explicit no-renderer API with every input assertion preserved; source lint repairs use context/state parameters and equivalent guards without suppressions. A fresh committed-lock run must pass before accepting the repaired source.

Pixel inspection found a runner-pointer tooltip over the captured library. Capture mode now neutralizes only interaction events while preserving real framebuffer replies and viewport/DPI metadata. Normal interactive input is unchanged. The single PWDS design also pins native popup styling to dark, with an OS-light-theme regression contract. These fixes still require the fresh native run.

Cargo generated the new dependency lock on hosted Rust 1.96.1. Independent review verified its hashes, all patch hunks, preservation of existing package versions/checksums and the expected native dependency graph. The reviewed lock is committed at 39e985a; final repaired-source acceptance still requires a passing run. A resolved candidate cannot make the initial run green. Matched eframe/egui-wgpu 0.36.2 use wgpu 30 through the reexport.

Required hosted checks remain:

```sh
cargo test --locked --no-default-features --lib --tests
cargo test --manifest-path crates/gamestrike-audio-core/Cargo.toml --locked --offline
cargo test --locked --no-default-features --features native-ui --lib --tests --bin gamestrike-desktop
cargo fmt --all -- --check
cargo fmt --manifest-path crates/gamestrike-audio-core/Cargo.toml -- --check
cargo clippy --locked --no-default-features --features native-ui --bin gamestrike-desktop -- -D warnings
cargo build --locked --release --no-default-features --features native-ui --bin gamestrike-desktop -j 2
```

The workflow runs these commands on hosted runners. The initial diagnostic results above and repair run [39703364559](https://github.com/Gerbzz/GameStrike/actions/runs/39703364559) establish actual execution; the latter passes native contracts and formatting but still reports four desktop/adapter Clippy warnings. Final repaired-head results must be checked separately.

Observed local checks: 15 pure PowerShell metadata contracts passed after a negative
control exposed acceptance of a NaN pose; finite-number and boolean checks correct it.
These exercise only metadata fixtures, not a native process. Three PowerShell files
parse, Cargo TOML/native-feature/asset-reference checks pass, and new-file whitespace
checks pass. Those were the pre-publication results. Current hosted evidence above supersedes the earlier unexecuted-contract status; native renderer tests and actual shader/framebuffer verification still require passing runs at the repaired source SHA.

## Native acceptance path

Set `GAMESTRIKE_SOURCE_SHA` to the exact checked-out commit **at compile time**.
`scripts/capture-native.ps1` is a prepared Windows hosted-runner harness. It requires
the new native executable under that checkout's `target/release`, enforces process
timeouts/nonzero exits, and rejects missing screenshots/metadata or mismatched source
identity. It cannot run outside its explicit hosted-CI guard. All nine cases executed successfully in the initial diagnostic run; that run's other failures still prevent overall acceptance.

The executable uses egui's real window framebuffer screenshot event. It waits for
generated synthetic collection data, texture preparation and a recorded wgpu callback,
then validates the readback and writes PNG plus JSON. Window exit before successful
capture is an error. Synthetic mode constructs no filesystem/process/audio host and
rejects import/launch/refresh intents. It cannot read a real catalog. Existing evidence
files are never overwritten. The intentionally dark first 0.5 seconds are excluded
from automated nonblank captures; frozen interactive engineering inspection can still
use them. Human pixel/motion review is required after automated content checks.

Capture poses: 0.5, 2, 3.5, 4.5, 5.5 seconds, skip/reduced motion and target 1672x941.
Metadata labels the native renderer and synthetic library. Hosted captures do not
establish acceptance on another machine or resolve application-control trust.

## Remaining work

- Passing exact-source format/lint/contracts and fresh native captures after repairs; measured GPU cost and wider hardware acceptance.
- Full 3D motion/material parity: individual flying/rotating frame parts, camera
  choreography, environment reflections and bloom remain beyond this first slice.
- Matching licensed TTF/OTF fonts; default egui fonts are engineering-only.
- Remaining native launcher management/settings/controller parity and OS preference
  integration. Ctrl/Cmd+O or drop imports HTML/ZIP; HTML playback is offline here.
- Native managed media storage/decoding/output, soundboard and voice host adapters.
  Audio section states are deliberately unavailable; there are no fake meters/devices.
  The carried audio core has synthetic contracts but no real selectable microphone.
- Separately verified real capture/endpoint distribution and routing, with music
  excluded. No driver, device-default or security change is part of this slice.
- Official visible YouTube player integration remains a full-scope delivery item,
  deferred from this first native artifact. No audio extraction/hidden playback and
  no provider sign-in requirement for the core application.

## Single-theme and navigation cue follow-up — 2026-10-22

The fixed native PWDS orange design remains unchanged. The preserved browser preview now removes alternate-dashboard selection and migrates stale theme preferences to Collection; it is preview evidence only. The initial pitched navigation cues are provisional engineering source, not an approved final sound design. A separate original tactile contact-texture proposal is preserved for listening review. Scheduling controls include repeat limits, bounded volume and two transient voices. Native audio output is not connected, so neither synthetic PCM tests nor the browser preview establish native cue playback. No console samples were copied.
