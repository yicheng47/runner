# #1213 m2 — Git worktrees in the app

## Starting point

- Issue: [#1213](https://github.com/yicheng47/runner/issues/1213), milestone 0.20. m1, the `runnerd` service and CLI, merged in [#1350](https://github.com/yicheng47/runner/pull/1350) (`be023414`). This is m2, the second and last pull request.
- Branch: `feat/1213-worktrees-app`, new from `origin/main` at `be023414`. Open a new PR for it.
- Worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1213-worktrees-app`, with its own Cargo `target/`.
- Design: `runner/desktop/specs/1213-worktrees.pen` in `yeelinks_design` (checked out at `~/repos/yeelinks/yeelinks_design`; pull before opening, read only), revision `685b8c9`, board `A0AUx`. Sections: 1 the launcher's Checkout control and its folder coupling, 2 New worktree…, 3 the sidebar rules with their edge-case table and both views, 4 the Info panel, 5 cleanup at archive. Where the board and the spec differ, the spec wins.
- Other worktrees are active (`feat-1366`, `fix-1356`). Stay inside this worktree, run heavy workspace suites one at a time, use a `/tmp/1213-m2-*` prefix for every scratch folder and fixture, and take `script/qa-lock` before any live QA.

## Problem and outcome

m1 gave `runnerd` worktrees: create-and-start, checkout state derived from a cwd, removal under `.worktrees/`, restore of a missing worktree, and `runner worktree`. Nothing in the app uses them yet. m2 builds every app surface in the spec [`docs/features/1213-worktrees.md`](../../features/1213-worktrees.md), which is the contract: the launcher's Checkout control and New worktree…, the sidebar's branch line and the Active location change, the Info branch row, Remove worktree at archive, and the restore fallback. Runner still stores nothing about worktrees (decisions 9 and 10): every display comes from the session's or mission's cwd through m1's requests.

## Read first

- The spec in full; then m1's requests in `crates/runner-core/src/protocol/{worktree.rs,api.rs}` (`worktree_check`, `worktree_create`, `worktree_list`, `worktree_remove`, `worktree_checkout`, `worktree_default_base`) and `changes_branches` for the base list.
- `runnerd`: `crates/runner-daemon/src/files/worktree.rs` (`worktree_checkout_state`, `restore_if_missing`) and the unarchive paths in `crates/runner-daemon/src/ops/{session.rs,mission.rs}`.
- Launcher: `crates/runner-app/src/surfaces/start_chat/` (`project_field.rs`, `launcher/`, `launch.rs`); #1345's pickers in `crates/runner-app/src/surfaces/changes/` and Files' root picker in `crates/runner-app/src/surfaces/files/`.
- Sidebar: `crates/runner-app/src/surfaces/sidebar/` (`active.rs` `active_location`, `rows.rs`, `rows_render.rs`, `link.rs`, `archive.rs`).
- Info: `crates/runner-app/src/surfaces/panes/panel.rs` and the mission rail in `crates/runner-app/src/surfaces/mission_workspace/`.
- Archived: `crates/runner-app/src/surfaces/settings/archived.rs`; strings in `crates/runner-app/i18n/{en,zh-hans}.toml`.

## Deliverables

1. **Launcher (section 1) and New worktree… (section 2):** the Checkout control in the Location row, the folder-decides coupling, the Checkout menu in #1345's picker style, the New worktree form with its empty focused branch field, base chip and hint line built from `worktree_check`, and starts that pass `worktree` in one request. Chats, terminals and missions alike; no control for remote sessions or empty split panes.
2. **Sidebar (section 3):** the branch line in Projects, `<project> › <branch>` without subfolders in Active, and every edge case in the spec's list (main checkout on another branch, outside worktrees, detached `HEAD`, "2 checkouts", "missing · <branch>", truncation, remote offline, before git answers). Refresh from the watcher #634 runs, never by polling per row.
3. **Info (section 4):** the `branch` row only in linked worktrees, with the "Worktree · from <base>" marker from Changes' `auto_base`, in the chat panel and the mission rail.
4. **Archive (section 5):** Remove worktree `<branch>` in the archive confirmation for Runner's worktrees the archive empties, checked when clean and unavailable with the reason when dirty, one row per worktree for a chat with linked missions, a toast when git still refuses, and the branch always kept.
5. **Restore fallback (decision 6):** when restoring or resuming fails because the branch is gone or ambiguous, say so and offer **Restore in main checkout**, warning that a Claude chat starts without its history. m1 has no request for this, so add one to `runnerd`: an option on `session_unarchive`, `mission_unarchive` and `session_resume` that sets the stored cwd to the main checkout root, keeping the subfolder when it exists there. It is optional with a serde default, takes the worktree gate like the other registration paths, and gets tests. This is the only `runnerd` change in m2, so the PR is not `frontend-only`.
6. **Docs:** the guide pages the spec names that m1 did not finish ([Chats and windows](../../guide/guides/chats-and-windows.md), [Missions](../../guide/concepts/missions.md), [Your first mission](../../guide/getting-started/first-mission.md), [Build a crew](../../guide/guides/build-a-crew.md)) with their `zh-CN` mirrors, the spec's Delivery row for m2, and its Status as implemented.

Out of scope: everything in the spec's Non-goals. The known m1 follow-up, where a deleted outside folder named like a live branch counts as a missing worktree, stays out of this PR.

## Validation and handoffs

- **Automated:** the spec's Automated items that m2 owns, especially **Location strings** for every case in the design's table, plus app tests for the launcher states, the archive option and the restore fallback. Run `cargo test -p runner-app`, `cargo test -p runner-daemon` and `cargo test -p runner-core` for the restore option, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` (and `--features updater` on macOS), `cargo fmt --all -- --check`, `git diff --check`, and `python3 script/check-guide-translations.py`. App tests that start git, watchers or timers follow the no-race rule in `AGENTS.md`; the coder runs them 20 times before handoff and states the count.
- **Live checks (`qa`):** after a clean review, the spec's eight m2 checks with the spec's `/tmp` fixture under the `/tmp/1213-m2-` prefix, plus its selected regression cases, on macOS in the dev app (`make run`), including dark and light for the sidebar check. Windows is required by the spec; this machine has none, so `qa` records it as a gap and Jason decides on `needs-live-test`. Record results in `docs/tests/runs/1213-worktrees-mac.md`, appending to m1's run, and in the PR description.
- **Handoffs:** `coder` sends the unpushed candidate SHA to `reviewer` and iterates until the review says no must-fix issues remain; `reviewer` then hands the reviewed SHA to `qa`. A QA failure goes back to `coder` and through `reviewer` again before `qa` reruns the affected checks.
- **Long commands:** an Antigravity turn that ends while a command still runs can show as idle (#1336). The coder waits for each `cargo` run to finish before ending its turn and reports the result.

## Authorization

Jason authorized the **hydrogen trio** on 2026-11-17 for #1213 m2. Commit, folding this brief into the implementation commits, push `feat/1213-worktrees-app` with `git push --force-with-lease`, and open a PR against `main` labelled `risk-high`, `core-review` (the workflow adds it for the `runnerd` change), `reviewed` after the clean review, and `needs-live-test` while any required check, Windows included, has not passed. Drive CI green, then stop for Jason to review and merge.

## Finish mission (2026-11-18)

The hydrogen trio's mission ended before CI passed. The codex trio finishes PR [#1379](https://github.com/yicheng47/runner/pull/1379) in the same worktree, `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1213-worktrees-app`.

### State at start

- PR #1379 is open and not a draft. Its head `0ef33e83` is the reviewed candidate `39355090` plus the QA record commit, based on `be023414`; `origin/main` has moved 16 commits since then, including #1377 (a bounded per-session PTY writer across the session manager's spawn paths) and #1381. Labels: `risk-high`, `core-review`, `needs-live-test`, `reviewed`.
- macOS live QA passed every local check. The remote companion checks are **Blocked** because that run looked for a local relay at `localhost:5173` (see `docs/tests/runs/1213-worktrees-mac.md`); no local relay is needed. Jason has waived native Windows (m2-8): Windows CI covers it.
- [CI run 44066834407](https://github.com/yicheng47/runner/actions/runs/44066834407) failed every Rust job:
  - **All four platforms:** `runner-app surfaces::start_chat::launcher::tests::launcher_new_worktree_form_validation_hint_states_and_launch_requests` timed out on both tries at `start_chat/mod.rs:531` ("timed out waiting for chat launch attempted with worktree and failure shown on launcher"). The test passed locally on `be023414`, but CI tests the trial merge with `main`, so suspect the new `main` first.
  - **Windows:** `runner-app surfaces::sidebar::tests::sidebar_shows_worktree_branches_in_both_views_and_tracks_git_switch_and_missing_via_watcher` panics at `sidebar/tests.rs:3615` with OS error 32, because the file is in use by another process. The test removes or renames a path while a watcher or git process still holds it.
  - **Windows, FLAKY:** `runner-daemon files::tests::removing_watched_root_emits_changed_and_marks_root_missing` (`files/tests.rs:1459`) reported an extra `src/deep` in the changed set. The test already exists on `main`, but this PR changes `files/watch.rs`.

### Work

1. **coder:** rebase onto `origin/main`. Reproduce the launcher failure with `cargo nextest run --locked -p runner-app --profile ci --cargo-profile ci -E 'test(launcher_new_worktree_form)'`, which is a CI-specific failure, so nextest is allowed here. Fix the cause in the product code or in the test, whichever is at fault. Fix the Windows sidebar failure by reasoning from the code, since this machine has no Windows: wait for the holder to release the path, or let the test retry the removal, and never add a fixed sleep. For the flaky daemon test, use `main`'s Windows CI history to decide whether this PR's `watch.rs` change caused it. If it did, fix it here. If it did not, leave it for its own PR and say so in the PR description. Follow the no-race rule, and run every changed test 20 times and state the count. Fold the fixes into `39355090`'s implementation commit, keep the QA-record commit, remove `reviewed` before pushing a code change, push with `--force-with-lease`, and drive all four Rust jobs green.
2. **reviewer:** review the delta from the reviewed candidate (`git range-diff be023414..39355090 origin/main..<new head>` plus the fix diff), including any rebase conflict resolutions. Say which live checks the delta requires: test-only fixes need none, while a product change in the launcher, sidebar or watcher reruns the affected local m2 checks. A clean review re-adds `reviewed` with a PR comment naming the commit.
3. **qa:** run any local checks the clean review names, then the Blocked remote companion checks: the remote parts of m2-1/4/5, the host-derived chat and mission branch/base, quiet switch → missing → recreation, disconnect/reconnect, remote launcher exclusion, and `REMOTE-WIN-03`. Run them on the [same-Mac two-host relay fixture](../../tests/full-smoke-test.md#same-mac-two-host-relay-fixture-macos): two private-`HOME` candidate wrappers enrolled with the hosted relay through Chrome's existing signed-in Runner account, with only Client → Host paired using a disposable PIN. It needs no local relay and no second computer. Follow its enrollment and teardown-to-baseline rules exactly, never touch a pre-existing computer on the account, and say in the record that both ends ran the same build on one Mac. Append the retry to `docs/tests/runs/1213-worktrees-mac.md`, and remove `needs-live-test` only when every required check has passed.
4. **coder**, last: bring the PR description up to date (it still says "CI: Pending") with the new head, the CI result and the QA verdict.

### Authorization

Jason authorized the **codex trio** on 2026-11-18 to finish #1379. The crew may commit, fold this section into the implementation commit, rebase `feat/1213-worktrees-app` onto `origin/main`, push it with `git push --force-with-lease`, and edit the PR's description and labels as `AGENTS.md` describes. Do not merge: this is a `core-review` PR, so stop for Jason's review.
