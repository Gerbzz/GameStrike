# Native PWDS visual restoration — 2026-10-23

Scoped on native PR #15 source `0fa9968811645e543f550b191885f697fbdb67ca`,
not on `main`. At inspection, main was `096b7d8ade58eec2d909872da86ef576c4c6230d`;
the native branch was 98 commits ahead and two behind main, stacked on
`gamestrike-chrome-3d`. This patch does not roll back or replace either branch.

## Direction

Restore the reference's angular console chassis: near-black metal faces, layered
machined copper bevels, concentrated orange highlights, stronger glass tube
reflections, triangulated background mesh, a physical illuminated selection collar,
and wide display lettering. It remains one native Rust/egui/wgpu PWDS theme.
Games, Soundboard, Voice and Music retain their existing routes and behavior.

## Changes

- Separate casing/lip/inner-bevel facets replace the uniform cream-brown edging.
  Header and selected-title plates have clipped fine metal grain; the emblem gains
  an angular mounting plate and the existing screw sockets gain slots.
- Selected cartridges receive a wider orange/gold collar and directional highlight,
  while inactive cartridges stay darker. Pointer hit rectangles/chamfers are unchanged.
- Existing bundled OFL fonts are converted to native TTF derivatives with preserved
  outlines/metrics, distinct names, embedded licenses and proportional fallback.
  No system fonts, new runtime dependencies, or downloaded font requests are used.
- Three complete cover rows now fit in the 1672×941 and 1250×800 wells. Column
  counts remain seven/six. Row height, hit testing, scroll limits and keyboard reveal
  share the same geometry. No trailing gap/rounding residue creates a phantom bar.
- Collection filters sit below the four primary routes instead of at the bottom of
  a large blank sidebar; the remaining bay carries vents and the existing import shortcut.
- The isolated synthetic capture library contains 21 original geometric illustrations
  across seven scene families instead of 12 repeated stripe patterns. Every fixture
  retains `OWNED SYNTHETIC FIXTURE` provenance, each visible card says `SYNTHETIC
  FIXTURE`, and the header says `SYNTHETIC LIBRARY`. No commercial cover pixels or
  fabricated real-library entries are added. Real catalog counts/art are untouched.

## Boundaries and verification

No Music transport/decoder/output, communication audio, filesystem/process host,
application authority, dependency version, startup timing, reduced-motion policy,
capture safety guard, CI workflow or README evidence image is changed.

Preparation checks verified the materialized base files against their GitHub blob
SHAs. FontTools verified all source-to-TTF glyph outlines, cmap, advance/sidebearing
metrics and original copyright, plus renamed families and embedded OFL text.
Independent float32 arithmetic checked layout at 640×360, 880×620, 960×600,
1250×800, 1672×941 and 2560×1440. Those are source/asset checks, **not native execution**.

New/extended Rust contracts cover complete third-row visibility, sidebar/filter
containment, no phantom scrolling, fixture count/provenance and deterministic,
bounded, opaque, distinct fixture illustrations. The hosted native checks must run
on the published exact source before claiming them passed. A real egui/wgpu
framebuffer capture and human pixel review are still required for visual acceptance;
no mockup or browser rendering is evidence of the native change.

The existing `scripts/capture-native.ps1` acceptance path should be used unchanged
after formatting, tests, fatal Clippy and the optimized native build succeed. Review
1672×941 and 1250×800, all existing frozen startup poses, skip/reduced motion, a
small actual catalog, longer/non-Latin titles, and keyboard/pointer navigation.
Hosted synthetic evidence cannot establish actual-library, physical-device or
speaker-output acceptance. This document does not authorize release or deployment.
