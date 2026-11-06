# 634 — plan

The phases of [feature 634](../../features/634-file-preview.md) in detail: how each mission lands, what it touches, how it is checked, and the gate before the umbrella reaches `main`. The program's state and its binding decisions are in [README.md](README.md), and the dated log is [impl_log.md](impl_log.md). The spec wins on what a person sees; the design file `runner/desktop/specs/634-file-preview.pen` in `yeelinks_design` wins on how it looks; this plan wins on how it is built and in what order.

## Shape

- **The umbrella** is `feat/634-file-preview` in `.worktrees/feat-634-file-preview`. It already holds the spec, and the draft pull request [#1157](https://github.com/yicheng47/runner/pull/1157) takes it into `main`. A commit of its own adds the branch to `ci.yaml`'s `pull_request` and `push` triggers, as `nightly-remote` did for #836, so mission PRs and pushes get the macOS, Windows and Linux jobs. The landing PR's last commit removes the trigger again.
- **Each mission** branches from `origin/feat/634-file-preview` as `feat/634-m{n}-{slug}` in `.worktrees/feat-634-m{n}-{slug}`, commits its brief on that branch as `docs/impls/briefs/634-m{n}-{slug}.md`, and opens its PR against the umbrella. CI is green on every job before it merges. It reaches the umbrella as a fast-forward: the branch is rebased onto the umbrella tip and the umbrella is pushed to the branch head, which GitHub records as the PR's merge, since the repository allows only merge commits. The umbrella stays linear.
- **The umbrella follows `main` by rebase** (Jason, 2026-11-04), not by merging `main` in as AGENTS.md describes for umbrellas, so its history stays its own commits on top of `main`. It is rebased onto `origin/main` between missions and pushed with `git push --force-with-lease`. When a mission PR is open at that moment, its branch follows with `git rebase --onto origin/feat/634-file-preview <old umbrella tip>` before the mission continues.
- **Crews.** Each mission is a duo (codex or claude, Jason's pick per mission). A mission that changes what a person sees gets the `qa` role after its clean review, under Test Scope; m1 changes nothing a person sees and needs none.
- **Labels.** Each mission PR takes the repo's labels as on `main`. m1 touches the client protocol's request surface and adds a migration, so it is `core-review`, and the umbrella's landing PR is too.
- **Guide pages.** Each mission updates the guide pages for what it adds (the spec's Guide pages section), so the umbrella's guide is current when it lands.
- **Builds.** Nightlies stay on `main`. An installed umbrella build for daily use is cut only when Jason asks.

## Order

| Mission | What | Waits on |
| --- | --- | --- |
| m1 — runnerd file service | The file and git requests, the watcher, root resolution and the saved root, in `runner-core` and `runner-daemon`. No UI | — |
| m2 — side panel views | #1072's internal view contract, Info and Sessions on it, the header with Files on the right, the terminal tab's panel. Files is an empty view until m3 | — (runs beside m1) |
| m3 — the Files view | The root and its states, the worktree picker, the tree with git status, icons, keyboard and live refresh | m1, m2 |
| m4 — the file viewer | File tabs and their overflow, the widened panel, the code view as plain text, Open in, find, and the edge states | m3 |
| m5 — highlighting and Markdown | The highlighter spike and tree-sitter highlighting in the terminal palette's colours, and rendered Markdown | m4; the spike can run any time |
| m6 — Settings → Side panel | The Panel and Files pages and every option, wired to m2 to m5 | m5 |
| m7 — file links open in the preview | From Jason's live run: the width rule, ⌘-clicked links in the panel, option H's header, every open keeping a tab, zoom in the viewer | m6 |
| m8 — side panel surfaces | The header's open surfaces as tabs with a `+` menu and keys, the empty list, Sessions permanent, Settings' Surfaces card | m7 |
| m9 — fixes from Jason's live run | The code view's scrollbars, tree rows and guides, Files wide with an empty view, the file finder and `files_index` | m8 |
| m10 — fixes from the umbrella's review | Bounded Markdown nesting and status, ignored open files, rescans, branch refs, closing Files, and the review's notes | m9 |

m1 and m2 touch different crates and run in parallel. m3 to m6 are sequential because each builds on the surface the previous one adds. m5's spike touches nothing else, so it can run as a separate short task earlier if a crew is free, and m5 then starts from its result.

## Where the work runs

Decided by Jason on 2026-11-04, so that remote file access can later come from the other computer's `runnerd`.

- **runnerd** does everything that touches the disk or git: resolving a session's root, listing a folder, `git status`, `git worktree list`, reading a file and watching for changes. It shells out to `git` resolved through the captured login-shell PATH (`shell_path.rs`), as agent spawns do.
- **The app** draws the tree and the file, keeps the tabs, highlights code, parses Markdown and loads icon themes. A file's bytes cross the socket once per read, well inside the 64 MB frame limit (`wire::MAX_FRAME`) at the spec's 10 MB file limit. Highlighting stays in the app so `runnerd` carries no grammars and the protocol carries plain bytes, not styled spans.
- **Paths are relative to a root.** Every request names a root and a path inside it, and `runnerd` refuses a path that resolves outside the root: `..`, an absolute path, or a symlink that leaves it. A local client may root the tree anywhere it chooses; the rule matters once a remote client sends paths.
- **Remote sessions** are known to the app, not to the local `runnerd`: the app shows the spec's "not available" state for a chat, terminal or mission on another computer without asking.

## Phase 1 — m1, the runnerd file service

### Commits

1. **Types**, in a new `runner-core/src/protocol/files.rs`.
   - `FilesSubject`: a session (a chat or a terminal) or a mission.
   - `FilesRoot`: its path, the checkout's name, whether it is a git checkout (main or linked worktree, with branch and short commit) or a plain folder, the working directory below the top when there is one (for the `cwd` tag), and whether it is the saved root or the session's own checkout.
   - `FilesRootOutcome`: a root; "choose a folder" for a chat or terminal outside a project; or "missing", with the path that no longer exists.
   - `DirEntry`: name, kind (folder, file, symlink), whether it is ignored, and size for files.
   - `GitStatus`: each changed path with its status (modified, added, renamed with its old path, untracked, unmerged; deleted is parsed and kept for #1141, and the tree leaves it out), the changed count, and the ignored folders. A pure rollup function beside it gives a folder's dot from the changed paths, for the app and the tests.
   - `WorktreeEntry`: path, branch or detached, short commit, main or linked, locked, prunable.
   - `FileContent`: text with its bytes, size and modified time; binary with its size; too large with its size; or missing.
2. **Git and the disk**, in a new `runner-daemon/src/files/`.
   - `git.rs`: `git rev-parse --show-toplevel` and `--git-dir`, which only the watcher runs; `git status --porcelain=v2 -z --untracked-files=all --ignored=matching` and its parser; `git worktree list --porcelain -z` and its parser. A request runs git at most once; the one exception is a git before 2.36, which lacks `-z`, where the first worktree list in a `runnerd` process is refused and retried without it, and later lists run without it (Jason, 2026-11-04). A missing `git`, as on a Windows machine without `git.exe` on PATH, makes every root plain and is logged once.
   - `fs.rs`: listing a folder without running git (`.git` left out, ignored entries marked from the last status), reading a file (over 10 MB not read; binary when the first 8 KB holds a NUL), and the path check above.
   - `root.rs`: resolution, in this order. A saved root that still exists. Otherwise the session's own checkout, the top of the worktree holding the session's working directory, read from one `git worktree list` with its branch and commit, the directory found as `terminal::link_cwd` finds it (the row's `cwd`, then its project's), or from the mission's `cwd`. A chat or terminal with no project gets "choose a folder". A working directory outside git gives a plain root at that directory. A saved root that is gone is cleared and resolution continues; an own checkout that is gone, such as a mission worktree removed after merging, gives "missing".
3. **The saved root and recent folders.** A migration adds a nullable `files_root` column to `sessions` and to `missions`, so the choice is deleted with its row. Recent folders are one `app_state` key, the last ten chosen folders, newest first. Setting a root to nothing returns to the session's own checkout.
4. **The watcher**, in `files/watch.rs`. One watcher per root, shared by every subscriber and dropped with the last one or when its client disconnects.
   - macOS and Windows: one recursive watch on the root. Events under an ignored folder from the last status (`target/`, `.worktrees/`, `node_modules/`) are dropped before the debounce.
   - The git directory from `--git-dir` is watched for `index` and `HEAD`. For a linked worktree it is `<main>/.git/worktrees/<name>/`, not the worktree's `.git` file.
   - Linux: inotify needs a watch per folder, and a recursive watch over a built `target/` can exhaust `max_user_watches`. Linux walks the folders that are not ignored and watches each without recursion, adding watches as folders appear. See the Linux section for its fallback.
   - Debounced with a short trailing delay and a maximum wait, so a long build still refreshes; one `git status` runs at a time and later changes coalesce into the next run.
5. **The requests and events**, in `protocol/api.rs`, all slow: `files_root(subject)`, `files_set_root(subject, path or none)`, `files_list(root, folder)`, `files_status(root)`, `files_worktrees(root)`, `files_read(root, path)`, `files_watch(root)`, `files_unwatch(root)` and `files_recent_folders()`. Pushed events: `files/changed` with the root and the folders and files that changed, and `files/status` with the new `GitStatus`. The CLI gets no commands; its exhaustive recorder test keeps passing.

### Checks

- **Parsers**: porcelain v2 with modified, added, renamed (with its old path), copied, untracked, unmerged, ignored, and paths with spaces and newlines under `-z`; the worktree list with the main checkout, a linked, a detached, a locked and a prunable worktree, and paths with spaces.
- **Rollup**: a folder's dot from nested changes; an ignored folder never carries one.
- **Root resolution**: outside a project, in a project, a working directory below the top, a linked worktree, a non-git folder, a saved root, a saved root that is gone, and an own checkout that is gone.
- **Paths**: `..`, an absolute path and a symlink out of the root are refused; a symlink inside the root reads.
- **Reading**: text, binary, exactly 10 MB and one byte over, and a missing file.
- **A fixture repository**, built with the git CLI in a temporary folder, with modified, added, renamed, untracked, ignored and binary files: the statuses match `git status`. With a worktree under `.worktrees/`: a session there resolves to the worktree with its own statuses; the worktree list names both with their branches; rooting at the main checkout shows `.worktrees/` ignored and none of the worktree's changes.
- **The watcher**: an edit to a tracked file gives one `files/changed` and one `files/status` within the debounce window; 10,000 files written into an ignored `target/` give no status run; a commit in a linked worktree refreshes its status.
- **The migration**, from the current schema, and the saved root's round trip through `files_set_root` and `files_root`.
- No live checks: nothing a person sees changes.

## Phase 2 — the Files view

### m2 — side panel views

1. **The view contract** (#1072, internal only). A side-panel view registers its id, label and icon, its group (the session's views, or the project's), which surfaces it appears on (a chat, a terminal-only tab, a mission), and whether it is turned on, which m6 wires to Settings. It is drawn from a context naming the subject, its working directory, its project and whether it runs on another computer. Info in a chat's panel and Sessions and Info in a mission's move onto it with no visible change. Files registers in the project group and is empty until m3.
2. **The header** (board `uBcLR`): the session's views as icons on the left, the project's as labelled buttons on the right beside the collapse control. Applied to a chat's panel and a mission's. m7 replaced it with option H (Jason, 2026-11-05): the open view's name on the left, every view as a bare icon button on the right; m8 replaced that with the open surfaces as tabs and a `+` menu.
3. **The terminal tab's panel** (board `F9OyhG`): a tab of only terminals gets a side panel holding Files alone, with only the Files button, the same open state and width as a chat's panel, and the same toggle. A tab that splits a chat with a terminal still follows the last focused chat.

Checks: the contract's registration and grouping in unit tests; the header and the terminal tab's panel rendered from snapshots in both themes. Live, by `qa` on macOS: the header in a chat, a mission, a terminal-only tab and a mixed tab; Info and Sessions unchanged; the panel toggle and resize unchanged. `frontend-only`.

### m3 — the Files view

1. **The root and its states** (board `v5dda`, and `F9OyhG` for the edge states): "Choose a folder to show" with the path field, Browse through the system picker, the projects and the recent folders; the root header with the checkout's name, a branch icon for a linked worktree, the changed count and "Reading status…" while the first status runs; a plain tree outside git; "This folder no longer exists" with the main checkout and the picker; and "files from another computer aren't available" for a remote session.
2. **The worktree picker**: the field filters the list or takes a path that Enter applies; the worktrees with branch and short commit, the main checkout first; then the recent folders, and the projects for a chat or terminal outside a project; then Choose folder…. A chosen root is saved through `files_set_root`.
3. **The tree** (board `mZ6zP`): virtualized rows on GPUI's `uniform_list`, the app's first; folders load as they open; ignored entries dimmed; the status letters in their colours, folder dots and the changed count from `files/status`; the working directory below the top expanded and tagged `cwd`; right-click with Show files from here, Copy path and Reveal in Finder (Show in Explorer on Windows, Open containing folder on Linux, pending the README's open question). A click on a file selects it; opening it arrives in m4.
4. **Keyboard**: ↑ and ↓, → and ←, Space and Enter (wired to tabs in m4), type-ahead, the hover tint and the focus ring.
5. **File icons** (board `s5WSq0`): a loader for Zed's icon-theme JSON, and Catppuccin (Latte and Mocha following light and dark), Material and Minimal bundled with their licences. Catppuccin is used until m6 adds the choice.
6. **Live refresh**: the view subscribes with `files_watch` while it is shown and unsubscribes when hidden, and applies `files/changed` and `files/status` in place, keeping expansion and scroll.

Checks: the tree model (expanding, collapsing, rollup display, type-ahead and keyboard moves), the picker's filtering and path entry, and every root state rendered from snapshots in both themes. Live, by `qa` on macOS, from the spec's Verification: a fixture repository's letters match `git status`; an agent's edit updates the row without a refresh; a chosen root survives switching tabs and restarting the app, per chat and per mission; the picker lists the main checkout and a worktree, and rooting at the main checkout shows `.worktrees/` dimmed; the Runner repository with `target/` built opens without a visible stall, and a `cargo build` does not churn the status.

## Phase 3 — the file viewer

### m4 — the file viewer

1. **Tabs** (board `mZ6zP`): a single click opens the italic preview tab that the next single click replaces; a double-click on the file or the tab keeps it; an open file switches to its tab (m7 removed the preview tab: every open keeps its tab); × on the active tab and on hover; a changed file's letter; closing the last tab returns to the Files view. Space and Enter from the tree open the preview and a kept tab.
2. **Overflow**: the strip scrolls sideways with faded edges; the list button with the count lists every open file with its folder. ⌃Tab and ⌃⇧Tab (Ctrl+Tab and Ctrl+Shift+Tab on Windows and Linux) move between file tabs, and ⌘W (Ctrl+Shift+W) closes the active one while the panel has focus; with focus elsewhere ⌘W keeps closing the pane.
3. **The widened panel**: the file on the inner side and the tree on the outer edge, with the tree toggle; the panel grows by the tree's width. In a mission (board `F9OyhG`) the mission and its terminal drawer keep at least 420 px, and a wide Sessions roster places two cards per row. As room runs short the file narrows to the standard panel width while a tree that is on keeps showing; m7 changed this from hiding the tree first (Jason, 2026-11-05).
4. **The code view**: virtualized lines with line numbers, in the terminal's font and size until m6, scrolling sideways with no wrapping; selection and ⌘C (Ctrl+C); ⌘F opening #897's find bar over the file with the match count and next and previous; ⌘-click on a line opens the file there in the chosen editor. Text that is not valid UTF-8 is decoded lossily. Plain text until m5.
5. **Open in** (board `mZ6zP`): the control shows the app the file opens in and opens the active file at the line in view; its chevron lists the editors `file_links.rs` finds, the system default app and the reveal row; picking an editor saves the editor, which m7 made the only place to choose it.
6. **The edge states** (board `F9OyhG`): a binary file's name and size with Open in default app; a file over 10 MB with its size and Open in; a file deleted while open kept dimmed under "was deleted", its tab with a red D, closing the only action. An open file reloads on `files/changed` and keeps its scroll position. Switching tabs reveals the active file in the tree.

Checks: the tab model (preview replacement, keeping, reuse, closing the last), the overflow list, the 420 px width arithmetic, ⌘W's focus scoping, find over a file, and each edge state from snapshots. A 10,000-line file renders only the visible lines (`render_counts.rs`). Live, by `qa` on macOS: preview and kept tabs, the overflow and its list, the shortcuts, Open in with each installed editor, a file deleted while open, the binary and large placeholders, and a mission with a file open as the window narrows.

### m5 — highlighting and Markdown

1. **The spike**, the first commit or a short task before the mission: `arborium` against the `tree-sitter` and `tree-sitter-highlight` crates with individual grammars, as Zed uses. It checks that the highlighter returns styled spans with capture names GPUI can draw, and measures each option's build time on the macOS, Windows and both Linux CI jobs and the app's size with about 25 grammars: Rust, TypeScript, TSX, JavaScript, Python, Go, C, C++, C#, Java, Kotlin, Swift, Ruby, PHP, Lua, Bash, JSON, TOML, YAML, Markdown and its inline grammar, HTML, CSS, SQL, Dockerfile and Make. The result and its numbers go in the impl log. If the cheaper option adds more than about 10 MB to the app or 2 minutes to a CI job, the crew asks Jason before continuing.
2. **Highlighting**: language from the file name and extension; spans computed on the background executor and cached per line for the virtualized view, so a 10 MB file never stalls a frame; recomputed when the file reloads. Any other language stays plain text.
3. **Colours**: Zed's capture names mapped to colours, a dotted name falling back to its head (`type.builtin` to `type`); a hand-tuned map for each bundled terminal palette (Runner, Catppuccin Mocha, Rosé Pine Dawn) and a map derived from the 16 colours for any other; light and dark follow the terminal palettes already chosen in Settings → Appearance.
4. **Markdown** (board `n3y8HZ`): rendered with 20 px side margins, then headings, lists, code blocks highlighted as code, inline code, links and local images, read through `files_read` and decoded in the app; the `</>` button switches to the highlighted source. Links open in the browser, and a relative link to a project file opens it in a new tab. The parser is `pulldown-cmark` unless the spike finds a reason against it; the mission feed's `mission_markdown.rs` stays as it is. Tables, task lists and inline HTML follow the README's open question.

Checks: language detection, the capture fallback, the derived map, highlighting kept off the UI thread for a 10 MB file, each Markdown block from snapshots in both themes, and link routing. Live, by `qa` on macOS: Rust, TypeScript and TOML files highlighted in light and dark; a README with images and relative links; the source toggle.

## Phase 4 — m6, Settings → Side panel

Board `c518A`. A Side panel group after App, with a Panel page and a Files page; strings in English and Simplified Chinese (`i18n/en.toml`, `i18n/zh-hans.toml`); stored with the app's other settings in `app_settings.rs`.

1. **Panel**: Show the side panel, which removes the panel's toggle from every tab, makes the panel shortcut do nothing and dims the view switches below; Info and Files switches; a mission's Sessions always shown; a chat or terminal with every view off has no panel. The Views card starts with the live header preview, and its rows are grouped as This chat and Project (m7 made them one list in the header's order, and m8 made the card Surfaces). There are no grips in v1: reordering comes with #1141, when a group first has two views.
2. **Files**: File icons (Catppuccin, Material, Minimal); Font, the searchable list of Same as terminal, Bundled and Installed, reusing #931's list, with a missing saved font falling back to JetBrains Mono while the choice is kept; Font size, the stepper over the terminal's range (8 to 32 px since #930) that follows Terminal font size until changed, then shows Reset; Syntax highlighting; Open with a single click, which when off opens every click as a kept tab (removed in m7 with the preview tab); Markdown files open as Rendered or Source.

Checks: each setting's default and round trip, the font fallback, Font size following the terminal until changed and again after Reset, and both pages from snapshots in both themes and both languages. Live, by `qa` on macOS: each option changes the view it names, and turning the panel off removes its toggle and shortcut.

## m7 — file links open in the preview

From Jason's live run of the umbrella on 2026-11-05; brief [634-m7-link-preview.md](../briefs/634-m7-link-preview.md). Seven changes in `runner-app` only:

1. **The width rule**: `side_panel::widened` narrows the file first, down to the standard width, while a tree that is on keeps its width; the tree hides only when its toggle is off.
2. **File links open in the side panel**: each terminal's element carries its surface's `FileLinkRoute` (the chat panel for panes and terminal-only tabs, the mission rail for members and the drawer shell). A ⌘-click asks that panel's Files view, made unshown if the panel never showed it, whether the path is inside the root; the check canonicalizes off the UI thread and waits for a root still resolving. Inside, it opens a tab at the line and the owner opens the panel on Files; otherwise `file_links::open_file_link` runs as before. The tooltip waits for the same answer.
3. **General → Open file links in is removed**, with its strings; `file_link_editor` stays, set by the Open in chevron.
4. **The side panel header is option H** (added by Jason on 2026-11-05): the open view's name on the left and the surface's views on the right as 24 px icon buttons, then collapse; one view showed the title and collapse only. m8 replaced it.
5. **No preview tab** (Jason, 2026-11-05): `FileTabs::open` gives every file a tab of its own beside the active one, or switches to its tab; the preview flag, keeping, the italic style and Settings → Side panel → Files → Open with a single click (`files_single_click`, ignored when an old settings file has it) are gone.
6. **Zoom in the viewer** (Jason, 2026-11-05): in the existing `FileViewer` key context, guarded so it acts only while the viewer has focus, the effective zoom-in, zoom-out and zoom-reset combos, rebinds included, are bound after app zoom's to `ZoomFileIn`, `ZoomFileOut` and `ZoomFileReset`, also inside its find field. They step `files_font_size` by 1 px within 8 to 32 px, or clear it; the tree and the rest of the app keep app zoom.
7. **The tree's colour** (Jason, 2026-11-05): beside an open file the tree column fills with `theme::panel()`, the design's `$panel`, one step lighter than the file on `$bg`; the selected and hover tints keep their contrast on it.

Checks: the width rule for a chat and a mission, routing from a real ⌘-click and from each owner, every fallback, the preview at its line from a collapsed panel, and the tooltip's wording. Live, by Jason: the four checks in the brief, with check 1 adding several clicked files each keeping a tab and check 2 opening the file in a tab rather than a preview; a fifth for the header in a chat, a mission and a terminal tab, switching views from it; and a sixth for zoom in a focused file against zoom elsewhere; check 1 also looks for the tree's background lighter than the file's.

## m8 — side panel surfaces with a + menu

Jason's model after t3code, 2026-11-05; brief [634-m8-panel-surfaces.md](../briefs/634-m8-panel-surfaces.md); board `cwksG` (A `du8Q4`, B `GfSLO` with the menu `r2y8b`, C `h2gjjR`, D `jdKtZ`). In `runner-app` only, with no runnerd, protocol or storage change beyond one app setting:

1. **The model** (`side_panel.rs`): `OpenSurfaces` holds the opened surfaces in the order they opened and the active one, replacing the single chosen view in `chat_panel_view` and `rail_view` with the same lifetime: the window's chat panel and each mission's rail, saved per mission on remote switches as before. `tabs` puts the permanent tab first (a mission's Sessions, or the only surface on offer) and then the open ones Settings offers; `shown` is the active tab or the first; `None` is the empty list. Opening switches to an open surface instead of adding it again; closing the shown tab shows the one before it, Sessions in a mission. Settings turning a surface off closes its tabs, from each window's settings revision.
2. **The header** (`header_tabs`): 28 px tabs of icon and name with 8 px sides and 4 px between, the shown one raised and semibold with a × where it closes; then the 24 px `+` where the menu offers anything; a flex gap; collapse. The tabs and `+` stop the titlebar drag; the gap keeps it.
3. **The `+` menu**, 240 px wide under `+` through `popup_layer`: Info then Files, each with icon, name, a check mark when open and its key in a keycap. A click or the key opens or switches and closes the menu; arrows and Enter pick; Esc and a click outside close it, returning focus to where it was. The keys are a key handler on the menu's own focus, never a global binding.
4. **The empty list**: a chat's panel with nothing open shows "Open a surface", the same rows with keys and no check marks, and "Press a surface's letter to open it". The list takes focus when the last tab closes and when the collapsed panel opens on it.
5. **What opens**: `new_chat_panel` (Info by default, Files or Nothing; removed on 2026-11-06, when a new chat's panel always opens on the list) seeds the window's chat panel and resets it in `finish_local_chat_launch` when a new chat starts; a ⌘-clicked terminal link opens the Files tab if closed and switches to it, in a chat and a mission.
6. **Settings → Side panel → Panel**: the Views card becomes Surfaces, previewing the header a new chat shows; Info and Files keep their switches and show their keys; "A new chat's panel opens" is a select; the note says Sessions is a mission's permanent first tab. Strings in English and Simplified Chinese.

Checks: the model's tabs, shown surface, open, close and Settings closing tabs for a chat, a mission and a terminal-only tab; the keys and their modifiers; the header, menu and empty list in a real window in both themes and both languages, with focus and keys; a new chat for each Settings value; the Settings card and its select; old settings files loading. Live, by `qa` on macOS in the dark theme: the brief's six checks.

## m9 — fixes from Jason's live run

Jason's run of the umbrella, 2026-11-06, with one coder working uncommitted while he tested and no brief; the [log](impl_log.md) has what it changed and its review.

## m10 — fixes from the umbrella's review

The two reviewers' findings on the whole umbrella, 2026-11-06, every one fixed by decision of Jason's. The [log](impl_log.md) has each finding's outcome and the notes left, with why.

Checks: Markdown nested past the limit in a process of its own on a 512 KiB stack; a status past its limits fitting a frame, an oversized event replaced by a lag; a soft reset in a main checkout, a linked worktree and a reftable repository; a rescan refreshing nested folders; an ignored open file refreshing and showing as deleted; the width after closing Files. Live, by `qa` on macOS in the dark theme: the checks in the pull request.

## Linux

Every mission builds and passes its tests on both Linux CI jobs. The pieces that differ, and what ships if one proves hard (Jason, 2026-11-04):

| Piece | Linux | Fallback that does not hold the umbrella |
| --- | --- | --- |
| Watching (m1) | Per-folder inotify watches over the folders that are not ignored | A `git status` every five seconds while the view is shown, with the limit logged |
| Reveal (m3, m4) | "Open containing folder" through `xdg-open`, as the mission rail does | — |
| Open in (m4) | The editors `file_links.rs` already launches, and `xdg-open` | — |
| Choose folder… (m3) | GPUI's system picker through the desktop portal | The path field alone |
| Grammars (m5) | Compiled with `cc` in the Linux CI container on both architectures | None: they must build, and the spike checks it |
| Installed fonts (m6) | #931's Linux list from the platform text system | Same as terminal and Bundled |

A fallback that ships gets a follow-up issue in the next milestone.

## Windows

`git.exe` must be on PATH, and without it every root is plain (m1). Porcelain paths use `/` and are converted; path comparison is case-insensitive; CRLF files show no stray characters (m4). Shortcuts are the spec's Windows forms.

The watcher cannot see a `ReadDirectoryChangesW` buffer overflow: notify 6, which the event bus also uses, drops it silently, and a build's churn in ignored folders fills the same buffer as an agent's edits, so an edit in that burst would be lost without a signal (found in m1, 2026-11-04). Windows therefore runs a `git status` every five seconds while a root is watched, as Linux does on its fallback, and sends `files/status`, with `files/changed` for the root, only when the status changed; an edit to a file that was already changed can still wait for the next event. Upgrading notify, which might report the overflow, stays out of the program because it touches the event bus (Jason, 2026-11-04).

## The gate before landing

1. Every mission merged into the umbrella; the umbrella rebased onto `main`; CI green on every job.
2. The spec's Verification run end to end on the umbrella head by a `qa` mission on macOS, Windows and Linux, with each platform's result recorded in `docs/tests/634-file-preview.md`. A platform that cannot be checked is recorded as a gap and holds the landing until Jason accepts it; a Linux piece on its fallback is recorded with its follow-up issue.
3. The large-repository smoke on each platform: the Runner repository with `target/` built opens without a visible stall, and a `cargo build` does not churn the status.
4. Daily use of an umbrella build if Jason chooses it (the README's open question).
5. The landing PR, [#1157](https://github.com/yicheng47/runner/pull/1157) out of draft, umbrella into `main`, labelled `core-review`, `risk-high` and `needs-live-test` until step 2 passes; its last commit removes the CI trigger. It merges with a merge commit, #634 closes, and #1141 builds on `main`.
