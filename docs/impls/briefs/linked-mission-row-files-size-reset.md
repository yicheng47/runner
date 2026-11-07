# Linked mission rows span the group; Files font size loses Reset

Two small UI fixes Jason asked for on 2026-11-07, in one PR with one commit each. Work in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-linked-mission-row-files-size-reset` on `fix/linked-mission-row-files-size-reset`, based on `origin/main` at `c33e9d475303a5fbd5586dc06c941269e6428fdf`. There is no tracking issue or design frame; this brief is the source.

## 1. A linked mission's highlight is the whole row

**Problem.** Inside a linked group, a mission row's hover and selected highlight starts just before its flag and leaves the space under the link lines empty, so the row reads as a smaller card nested in the group. `linked_mission` indents the row with left padding on a wrapper around it, so the row shell's fill, border, hover and click area all start there. Clicking left of the flag does nothing. The group's head row already spans the outline's inner width.

**Outcome.** The row spans the outline's inner width and keeps its content where it is, by moving the indent into the row's own padding.

- A linked mission's row reaches the outline's inner edge on the left and right, in Projects and in Active. Active's outline has no gutter, so its reach differs from Projects'.
- The flag stays exactly 20 px right of the chat's mark, including under a pinned head, whose mark sits 16 px further right. Title, trailing slot and link lines do not move, and the lines draw over the fill.
- The whole row is the hover, click, context-menu and drag target.
- The mission row shows its fill with no border of its own, selected or hovered, as the head row does through `bordered: false`. Its own border would double up against the outline. The outline's selected style is unchanged.
- No fill pokes past the outline's rounded corners, including the last mission's bottom corners.
- The inline rename row for a linked mission lines up the same way.
- A mission outside a group looks and behaves as it does today.

**Read first**, under `crates/runner-app/src/surfaces/sidebar/`: `linked_mission`, `head_shape`, `active_link_group`, `render_link_group` and the geometry constants in `link.rs`; `render_mission_row` and `render_inline_rename_row` in `rows_render.rs`; `render_active_row` in `active.rs`; `sidebar_row_shell` in `elements.rs`.

**Tests.** Geometry tests in `link_tests.rs` derive the flag as `row.left() + 11`, which stops holding; read `SIDEBAR_MARK` instead. Add assertions in both views that the row reaches the outline's inner edge and the flag is still 20 px from the chat's mark.

## 2. Files font size has no Reset

**Problem.** Settings → Side panel → Files → Font size shows a Reset button once the size is changed. Jason: "the change of the font size no need reset. We don't need a default one to return to."

**Outcome.**

- Remove the Reset button from the row. The stepper stays and works as today.
- Keep the data model: `files_font_size` stays `Option<u16>`, and an unchanged size still follows the terminal's. No settings or storage change.
- Remove `⌘0` (Ctrl+0) from the viewer (Jason, 2026-11-07, during this mission, replacing this brief's first instruction not to change it). Remove `ZoomFileReset`, its binding and its handler, so that with focus in the viewer the key reaches the ordinary app zoom reset, as it does everywhere else; that fall-through is the operator's choice, not Jason's words. `⌘+` and `⌘−` keep stepping only the viewer's text. Nothing in the UI writes `None` any more, so drop the parameters and branches this leaves dead. A changed size no longer has a way back to following the terminal.
- Description, in `crates/runner-app/i18n/`. English: "Follows Terminal font size until changed. Rendered Markdown text scales with it." Chinese: "更改前跟随终端字号。渲染后的 Markdown 文本随之缩放。"
- Keep the shared `ui.reset` string; other screens use it.
- Update the Font size sentence under Set up the side panel in `docs/guide/guides/chats-and-windows.md` and its `docs/guide/zh-CN/` counterpart, and the `⌘0` clause on that page and the viewer's `⌘0` row in `docs/guide/reference/keyboard-shortcuts.md`, with their counterparts. Update the Viewer · Font size line and the Zoom in the viewer decision in `docs/features/634-file-preview.md`, noting Jason's decisions and date.
- Rework `the_files_page_sets_every_option_and_the_size_resets_to_follow_the_terminal` in `surfaces/settings/page/side_panel.rs` to assert that no Reset appears after the size changes.

## Validation

Automated: the `runner-app` tests and workspace Clippy, plus the two-platform CI.

Live checks, run by `qa` on the reviewed candidate. Each is tied to a change above; do not expand beyond them.

1. **Projects, hover and selected.** A linked mission's fill spans the outline's inner edge to its right edge with no row border, the flag and link lines stay put, and no corner pokes past the outline. Reason: the row's geometry and border change.
2. **Active, hover and selected.** The same, with Active's two-line rows. Reason: Active builds its rows and outline separately.
3. **Pinned head.** The same in a group whose chat is pinned, where the flag still sits 20 px right of the chat's mark. Reason: a pinned head moves the mark, and the row's padding must follow it.
4. **Clicking and dragging from the newly covered area.** Clicking left of the flag opens the mission, right-clicking there opens its menu, and dragging a linked mission from there reorders it within its group and out of it. Reason: the row's hit area grows. Jason ran the drag by hand on `b85cac6a` and it passed, since `qa`'s runtime could not post real drags.
5. **Settings → Side panel → Files → Font size.** Change the size: no Reset appears, the stepper still changes the viewer's text, and the description reads as above in English and Chinese. Back up the development setting first and restore it afterwards. Reason: the button and copy change.
6. **Viewer zoom keys.** With focus in a file and in a rendered Markdown page, `⌘0` leaves the viewer's text size alone and resets the app's zoom, and `⌘+` and `⌘−` still change only the viewer's text. Back up any development setting this changes and restore it afterwards. Reason: the shortcut's routing changes.

## Authorization

Jason approved this mission on 2026-11-07, and the claude trio is the crew that the route for a change with live checks names. It may commit, push and open one PR against `main`.
