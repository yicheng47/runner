# 634 m1 — runnerd file service

## Starting point

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-634-m1-files-service`, on branch `feat/634-m1-files-service`, cut from the umbrella `origin/feat/634-file-preview` at `2e24068d`. This brief is the branch's tip. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. The umbrella follows `main` by rebase, and this branch follows the umbrella by rebase (`git rebase origin/feat/634-file-preview`); never merge either into it. m2 runs in parallel on `feat/634-m2-panel-views` in `runner-app` only; if it merges first, rebase onto the new umbrella.

## Problem and outcome

#634 adds a Files view to the side panel: a project tree with git status and a read-only file viewer. Jason decided on 2026-11-04 that `runnerd` serves the files, so remote file access can later come from another computer's `runnerd`. This mission builds that service in `runner-core` and `runner-daemon`, behind client-protocol requests and pushed events, with no UI: m3 builds the tree on it.

## Read first

- [The plan](../634-file-preview/plan.md), sections Where the work runs, Phase 1 and the Linux and Windows notes. It is the authoritative scope for this mission; this brief adds only what the plan leaves to the mission. The [spec](../../features/634-file-preview.md) wins on behaviour, especially Files view and its root and Tree with git status.
- `crates/runner-core/src/protocol/api.rs` (`daemon_api!`, fast vs slow) and `protocol/mod.rs`.
- `crates/runner-daemon/src/session/manager/terminal.rs` (`link_cwd`), `shell_path.rs`, `events.rs` (`emit`), `event_bus/mod.rs` (the existing `notify` usage), `db/migrations.rs` and `db/app_state.rs`.
- `crates/runner-cli/src/command.rs` (the exhaustive recorder test over `API_OPERATIONS`).

## Deliverables

As the plan's Phase 1 lists them, in commits that each build and pass:

1. `runner-core/src/protocol/files.rs`: `FilesSubject`, `FilesRoot`, `FilesRootOutcome`, `DirEntry`, `GitStatus` with a pure rollup function, `WorktreeEntry`, `FileContent`.
2. `runner-daemon/src/files/`: `git.rs` (rev-parse, `git status --porcelain=v2 -z --untracked-files=all --ignored=matching`, `git worktree list --porcelain -z`, both parsers; `git` through the login-shell PATH; missing `git` makes roots plain, logged once), `fs.rs` (listing with `.git` hidden and ignored entries marked, reading with the 10 MB limit and the NUL-in-8 KB binary rule, the path check), `root.rs` (resolution in the plan's order).
3. The migration adding nullable `files_root` to `sessions` and `missions`, and recent folders (last ten, newest first) under one `app_state` key.
4. `files/watch.rs`: one shared watcher per root, released with its last subscriber or its client's disconnect; ignored-folder events dropped before a trailing debounce with a maximum wait; the git directory from `--git-dir` watched for `index` and `HEAD`, including a linked worktree's `<main>/.git/worktrees/<name>/`; on Linux, per-folder non-recursive watches over folders that are not ignored, falling back to a five-second status poll while watched when the watch limit is hit; one status run at a time, later changes coalesced.
5. The slow requests `files_root`, `files_set_root`, `files_list`, `files_status`, `files_worktrees`, `files_read`, `files_watch`, `files_unwatch`, `files_recent_folders`, and the pushed events `files/changed` and `files/status`. No CLI commands; the recorder test keeps passing.

Names and exact field shapes are yours where the plan does not fix them; keep them serde-stable, since a later companion protocol will mirror them. The path check is not optional: every request is a root plus a relative path, and `..`, absolute paths and symlinks leaving the root are refused.

Non-goals: any `runner-app` change, CLI commands, deleted-file display (#1141), remote access itself, and new crates beyond what the watcher already uses. If a design question arises that the plan and spec do not settle, ask Jason through Runner rather than choosing silently.

## Validation

The plan's Phase 1 Checks: the porcelain and worktree parsers (renames with old path, copied, untracked, unmerged, ignored, spaces and newlines under `-z`; main, linked, detached, locked, prunable), rollup, root resolution (eight cases), path refusal, read outcomes (text, binary, exactly 10 MB and one byte over, missing), a fixture repository built with the git CLI in a temp dir whose statuses match `git status`, the `.worktrees/` fixture, the watcher (one event pair per edit; 10,000 files into ignored `target/` cause no status run; a commit in a linked worktree refreshes), and the migration plus the saved root's round trip. Workspace clippy and tests with the CI profile, logged with exit codes. Linux and Windows behaviour is proved by CI's jobs.

No live checks: nothing a person sees changes. The reviewer's clean review says so.

## Handoffs

The coder implements, tests and fixes. The reviewer waits for the coder's Runner handoff, then reviews the whole working-tree diff against the plan, spec and this brief, must-fix first with file:line pointers, checking in particular: the path check and symlinks, the watcher's lifetime and debounce under a long build, the linked worktree's git directory, Linux watch limits, the migration, and that nothing blocks a daemon request thread longer than one git run.

## Authorization

Approved crew: the claude duo. Once review is clean the crew may commit, push this branch and open a PR against `feat/634-file-preview` (not `main`), titled `feat(files): runnerd file service (#634 m1)`, labelled `risk-high` (with `core-review`, which the workflow adds), and drive CI green on every job. Jason merges it into the umbrella as a rebase merge; the crew does not merge.
