# #1460 — Image preview in the Files viewer

Jason approved the neon trio (Antigravity `gemini-4-neon` coder, Codex `gpt-6.2-astra` reviewer, Codex `gpt-6.2-sol` qa) on 2026-11-25. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1460-image-preview` on the branch `feat/1460-image-preview` (base `origin/main` `e83a85cc`, its own `target/`). Never touch the root checkout or sibling worktrees.

## Problem and outcome

An image opened in the Files viewer shows only its name, size and **Open in default app**, and its Diff shows only two sizes. Outcome: PNG, JPEG, GIF, WebP, BMP and SVG files open in an image tab with fit, zoom, pan, live reload and a checkerboard behind transparency, and a changed image's Diff shows both versions, stacked or side by side with **Unified | Split**.

The spec is authoritative: [`docs/features/1460-image-preview.md`](../../features/1460-image-preview.md). It holds the scope, states, design rules, implementation steps and checks. The signed-off design is `runner/desktop/specs/1460-image-preview.pen` in the design repo, boards `jhAti`, `R9fIx` and `OTufM`; the spec states everything the build needs from it.

## Read first

- The spec.
- `surfaces/files/open_file.rs`: `Content`, `Read`, `read_image`, `read_file_since`, `apply_read`, `refresh_ignored`, `IGNORED_REFRESH`, `zoom`, `zoom_text`, `toggle_source`, `shows_rendered`, `effective_diff_mode`, `load_tab_diff`.
- `surfaces/files/open_file/render.rs`: `render_code_diff_switch`, `render_code_subbar`, `render_placeholder`, `render_file`, `render_tab_diff`.
- `surfaces/changes/mod.rs` (`LoadedDiffKind`, `PreparedDiffBody`, `prepare_diff_body`) and `surfaces/changes/render.rs` (`render_shared_diff_header`, `render_shared_diff_placeholder`).
- `ui/markdown/links.rs` (`image_format`) and `ui/markdown/draw.rs` (`image`); `surfaces/side_panel.rs` (`SPLIT_MIN_DIFF_WIDTH`, `effective_diff_mode`).
- `runner-core` `protocol/files.rs` (`FileContent`, `MAX_READ_BYTES`), `protocol/changes.rs` (`DiffBody`), `protocol/api.rs` (`files_read` and `changes_diff`), `protocol/wire.rs`; `runner-daemon` `files/fs.rs` (`read`) and `files/changes.rs` (`changes_diff`).
- GPUI `gpui-pre` 0.3.7: `elements/img.rs`, `platform.rs` (`Image::to_image_data`, `remove_asset`), `assets.rs` (`RenderImage`), `svg_renderer.rs` (`parse_svg`, `render_parsed`), `interactive.rs` (`PinchEvent`).
- `docs/guide/guides/chats-and-windows.md` and its zh-CN twin; `docs/guide-authoring.md`; `crates/runner-app/i18n/`.

## Deliverables

Follow the spec's implementation steps, each a commit that builds and passes its tests:

1. **Image tab.** Binary stamp, reading and decoding, the shared canvas view, subbar, states, **Image | Diff**.
2. **SVG.** Rendered and Source in the tab, with sharp zoom.
3. **Diff.** The `want_bytes` protocol addition and its daemon side, the two layouts, one-sided changes, SVG Diff.
4. **Guide and tests.** Both locales, and the spec's automated tests.

Not in scope, as the spec lists: terminal ⌘-click (#1458), feed images, remote images in Markdown (#1455), copying or editing images, other formats, remote windows, and any setting.

## Validation and handoffs

- **@coder:** `cargo test -p runner-app -p runner-core -p runner-daemon`, `make clippy`, `cargo fmt --all -- --check`, `git diff --check`, `python3 script/check-guide-translations.py`. Any new test that waits on background work, a watcher or a timer runs `cargo nextest run -p <crate> --stress-count 20 -E '<filter>'`, and the handoff states the count. Hand `@reviewer` the local commits with a summary and the checks run.
- **@reviewer:** reviews the commits against the spec, especially protocol compatibility in both directions, decoding off the UI thread with size checks before decoding, releasing replaced images and textures, stale reads, same-size reload, ignored-file polling, and that text tabs, text diffs and non-image binaries behave as before. The clean review names the live checks and the candidate commit.
- **@qa:** the spec's four live checks on the macOS development app, holding `script/qa-lock`, with `WS-FILES-02`, `WS-FILES-03`, `WS-CHANGES-06` and `WS-CHANGES-08` as the spec selects them. It records pass or fail per check with screenshots in `docs/tests/runs/1460-image-preview-mac.md`. Failures go back to @coder, and the fixed candidate is reviewed again before qa reruns the affected checks. A gesture the tools cannot send, such as pinch, is recorded as Blocked, not passed.

## Authorization

Approved crew: the neon trio. The crew may commit, push `feat/1460-image-preview` and open a pull request against `main` titled `feat(files): preview images in the Files viewer` that closes #1460, with the labels `risk-medium` and `needs-live-test`; the workflow adds `core-review` for the protocol change. Fold this brief and the spec into the commits. Remove `needs-live-test` once every selected check has passed, and drive CI green on all platforms. Do not merge: Jason reviews `core-review` pull requests.
