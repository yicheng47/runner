# #1192 — Rounded corners for the client-decorated Linux window

Jason asked for a codex duo to fix [#1192](https://github.com/yicheng47/runner/issues/1192) on 2026-11-10. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1192-linux-rounded-corners` on `fix/1192-linux-rounded-corners`, based on `origin/main` at `704b5f30`. The issue holds the expected behaviour except for Jason's 2026-11-10 clarification below: use 8px corners and a required shadow, round edge-reaching panels individually without patching GPUI, and accept square menu/popover corners.

## Problem

On Linux the app draws its own decorations, but the window is a plain rectangle out to its edges. The window opens `Opaque` (`WindowMaterial::Solid`), the resize frame paints its 6px client-side border with `theme::sidebar()`, and nothing rounds the content. Other client-decorated apps on GNOME/KDE, and Runner on macOS, have rounded corners.

## Read first

- `crates/runner-app/src/platform_ui/linux.rs`: `frame_insets`, `resize_frame`, `render_resize_frame` and their tests.
- `crates/runner-app/src/platform_ui/desktop.rs`: where the shared Windows/Linux chrome wraps its frame in `linux::resize_frame`.
- `crates/runner-app/src/surfaces/windowing/windows.rs`: the `cx.open_window` options (`window_decorations`, `window_background`).
- `crates/runner-app/src/surfaces/app_shell/mod.rs`: `render_app_shell` calls `window.set_background_appearance(...)` on every render, so a background chosen only at open is overwritten on the next frame.
- `crates/runner-app/src/appearance.rs`: `WindowMaterial::effective` / `background`.
- GPUI's `examples/window_shadow.rs` in `gpui-pre-0.3.7` (Cargo registry): the reference pattern for transparent client-decorated windows, per-corner rounding from `Tiling`, and `set_client_inset`. GPUI's X11 backend already reports `Decorations::Server` when no compositor is present (`gpui-pre-linux` `x11/client.rs`, `check_compositor_present`).

## Deliverables

1. **Transparent background only for client decorations.** On Linux, when `window.window_decorations()` is `Decorations::Client`, the window background is `WindowBackgroundAppearance::Transparent`; with `Decorations::Server` it stays `Opaque`. Make the per-render `set_background_appearance` call respect this, so it does not reset the window to opaque. macOS and Windows behaviour is unchanged.
2. **Rounded floating corners.** While client-decorated, the resize border is transparent, and the shell has 8px rounded corners with a 1px hairline border in an existing theme border/divider token and a soft drop shadow inside the existing 6px inset. A corner is rounded only when both of its edges are untiled, and nothing is rounded while maximized or fullscreen; derive this from the same inputs as `frame_insets`, as a pure function with unit tests. GPUI clips only to rectangles: round the outer corners of the caption, sidebar, main surface, Settings and any other panel that paints out to a window corner using the same corner selection. A menu or popover opened at a window corner may still paint a square corner; Jason accepted that limitation without a GPUI patch.
3. **No see-through content.** The content inside the rounded shell keeps an opaque background in light and dark themes, so the transparent window shows only outside the corners and in the resize border. Tiled edges and maximized or fullscreen windows have no transparent gap. Menus and popovers have the accepted corner-clipping limitation in deliverable 2.
4. **Resizing unchanged.** The eight resize regions, their cursors and `set_client_inset` keep working as today.
5. **Docs.** Update the window paragraph of `docs/arch/linux.md` on rounded client-decorated corners, the server-decoration fallback and the accepted menu/popover clipping limitation. No guide page describes Linux window corners, so no guide change is needed unless you find one.

**Non-goals:** no change to the resize border width, no Glass material on Linux, no macOS or Windows chrome change, no new setting.

## Tests

- Unit tests for the corner selection: floating rounds all four; each tiled edge squares its two corners; maximized and fullscreen round none; server decorations round none.
- A unit test for the background choice: Transparent for client decorations, Opaque for server decorations.
- Extend the existing `render_resize_frame` tests if the frame's structure changes, and keep `normal_edges_cover_eight_directions_without_content_or_tiled_edges` passing.
- Gates: `runner-app` tests, workspace Clippy with warnings denied, `cargo fmt --all --check`. Drive macOS, Windows and Linux CI green; Linux CI compiles and runs the `cfg(target_os = "linux")` code.

## Live checks (Jason, on Linux)

No crew member can run these: `qa` works through computer use on the macOS development app, and the corners only exist on a real Linux compositor. Do not add `qa`. List these in the PR description and keep `needs-live-test` on; Jason runs them on a Linux machine.

- **J1, floating.** On GNOME Wayland, a floating window has four 8px rounded corners with transparent areas outside them, a 1px hairline border and a soft drop shadow inside the 6px resize margin; hover the close button and drag each edge and corner to check that the rounded shell remains intact and resizing still works. Reason: deliverables 2 and 4.
- **J2, tiled, maximized, fullscreen.** Snap the window to the left half, maximize it, then make it fullscreen: tiled edges and the maximized or fullscreen window have square corners and no transparent gap. Reason: deliverables 2 and 3.
- **J3, no see-through content.** In light and dark themes, the sidebar, a terminal tab with the sidebar shown and hidden, and Settings keep opaque content and rounded outer corners. Open New chat/Add role, a confirmation dialog, Quit, the command palette, an agent update, and remote pairing/PIN dialogs: their scrims and any edge-reaching drawer panel/footer keep the same rounded corners. Menus and popovers keep opaque content but may paint a square corner when opened at a window corner because GPUI clips only to rectangles; this is an accepted limitation. Reason: deliverables 2 and 3.
- **J4, server decorations (where available).** On an X11 session without a compositor, or a compositor that forces server-side decorations, the window looks as it does today. Reason: deliverable 1.

## Handoffs

The coder implements and commits, then hands the candidate to `reviewer`. The reviewer reviews the diff only. The PR uses `Fixes #1192` and carries `risk-medium`, `frontend-only` and `needs-live-test`; the reviewer adds `reviewed` with its clean review. This brief folds into the implementation commit.

## Authorization

The codex duo may commit, push this branch and open one PR against `main`. It does not merge. Exception to Test Scope: no `qa` role is added after review; the live checks above are left to Jason.
