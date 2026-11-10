# #1238 — the remote launcher's Change menu has a readable width

Jason asked for a codex duo to fix [#1238](https://github.com/yicheng47/runner/issues/1238) on 2026-11-10. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1238-launcher-change-menu-width` on `fix/1238-launcher-change-menu-width`, based on `origin/main` at `1cd12945485634416381f854e676825a581cc5b6`.

## Problem

In a window showing another computer, **Change** on the launcher's default page opens the host's folder menu only as wide as the link, about 36px: the heading wraps one character per line and the rows cannot show their names. From reading the code, not yet reproduced: `FolderPicker` falls back to its anchor's width when no popup width is set (`surfaces/front_page/folder_picker.rs`, the `popup_layer` call at the end of its render), and the launcher anchors the picker inside the `launcher-change` link (`surfaces/start_chat/launcher.rs`, `FolderPicker::anchor` under `.id("launcher-change")`) and creates it without `set_popup_width`. The New project menu (`surfaces/remote_client/project.rs`) sets `320 * app_zoom` and is not affected. Local windows use the OS folder picker and are not affected.

## Outcome

Opened from **Change**, the host folder menu is `320 * app_zoom` wide, matching the New project menu, and its right edge lines up with the link's right edge, so it opens leftward over the launcher instead of hanging past the card. The existing viewport clamp in `popup_layer_positioned` keeps it inside a narrow window. Unchanged: the New project menu keeps its left-aligned 320px popup, and the menu's contents, Browse navigation, Use folder and dismissal behave as today.

## Read first

- `crates/runner-app/src/surfaces/front_page/folder_picker.rs`: `set_popup_width`, the render's `popup_layer` call, and the test `new_project_menu_has_a_readable_width_independent_of_the_plus_anchor`.
- `crates/runner-app/src/surfaces/start_chat/launcher.rs`: where the picker is created (`FolderPicker::new`) and where the `launcher-change` link renders its anchor.
- `crates/runner-app/src/surfaces/remote_client/project.rs`: the New project menu's width, the pattern to follow.
- `crates/runner-app/src/ui/menu.rs`: `popup_layer`, `popup_layer_right_aligned` and `popup_layer_positioned`.

## Deliverables

1. Give the launcher's picker a popup width of `320 * app_zoom` when it is created, as the New project menu does, and keep it in step with the zoom setting the same way that menu does.
2. Let the launcher's picker open right-aligned to its anchor, through a small option on `FolderPicker` that selects `popup_layer_right_aligned`; the New project menu keeps the default left alignment. Prefer the smallest shape that fits the existing code over a new abstraction.
3. A test beside the New project one, at zoom 1 and 1.5: open the launcher in a remote window, click `launcher-change`, and assert that `HOST_FOLDER_MENU` is `320 * zoom` wide and that its right edge equals the link's right edge (or lies inside the window when the clamp applies). The test must fail without the fix; say so in the handoff.
4. `docs/guide/guides/remote-access.md` describes **Change** without its width, so no guide change is expected; update it only if the reviewer finds a sentence the fix contradicts.

**Non-goals:** no change to the menu's contents, styling or the New project menu's placement, and no change to local windows' OS folder picker.

## Validation

`runner-app` tests (including the folder picker and remote client tests), workspace Clippy with warnings denied and `cargo fmt --all --check`. Drive macOS and Windows CI green.

## Live checks (Jason)

Crew slots do not contact the hosted service, so neither slot runs the app. Jason runs these checks himself, and the PR keeps `needs-live-test` until they pass:

- **L1:** in a window showing another computer, **Change** on the launcher opens a readable menu listing projects and recents, and **Browse folders** navigates the host's folders and **Use folder** picks one.
- **L2:** the menu stays inside the window at a narrow window width and at 150% zoom.

## Handoffs

The coder implements and commits, then hands the candidate to `reviewer`. Fold review fixes into the commit and re-review until clean. The PR uses `Fixes #1238`, names L1 and L2 for Jason, and carries `risk-low`, `frontend-only` and `needs-live-test`.

## Authorization

The codex duo may commit, push this branch and open one PR against `main`. It does not merge.
