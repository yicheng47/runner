# 1141 — Git changes view

> Tracking issue: [#1141](https://github.com/yicheng47/runner/issues/1141). Scheduling and priority live on the issue and in the [roadmap](../roadmap.md). Platforms: macOS, Windows and Linux.
> Status: spec agreed by Jason on 2026-11-13, with the [decisions](#decisions-settled-with-this-spec) below; m1 is in development. The design boards `B38Ah` and `o98BJ` still show the keyboard and Viewed-box details this spec changed, and are updated before m2.
> Design: `runner/desktop/specs/1141-git-changes.pen` in `yeelinks_design`, revision `563d27b`. Boards: `H5T5Lj` (where the file list sits, screen `D4i2wZ`), `EqrNQ` (from Files to the diff, screen `pCdt6`), `B38Ah` (keyboard map, table `TU4G7`), `Xc2TS` (Split with the panel maximized, screen `tv7xO`) and `o98BJ` (viewed files, screen `TjFT3`).
> Builds on: [#634](./archive/634-file-preview.md), whose side panel, `runnerd` file service, watcher, status letters, code view and highlighter this reuses. Related: [#1293](https://github.com/yicheng47/runner/issues/1293) (line comments and review to the agent, the next stage), [#1072](https://github.com/yicheng47/runner/issues/1072) (side panel views).

## Motivation

Reading what an agent changed is the most common reason to leave Runner. #634 shows the project and marks what changed in it; this feature shows the diff itself and keeps track of which changed files the reviewer has read, so the work can be judged without leaving the cockpit.

## Scope

### The Changes view

Changes is a surface of the side panel, beside Info and Files, opened from the `+` menu with the key `C`. It works like lazygit (Jason, 2026-11-03): a list of changed files beside the diff of the selected file. Moving the selection with ↑↓, j/k or a click swaps the diff at once, and no file opens as a tab.

- **Layout.** The list sits on the panel's outer edge, where the Files tree is, and the diff on the inner side beside the chat (Jason, 2026-11-13). Changes opens wide, as Files does with a file open: the diff takes Files' saved file width and the list Files' tree width, with the same divider and the same 160 to 480 px range, so switching between Files and Changes neither moves the list nor resizes the panel.
- **Where it is offered.** Wherever Files is: a chat's panel, a mission's panel after Sessions, and a terminal-only tab's panel. A terminal-only tab, which today shows Files alone as a permanent tab, shows Files and Changes as two permanent tabs, with no × and no `+`.
- **Its root.** Changes shows the checkout of Files' root, saved by `runnerd` for the chat, terminal or mission, and covers the paths under that root: the whole checkout when the root is its top, or the folder's subtree after Show files from here. Changes has no picker of its own; the root is chosen in Files. A chat outside a project with no root chosen shows Files' "Choose a folder to show" screen, and a root that no longer exists shows Files' missing-folder screen.
- **Remote sessions.** A chat, terminal or mission running on another computer shows the same state as Files: files from another computer aren't available. Remote support is deferred, not excluded (Jason, 2026-11-13): every diff is computed by `runnerd`, so the other computer's `runnerd` can serve it later without a new model in the app.

### The file list

- **Rows.** Each row shows the Viewed box, the status letter in #634's colours (`M` modified, `A` added, `R` renamed, `D` deleted, `U` untracked, `!` conflicted), the file name, its folder dimmed, and its `+`/`−` line counts. A binary file shows `bin` instead of counts. The full path is in the row's tooltip, and a renamed file's tooltip also names its old path.
- **Summary.** The section header shows the file count, how many are viewed, and the total `+`/`−`: "7 files · 3 viewed +159 −47".
- **Flat or tree.** Two buttons in the summary row switch between a flat list, the default, and a tree grouped by folder whose folders expand and collapse like the Files tree's. The choice is remembered for every Changes view.
- **Deleted files** are listed here and show as all removed. The Files tree still leaves them out.
- **Order.** Rows sort by path, so a file keeps its place when its counts change.

### Commits and what is compared

The list has two sections: Commits above, files below.

- **Commits** starts with "Uncommitted changes" and its file count, then the branch's commits since its base, newest first, each with its short commit id and subject, and its author and date in the tooltip. The section header names the branch, or "detached at" with the short commit, and the commit count.
- **The base** (Jason, 2026-11-13) is where `HEAD` split off the remote's default branch: the merge base of `HEAD` and `origin/HEAD`, falling back in turn to `origin/main`, `origin/master`, then a local `main` or `master` when the repository has no `origin`. The commits are those reachable from `HEAD` and not from the base, so a branch worktree lists its own commits, the main checkout lists its unpushed commits, and a checkout level with its base lists none. With no base at all, Commits shows only "Uncommitted changes". A merge commit made by merging the base into the branch is listed; the base's own commits are not. At most 200 commits are listed, followed by "N older commits not shown".
- **Selection.** Selecting a row in Commits shows its files below and selects the first. Changes opens on "Uncommitted changes" when it has changes, otherwise on the newest commit, otherwise on "No changes since <base>".
- **The label** in the diff header always says what is compared, since a shared checkout mixes the human's edits and other sessions' changes and must not imply the diff is the session's:
  - "Working tree vs HEAD" for uncommitted changes, which include staged, unstaged and untracked files in one diff per file, as `git diff HEAD` with untracked files added.
  - "a1f3c9e vs its parent" for a commit, "a1f3c9e vs its first parent" for a merge commit, and "a1f3c9e, the first commit" for a root commit.
  - "Working tree, no commits yet" in a repository whose `HEAD` has no commit, where every file shows as added.

### The diff

- **Header.** The file name, its folder abbreviated (`crates/…/src`), a renamed file's old path before an arrow, its `+`/`−` counts, the Unified and Split switch, and the comparison label.
- **Unified** shows both line-number columns, a `+` or `−` gutter, and added and removed line backgrounds. Each hunk starts with its `@@` row, which names the function git gives it. Three lines of context surround each change.
- **Split** shows the old file on the left and the new on the right, a changed line paired across, and empty space where one side has no line. Long lines scroll both sides together.
- **Unified or Split** is chosen in the diff header and remembered for both Changes and a file's diff in Files.
- **Highlighting** uses #634's highlighter in the terminal theme's colours. Each side is highlighted as a whole file, the old version for removed lines and the new for added and context lines, so a removed line inside a block comment or string is coloured correctly. A file in a language without a bundled grammar shows plain.
- **Rendering** is virtualized like the code view, so a lockfile's thousands of changed lines do not stall the frame. Long lines scroll sideways and are never wrapped. The font and size are Settings → Side panel → Files' Viewer font and size, and zoom in the diff steps that size as it does in the viewer.
- **Selection.** Text can be selected and copied with ⌘C (Ctrl+C), which copies the lines' text without the gutter.
- **Edge cases**, each a placeholder in place of the diff, under the same header:
  - A binary file: its old and new sizes, and Open in default app.
  - Either side over 10 MB, #634's limit for reading a file: its old and new sizes, and Open in.
  - A change only to line endings, as when a file moves between LF and CRLF: "Only line endings changed". Elsewhere a CRLF line shows without its carriage return, on every platform.
  - A change only to the file's mode: "Mode changed from 100644 to 100755". A mode change with content changes notes the mode in the header.
  - A submodule whose commit moved: "Submodule moved from abc1234 to def5678". A nested repository that is not a submodule is not listed, as in Files.
  - A conflicted file shows its working-tree diff against `HEAD`, conflict markers included, with "Conflicted" in the header.
  - A symlink shows the change to its target as text, as git does.
- **Live refresh.** #634's watcher already reports edits, the index, `HEAD` and the branches. The list, the commits and the open diff refresh within its debounce window, and the selection stays on the same path when it still has changes. When the selected file's changes go away, as when it is committed, the selection moves to the next row.

### Viewed files

- **Marking.** Each changed file has a Viewed box at the start of its row. A click on the box, Space while the list has focus, or the "Mark <file> viewed" box at the end of its diff toggles it. Nothing is marked automatically.
- **Look.** Viewed rows dim their name and counts, and the summary counts them.
- **Changed since viewed.** When a viewed file's diff changes in Uncommitted changes, its box clears and turns amber, and its folder hint reads "changed" in amber, until it is viewed again.
- **What a mark belongs to** (Jason, 2026-11-13): a mark is the checkout, the path, and the two versions compared, by their git blob ids, `runnerd` hashing a working-tree file. A file shows as viewed whenever a mark matches what it compares, so a mark made in Uncommitted changes carries over when the agent commits the file unchanged, and a commit's files keep their marks since a commit never changes. A file shows as changed since viewed when it is not viewed and an earlier mark exists for its path in the same checkout, made in Uncommitted changes.
- **Persistence.** Marks are kept by `runnerd`, so they survive restarts, show in every window on the same checkout at once, and can follow remote sessions later. Marks for a checkout that no longer exists are pruned when `runnerd` starts.

### From Files to the diff

A changed file open in Files gets a Diff button in the tab bar beside Open in, shown only while the file has uncommitted changes (Jason, 2026-11-13). It turns the file's tab into its diff in place, under the same header as in Changes: its counts, Unified and Split, and "Working tree vs HEAD". Pressing it again returns to the file. The choice is kept per tab while the tab is open. When the file's changes go away, the button hides and the tab shows the file. A file's diff in Files has no Viewed box; viewing is done in Changes.

### Room for Split

- **Maximize.** The side panel's header carries the `maximize-2` button a pane shows in a split, with the same shortcut, `toggle-maximize-pane` (⇧⌘↩, Ctrl+Shift+Enter), while the panel has focus (Jason, 2026-11-13). Maximized, the panel takes the whole tab as a maximized pane does: a chat, or a mission and its drawer, is hidden, and the button turns into `minimize-2` on the Secondary fill, labelled Restore chat. With focus in a chat the shortcut keeps maximizing its pane as today.
- **Split's width.** Picking Split where each side would be narrower than about 360 px maximizes the panel. After Restore chat, the diff shows Unified until the panel is wide enough again; the remembered choice stays Split. The chat is never covered otherwise, so its 420 px minimum holds.
- The control belongs to the panel, so a wide file in Files, or Sessions in a mission, can be maximized the same way. Maximizing is not remembered across restarts.

### Keyboard

Keys apply only while focus is in the Changes view; the terminal keeps its own. Focus moves between three places: the Commits section and the files section of the list, and the diff.

| Action | Keys |
|---|---|
| Next or previous file, or commit | ↑ ↓, j k, while the list has focus |
| Move between Commits and files | Tab, ⇧Tab |
| Enter the diff | Enter, or a click on a line |
| Back to the list | Esc |
| Mark the file viewed | Space, while the files section has focus |
| Next or previous line | ↑ ↓, j k, while the diff has focus |
| Next or previous hunk | `]` and `[` |
| Scroll sideways | ← → while the diff has focus |
| Maximize or restore the panel | ⇧⌘↩ (Ctrl+Shift+Enter), as for a pane |

The line the diff's cursor is on is tinted; #1293 attaches comments to it. Keys for comments are in #1293.

### Settings → Side panel

The Surfaces card gets a Changes row with its key `C`, on by default. Turning it off closes open Changes tabs and removes it from `+`; a terminal-only tab then shows Files alone again. Changes has no settings page in this version: Unified or Split and flat or tree are remembered from the view itself, and the font is the Files page's.

### Where it runs

`runnerd` does everything that touches git, as for #634 ([Where the work runs](../impls/archive/634-file-preview/plan.md#where-the-work-runs)). Every request names a root and paths inside it, which `runnerd` refuses outside the root. The app draws the list and the diff, pairs Split's lines, and highlights both sides. Git runs through the login-shell PATH, with `--no-color`, `--no-ext-diff`, no pager and `core.quotePath=false`, and with `-z` wherever paths are listed.

New requests, all slow, beside #634's `files_*`:

| Request | Returns |
|---|---|
| `changes_commits(root)` | The branch or detached commit, the base it resolved to (or none), and the commits since it: id, short id, subject, author, date and parent count, at most 200 with the total |
| `changes_files(root, target)` | For `uncommitted` or a commit id: each changed file under the root with its status, old path for a rename, added and removed counts or binary, both blob ids, and whether a viewed mark matches; plus the totals |
| `changes_diff(root, target, path)` | The hunks as line ranges with their `@@` rows, and the old and new contents within #634's 10 MB limit per side, or a placeholder reason: binary, too large, mode only or submodule |
| `changes_set_viewed(root, path, old blob, new blob, viewed)` | Stores or removes a mark and pushes `changes/viewed` to every client watching that checkout |

Refreshes reuse `files_watch` and its `files/changed` and `files/status` events. The marks are a new table in `runnerd`'s database, keyed by the checkout's top path, the path and the two blob ids, added by a migration. The CLI gets no commands.

Remembered view choices (Unified or Split, flat or tree) are app settings in `ui-settings.json`, like Files' options.

## Non-goals

Read-only, as #634: no stage, unstage or discard; no commit, push or pull request creation; no editing. Line comments and sending a review to the agent are #1293. Storing reviews as records or syncing them to GitHub pull requests. Remote sessions in this version (deferred, see [The Changes view](#the-changes-view)). Choosing the base by hand, which the commits list replaced. Expanding the context around a hunk, and ⌘F in a diff; both can follow. Reordering the panel's surfaces with grips, which #634 deferred until Changes joined.

## Risks

- **Shared checkouts.** A chat or mission without its own worktree shares the checkout, so Uncommitted changes mixes the human's edits and other sessions'. The comparison label says what is compared; nothing implies the diff is the session's.
- **Base guessing.** Repositories differ: no `origin`, a default branch with another name, a remote that is not `origin`. The fallback order covers the common cases, and a wrong guess only lists extra or fewer commits; Uncommitted changes never depends on it.
- **Large diffs.** A regenerated lockfile or a vendored folder can add hundreds of thousands of lines. Counts come from one `git diff --numstat` per selection, the diff is fetched only for the selected file, and both are virtualized.
- **Highlighting two versions.** Highlighting the old and new files doubles the highlighter's work for a selection. It runs off the UI thread as #634's does, and the diff shows plain until it arrives.
- **Git as a subprocess.** Windows needs `git.exe` on PATH; without it Changes shows the not-in-git state, as Files does. CRLF is handled as above.
- **Core review.** The new requests and the migration touch the client protocol and the schema, so that work takes `core-review`.

## Delivery

No umbrella branch: each step stands on its own on `main`, so three sequential pull requests, each from its own branch and mission, replace #634's umbrella and its landing gate.

| Step | What | Crew | Labels |
|---|---|---|---|
| m1 — runnerd changes service | The four requests, base resolution, the viewed marks table and its migration. Nothing a person sees | agy duo | `core-review`, no live checks |
| m2 — the Changes view | The surface and its `C` key and Settings row, the list flat and tree, Commits, the Unified diff with highlighting and edge cases, live refresh, the keyboard, and viewed marks | agy trio | `frontend-only` |
| m3 — Split and Files' Diff button | Split, the side panel's maximize and the 360 px rule, and the Diff button on a file's tab | agy trio | `frontend-only` |

The crews use the Antigravity coder on `gemini-4-hydrogen` at `xhigh` with the codex reviewer, and the trios add the codex `qa` (Jason, 2026-11-13). m2 waits on m1, and m3 on m2. Each step updates the guide pages for what it adds.

m1 lands on `main` without changing anything that runs today: it adds requests no client sends until m2, changes no existing `files_*` request, event or table, and its migration only creates a table, so an older build still opens the database. Its `changes/viewed` event goes only to clients watching that checkout, and the app ignores event names it does not know.

## Verification

### Automated

- **Parsers:** `git diff --numstat -z` with renames, copies, binary files and paths with spaces and newlines; unified hunks, including `\ No newline at end of file`; the commit log.
- **Base resolution,** in fixture repositories built with the git CLI: `origin/HEAD` set, only `origin/main`, only `origin/master`, no `origin` with a local `main`, no base, a detached `HEAD`, the main checkout level with its base and two commits ahead, a branch that merged its base in, and a repository with no commit.
- **Files and diffs** in a fixture with modified, added, renamed, deleted, untracked, binary, over-10 MB, CRLF-only, mode-only, conflicted and symlink files and a submodule: counts and statuses match git, and each placeholder appears where expected. A subfolder root lists only its subtree.
- **Viewed marks:** set and clear; a mark carries from Uncommitted changes to the commit that commits the file unchanged; an edit after viewing shows changed; a second client gets `changes/viewed`; the prune at start; the migration from the current schema.
- **App:** Split's line pairing, the 360 px rule, flat and tree grouping, the keyboard's focus moves, and CRLF-only detection.

### Live checks

The agy trio's `qa` runs these on macOS and Windows, and on Linux when a Linux machine is available, in a fixture repository under `/tmp` with a branch worktree two commits ahead of `origin/main`:

1. **List and summary** match `git status` and `git diff --numstat HEAD` for the fixture's file kinds, in dark and light. Reason: new view.
2. **Selection and keys:** ↑↓, j/k and clicks swap the diff at once; Tab, Enter, Esc, `[` `]`, ← → and Space act as the table says, and the terminal keeps its keys. Reason: new keyboard map.
3. **Commits:** the worktree lists its two commits and the main checkout none; selecting a commit shows its files and the label; a repository with no `origin` resolves its local `main`. Reason: base resolution.
4. **Live refresh:** an owned bounded agent edits a viewed file, which turns changed, then commits it; the file moves to the new commit, and an unchanged viewed file keeps its mark. Reason: watcher reuse and mark carry-over.
5. **Viewed marks** survive a restart of the app and of the development daemon, and show in a second window. Reason: persistence.
6. **Split and maximize:** Split in a narrow panel maximizes it; Restore chat returns to Unified; ⇧⌘↩ / Ctrl+Shift+Enter maximizes the panel with focus in it and the pane with focus in the chat; the chat keeps its 420 px. Reason: new panel control (m3).
7. **Files' Diff button** shows only for a changed file and toggles in place; Unified or Split is shared with Changes. Reason: new entry point (m3).
8. **Edge placeholders** for binary, over-10 MB, CRLF-only and conflicted files, on Windows with `core.autocrlf=true`. Reason: platform-specific diff display.

Selected regression cases: [WS-FILES-05](../tests/regression/workspace.md) (panel width, which Changes shares), [WS-PANE-07](../tests/regression/workspace.md) (pane maximize, whose shortcut the panel now shares) and [SET-PANEL-01](../tests/regression/settings.md) (the Surfaces card gains a row).

## Guide pages

[Chats and windows](../guide/guides/chats-and-windows.md) (a new "Review changes in the side panel" section, the Diff button under reading a file, maximizing the panel, and the Surfaces row), [Missions](../guide/concepts/missions.md) (Changes in the mission side panel) and [Keyboard shortcuts](../guide/reference/keyboard-shortcuts.md) (the Changes keys and the panel's maximize).

## Decisions settled with this spec

Agreed by Jason on 2026-11-13, beside those dated in the sections above. Where this spec and the design boards differ on these points, the spec wins until the boards are updated.

1. **Keyboard.** Three focus places and only `[` `]` for hunks, since ← → is the only keyboard way to scroll long lines, which never wrap. The board `B38Ah` still shows two focus zones and ← → for hunks.
2. **The box at the end of a diff** is a manual toggle, as the issue says, and carries no `Space` keycap, since Space acts only in the list. The board `o98BJ` note still reads as marking the file automatically "once it has been read to the bottom".
3. **Terminal-only tabs** get Changes beside Files as two permanent tabs.
4. **Out of this version:** surface reordering grips, expanding hunk context, and ⌘F in a diff.
5. **Delivery** as three pull requests to `main` without an umbrella, on the agy crews.
