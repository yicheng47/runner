# 634 m7 — links open in the preview, and the tree stays when it is on

## Starting point

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-634-m7-link-preview`, on branch `feat/634-m7-link-preview`, cut from the umbrella `origin/feat/634-file-preview` at `368ee2af` (m1–m6 merged, rebased onto `main` `d432cba3`). This brief is the branch's tip. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. Follow the umbrella by rebase, never by merge.

The working tree already holds the operator's uncommitted change for item 1, in `crates/runner-app/src/surfaces/side_panel.rs` and `crates/runner-app/src/surfaces/files/tests.rs`. Its tests have not been run. Verify it, fix it if needed, and make it part of this mission's commit.

## Problem and outcome

Jason's live run of the umbrella (2026-11-05) found two problems.

- **The tree hides under the 420 px rule while its toggle still shows on.** The rule hides the tree before narrowing the file, so with a saved file width of 554 the tree needs a window of about 1,214 px, and the tree button appears to do nothing.
- **⌘-click on a path in a terminal opens the external editor.** Runner now has its own viewer, so the link should open there first.

The outcome:

- the tree shows whenever its toggle is on;
- ⌘-click opens the file in the side panel's preview at that line;
- General → Open file links in is gone, and the Open in control is the only place to choose an editor.

## Decisions (Jason, 2026-11-05)

1. **The file narrows first, and a tree that is on always shows.** As room runs short, the file narrows from its saved width down to the panel's standard width while the tree keeps its width. Only then does the chat or mission give up its 420 px. The tree hides only when its toggle is off. The saved file width caps the file; it no longer reserves room ahead of the tree. A mission behaves the same, and the two-per-row Sessions roster is unchanged.
2. **File links open in the preview first.** A ⌘-click (Ctrl+click on Windows and Linux) on a file path in any terminal opens it in the side panel that terminal's surface shows: a chat pane, a mission member or its drawer shell, or a terminal-only tab. It opens as the preview tab, at the link's line, when all of these hold:
   - the side panel is enabled;
   - the Files view is on;
   - the session is local;
   - the path resolves inside that panel's Files root.

   It opens the panel if it is collapsed, switches it to Files, and widens it as an opened file does. Otherwise the link falls back to the saved editor exactly as today. URLs are unchanged. The link tooltip names what the click will do.
3. **General → Open file links in is removed.** The stored `file_link_editor` stays, set only by the Open in control's chevron. It drives Open in and the fallback above. Remove the General row, its strings in both locales and its tests, and update the Open in menu's wording that refers to General.

Non-goals: file-name search in the tree (Jason, 2026-11-05: after landing, separate issue), ⌘⇧-click variants, and any daemon or protocol change. If a question arises that these decisions, the spec and the design do not settle, ask through Runner.

## Read first

- [The spec](../../features/634-file-preview.md): the 420 px rule (line 18), Open in (lines 54 and 79). Update both to the decisions above.
- [The plan](../634-file-preview/plan.md), m4 item 3 and m6, and [the README](../634-file-preview/README.md), "The tree is resizable": both say the tree hides first. Update them, and add the decisions under the README's binding decisions.
- `side_panel.rs` (`widened`, `Widened`), `surfaces/files/mod.rs` (`FilesPanel::layout`, `widening`), `files/viewer.rs` (`tree_shown`, `open_at_line`, `open_link`, `launch`, `open_in`), `file_links.rs` (`open_file_link`, `link_tooltip`), `terminal/element.rs` (the ⌘-click release), the panes' header and the mission rail where each surface's `FilesPanel` lives, and `settings/page/general.rs`.
- The guide: [Chats and windows](../../guide/guides/chats-and-windows.md) (Read a file in the side panel, and ⌘-click file links), [Missions](../../guide/concepts/missions.md), [Keyboard shortcuts](../../guide/reference/keyboard-shortcuts.md) and the Settings reference, in both locales.

## Validation

Unit tests:
- the width rule, for a chat and a mission;
- the tree toggle always showing the tree;
- link routing from each surface: chat, mission member, drawer shell and terminal-only tab;
- each fallback: panel off, Files off, outside the root, and remote;
- the preview opening at the line and opening a collapsed panel;
- the tooltip's wording;
- General no longer listing the row, with Open in still saving the editor.

Run `runner-app` tests, workspace clippy with the CI profile and `--features runner-app/updater`, fmt and `script/check-guide-translations.py`, logged with exit codes.

Live checks are required. Jason runs them himself, so no `qa` role is added.
1. With the tree on, narrowing the window narrows the file while the tree stays, and the toggle hides and shows it.
2. ⌘-click on a path inside the root in a chat terminal, a mission member and a terminal-only tab opens the preview at that line, opening a collapsed panel.
3. ⌘-click on a path outside the root still opens the editor.
4. General has no Open file links in, and Open in's chevron still changes the editor.

## Handoffs

The coder implements, tests and fixes. The reviewer waits for the coder's Runner handoff, then reviews the whole diff against these decisions and the spec, must-fix first with file:line pointers. It checks in particular:
- the root check for link paths, including symlinks and `..`;
- that no file read happens on the UI thread;
- focus after a link opens the panel;
- that no stale reference to the removed General row remains in code, strings or docs.

## Authorization

Approved crew: the claude duo. Once review is clean, the crew may commit, push this branch and open a PR against `feat/634-file-preview` titled `feat(ui): file links open in the preview (#634 m7)`. It is labelled `risk-medium`, `needs-live-test` and `frontend-only` if nothing outside `runner-app` changes, and the crew drives CI green on every job. The operator merges it into the umbrella after Jason's live checks; the crew does not merge.
