# 634 — File preview and code review

> Tracking issue: [#634](https://github.com/yicheng47/runner/issues/634). Scheduling and priority live on the issue and in the [roadmap](../roadmap.md). Platforms: macOS and Windows.
> Status: phased spec; the bounded read-only first delivery was approved on 2026-10-22 (see below), while Git diff/review remains planned. Filed 2026-09-17, closed as not planned on 2026-09-22 under the "not an agent development environment" non-goal, and reopened by Jason on 2026-10-08 for 0.15: reading an agent's changes is still the most common reason to leave Runner, so the read-only viewer is back in and code review moved from Later into scope as phase 4. On 2026-10-09 Jason moved it to 0.14 as the release's key feature. Design first, in `runner/desktop/specs/793-634-glass-and-files.pen` in `yeelinks_design`, which it shares with liquid glass ([793](./archive/793-liquid-glass.md)) so the panel is designed in the new chrome, for sign-off before any code.
> Related: [#458](https://github.com/yicheng47/runner/issues/458) (⌘-click file links, `file_links.rs`), [704](https://github.com/yicheng47/runner/issues/704) (session send, the delivery for a review sent to a direct chat; closed unbuilt on 2026-10-11), [826](./archive/826-targeted-messaging.md) (directed messages that wake only their recipient), [403](../product/vision.md#6-non-goals-long-view) (worktree isolation, still dropped).

## Approved first delivery — 2026-10-22

The user approved a screenshot-inspired tabbed right panel for both direct chats and Teams: preserve Info/Roles/Meta, add Files, closable canonical file tabs and a new-tab home, and provide bounded read-only Markdown/text/raster previews. Reuse the existing panel shells, splitters, terminal drawer, external file opener and Markdown renderer. Unsupported formats use an external-open fallback; no new editor, upload/share workflow, webview or daemon protocol is introduced.

This delivery narrows phases 1–2 below: lazy directory browsing and file content preview only. Git status letters, Changed only, unified diffs, comparison bases and review delivery remain later work, not implemented claims. The supplied three screenshots are the approved interaction reference; existing design-system tokens and native platform chrome remain authoritative. Implementation plan: `docs/impls/2026-10-22-workspace-panel-files.md`. User guide: `docs/guide/guides/files-and-previews.md` and the Sessions/Missions concept pages.

## Motivation

Reviewing what an agent changed is the most common reason to leave Runner. A session finishes a turn, and the human opens an editor or a git client to see which files moved and to read the diff. #458 made cited paths ⌘-clickable, which covers jumping to one file, but not "what did this session touch" across the project.

Runner does not need to become an editor. The human's job in the cockpit is to judge the work, and a read-only view of the project with its changes marked is enough for that. Editing stays in the editor the user already picked. What Runner adds over an editor is the route back: a review written on the diff goes to the agent that made the change.

Prior art, checked 2026-09-17:

- **Orca** (Electron, Monaco): file explorer colored by git status (untracked, modified, staged, ignored) with create, rename, delete and move; a combined diff against the worktree's start ref, switchable to any commit or branch; hunk and line staging; commit and push; diff line comments batched and sent to a chosen agent.
- **Termio** (native Swift): an inspector with Files, Search, Changes, Issues and Info tabs. The tree only dims ignored files; changed files live in the separate Changes tab (porcelain status with +/− counts, staged and unstaged, a History tab, branch compare). The diff opens as an overlay over the terminal. Files open in an editor with syntax highlighting. Commit and push are deliberately left to the terminal; a "copy diff" action exists for pasting into an agent prompt.

Runner takes the read-only half: Termio's restraint on writes, Orca's git status on the tree itself instead of a second list, and Orca's annotate-and-send routed through the crew.

## Scope

### Phase 1 — project tree with git status

- A **Files** panel rooted at the session's working directory, resolved the way #458 resolves it: session cwd, then the project directory, then the default working directory. In a mission it is a third rail view beside Roles and Meta, rooted at the mission cwd. A direct chat gets it as a view in the chat side panel (#756).
- Directories load lazily. `.git` is hidden; gitignored entries are dimmed, not hidden.
- Each row carries its git status from `git status --porcelain=v2 -z --untracked-files=all`: a color and a letter for modified, added, deleted, renamed and untracked. A folder shows a marker when anything beneath it changed. Deleted files stay in the tree at their old path so they can still be opened as a diff.
- A **Changed only** filter collapses the tree to changed files and their parent folders. That is the changes list, without a second tab. The panel header shows the changed file count and total +/−.
- Live refresh from a `notify` watcher on the worktree plus the git index and HEAD, debounced.
- Outside a git repo: a plain tree, with no status and no filter.

### Phase 2 — read-only viewer

- Clicking a changed file opens its diff; clicking an unchanged file opens its content. Both are read-only.
- Diff: unified, line numbers for both sides, added and removed line backgrounds. An untracked file shows as all added, a deleted file as all removed. Binary and oversized files show a placeholder with the size.
- Files and diffs open as closable tabs in the side panel, beside its Info and Files tabs, so the terminal stays visible while you read; the panel drags wider for a diff. The tab strip is in frame `U2p3jT` of the shared design file. Decided by Jason on 2026-10-09, replacing an overlay over the terminal pane.
- ⌘-click on any line opens the file at that line through the #458 opener (`file_links.rs`, the Settings → General editor choice).
- Virtualized rendering, so a multi-thousand-line file or diff does not stall the frame.

### Phase 3 — diff base

- Default base is HEAD: the uncommitted work in the checkout.
- A small base picker: HEAD, or a branch compared from its merge-base. A mission running in its own branch worktree, as Runner's own missions do, picks `origin/main` to see everything the branch did, its commits included.
- The base label always says what is compared, for example "Working tree vs HEAD" or "Working tree vs origin/main".

### Phase 4 — review to the agent

- Select one or more lines in a diff and write a comment. Comments collect into a pending review across files, shown with a count in the viewer header, and can be edited or removed before sending.
- **Send review** delivers the batch as one message: in a mission, to a chosen slot as a directed message (`--to @coder`), defaulting to the slot whose session the panel belongs to; in a direct chat, to that session as a prompt, the same delivery as #704's session send.
- Each comment carries its file path, line range, the quoted lines and the text, so the agent can act without the human pasting anything. Orca can only send to agents in one worktree; Runner routes the review through the crew.

## Non-goals

Editing files; stage, unstage or discard; commit, push or PR creation; create, rename, delete or move in the tree; project-wide content search; merge-conflict UI; GitHub, Linear or Jira panels; storing reviews as records or syncing them to GitHub pull requests; worktree creation (#403 stays closed). Split diff view and syntax highlighting are out of v1 — the workspace has no highlighter dependency today, and ⌘-click to the editor covers the gap.

## Risks

- **Large repos.** The tree walk and `git status` on a monorepo must stay off the UI thread. A build writing tens of thousands of files into an ignored `target/` must not trigger a status rerun per event.
- **Git as a subprocess.** Shell out to `git` resolved through the login-shell PATH (`shell_path`, as agent spawns do) rather than adding libgit2. Windows needs `git.exe` on PATH and CRLF-aware diff display.
- **Shared checkouts.** A chat or mission without its own worktree shares the checkout, so the diff mixes the human's edits and other sessions' changes. The base label must say "Working tree vs HEAD", not imply the diff is the session's.
- **New rendering work.** GPUI has no tree, code or diff view to reuse; the viewer is new element work on a virtualized list.
- **Review delivery while busy.** A review sent to a working agent waits like any crew message; it must not be typed into a running turn or lost if the session is stopped.

## Verification

- Unit tests for the porcelain v2 parser (renames, untracked, unmerged, paths with spaces), the folder rollup, and the Changed only filter.
- A fixture repo with modified, added, deleted, renamed, untracked, ignored and binary files: tree letters match `git status`, and each diff matches `git diff HEAD` for that path.
- An agent edits a file while the panel is open: the row and the open diff update within the debounce window without a manual refresh.
- Large-repo smoke: the Runner repo with `target/` built opens the panel without a visible stall, and a `cargo build` does not churn status.
- A branch worktree with two commits shows both against `origin/main` and neither against HEAD.
- A review with comments on two files arrives at the chosen slot, or at the direct chat, as one message carrying paths, line ranges, quoted lines and comments.
- macOS and Windows.

Design: `runner/desktop/specs/793-634-glass-and-files.pen` in `yeelinks_design`, shared with [793](./archive/793-liquid-glass.md), for sign-off before any code. Frames: `U2p3jT` (side panel tabs, Info), `F0DYX` (Files tab, changes and the tree) and `veCIr` (a file tab, the diff with the tree beside it).
