# 634 — File preview

> Tracking issue: [#634](https://github.com/yicheng47/runner/issues/634). Scheduling and priority live on the issue and in the [roadmap](../roadmap.md). Platforms: macOS, Windows and Linux; a Linux piece that proves hard ships with a fallback and a follow-up issue rather than holding the feature (Jason, 2026-11-04).
> Status: spec and design complete; implementation planned. Filed 2026-09-17, closed as not planned on 2026-09-22 under the "not an agent development environment" non-goal, and reopened by Jason on 2026-10-08. On 2026-11-03 it was narrowed to the file preview: the diff, commits and review to the agent moved to [#1141](https://github.com/yicheng47/runner/issues/1141), which builds on this. On 2026-11-04 it became a six-mission program on the umbrella branch `feat/634-file-preview`; the [program record](../impls/634-file-preview/README.md) and its [plan](../impls/634-file-preview/plan.md) say how it lands.
> Design: `runner/desktop/specs/634-file-preview.pen` in `yeelinks_design`. Boards: `mZ6zP` (the panel with file tabs, the tree, Open in and tab overflow), `n3y8HZ` (Markdown), `s5WSq0` (file icons), `v5dda` (where the tree starts), `F9OyhG` (terminal tabs, a mission with a file open, edge states, and viewer and tree states), `c518A` (the Settings Side panel group, its Panel and Files pages and every option) and `uBcLR` (the side panel header with Files on the right, superseded), the last four from 2026-11-03; '634 — Side panel header · options (2026-11-05)' with '… G and H in place', where option H was chosen; and '634 — Side panel surfaces · after t3code (2026-11-05)' (`cwksG`), which replaced option H.
> Related: [#458](https://github.com/yicheng47/runner/issues/458) (⌘-click file links, `file_links.rs`), [#1072](https://github.com/yicheng47/runner/issues/1072) (side-panel views), [#1141](https://github.com/yicheng47/runner/issues/1141) (the git changes view), [#931](https://github.com/yicheng47/runner/issues/931) (installed monospace fonts, built first and reused here), [#403](../product/vision.md#6-non-goals-long-view) (worktree creation, still dropped).

## Motivation

Reading what an agent changed is the most common reason to leave Runner. #458 made cited paths ⌘-clickable, which covers jumping to one file but not seeing the project and what moved in it. Runner does not need to become an editor: a read-only view of the project with its changes marked lets the human judge the work without leaving the cockpit, and editing stays in the editor the user already picked.

## Scope

### Files view and its root

The Files view is a surface of the side panel: beside Info in a chat's panel, and beside Sessions and Info in a mission's. The panel header holds only the surfaces you opened, as tabs of icon and name, the shown one raised and semibold with a × to close it; after them a `+` lists every surface the panel offers, in the order Info, Files, each with a check mark when open and its key in a keycap, `I` and `F` (Jason, 2026-11-05, after t3code, replacing option H's title and row of icon buttons). Picking a surface from `+`, or pressing its key while the menu has focus, opens it or switches to its tab; each opens once, and Files stays one tab holding its own file tabs. Closing a tab shows the one before it. A chat's panel with nothing open shows "Open a surface" over the same rows and keys, which act while that list has focus, never globally, and the header shows only `+` and collapse. A mission's Sessions roster is a permanent first tab, with no × and never in `+`, so a mission never shows the empty list. A tab of only terminals, which had no side panel, gets one holding the Files tab alone, permanent, with no × and no `+`, as does any panel offering a single surface; in a tab that splits a chat with a terminal, the panel still follows the last focused chat. A new chat's panel opens the surface Settings names, Info by default, and opening a file from a terminal link opens the Files tab if it is closed and switches to it. A chat, terminal or mission running on another computer through remote access has no Files view: it says files from another computer aren't available, and reading them is out of scope for now (Jason, 2026-11-03).

Opening a file widens the panel beside the chat, or the mission and its terminal drawer, which keep at least 420 px. As room runs short, the file narrows from its saved width down to the standard panel width while the tree keeps its width; only then does the chat or mission give up its 420 px. The tree hides only when its toggle is off, and the saved file width caps the file rather than reserving room ahead of the tree (Jason, 2026-11-05). When a mission's panel is wide, the Sessions roster places two cards per row.

Where the tree starts depends on the session (Jason, 2026-11-03):

| Session | The Files view opens on |
|---|---|
| A chat or terminal outside a project | Nothing: it asks which folder to show |
| A chat or terminal in a project | Its own checkout, from the top: the worktree its working directory is in, or the main checkout |
| A mission | Its own checkout, from the top, found the same way from the mission's working directory |
| Any session whose root was changed | The checkout or folder chosen, saved by `runnerd` for that chat, terminal or mission |

- **Empty start.** Outside a project a chat's folder is only the default working directory, so the view shows "Choose a folder to show" with a path field (typed, or Browse through the system picker), the user's projects and recently chosen folders. Picking one shows its tree.
- **One checkout at a time.** The tree is rooted in a single checkout, found with `git rev-parse --show-toplevel` from the working directory, so a mission in `.worktrees/feat-634-file-preview` opens with that worktree's files at the top level and its own git status. A working directory below the checkout's top opens expanded and tagged `cwd`. In the main checkout, `.worktrees/` is an ordinary gitignored folder, dimmed like `target/`.
- **The worktree picker** (after Zed's, decided 2026-11-03). The tree header's root names the checkout, with a branch icon for a linked worktree, and opens a picker: a field reading "Select a worktree or type a path…", every worktree of the repository from `git worktree list --porcelain` with its branch and short commit, the main checkout first; then recent folders, and the projects for a chat outside a project; then Choose folder…, which opens the system picker. Typing filters the list, and Enter on a path roots the tree there. Zed's "create a worktree" rows are left out: creating worktrees stays out of scope (#403).
- **Re-rooting from the tree.** Right-clicking a folder offers Show files from here, Copy path and Reveal in Finder (Show in Explorer on Windows).
- The header's changed count covers the checkout shown. While `git status` runs the tree is already browsable and the header reads "Reading status…"; letters and the count arrive when it finishes.
- Outside a git repository the tree is plain, with no status, and the picker lists no worktrees.
- **A root that no longer exists.** A remembered root that is gone is forgotten and the session's own checkout opens instead. When that is gone too, as when a mission's worktree is removed after merging, the view says "This folder no longer exists" and offers the main checkout or the picker.

### Tree with git status

- Directories load lazily. `.git` is hidden; gitignored entries are dimmed, not hidden.
- Each changed file carries a coloured status letter from `git status --porcelain=v2 -z --untracked-files=all`: modified, added, renamed and untracked. A folder shows a dot when anything beneath it changed. Deleted files are left out until #1141 can show them as a diff.
- Live refresh from a `notify` watcher on the worktree plus the git index and HEAD, debounced.
- **Keyboard.** With the tree focused, ↑ and ↓ move the selection, → expands a folder or moves into it, ← collapses it or moves to its parent, Space or Enter opens the selected file in a tab, and typing jumps to the next entry starting with those letters. A hovered row is tinted; the keyboard-focused row draws the focus ring. Switching file tabs reveals the active file in the tree, expanding its folders and scrolling it into view.
- File icons come from open-source themes in Zed's icon-theme format: Catppuccin (default, following light and dark), Material and Minimal, chosen in Settings → Side panel → Files.

### Read-only file viewer

- Each file opens as a tab above the file, and the tab stays open (Jason, 2026-11-05: there is no preview tab, which proved too hard to discover). A click in the tree, Space or Enter, a relative link in a rendered page and a ⌘-clicked terminal link each open the file in a tab of its own beside the active one; a file already open switches to its tab, and at a cited line scrolls there. A double-click does nothing more than a click. The active tab shows ×, the others on hover; closing the last tab returns to the Files view. A changed file's tab carries its status letter.
- Opening a file widens the panel: the file takes the inner side and the tree moves to the panel's outer edge, on the app's panel colour, one step lighter than the file, which stays on the terminal's background (Jason, 2026-11-05). Without a file open the tree keeps the panel's own background. The tree toggle hides or shows the tree, and the panel grows by the tree's width rather than squeezing the file. Dragging the divider between the file and the tree sizes the tree from 160 to 480 px, saved like the file's width; the panel keeps its width, so the file gives up what the tree gains, never narrowing below the standard panel width.
- When the tabs outgrow the strip it scrolls sideways with faded edges, and a list button with the count lists every open file with its folder. ⌃Tab and ⌃⇧Tab (Ctrl+Tab and Ctrl+Shift+Tab on Windows) move between file tabs and ⌘W (Ctrl+Shift+W) closes the active one.
- Code shows line numbers and is syntax-highlighted in the terminal theme's colours (see Syntax highlighting below), in the font and size set in Settings → Side panel → Files. Rendering is virtualized, so a multi-thousand-line file does not stall the frame. Long lines scroll sideways and are never wrapped (Jason, 2026-11-03).
- Text can be selected and copied with ⌘C (Ctrl+C on Windows). ⌘F (Ctrl+F) opens the terminal's find bar from #897 over the file, with the match count and next and previous.
- **Zoom in the viewer** (Jason, 2026-11-05). With focus in the file viewer, in the code, its find field or a rendered page, zoom in and zoom out (⌘= or ⌘+ and ⌘−; Ctrl on Windows and Linux, and any rebinding of those shortcuts) step Settings → Side panel → Files → Font size by 1 px within 8 to 32 px, starting from the terminal's size it follows, and zoom reset (⌘0) clears it to follow Terminal font size again. They do not change the app's zoom, and rendered Markdown scales with the size. With focus anywhere else, the tree included, zoom stays app-wide.
- A binary file shows its name and size with Open in default app. A file over 10 MB is not read: it shows its size and an Open in button for the control's app. A file deleted while its tab is open keeps the last version read, dimmed under a "was deleted" banner, and its tab carries a red D; closing the tab is the only action.
- Markdown opens rendered (headings, lists, code blocks, links, local images); a `</>` button switches to the highlighted source. Links open in the browser; a relative link to a project file opens it in a new tab.
- An **Open in** control shows the app the file opens in; clicking it opens the active file there at the line in view. Its chevron lists the installed editors, then the system default app and Reveal in Finder. The chevron is the only place to choose the editor (Jason, 2026-11-05): the editor picked there is what Open in opens and what a ⌘-click on a path falls back to. ⌘-click on a line opens the file at that line.

### File links from a terminal

A ⌘-click (Ctrl+click on Windows and Linux) on a file path in any terminal opens it in the side panel that terminal's surface shows: a chat pane, a mission member or its drawer shell, or a terminal-only tab (Jason, 2026-11-05). It opens in a tab at the link's line, with the line selected, when the side panel is enabled, its Files view is on, the session is local, and the path resolves inside that panel's Files root; symlinks and `..` resolve first, off the UI thread, so a path that leaves the root through either falls outside, as `runnerd` would refuse to read it. It opens the panel if it is collapsed, opens the Files tab if it is closed and switches to it, and widens it as an opened file does; focus stays in the terminal. A Markdown file cited at a line shows its source there. Otherwise the link opens in the chosen editor exactly as before, and URLs are unchanged. The link's tooltip names the side panel or the editor, and waits until it is known which.

### Syntax highlighting

Highlighting follows Zed's approach (Jason, 2026-11-03), after comparing it with Orca, which uses Shiki and VS Code's TextMate grammars, and cmux, which uses Highlightr and highlight.js:

- **tree-sitter with bundled grammars.** About 25 common languages are compiled in, either through `arborium`, which bundles grammars behind per-language features, or through the individual `tree-sitter-*` crates as Zed does; a short spike decides, checking that the highlighter returns styled spans GPUI can draw rather than only HTML. A file in any other language shows as plain text. Highlighting runs off the UI thread, so a file up to the 10 MB limit does not stall the frame.
- **Zed's capture names.** Highlight queries tag text as `keyword`, `string`, `type`, `function`, `comment`, `number` and so on; a theme maps those names to colours, and a dotted name such as `type.builtin` falls back to `type`.
- **Colours follow the terminal theme.** Each bundled terminal palette (Runner, Catppuccin Mocha, Rosé Pine Dawn) carries a hand-tuned syntax map, and a palette without one, such as one #934 imports, gets a map derived from its 16 colours. The code therefore matches the terminal beside it in light and dark mode, through the terminal palettes already chosen for light and dark in Settings → Appearance; there is no separate code theme.
- **No language servers.** Highlighting needs only grammars. Zed itself leaves LSP colouring (`semantic_tokens`) off by default, and Orca and cmux use none.

### Settings → Side panel

Settings gets a **Side panel** group after App (Jason, 2026-11-03), with a page for the panel and a page for each view that has settings. The Changes view (#1141) and plugin views (#1072) add pages to the group later.

- **Panel.** Show the side panel (on by default) turns the whole panel off: its toggle leaves every tab and the panel shortcut does nothing, and the switches below keep their values, dimmed and inactive. A chat or terminal with every surface off has no panel. ⌘-click on a path still opens the file in the chosen editor when Files is off.
  - The Surfaces card (Jason, 2026-11-05; board `cwksG`) starts with a real-size preview of the header a new chat shows, which redraws as the card changes: by default Info open, `+` and collapse. The Info and Files rows keep their switches (both on by default), which now mean offered in the `+` menu; turning one off closes its open tabs. Each row shows its key. A last row, "A new chat's panel opens", picks Info (the default), Files or Nothing, which opens on the surface list. The note under the card says a mission always shows Sessions as its first tab, which cannot be closed.
  - Changes (#1141) adds its own row and key when it lands; until then no row shows it.
  - Reordering (Jason, 2026-11-03): once Changes joins after #1141, the rows get grips. Dragging a row, or ⌥↑ and ⌥↓, moves it, and the order applies to the `+` menu in every tab. v1 shows no grips.
- **Files**, every option with its default first:
  - Tree · File icons: **Catppuccin**, Material, Minimal. Moved here from Appearance.
  - Viewer · Font: a searchable list. **Same as terminal**, then **Bundled**, JetBrains Mono, the only font Runner ships, then **Installed**, every monospace font on the computer. It sets code and Markdown code blocks. Listing installed fonts is #931, built first on its own so people can use their own font in the terminal, and this list reuses it (Jason, 2026-11-03); the terminal's Font family shows the same list. The app's UI font stays Inter and is not a setting; a saved font missing at launch falls back to JetBrains Mono and the choice is kept. No more fonts are bundled: each Nerd Font family adds about 10 MB to the app.
  - Viewer · Font size: a stepper like Terminal font size, over the terminal's range (8 to 32 px since #930). It follows Terminal font size (13 px by default) until changed; a Reset link then appears and returns it to following the terminal. Rendered Markdown text scales with it.
  - Viewer · Syntax highlighting: a switch, **on**.
  - Viewer · Markdown files open as: **Rendered**, Source.
  - Gitignored entries are always dimmed, with no setting. The editor is chosen only in the Open in control's chevron; General has no Open file links in (Jason, 2026-11-05).

### Where it runs

`runnerd` serves the files (Jason, 2026-11-04), so remote file access can later be served by the other computer's `runnerd` without a new model in the app. Resolving a session's root, listing folders, `git status`, the worktree list, reading a file and watching for changes are client-protocol requests and pushed events, and every path is relative to a root that `runnerd` will not read outside. The chosen root is saved in `runnerd`'s database with its chat, terminal or mission, and the recent folders beside it. The app draws the tree and the file, keeps the tabs, highlights code and parses Markdown, so `runnerd` carries no grammars and the protocol carries plain bytes. The plan's [Where the work runs](../impls/634-file-preview/plan.md#where-the-work-runs) has the request list.

## Non-goals

The diff, commits and review to the agent (#1141). Files of a chat, terminal or mission on another computer. Language servers, and with them go-to-definition, hover and diagnostics; ⌘-click into the chosen editor covers them. A code theme separate from the terminal theme. Editing files; create, rename, delete or move in the tree; stage, commit or push; project-wide content search; worktree creation (#403 stays closed).

## Risks

- **Large repos.** The tree walk and `git status` on a monorepo must stay off the UI thread. A build writing tens of thousands of files into an ignored `target/` must not trigger a status rerun per event, and neither must a sibling worktree's build under `.worktrees/`. On Linux, inotify needs one watch per folder, so a recursive watch over a built `target/` can exhaust the watch limit; the plan's Linux section has the per-folder approach and its fallback.
- **Git as a subprocess.** `runnerd` shells out to `git` resolved through the login-shell PATH (`shell_path`, as agent spawns do) rather than adding libgit2. Windows needs `git.exe` on PATH. The picker's list is one `git worktree list` run when it opens.
- **Shared checkouts.** A session without its own worktree shares the checkout, so the status mixes the human's edits and other sessions' changes; the tree must not imply the changes are the session's.
- **New dependencies.** tree-sitter grammars are C compiled into the app, one per language, which adds build time on every platform and size to the app; each grammar keeps its own licence. Markdown rendering adds a parser. Bundled icon themes keep their licences (Catppuccin Icons MIT, Material Icon Theme MIT).
- **New rendering work.** GPUI has no tree or code view to reuse; both are new element work on a virtualized list.

## Verification

- Unit tests for the porcelain v2 parser (renames, untracked, unmerged, paths with spaces), the `git worktree list --porcelain` parser (detached and locked worktrees, paths with spaces), the folder rollup, and root resolution: outside a project, in a project, a subfolder working directory, a worktree, and a non-git folder.
- A fixture repo with modified, added, renamed, untracked, ignored and binary files: tree letters match `git status`.
- A fixture repo with a worktree under `.worktrees/`: a session there opens on the worktree with letters matching `git status` run inside it; the picker lists the main checkout and the worktree with their branches; picking the main checkout shows `.worktrees/` dimmed and without the worktree's letters.
- An agent edits a file while the panel is open: the row and the open file update within the debounce window without a manual refresh.
- A chosen root survives switching tabs and restarting the app, per chat and per mission.
- The header in a chat, a mission and a terminal-only tab: the `+` menu's rows, check marks and keys, the empty list and its focus, opening, switching and closing tabs, including the last and the permanent ones, and each value of "A new chat's panel opens".
- Large-repo smoke: the Runner repo with `target/` built opens the panel without a visible stall, and a `cargo build` does not churn status.
- macOS, Windows and Linux.

## Guide pages

[Chats and windows](../guide/guides/chats-and-windows.md) (the side panel's Files view and its root, the terminal tab's panel and Settings → Side panel), [Missions](../guide/concepts/missions.md) (the mission side panel) and [Keyboard shortcuts](../guide/reference/keyboard-shortcuts.md) (file tabs).
