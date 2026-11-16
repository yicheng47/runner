# Mission Brief: Fix Windows CI flakes on `main` (#1357)

**Worktree:** `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1357-windows-ci-flakes`
**Branch:** `fix/1357-windows-ci-flakes` (from `origin/main` at `0e4f9948`)
**Tracking issue:** [#1357](https://github.com/yicheng47/runner/issues/1357) (also covers latest `main` run `43838660457` failure)

## Authorization & Workflow

- Approved crew: `agy duo` (`coder` = `impl-agy` on `gemini-4-hydrogen`, `reviewer` = `reviewer` on `gpt-6-astra`).
- You are authorized to commit, push `fix/1357-windows-ci-flakes`, open a **non-draft** PR targeting `main` (`Closes #1357`), and drive CI green on macOS, Windows, and Linux. Do **not** merge the PR.
- **Commit hygiene:** Fold this brief commit into clean fix commit(s) before pushing/opening the PR so no standalone brief commit remains in PR history (`git rebase` / `--amend` + `--force-with-lease` on review/CI updates).
- **PR labels:** Apply `risk-low` + `no-behavior-change` if test-only (or `risk-medium` if product code also changes), and have `@reviewer` apply `reviewed` + post a comment naming the exact final head SHA. No live UI QA is required unless product UI/app behavior changes.

## Problem Statement

Two PRs already addressed the Linux `files_diff_toggle` flake (#1354) and proposed general CI retry config (#1355, open), leaving **three Windows `runner-app` CI failures** breaking `main`:

1. **`surfaces::changes::tests::removing_root_after_load_transitions_to_missing_when_shown_or_reopened`** (`crates/runner-app/src/surfaces/changes/tests.rs:1224`, #1357):
   - Failed in both `Rust / Windows` attempts of run `43813534315` (`c6ce6337`) with `timed out waiting for m-live missing screen` (15 s `h.wait` deadline).
   - After `std::fs::remove_dir_all(&wt_live)`, the test manually injects `FilesEvent::Changed` and waits for `CHANGES_MISSING`. Investigate why Windows either holds an open handle/delete-pending dir (e.g., `notify` `ReadDirectoryChangesW` watcher / in-flight `git`), fails `FilesRootOutcome::Missing` detection in `runner-app`/`runner-daemon`, or drops/overwrites the state transition with an in-flight refresh on `0e4f9948`.

2. **`surfaces::files::tests::a_selection_left_elsewhere_does_not_block_clicking_a_linked_image`** (`crates/runner-app/src/surfaces/files/tests.rs:2859`, #1357):
   - Failed on `Rust / Windows` in run `43813534315` attempt 2 (line `2890`): `page.selected_text()` was empty (`None` or `""`) immediately after `simulate_mouse_down` -> `simulate_mouse_move` -> `h.settle()` -> `simulate_mouse_up` -> `h.settle()`.

3. **`surfaces::files::tests::a_drag_selects_rendered_text_across_rows_and_copies_it`** (`crates/runner-app/src/surfaces/files/tests.rs:2589`, latest `main` `0e4f9948` CI run `43838660457`, job `133201544973`):
   - Failed on `Rust / Windows` (line `2618`) with `panicked at ... copied text` (`read_from_clipboard().and_then(|item| item.text())` was `None`) after the exact same `FILE_MD_TEXT` `simulate_mouse_down` / `simulate_mouse_move` / `simulate_mouse_up` drag + `simulate_keystrokes(&primary("c"))` sequence.

## Investigation & Fix Requirements

1. **Root-cause both failure classes honestly — do not mask with fixed sleeps or weakened assertions:**
   - **Windows markdown drag selection / clipboard:** Inspect `crates/runner-app/src/surfaces/files/markdown.rs` (`selectable`, `begin_selection`, `drag_to`, `TextLayout::index_for_position`, debug bounds vs `FILE_MD_TEXT {row}`) and GPUI's `VisualTestContext` / `TestPlatform` mouse/focus/clipboard behavior on Windows (`primary("c")`, whether `TestPlatform` shares OS clipboard across parallel tests vs test-local clipboard, whether `layout` glyph positioning / `80px` drag / frame settle timing misses `drag_to` or `copy`). Also audit sibling `FILE_MD_TEXT` drag/copy tests in `files/tests.rs`.
   - **Windows Changes missing-root:** Inspect `crates/runner-app/src/surfaces/changes/mod.rs` (`FilesRootOutcome::Missing`), `crates/runner-daemon/src/files/{watch.rs,changes.rs,root.rs}`, and `changes/tests.rs` lines 1217-1303 (and other `CHANGES_MISSING` tests). Check Windows directory-removal / watcher-handle / git error classification vs injected `FilesEvent::Changed` race with `watch_root`. Fix any real Windows product bug cleanly if missing-root classification or watch cleanup is broken, plus make the test deterministic against watcher timing (`h.wait` / clean state convergence, no live-source event injection races).
2. **Keep changes tightly scoped:** Do not touch unrelated files or merge PR #1355's workflow changes into this branch.
3. **Post-PR verification on real Windows CI:** Because we run locally on macOS, the real `Rust / Windows` GitHub Actions job is critical proof. Once reviewed and pushed, watch `gh pr checks <pr> --watch` closely (and rerun/stress-check if appropriate while keeping fixes clean and re-reviewed if modified).

## Validation & Handoff

- **@coder:** Run locally:
  - `cargo test --locked -p runner-app surfaces::changes::tests`
  - `cargo test --locked -p runner-app surfaces::files::tests` (loop touched tests 20+ times locally)
  - `cargo test --locked -p runner-daemon files` (if `runner-daemon` files change)
  - `cargo fmt --check` and `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` (plus `--features updater`)
  Hand off unpushed local commit(s) to `@reviewer` first per `agy duo` rules.
- **@reviewer:** Audit root-cause accuracy (confirm assertions were not weakened, no fixed `sleep`s or race-masking hacks, sibling `FILE_MD_TEXT` / `CHANGES_MISSING` tests checked, clean commit folding). Once clean and PR is open with all CI green on macOS/Windows/Linux, stamp `reviewed` with a PR comment naming the exact final head SHA.

**STOP condition:** Report ready to operator once non-draft PR is open, folded cleanly, `reviewed` is stamped on final head SHA, appropriate labels applied, and all CI checks (`Rust / macOS`, `Rust / Windows`, `Rust / Linux x86_64/aarch64`) pass.
