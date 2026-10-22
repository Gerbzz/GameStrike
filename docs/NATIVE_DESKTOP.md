# Native Rust desktop slice

This is an uncompiled engineering slice, not a finished or verified replacement launcher.
The existing launcher/player and artwork remain intact. The new binary is
`gamestrike-desktop`; its `native-ui` feature is independent of `player` and Wry.

The source contains native egui collection/navigation controls, Rust meshes and a
retained wgpu/WGSL PWDS backdrop/tube pass, a deterministic startup clock, typed
worker-backed library/import/favorite/launch services, and a synthetic capture path.
Imported HTML games remain in the optional separate `gamestrike-player` process.
Core application UI is never rendered by that player or a browser.

## Evidence status

No new native binary, Rust test result, shader validation result or native screenshot
has been produced. Test files are contract sources, not passing-test evidence.
`Cargo.lock` needs legitimate Cargo resolution for the newly declared optional native
dependencies before `--locked` checks can run. Do not hand-edit transitive checksums.

First authorized hosted validation must resolve/review the dependency lock, run the
library/store/native/application contracts and standalone audio contracts, format and
lint the native target, then build the actual executable. Use Rust 1.96.1 and matched
eframe/egui-wgpu 0.36.2 (wgpu 30 through the reexport).

```sh
cargo test --locked --no-default-features --lib --tests
cargo test --manifest-path crates/gamestrike-audio-core/Cargo.toml --locked --offline
cargo test --locked --no-default-features --features native-ui --lib --tests --bin gamestrike-desktop
cargo fmt --all -- --check
cargo fmt --manifest-path crates/gamestrike-audio-core/Cargo.toml -- --check
cargo clippy --locked --no-default-features --features native-ui --bin gamestrike-desktop -- -D warnings
cargo build --locked --release --no-default-features --features native-ui --bin gamestrike-desktop -j 2
```

These commands are instructions for a separately authorized runner, not a report of
execution. The cross-platform workflow now declares hosted lock-resolution and Windows native build/test/capture jobs. Publication and actual run results must be recorded separately; wiring alone is not compilation evidence.

Observed local checks: 15 pure PowerShell metadata contracts passed after a negative
control exposed acceptance of a NaN pose; finite-number and boolean checks correct it.
These exercise only metadata fixtures, not a native process. Three PowerShell files
parse, Cargo TOML/native-feature/asset-reference checks pass, and new-file whitespace
checks pass. All 43 new Rust test declarations and the 53 carried audio contracts
remain unexecuted. No Rust formatting or shader validation result exists.

## Native acceptance path

Set `GAMESTRIKE_SOURCE_SHA` to the exact checked-out commit **at compile time**.
`scripts/capture-native.ps1` is a prepared Windows hosted-runner harness. It requires
the new native executable under that checkout's `target/release`, enforces process
timeouts/nonzero exits, and rejects missing screenshots/metadata or mismatched source
identity. It cannot run outside its explicit hosted-CI guard. The script has not run.

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

- Compile/format/test/shader validation, actual native captures and measured GPU cost.
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

The fixed native PWDS orange design remains unchanged. The preserved browser preview now removes alternate-dashboard selection and migrates stale theme preferences to Collection; it is preview evidence only. Original short fixed-pitch navigation, confirm, back and error cues replace layered sweeps and ambient sound, with repeat limits, bounded volume and two transient voices. The pure Rust cue policy/PCM and its contract source mirror that direction, but a native audio-output adapter is not connected and no native cue playback is claimed. No console samples were copied.
