# 634 m4 — the file viewer

## Starting point

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-634-m4-file-viewer`, on branch `feat/634-m4-file-viewer`, cut from the umbrella `origin/feat/634-file-preview` at `bf77e138` (m1, m2 and m3 merged, rebased onto `main`). This brief is the branch's tip. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. Follow the umbrella by rebase (`git rebase origin/feat/634-file-preview`), never by merge.

## Problem and outcome

m3 left a Files tree whose click only selects and whose Space and Enter emit `FileOpen` with nothing listening. This mission opens files: tabs with the preview and kept states, their overflow, the widened panel, a read-only code view as plain text, Open in, find and the edge states. Highlighting and rendered Markdown are m5's; Settings is m6's.

## Read first

- [The plan](../634-file-preview/plan.md), Phase 3, m4: the authoritative scope. The [spec](../../features/634-file-preview.md), Read-only file viewer, wins on behaviour.
- The design, exported as PNGs in this worktree's ignored `target/design-ref/` from `runner/desktop/specs/634-file-preview.pen` (`yeelinks_design` `bde8051`). They win on visual detail; match them with theme tokens, sample data is illustrative; do not delete them or open `.pen` files.
  - `tree-on-the-right.png`: tabs, the widened panel with the tree on the outer edge, the code view and Open in. Its colours on code are m5's highlighting; draw plain text.
  - `control-options.png`: the Open in control's options; use the one `tree-on-the-right.png` draws.
  - `open-in-menu.png`: the Open in menu open.
  - `tab-overflow-options.png` and `tab-overflow-details.png`: the overflowing strip, its faded edges, the list button with the count and the open list; the preview tab's italic.
  - `mission-with-file-open.png`: a mission with a file open, for the 420 px rule.
  - `edge-files.png`: binary, too large, deleted while open.
  - `tree-and-viewer-states.png`: long lines scrolling sideways and find over the file.
  - `markdown-preview.png`: m5's, for orientation only.
- m1's `files_read` and `FileContent` (`crates/runner-core/src/protocol/files.rs`, `protocol/api.rs`), and the `files/changed` event m3 already applies.
- m3's Files view and its `FileOpen` hook, `side_panel.rs`, the panel's width and resize code, `terminal/find.rs` (#897's find bar), `file_links.rs` (editors, ⌘-click and Settings → General → Open file links in), `render_counts.rs`, and `keymap.rs` for ⌘W's current pane binding.
- Reference only, no dependency: Zed's `editor` crate and gpui-component's code/text view, for selection across virtualized lines and horizontal scrolling. Read how they do it before writing those two parts; Runner's own code view stays small and read-only (Jason, 2026-11-04: build it ourselves, take neither as a dependency, since Zed's editor is tied to its workspace and GPL, and gpui-component pins its own GPUI).

## Deliverables

The plan's m4 items 1–6: tabs, overflow, the widened panel, the code view, Open in and the edge states, with the spec's shortcuts on every platform. Plus:

7. **Guide**: opening files, tabs and their shortcuts in [Chats and windows](../../guide/guides/chats-and-windows.md), [Missions](../../guide/concepts/missions.md) and [Keyboard shortcuts](../../guide/reference/keyboard-shortcuts.md), in both locales.

Non-goals: highlighting, Markdown rendering (show Markdown as plain text until m5), Settings, any change to m1's daemon behaviour. A small protocol addition the viewer genuinely needs is allowed but must be justified in the handoff, since it makes the PR `core-review`. If a design question arises that the plan, spec and PNGs do not settle, ask through Runner.

## Validation

Unit tests: the tab model (preview replacement, keeping by double-click on the file or the tab, reuse, closing the last), the overflow list, the 420 px width arithmetic and the order in which the tree hides then the file narrows, ⌘W's focus scoping, find over a file, reload on `files/changed` keeping scroll, lossy UTF-8, and each edge state from snapshots in dark. A 10,000-line file renders only the visible lines (`render_counts.rs`). No file read on the UI thread. `runner-app` tests and workspace clippy with the CI profile, logged with exit codes.

Live checks are required and run by `qa` after the clean review, on macOS, dark theme only. Jason deferred these for the program on 2026-11-04, so the clean review must not select them and qa records them as deferred, not run: anything needing the real pointer (drags, resize, hover-only states), remote access, and the light theme. The rest of the plan's m4 live list applies: preview and kept tabs, the overflow and its list, the shortcuts, Open in with each installed editor, a file deleted while open, the binary and large placeholders, and a mission with a file open as the window narrows (narrowed by keyboard or window zoom, not by dragging).

## Handoffs

The coder implements, tests and fixes. The reviewer waits for the coder's Runner handoff, then reviews the whole diff against the plan, spec, PNGs and this brief, must-fix first with file:line pointers, checking in particular: no read or decode on the UI thread, virtualization of a large file, selection and copy across lines that scroll out of view, ⌘W's scoping, the 420 px rule in a mission, and the deleted-file state on `files/changed`.

## Authorization

Approved crew: the claude duo, with the codex `qa` role added by the operator after the clean review. Once review is clean the crew may commit, push this branch and open a PR against `feat/634-file-preview` (not `main`), titled `feat(ui): the file viewer (#634 m4)`, labelled `risk-medium` and `needs-live-test`, plus `frontend-only` if nothing outside `runner-app` changes, and drive CI green on every job. The operator merges it into the umbrella; the crew does not merge.
