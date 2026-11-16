# #1345 — Changes: choose the checkout and the base

## Starting point

- Issue: [#1345](https://github.com/yicheng47/runner/issues/1345), milestone 0.19. Follow-up to #1141 (m1 #1314, m2 #1339, m3 #1343, all merged).
- Branch: `feat/1345-changes-checkout-and-base`, already pushed. Draft PR [#1348](https://github.com/yicheng47/runner/pull/1348) holds the spec commit `ac3f02b9`; finish the work on this PR rather than opening another.
- Worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1345-changes-checkout-and-base`, with its own Cargo `target/`. Base: `origin/main` at `69ba26d0`.
- Design: `runner/desktop/specs/1345-changes-checkout-and-base.pen` in `yeelinks_design` at `003d626`, signed off by Jason on 2026-11-16. Board `PxxiI`: default panel `InitT`, chosen base with picker open `dzu8A`, Base row states `VCgPS`, checkout picker open `ebb17`, viewing another checkout `KHUOM`, branch row states `VH7ml`; components `cmp/BasePicker` `Bqvfh` and `cmp/CheckoutPicker` `gFJbt`. Where the board and the spec differ, the spec wins.
- Other worktrees are active (`feat-1165`, `feat-1171`, `feat-1279`, `feat-1295`). Stay inside this worktree, run heavy workspace suites one at a time, and take `script/qa-lock` before any live QA.

## Problem and outcome

Changes shows the chat's own checkout against a guessed base. This adds a **Base row** with a branch picker (Auto by default, remembered per checkout and branch), turns the **branch row into a checkout picker** when the repository has several worktrees (sharing Files' remembered root), **removes the comparison label** from Changes' diff header, and fixes **Files and Changes stopping 8 px above the panel's bottom edge**. The spec [`docs/features/1345-changes-checkout-and-base.md`](../../features/1345-changes-checkout-and-base.md) is the contract.

## Read first

- The spec, all of it; then #1141's [Commits and what is compared](../../features/archive/1141-git-changes.md#commits-and-what-is-compared) and [Where it runs](../../features/archive/1141-git-changes.md#where-it-runs).
- `runnerd`: `crates/runner-daemon/src/files/changes.rs` (`resolve_base`, the `branch` target), `crates/runner-daemon/src/files/mod.rs` (`worktrees`, `set_root`).
- Protocol: `crates/runner-core/src/protocol/{changes.rs,files.rs,api.rs,tests.rs}`.
- App: `crates/runner-app/src/surfaces/changes/{mod.rs,model.rs,render.rs,tests.rs}`, Files' root picker `crates/runner-app/src/surfaces/files/{panel_view/roots.rs,picker.rs}`, `crates/runner-app/src/app_settings.rs`, the panel bodies in `crates/runner-app/src/surfaces/panes/header.rs` (`render_chat_side_panel`) and `crates/runner-app/src/surfaces/mission_workspace/rail.rs`, and `crates/runner-app/i18n/{en,zh-hans}.toml`.

## Deliverables

1. **`runnerd` and protocol:** an optional base ref on `changes_commits`, `changes_files` and `changes_diff` (branch refs only, verified; a miss resolves Auto and sets `base_missing`); `auto_base` and `base_missing` on `ChangesCommits`; the new `changes_branches(root)`; `uncommitted: Option<usize>` on `WorktreeEntry`. Every new parameter and field optional with serde defaults. Fixture-repo tests and protocol samples as the spec's Automated section lists.
2. **Base row and `BasePicker`:** the row, chip, picker (filter, Auto, local, per-remote sections, keys), the `changes_bases` setting keyed by checkout top and branch (checkout alone when detached), Auto removing the entry, and the missing-base notice.
3. **Checkout picker:** plain branch row with one checkout; kind icon, branch button and chevron with several; the `CheckoutPicker` rows (branch, short commit, uncommitted count, shortened path, session label from the app's sessions and missions); picking calls `files_set_root` like Files' picker; the tint and return button while viewing another checkout; the removed-worktree notice. Files' header picker rows gain the same count and session label.
4. **Diff header:** remove the comparison label from Changes only, with its now-unused i18n keys and tests; Files' Diff view keeps `changes.working_tree_vs_head` / `changes.working_tree_unborn`.
5. **Panel bottom:** keep the 8 px body padding for Info and the empty list only; Files and Changes fill to the bottom in the chat panel and the mission rail, maximized or not.
6. **Docs:** the guide pages the spec names, with their `zh-CN` mirrors; mark the spec's Status as implemented in this PR.

Out of scope: everything in the spec's Non-goals.

## Validation and handoffs

- **Automated:** `cargo test -p runner-daemon`, `cargo test -p runner-core`, `cargo test -p runner-app`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` (and `--features updater` on macOS), `cargo fmt --all -- --check`, `git diff --check`, `python3 script/check-guide-translations.py`.
- **Live checks (`qa`):** after a clean review, the spec's 8 live checks on macOS in the dev app (`make run`) with the spec's `/tmp` fixture, plus regression WS-FILES-05. Record them in `docs/tests/runs/1345-changes-checkout-and-base-mac.md` and in the PR description.
- **Handoffs:** `coder` sends the unpushed candidate SHA to `reviewer` and iterates until the review says no must-fix issues remain; `reviewer` then hands the reviewed SHA to `qa`. A QA failure goes back to `coder` and through `reviewer` again before `qa` reruns the affected checks.

## Authorization

Jason authorized the **carbon trio** on 2026-11-16 for #1345. Commit, folding this brief and the spec commit into the implementation commits, and push `feat/1345-changes-checkout-and-base` with `git push --force-with-lease`. Update draft PR #1348's description, mark it ready for review, and label it `core-review`, `risk-medium`, `reviewed` (after the clean review) and `needs-live-test` (until every selected live check passes). Drive CI green, then stop for Jason to review and merge.
