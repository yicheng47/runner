# 634 m11 — the second umbrella review's findings

## Starting point

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-634-m11-review2-fixes`, on branch `feat/634-m11-review2-fixes`, cut from the umbrella `origin/feat/634-file-preview` at `aa6b0c4f` (m1–m10 merged, on `main` `324dbacb`). This brief is the branch's tip. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. Follow the umbrella by rebase, never by merge.

## Problem

The second whole-umbrella review of #1157 on `aa6b0c4f`, by `reviewer-opus` (Claude) and `reviewer-astra` (Codex), found two must-fix defects where the file preview reaches Runner's core, and six should-fix ones. Both reviewers found the must-fix pair independently and agreed each other's lists. The full reports, with locations, failure scenarios and measurements, are PR #1157's comments [6681535832](https://github.com/yicheng47/runner/pull/1157#issuecomment-6681535832) (opus) and [6681539188](https://github.com/yicheng47/runner/pull/1157#issuecomment-6681539188) (astra): read both first with `gh api repos/yicheng47/runner/issues/comments/<id> --jq .body`.

## Decisions (the operator, under Jason's mandate, 2026-11-06)

1. **M1, the `owners` lock** (`daemon/server.rs:551`, `files/mod.rs:133-150`, `files/watch.rs:64`): `files_watch` does `check_root`, the `git` lookup and the canonicalize before taking `owners` or `watches`; only the `closed` check, the map update and the thread spawn stay under them, keeping the guarantee that a watch in flight cannot outlive the connection's removal. Add a test that holds watch preparation at a gate while another connection observes terminal input, beside the existing disconnect-race test.
2. **M2, ignored lookups on the UI thread** (`protocol/files.rs:233-270`, `files/tree.rs:162-191`, `375-398`, `files/view.rs:689-699`): index the ignored set when a status is installed so a lookup costs the path's depth, not the list's length; make `rollup` use it; and rebuild the flattened tree once per listing batch, not once per folder. Add a test or benchmark-style test over a many-folder, many-ignored fixture that bounds both `rollup` and a status install.
3. **S1, hidden views** (`files/view.rs:728-751`): a hidden view ignores `Changed` and `Status` as it already ignores `Resync`; showing it refreshes, as today. Add the two-subject same-root test astra describes.
4. **S2, canonicalize errors** (`files/fs.rs:55-63`): walk up to a parent only on not-found and not-a-directory; refuse every other error. A dangling link still reads as missing. Add opus's long-link case as a test.
5. **S3, Linux watcher and a new directory symlink** (`files/watch.rs:457-468`): the dynamic create/rename path uses non-following metadata, as the initial walk does.
6. **S4, clean filters**: accepted, not changed. It needs a repository whose local config someone set, and Runner runs as that same user. Correct the comment by `core.fsmonitor=false` (`files/git.rs`) so it claims only what it does, and record the decision in the program README's decisions.
7. **S5, slow status**: a status that later arrives clears the matching status error, and runnerd coalesces in-flight status work per root so a re-show or a new watcher does not start a second `git status` while one runs. No new `git` timeout.
8. **S6, the saved root** (`files/root.rs:24-31`, `files/mod.rs:243-245`): forget a saved root only when its path is confirmed missing or not a folder; any other metadata error keeps it and shows the error. Whether an unmounted volume counts as gone stays as the spec says (forgotten) and goes to Jason as a question in the PR, not a change.
9. **Note, Ctrl+Shift+O with Files off** (`app_shell/actions.rs:382`, `panes/header.rs`): when there is no view to search, the action propagates, so the terminal gets the chord back on Windows and Linux.

Out of scope, by the program's recorded decisions: the check-to-read race and tying a root to its subject (both before remote access), keeping a watcher alive after its last subscriber, and every other note in the two reports.

## Validation

Each fix has a regression test that fails without it. Run `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`, workspace clippy with the CI profile and with `--features runner-app/updater`, fmt and `script/check-guide-translations.py`, logged with exit codes. Update the spec, the plan (an m11 section), the program README (table row and decision 6's record) and `impl_log.md`, which lists each round-2 finding and note with what was fixed, left and why.

The reviewer decides from the diff whether live checks are needed and names them in its clean review; Jason's live test passed on `aa6b0c4f`, so only checks this diff makes necessary run. If it names any, the operator adds the codex `qa` role after the clean review.

## Handoffs

The coder implements, tests and fixes, then hands the candidate to the reviewer through Runner. The reviewer reviews the whole diff against these decisions and the two round-2 reports, must-fix first with file:line pointers, and checks in particular that nothing outside the file preview changes behaviour, that the `owners` lock is never held across filesystem work, and that no UI-thread path still scans the ignored list per row.

## Authorization

Approved crew: the claude duo. Once review is clean, the crew may commit, push this branch and open a PR against `feat/634-file-preview` titled `fix(files): the second umbrella review's findings (#634 m11)`, labelled `risk-medium` (the Core review workflow adds `core-review`), plus `needs-live-test` if the clean review names live checks, and drives CI green on every job. The reviewer adds `reviewed` with its clean review. The operator merges it into the umbrella; the crew does not merge.
