# 633 — Replace the app icon with the Trail mark

Tracking issue: [#633](https://github.com/yicheng47/runner/issues/633). Feature, P2. Shipped 2026-09-17 in [#640](https://github.com/yicheng47/runner/pull/640). Direction **3 · TRAIL** from *Spec — Brand mark exploration · v1* in `design/runner.pen` (frame `RuZ6e`, app tile `cweXT`), picked by Jason on 2026-09-17. The icon files were made by a codex solo crew mission from the [brief](../briefs/633-m1-trail-app-icon.md); the in-app brand mark was updated during Jason's smoke; the driving Claude session landed it.

## Why

The old mark was three thin chevrons with two ghosts at 40 %. It went wispy on the Dock and read as `>` plus noise at 16 px. Trail keeps the terminal-prompt idea, which is agent-agnostic and name-agnostic, and draws it at a weight that holds at small sizes: one prompt ahead with two echoes fading behind it.

## What ships

- **`design/app-icon.svg`**: the Apple template (transparent 1024 canvas, 824 body at 100, 100) with the design tile mapped by one uniform scale, 824 / 128. Body radius 186.6875; three chevrons on the lucide `chevron-right` shape with strokes 57.9375, 57.9375 and 61.8, opacities 0.28, 0.55 and 1, `#00FF9C` on `#0E0E10`. Explicit canvas coordinates, no nested viewBoxes.
- **Renders**: `design/app-icon.png` (1024) and `assets/icon.png` (512) from that SVG with resvg, transparent background. `assets/icon.png` feeds the Dock icon set at launch, the `.icns` built by `script/bundle-mac`, the About, Updates and update-dialog images and both READMEs.
- **`assets/icon.ico`**: rebuilt from the new PNG with the `script/make-ico.py` procedure, 16, 32, 48 and 256 px, for `Runner.exe` and the installer.
- **In-app brand mark**: `BRAND_MARK` in `crates/runner-app/src/assets.rs` draws the Trail chevrons on a 72-unit grid in `currentColor`, with `brand_mark_uses_trail_geometry` pinning it.

## Verification

- Solo mission `01M2Q6A9GMS06CPGVH2HGME89A`: a full-bleed render of the new mark against the Pencil export of `cweXT` at 1024 px differed by a mean of at most 0.04 per channel, with 59 of 1,048,576 pixels off by more than 32 (antialiasing). Both PNGs have the right size and transparent corners; the ico holds its four sizes; `cargo test -p runner-app` and `git diff --check` passed.
- The landing session checked the SVG coordinates against the design geometry and reran the checks on the branch rebased onto `main`.
- Jason's smoke passed on 2026-09-17.
