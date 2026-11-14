# #1141 m1 — the runnerd changes service

## Starting point

- Issue: [#1141](https://github.com/yicheng47/runner/issues/1141), the git changes view. This mission is m1 of three pull requests to `main` (the spec's [Delivery](../../features/1141-git-changes.md#delivery)).
- Branch: `feat/1141-git-changes`, which has no upstream yet: push it with `git push -u origin feat/1141-git-changes`, never to `main`.
- Worktree: `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1141-git-changes`, with its own Cargo `target/`.
- Base: `origin/main` at `6e383f8d`, which holds the spec.
- Parallel isolation: `.worktrees/feat-1288-agy-bypass-and-trust` is active. Stay inside this worktree; do not touch the root checkout or sibling worktrees.

## Problem and outcome

The Changes view (m2) needs `runnerd` to serve diffs, the branch's commits since its base, and viewed marks. m1 adds that service to `runner-core` and `runner-daemon` with tests, and nothing a person sees. It must not change anything that runs today: no existing `files_*` request, event, table or watcher behaviour changes, and the CLI gets no commands.

The spec [`docs/features/1141-git-changes.md`](../../features/1141-git-changes.md) wins on behaviour. The sections m1 implements: [Commits and what is compared](../../features/1141-git-changes.md#commits-and-what-is-compared), the edge cases under [The diff](../../features/1141-git-changes.md#the-diff), [Viewed files](../../features/1141-git-changes.md#viewed-files), [Where it runs](../../features/1141-git-changes.md#where-it-runs) and the Automated part of [Verification](../../features/1141-git-changes.md#automated).

## Read first

- The spec sections above, and #634's [Phase 1 plan](../archive/634-file-preview/plan.md#phase-1--m1-the-runnerd-file-service), the pattern this follows.
- `crates/runner-core/src/protocol/files.rs` (`FileStatus`, `GitStatus`, event names) and the `files_*` entries in `crates/runner-core/src/protocol/api.rs`.
- `crates/runner-daemon/src/files/` (`git.rs` for how git runs and is parsed, `mod.rs`, `root.rs`, `watch.rs`) and `crates/runner-daemon/src/daemon/server.rs` (how `files_watch` subscribers get pushed events).
- `crates/runner-daemon/migrations/README.md`, `crates/runner-daemon/src/db/migrations.rs` and `crates/runner-daemon/src/repo/` (SQL lives only behind `repo/` functions, per `docs/arch/arch.md`).
- The CLI's exhaustive request recorder test in `crates/runner-cli/src/command/tests.rs`.

## Deliverables

1. **Types** in `runner-core` (a `protocol/changes.rs` beside `files.rs`, or in `files.rs` if it fits better): the comparison target (uncommitted, or a commit id); the commits result (branch or detached commit, the resolved base or none, commits with id, short id, subject, author, date and parent count, at most 200 with the total); a changed file (path, `FileStatus`, a rename's old path, added and removed counts or binary, both blob ids, viewed, changed since viewed); the diff result (hunks as line ranges with their `@@` text, and the old and new contents, or a placeholder: binary or too large with both sizes, mode only with both modes, submodule with both commits; plus a mode note and a conflicted flag); the `changes/viewed` event.
2. **Git**, in `runner-daemon/src/files/`, run as `git.rs` runs it (login-shell PATH, `--no-color`, `--no-ext-diff`, no pager, `core.quotePath=false`, `-z` where paths are listed), every path checked against the root:
   - Base resolution in the spec's order (`origin/HEAD`, `origin/main`, `origin/master`, local `main`, local `master`), the merge base with `HEAD`, and the commits reachable from `HEAD` and not from it. Detached `HEAD`, no base, and a repository with no commit.
   - Changed files for uncommitted changes (`git diff HEAD` plus untracked files, whose lines runnerd counts) and for a commit (against its first parent, or the empty tree for a root commit), limited to the root's subtree, with rename detection.
   - One file's diff: hunks from git, contents within #634's 10 MB per side, and the placeholders. Leave CRLF in the contents; the app detects a CRLF-only change.
   - Blob ids for both sides; `runnerd` hashes a working-tree file the way git would.
3. **Viewed marks** (updated per Jason, 2026-11-14): keep marks in `runnerd`'s memory (no migration and no database table), keyed by checkout top, path and both blob ids, recording whether the mark was made on uncommitted changes (sticky once true). Setting and clearing a mark pushes `changes/viewed` to the clients watching that checkout.
4. **Requests** in `protocol/api.rs`: `changes_commits`, `changes_files`, `changes_diff` and `changes_set_viewed`, following the `files_*` entries. `changes_set_viewed` also takes the target, which "changed since viewed" needs; update the spec's request table to match what you build.

Not in m1: any `runner-app` code, guide pages (nothing user-visible changes), CLI commands, remote sessions.

## Validation and handoffs

- **Automated**, the spec's list: the parsers (numstat `-z` with renames, copies, binary and paths with spaces and newlines; hunks with `\ No newline at end of file`; the log); base resolution across the listed fixture repositories, built with the git CLI in temporary folders; the file-kinds fixture (modified, added, renamed, deleted, untracked, binary, over 10 MB, CRLF-only, mode-only, conflicted, symlink, submodule) matching git, and a subfolder root; in-memory viewed marks (set and clear, sticky uncommitted flag, carry-over when a file is committed unchanged, changed since viewed, a second client's event; no migration or startup prune).
- **Commands**: `cargo test -p runner-core -p runner-daemon -p runner-cli`, then `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` and `cargo fmt --all --check`. CI runs Windows and Linux.
- **Live checks**: none; nothing a person sees changes. The reviewer confirms this in its clean review.
- **Handoffs**: `coder` hands `reviewer` unpushed local commits by SHA and iterates until the review is clean.

## Authorization

Jason approved the agy duo for m1 on 2026-11-13. It may commit, push `feat/1141-git-changes` and open a PR against `main` titled for m1 with "Part of #1141" (not "Closes"), labelled `core-review` and (updated per Jason, 2026-11-14, after moving viewed marks into memory with no migration) `risk-medium`, and drive CI green. Fold this brief into the implementation commit. Stop at the open, reviewed, CI-green PR: Jason reviews `core-review` PRs before they merge.
