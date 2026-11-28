# 1460 — Image preview in the Files viewer

Tracking issue: [#1460](https://github.com/yicheng47/runner/issues/1460). It comes before Mermaid (#1446) and footnotes, callouts and `<details>` (#1448) (Jason, 2026-11-24).

Design: `runner/desktop/specs/1460-image-preview.pen` in the design repo, boards `jhAti` (opening an image), `R9fIx` (a changed image's Diff) and `OTufM` (SVG, states, light mode and behaviour), signed off 2026-11-25.

## Motivation

Agents constantly produce screenshots, charts and generated assets. Opening one in the Files viewer shows its name, "Binary file · size" and **Open in default app**, and its Diff shows only the two sizes, so checking one means leaving Runner. The viewer already draws local images inside Markdown; image files themselves never reach that path.

## Scope

In the unified viewer (`crates/runner-app/src/surfaces/files/open_file.rs` and `open_file/render.rs`), for the formats of `image_format` in `ui/markdown/links.rs`: PNG, JPEG, GIF, WebP, BMP and SVG, recognized by extension.

- **Image tab.** The image opens fitted inside a 16 px margin, centred, never above 100%, and clear of the zoom control. The subbar shows the name, then `3368 × 2196 · 886 KB` in place of the folder, then **Open in**. The tab keeps its place in the strip, its preview and pin behaviour, and the icon set's own icon.
- **Transparency.** A checkerboard of 8 px squares behind the image's own area only, with a dark and a light pair. The rest of the canvas is the viewer background.
- **Zoom.** A `− 12% +` stepper floats at the canvas's bottom right, on an opaque panel background with a border and shadow. − and + step the zoom; the percentage shows the current zoom and returns to fit when pressed. Steps run from fit to 800%. 100% draws one image pixel per point. ⌘+ and ⌘− (Ctrl+= and Ctrl+−) step the zoom while the image tab has focus, instead of the text size; ⌘0 keeps its app-wide meaning. Pinch zooms continuously and keeps the point under the pointer still; the stepper and keys keep the centre still. Double-click switches between fit and 100%.
- **Pan.** Scrolling pans an image larger than the canvas in both directions, and stops at its edges.
- **Per tab.** Each tab keeps its own zoom and position until it closes; nothing is saved and there is no setting.
- **Reload.** The tab reloads when the file changes on disk. Zoom and position stay when the dimensions are unchanged; otherwise it fits again. While the new version decodes, the old one stays on screen.
- **SVG.** Opens rendered, with the **Rendered | Source** toggle Markdown has; Source is today's text view, with find, and no zoom control. As on a rendered Markdown page, ⌘F switches to Source. Zooming redraws the vector, so it stays sharp. An SVG that cannot be parsed opens on Source.
- **GIF.** Plays in the tab and in Diff, and does no work while its tab is not shown.
- **Switch.** The header's switch reads **Image | Diff** for an image, including an SVG. On a deleted working-tree file, **Image** is disabled as **Code** is today.
- **Diff.** A changed image's Diff shows the old and new versions, captioned `− Before` and `+ After` with their dimensions and sizes. It follows **Unified | Split** and the choice Runner remembers: Unified stacks them, Split places them side by side, with the width rule, maximizing and restoring that text diffs have. Both are drawn at the same scale, with the fit chosen so that both are visible, and zoom and pan together. An added or untracked image shows only the new version, captioned `+ Added`; a deleted one only the old, captioned `− Deleted`; neither shows **Unified | Split**. The header, scope, **Viewed**, **Open in** and the footer stay, without the `bin` count. This holds for uncommitted changes, a branch and a commit, and for renames.
- **SVG Diff.** Rendered by default, with **Rendered | Source** in the header; Source is today's line diff.

States:

- **Over 10 MB.** Today's "too large" placeholder, in the tab and in Diff.
- **Too many pixels.** An image over 16,384 px on a side or 64 megapixels gets the same placeholder with its dimensions; its header is read before anything is decoded. A GIF whose frames together exceed 64 megapixels shows its first frame as a still.
- **Cannot be decoded.** "Can't show this image" and "{name} may be damaged or incomplete." with **Open in default app**. The tab tries again on the next change, since an agent may still be writing the file. In Diff, a side that cannot be decoded shows that message in its place and the other side still shows.
- **Deleted while open.** The last version stays, dimmed, under the banner text tabs show.
- **Not an image after all.** A raster file whose content is text, such as a Git LFS pointer, keeps today's text view and line diff, with **Code | Diff**.

Not in this change: ⌘-click from a terminal (#1458), images in the mission feed, remote images in Markdown (#1455), resizing or editing, copying an image, and TIFF, ICO or AVIF. Remote windows have no Files panel, so nothing here crosses the relay.

## Design

- **Reading.** Images are read with `files_read(root, path, true)`. `FileContent::Binary` carries no modified time today, and `apply_read` treats a binary of the same size as unchanged, so a screenshot regenerated at the same size would not reload. Add `modified` to `Binary`, optional and defaulted as `bytes` is, and keep a stamp for images as text has one.
- **Ignored files.** An open ignored file is read again every three seconds (`IGNORED_REFRESH`). For an image, ask without bytes first and fetch them only when the stamp changed, so an unchanged 10 MB image is not sent every three seconds. Against a daemon without `modified`, compare the bytes.
- **Diff protocol.** `changes_diff` gains `want_bytes: Option<bool>`, following `files_read`, and `DiffBody::Binary` gains optional `old` and `new` bytes, defaulted and skipped when absent. The daemon fills them only when asked and only for image extensions, from the blobs or working file it already reads. An older client never asks, and an older daemon answers without bytes, which shows today's size placeholder. Both sides fit `wire::MAX_FRAME` (64 MiB), as a 10 MB text diff does today. SVG needs nothing new: `DiffBody::Text` already carries both sides.
- **Decoding.** On the background executor, never on the UI thread, and dropped when the tab has moved on, as stale reads are now.
- **Downscaling.** GPUI samples linearly with no mipmaps, so a large screenshot drawn at 12% would look jagged. Prepare successively halved copies in the background and draw the smallest one that is at least the drawn size in device pixels. Never give GPUI a texture over 8,192 px on a side; use a halved copy instead. Animated frames are drawn as they are.
- **Memory.** GPUI keeps decoded images in its asset cache and their textures in the sprite atlas until told otherwise. Release both when an image is replaced, when its tab closes and when the panel goes away, so an agent rewriting a screenshot all day does not grow the app. Decoded images of tabs not shown may be dropped and decoded again.
- **SVG zoom.** Rasterize from the parsed SVG at the drawn size in device pixels, in the background, keeping the previous raster on screen until the new one is ready, within the same texture limit.
- **One view.** The image canvas (fit, zoom, pan, checkerboard, stepper) is one GPUI view that the tab and both Diff layouts share, so #1446 can reuse the stepper.

## Implementation steps

1. **Image tab.** The stamp for binary files, reading and decoding, the canvas view with fit, checkerboard, zoom, pan and reload, the subbar, states, and **Image | Diff**.
2. **SVG.** Rendered and Source in the tab, with sharp zoom.
3. **Diff.** The protocol addition with its daemon side, then the two layouts, one-sided changes and SVG Diff.
4. **Guide and tests.** The guide pages below and the tests in Verification.

## Guide pages

- `docs/guide/guides/chats-and-windows.md` and `docs/guide/zh-CN/guides/chats-and-windows.md`: reading a file (images, SVG, zoom, the limits), the **Code | Diff** paragraph, and the Diff paragraph's list of dedicated summaries.

## Verification

### Automated

- Reading: an image regenerated at the same size reloads; an unchanged ignored image is checked without sending its bytes; a daemon without `modified` still reloads.
- Protocol: the new fields round-trip; a request without `want_bytes` and a reply without `old` and `new` still parse; bytes come back only when asked, only for images, and not over the limit; uncommitted, branch, commit, added, deleted, untracked and renamed images.
- Canvas: fit never exceeds 100% and stays clear of the stepper; steps, the percentage button, double-click, keys and pinch; panning stops at the edges; zoom and position survive a reload with unchanged dimensions and reset otherwise; ⌘+ and ⌘− still step the text size in text tabs and in SVG Source.
- States: over 10 MB, too many pixels, damaged then repaired, deleted while open, a text file named `.png`, a GIF over the frame limit.
- Diff: Unified and Split layouts at one scale; one-sided changes without the switch; one undecodable side; SVG Rendered and Source; a non-image binary keeps its placeholder.
- Memory: replacing an image and closing its tab release the old decoded image.
- Gates: `cargo test -p runner-app -p runner-core -p runner-daemon`, `make clippy`, `cargo fmt --all -- --check`, `git diff --check` and `python3 script/check-guide-translations.py`, plus a 20-run nextest stress loop for any new test that waits on background work, a watcher or a timer.

### Live checks

On the macOS development app, by `qa` holding `script/qa-lock`, with an owned fixture repository under `/tmp` as the regression suite describes, plus a large Retina screenshot, a transparent PNG, a small icon, a JPEG, a WebP, a BMP, an animated GIF, an SVG, an image over 10 MB, a truncated PNG and a PNG over 64 megapixels:

1. **Image tab** (with `WS-FILES-03`): open each from the tree, the finder and a Markdown link. Each fits and is sharp, including the Retina screenshot; transparency shows; dimensions and size are right; the stepper, keys, pinch, double-click, scrolling and **Open in** work; ⌘+ and ⌘− in a text tab still change its text size; tabs keep their own zoom; dark and light themes, docked and maximized. A non-image binary keeps **Open in default app**. Reason: the tab, its controls and its keys are new.
2. **Reload and deletion** (with `WS-FILES-02`'s edit and delete steps): overwrite the open screenshot at the same dimensions and size, then at other dimensions, in a tracked and in an ignored folder; truncate and repair it; delete it. The image updates each time as the spec says. Then rewrite it 50 times and compare the app's memory before and after. Reason: reload and releasing images are new.
3. **SVG**: Rendered and Source, sharp at 400%, ⌘F to Source, a broken SVG. Reason: SVG changes from text to rendered by default.
4. **Diff** (with `WS-CHANGES-06` and `WS-CHANGES-08`): a modified image with changed dimensions, an added, an untracked, a deleted and a renamed one, and a modified SVG and GIF, in Uncommitted, All changes and one commit; Unified, Split in a narrow panel with Restore chat, and Split maximized; zoom and pan move both; **Viewed** works; the plain binary and over-10 MB files keep their placeholders. Reason: image Diff and its protocol are new.

Selected regression cases: `WS-FILES-02`, `WS-FILES-03`, `WS-CHANGES-06`, `WS-CHANGES-08`.
