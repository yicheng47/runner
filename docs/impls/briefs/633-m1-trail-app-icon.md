# 633 — Mission 1: replace the app icon with the Trail mark

Jason requested a codex solo crew mission on 2026-09-17. Work in `/Users/jason/repos/yicheng47/runner` on branch `feat/633-trail-app-icon`, whose tip is this brief on top of `main` (`2e8360f`). Build on top of it; do not rebase, squash or create another branch.

Read first: this brief and issue [#633](https://github.com/yicheng47/runner/issues/633) (`gh issue view 633`).

## Authorization

You implement, verify and self-review the whole diff against this brief. No subagents, new checkout or worktree. Stop with every change uncommitted: no commit, push or PR. Report once with `runner msg post`. You cannot open `design/runner.pen` (it is encrypted and only reachable through Pencil); everything you need from it is below.

## The mark (design `runner.pen`, frame `RuZ6e` "3 · TRAIL", app tile `cweXT`)

In design space the app tile is 128×128, fill `#0E0E10`, corner radius 29. Centered in it is a 72×72 mark (28 px inset on every side). The mark holds three chevrons, each a 42×42 box drawing the lucide chevron-right path `M8 4l8 8-8 8` from a 24×24 viewBox, stroke `#00FF9C`, round caps and joins, no fill. Box positions inside the mark, top-left corners: echo-2 at (0, 15) with opacity 0.28, echo-1 at (15, 15) with opacity 0.55, lead at (30, 15) at full opacity. Stroke widths are in design pixels, not viewBox units: 9 for both echoes, 9.6 for the lead.

A reference render of that tile, full-bleed at 1024×1024 (scale 8), is at `/private/tmp/claude-501/-Users-jason-repos-yicheng47-runner/3f311213-2329-4b31-b869-5768062a4d90/scratchpad/icon-probe/cweXT.png`. Read it; do not commit it.

## Deliverable

1. **`design/app-icon.svg`**, rewritten. Keep the existing Apple template: a transparent 1024×1024 canvas with the 824×824 body at (100, 100). Map the design tile onto the body with one uniform scale (824 / 128): body corner radius, mark inset, chevron boxes and stroke widths all scale by it. Write explicit coordinates, not nested `<svg>` viewBoxes, so every stroke width is in canvas pixels. Replace the comment block with a short one naming the source frames above; the old one describes the retired chevron layout.
2. **Renders.** `design/app-icon.png` at 1024×1024 and `assets/icon.png` at 512×512, both rendered from the new SVG with a transparent background. A rasterizer that works on this machine without system libraries: `uv run --no-project --with resvg-py python -c "import resvg_py; open('out.png','wb').write(bytes(resvg_py.svg_to_bytes(svg_path='design/app-icon.svg', width=1024, height=1024)))"`. Installing `librsvg` with Homebrew for `rsvg-convert` is also fine.
3. **`assets/icon.ico`**, regenerated from the new `assets/icon.png` with exactly the procedure in the docstring of `script/make-ico.py` (sips to 16, 32, 48 and 256, then `make-ico.py`). CI's Windows job checks that `Runner.exe` embeds exactly those four images from `assets/icon.ico`, so keep the four sizes.
4. **Nothing else.** `assets/icon.png` is already what the dock icon, the `.icns` built by `script/bundle-mac`, the About, Updates and update-dialog images, and both READMEs use, so no code or README text changes. Leave `design/runner.pen` alone.

## Verification

- **Geometry matches the design.** Render your SVG's mark full-bleed the way the reference is drawn (tile 128 scaled by 8 to 1024, no Apple inset; a throwaway SVG outside the repo is fine) and compare it pixel by pixel with the reference, for example with Pillow and numpy through `uv run --no-project --with pillow --with numpy`. Report the mean absolute difference per channel and how many pixels differ by more than 32 in any channel. Antialiasing noise is expected; a shifted chevron, a wrong stroke width or a wrong opacity is not.
- **Renders.** `sips -g pixelWidth -g pixelHeight` on both PNGs, and the corners of each PNG are fully transparent.
- **ICO.** `assets/icon.ico` holds four PNG images of 16, 32, 48 and 256 px; read the directory entries with a few lines of Python.
- `cargo test --locked -p runner-app --no-fail-fast --profile ci` (the embedded icon is loaded through `assets.rs`) and `git diff --check`. Report each exit code.

## Report

Branch and base commit, the files changed, the pixel comparison numbers, the render sizes and transparency check, the ico sizes, the checks with exit codes, what your self-review checked, and anything you could not prove (the look in the Dock, on the Windows taskbar and in the installer are Jason's smoke).
