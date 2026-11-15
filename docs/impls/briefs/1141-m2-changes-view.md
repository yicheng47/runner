# #1141 m2 — the Changes view

## Starting point

- Issue: [#1141](https://github.com/yicheng47/runner/issues/1141), the git changes view. This mission is m2 of three pull requests to `main` (the spec's [Delivery](../../features/1141-git-changes.md#delivery)). m1, the `runnerd` changes service, merged in [#1314](https://github.com/yicheng47/runner/pull/1314).
- Branch: `feat/1141-m2-changes-view`, which has no upstream yet: push it with `git push -u origin feat/1141-m2-changes-view`, never to `main`.
- Worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1141-m2-changes-view`, with its own Cargo `target/`.
- Base: `origin/main` at `c2bf622f`. This branch's first commit carries the spec's 2026-11-15 update (All changes, Language, the carbon coder) and this brief.
- Design: `runner/desktop/specs/1141-git-changes.pen` in `yeelinks_design` at `3d9bd0d`, signed off by Jason on 2026-11-15. Boards `H5T5Lj` (screen `D4i2wZ`, All changes selected), `B38Ah` (keyboard, table `TU4G7`) and `o98BJ` (viewed files, screen `TjFT3`). `Xc2TS` and `EqrNQ` are m3.
- Parallel isolation: `.worktrees/feat-1165-agent-cli-version-gate`, `feat-1279-codex-usage-credits` and `feat-1295-antigravity-osc7501` exist. Stay inside this worktree; do not touch the root checkout or sibling worktrees.

## Problem and outcome

Reading what an agent changed still means leaving Runner. m2 ships the Changes surface of the side panel: a list of changed files beside the Unified diff of the selected file, with Commits above (All changes, Uncommitted changes, then each commit), viewed marks, live refresh and the keyboard. It also adds the one daemon piece m1 lacks, the `branch` target for All changes.

The spec [`docs/features/1141-git-changes.md`](../../features/1141-git-changes.md) wins on behaviour; where the boards and the spec differ, the spec wins.

## Read first

- The spec, all of Scope except [From Files to the diff](../../features/1141-git-changes.md#from-files-to-the-diff) and [Room for Split](../../features/1141-git-changes.md#room-for-split), plus Where it runs, Language and Verification.
- m1's service: `crates/runner-core/src/protocol/changes.rs` and `crates/runner-daemon/src/files/changes.rs` (with `changes_tests.rs`), and how `resolve_base` finds the merge base.
- #634's Files surface, which Changes mirrors and reuses: `crates/runner-app/src/surfaces/files/` (`panel_view.rs` and `panel_view/` for the tree, roots and watch; `code.rs` for the virtualized code view; `highlight.rs` and `syntax.rs`), `surfaces/side_panel.rs` for surfaces, `+` and permanent tabs, and `surfaces/settings/page/side_panel.rs` for the Surfaces card.
- `crates/runner-app/src/i18n.rs` and `crates/runner-app/i18n/{en,zh-hans}.toml`.

## Deliverables

1. **`branch` target in `runnerd`.** `ChangesTarget` gains the branch's committed changes from the merge base to `HEAD`; `changes_files`, `changes_diff` and `changes_set_viewed` accept it. Marks made on it count for "changed since viewed" as the spec's Viewed files says. The target is absent when there is no base or no commit since it. Tests per the spec's All changes and Viewed marks bullets under Automated. Update the spec's request table if what you build differs.
2. **The Changes surface**: its key `C`, the `+` menu, the Settings → Side panel Surfaces row (on by default), Files and Changes as two permanent tabs on a terminal-only tab, and the shared list/diff widths with Files.
3. **The list**: rows, summary, flat or tree (remembered), deleted files, path order; the Commits section with All changes first, its selection and opening rules, the 200-commit cap, and the comparison label.
4. **The Unified diff**: header, line numbers, gutter and backgrounds, `@@` rows, two-version highlighting off the UI thread, virtualized rendering, selection and copy, every edge-case placeholder, and live refresh keeping the selection.
5. **Viewed marks**: the row box, Space, the box at the end of the diff (no keycap), dimming, the summary count, changed since viewed in amber, and `changes/viewed` from other clients.
6. **The keyboard** as the spec's table, except the maximize row, which is m3.
7. **Language**: every Runner string in both catalogs; Chinese wording follows VS Code's Simplified Chinese language pack terms (`microsoft/vscode-loc`) where VS Code has the same string, and Runner's existing catalog terms where Runner already names something.
8. **Guide pages**: the Changes parts of the spec's Guide pages (the new section in Chats and windows, Missions, and the Changes keys in Keyboard shortcuts), not the Diff button or maximize.

Not in m2: Split, the side panel's maximize and the 360 px rule, Files' Diff button (all m3); line comments (#1293); remote sessions.

## Validation and handoffs

- **Automated**: the spec's Automated list for what m2 builds, including the app tests for flat and tree grouping and the keyboard's focus moves. Run `cargo test -p runner-core -p runner-daemon -p runner-app`, then `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` and `cargo fmt --all --check`; macOS also runs Clippy with `--features updater`. CI runs Windows and Linux.
- **Live checks**, run by `qa` on macOS after a clean review, in a fixture repository under `/tmp` with a branch worktree two commits ahead of `origin/main`: the spec's checks 1 to 5 and 8, with regression cases WS-FILES-05 and SET-PANEL-01. Also check the view in 简体中文. Checks 6 and 7 are m3. qa takes the shared lock with `script/qa-lock acquire --timeout 540` first. Windows and Linux live checks are not available on this machine: record them as not run, and keep `needs-live-test` on the PR.
- **Handoffs**: `coder` hands `reviewer` unpushed local commits by SHA and iterates until the review is clean; then hands `qa` the reviewed candidate commit. A qa failure goes back to `coder`, and the fix is reviewed again before qa reruns the affected checks. qa records results in `docs/tests/runs/1141-git-changes-mac.md`.

## Authorization

Jason approved the agy trio for m2 on 2026-11-15, with its coder on `gemini-4-carbon` at `high`. It may commit, push `feat/1141-m2-changes-view` and open a PR against `main` titled for m2 with "Part of #1141" (not "Closes"), labelled `core-review` (the new target touches the client protocol), `risk-medium` and `needs-live-test`, and drive CI green. Fold this brief and the spec update into the implementation commits. Stop at the open, reviewed, CI-green PR: Jason reviews `core-review` PRs before they merge.
