# 634 m3 — the Files view

## Starting point

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-634-m3-files-tree`, on branch `feat/634-m3-files-tree`, cut from the umbrella `origin/feat/634-file-preview` at `2b4e1825` (m1 and m2 merged). This brief is the branch's tip. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. Follow the umbrella by rebase (`git rebase origin/feat/634-file-preview`), never by merge.

## Problem and outcome

#634's Files view exists as an empty side-panel view (m2) on top of `runnerd`'s file service (m1). This mission fills it: the root and its states, the worktree picker, and the tree with git status, icons, keyboard and live refresh. A click on a file only selects it; opening files is m4.

## Read first

- [The plan](../634-file-preview/plan.md), Phase 2, m3: the authoritative scope. The [spec](../../features/634-file-preview.md) wins on behaviour (Files view and its root, Tree with git status).
- The design, exported as PNGs in this worktree's ignored `target/design-ref/` from `runner/desktop/specs/634-file-preview.pen` (`yeelinks_design` `e58d8a0`). They win on visual detail; match them with theme tokens, sample data is illustrative; do not delete them or open `.pen` files.
  - `flow-outside-project.png`: choose a folder, then the tree.
  - `flow-worktree-picker.png`: a mission on its worktree, the picker, rooting at the main checkout.
  - `tree-on-the-right.png`: the tree with letters, dots, the `5 changed` count and the root header. Its file tabs and code are m4's.
  - `terminal-tab-files-panel.png`: the empty start in a terminal tab.
  - `edge-roots.png`: a missing root, a plain folder outside git, "Reading status…".
  - `tree-and-viewer-states.png`: keyboard focus versus hover, and "Files aren't available here" for a remote session. Its long-line and find panels are m4's.
  - `file-icon-sets.png`: Catppuccin, Material, Minimal.
  - `m2-header-files-on-right.png` and `m2-mission-with-file-open.png`: m2's header, for the live checks below.
- m1's API: `crates/runner-core/src/protocol/files.rs` (`FilesSubject`, `FilesRootOutcome`, `FilesRoot`, `DirEntry`, `GitStatus` and `rollup`, `WorktreeEntry`, the `files/changed` and `files/status` events) and the `files_*` requests in `protocol/api.rs`. `files_watch`/`files_unwatch` are tied to the app's connection by `daemon/server.rs`.
- m2's contract: `crates/runner-app/src/surfaces/side_panel.rs` (`PanelViewId::Files`, `PanelContext`, `files_view`).
- How the app calls the daemon and receives pushed events (`app_store`), the sidebar's project picker for `prompt_for_paths`, `file_links.rs` and `mission_workspace/rail.rs` for reveal.

## Deliverables

1. **Root states**: choose a folder (path field, Browse via the system picker, projects, recent folders); the root header (checkout name, branch icon for a linked worktree, changed count, "Reading status…" until the first status); a plain tree outside git; "This folder no longer exists" with the main checkout and the picker; and "Files aren't available here" for a remote subject, without calling the daemon.
2. **The worktree picker**: field filters or takes a path applied on Enter; worktrees (main first) with branch and short commit; recent folders, plus projects for a chat or terminal outside a project; Choose folder…. A choice is saved with `files_set_root`.
3. **The tree**: virtualized rows on GPUI's `uniform_list` (the app's first); folders load as they open; ignored entries dimmed; status letters in their colours, folder dots and the changed count from `GitStatus`; deleted entries not shown; the `cwd` folder expanded and tagged; right-click with Show files from here, Copy path and the reveal row: Reveal in Finder, Show in Explorer on Windows, and Open containing folder on Linux through `xdg-open` (decided 2026-11-04 by the operator: Linux has no common select-the-file call). Clicking a file selects it.
4. **Keyboard**: ↑ ↓ → ←, type-ahead, hover tint versus the focus ring. Space and Enter are wired by m4; leave a clear hook.
5. **File icons**: a loader for Zed's icon-theme JSON; Catppuccin (Latte and Mocha follow light and dark), Material and Minimal bundled with their licence files; Catppuccin until m6 adds the setting. Bundle only the icons the themes map, and keep the app's size growth small; say how much in the handoff.
6. **Live refresh**: `files_watch` while the view is shown, `files_unwatch` when hidden or the root changes; apply `files/changed` and `files/status` in place, keeping expansion and scroll.
7. **Guide**: the Files view, its root and the picker in [Chats and windows](../../guide/guides/chats-and-windows.md) and [Missions](../../guide/concepts/missions.md), in both locales.

Non-goals: opening files, tabs, the widened panel (m4); Settings (m6); any change to m1's daemon behaviour. A small protocol addition the tree genuinely needs is allowed but must be justified in the handoff, since it makes the PR `core-review`.

## Validation

Unit tests: the tree model (expand, collapse, rollup display, type-ahead and keyboard moves, refresh keeping expansion), the picker's filtering and path entry, the icon-theme loader's matching, and each root state from snapshots in both themes. `runner-app` tests and workspace clippy with the CI profile, logged with exit codes.

Live checks are required, run by `qa` after the clean review on macOS. The clean review names them; they include m3's own from the plan (a fixture repository's letters match `git status`; an agent's edit updates the row without a refresh; a chosen root survives switching tabs and restarting the app, per chat and per mission; the picker lists the main checkout and a worktree, and rooting at the main checkout shows `.worktrees/` dimmed; the Runner repository with `target/` built opens without a visible stall and `cargo build` does not churn status) and m2's four, which Jason moved here from #1161: the header on every surface in both themes, view bodies and mixed-tab focus, toggle and resize on all four surfaces, and native clicks never moving or zooming the window.

## Handoffs

The coder implements, tests and fixes. The reviewer waits for the coder's Runner handoff, then reviews the whole working-tree diff against the plan, spec, PNGs and this brief, must-fix first with file:line pointers, checking in particular: watch and unwatch pairing, no daemon request on the UI thread, virtualization on a large folder, ignored-folder handling, the remote state, and icon licences.

## Authorization

Approved crew: the claude duo. Once review is clean the crew may commit, push this branch and open a PR against `feat/634-file-preview` (not `main`), titled `feat(ui): the Files view with git status (#634 m3)`, labelled `risk-medium` and `needs-live-test`, plus `frontend-only` if nothing outside `runner-app` changes, and drive CI green on every job. The operator merges it into the umbrella; the crew does not merge.
