# #1430 — split the Files and Changes surfaces by responsibility

## Starting point

- Issue: [#1430](https://github.com/yicheng47/runner/issues/1430), the last open slice of the [#1248](https://github.com/yicheng47/runner/issues/1248) cleanup tracker.
- Branch: `refactor/1430-files-changes-split`
- Worktree: `/Volumes/samsung/repos/yicheng47/runner/.worktrees/refactor-1430-files-changes-split` (base `origin/main` `6ae678d7`, own `target/`).
- Other live worktrees: `feat-1489-kitty-graphics-protocol` (a running neon trio with a commit on `files/image.rs`), `feat-1458-file-open-setting` (a spec only, whose implementation will touch `files/links.rs` and `files/tabs.rs` later) and `fix-818-terminal-round-trips`. Leave them alone. If one of them merges first and conflicts, rebase onto `origin/main`.
- Fold this brief into the first commit.

## Problem and outcome

#1423 unified the Files and Changes viewer, and these are now the largest production files in the repo:

- `runner-app/src/surfaces/files/open_file.rs` (5,149 lines): one `impl UnifiedViewer` of about 3,900 lines, followed by an inline test module.
- `runner-app/src/surfaces/changes/mod.rs` (2,893), `changes/diff.rs` (2,631) and `changes/render.rs` (2,166).
- `runner-app/src/surfaces/files/mod.rs` (1,541).
- `runner-daemon/src/files/changes.rs` (2,462).

Outcome: each file is split into child modules named after what they hold. **This is a mechanical move only:** methods move into child-module `impl` blocks, types and free functions move into child modules, and inline test modules move to sibling `tests.rs` files. The only edits allowed besides moved code are `mod`/`use` lines and the minimal visibility widening the move requires (`pub(super)`, or `pub(in …)` for a path further up). Make no renames, logic changes, reordering inside a function, new comments or doc changes. Do not change any `Entity`, `cx.notify()`, subscription, `FocusHandle`, task or render/hit-test behaviour. If a move seems to need one of those, leave that code where it is and say so in the PR.

## Module plan (Jason approved it on 2026-11-27)

Names may be adjusted where the code says otherwise; give the reason in the PR. The PR description carries the final map.

1. **Files viewer, `files/open_file/`** (already has `find.rs`, `open_in.rs`, `render.rs`). The struct, `new`, the context, owner and chrome setters and the small dirty or uncommitted queries stay in `open_file.rs`.
   - `watch.rs`: `request` and `resolve`, the root sync and outcome, the status snapshot, `set_shown`, the watch chain, `apply_status`, `on_files_event`, `settings_changed`, and the diff mode and tree-resize helpers if they fit there better than staying.
   - `changes.rs`: `open_pinned_file`, `open_changes_file`, `sync_branch_base`, `reconcile_changes_files` and `apply_changes_viewed_optimistic`.
   - `tab_nav.rs`: tab pin, activate, cycle and close, `tab_shown`, `reset_tabs`, focus, `escape_to_rail`, `jump_hunk`, `on_viewer_key`, zoom, the `active_*` diff queries and `set_active_mode`.
   - `read.rs`: `show_line`, the ignored-tab refresh, `read_file*`, `apply_read` and `reload_changed`.
   - `uncommitted.rs`: from `reprobe_working_tree_exists` to `refresh_non_uncommitted_targets`.
   - `diff.rs`: `reconcile_diff_tabs`, `load_code_hunks`, `load_tab_diff` and `apply_tab_diff`.
   - `viewed.rs`: `toggle_viewed_for_tab`, `reconcile_failed_viewed_write` and `apply_viewed_event`.
   - `rendered.rs`: the image, SVG and Markdown rendered state, `toggle_source` and the link handlers.
   - `saved.rs`: the saved and restore state, with `SavedViewerTab` and `SavedViewerState`.
   - `test_support.rs`: the second `impl UnifiedViewer` (its test hooks), behind the same `cfg` it has now.
   - The inline tests go to `open_file/tests.rs`.
2. **`files/mod.rs`:** `FilesHub`, `GlobalFilesHub` and `hub()` go to `hub.rs`, and `DirtyTracker` goes to `dirty.rs`. `FilesPanel` stays.
3. **Changes view, `changes/`** (already has `model.rs`):
   - `root.rs`: from `request` to `recover_from_root_error` (the choose screen, recent, typed path, browse, watch and files events).
   - `snapshot.rs`: `load_snapshot*`, `apply_snapshot`, `select_target`, `apply_files_list` and `select_file_by_path`.
   - `viewed.rs`: the viewed toggles and events, with the viewer's viewed callbacks.
   - `pickers.rs`: the base and checkout pickers.
   - `list.rs`: the tree view, list rows, folders, focus and list keys.
   - `diff_body.rs`: `LoadedDiffKind`, `PreparedDiffBody` and `prepare_diff_body`.
   - `test_support.rs`: the second `impl ChangesView`.
   - Split `render.rs` into `render/{list,branch,choose,shared_diff}.rs`. The shared header, placeholder and footer functions go to `shared_diff.rs`.
   - Split `diff.rs` into `diff/{view,shape,element}.rs`. `UnifiedDiffView` goes to `view.rs`, the `Shaped*` types and `shape_*` functions to `shape.rs`, and `UnifiedDiffElement`, `DiffPrepaint`, `position_at` and `register_mouse` to `element.rs`. Its inline tests go to `diff/tests.rs`.
4. **Daemon, `runner-daemon/src/files/changes/`.** The public `changes_*` ops, `ViewedMarks` and `validate_relative_path` stay in `changes.rs`.
   - `git.rs`: the `git_cmd`/`run_git*` helpers and base and branch resolution.
   - `collect.rs`: `DiffContext` and the `collect_*`/`diff_tree_entries` helpers, including the uncommitted ones.
   - `working.rs`: working-tree reads, metadata, hashing and the no-index diff helpers.
   - `parse.rs`: the commit, numstat, raw and hunk parsers, plus `Sha1`.
   - `changes_tests.rs` keeps its tests. Only fix its paths if a moved item requires it.

**Out of scope:** `files/image.rs` (#1489 is on it), `files/links.rs` and `files/tabs.rs` (#1458), `ui/markdown/`, and anything outside these surfaces.

## Validation and handoffs

- **@coder:** Make one commit per surface, in this order: Files viewer with `files/mod.rs`, then the Changes view, then the daemon. Each commit must build and pass its tests on its own. Before handing off, run `cargo test -p runner-app`, `cargo test -p runner-daemon files::`, `make clippy`, `cargo fmt --all -- --check` and `git diff --check`. Other missions are running on this Mac, so do not run workspace-wide suites concurrently with heavy builds you started. Hand `@reviewer` the unpushed head SHA, a before/after line count for each file, and the final module map.
- **@reviewer:** For each commit, check with `git show --color-moved=dimmed-zebra --color-moved-ws=allow-indentation-change` that every change is a moved block, a `mod`/`use` line or a visibility widening. Treat anything else as must-fix unless it is plainly required by the move. Confirm that no test or golden changed its assertions. Under [Test Scope](../../../AGENTS.md#test-scope), a strictly mechanical move needs no live checks: say so in the clean review. If the diff crosses a GPUI ownership or reactivity boundary, name the live checks instead. Add `reviewed` to the PR once it is open, and comment with the reviewed SHA.

## Authorization

Approved crew: `neon duo` (`coder` and a Codex `reviewer` on `gpt-6.2-astra`). After `@reviewer` passes, `@coder` may push the branch and open a PR against `main` with `Fixes #1430`, labelled `no-behavior-change` and `risk-low`, then drive CI green.

**Exception, approved by Jason on 2026-11-27:** `@coder` may merge the PR with `gh pr merge <n> --merge`, but only when all of these hold:

- The current head is `reviewed`.
- Every check in `gh pr checks` has passed on that head. Do not use `--auto`.
- The PR carries no `core-review` or `needs-live-test` label.

If the reviewer names live checks, add `needs-live-test`, do not merge, and report to the operator, which adds `qa`. A rebase that resolves a conflict counts as a code change and needs a fresh review. After merging, post the merged PR number and merge SHA. The operator runs the post-mission cleanup, so do not archive this mission or remove the worktree yourself.
