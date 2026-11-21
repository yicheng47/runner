# Brief: #1403 — Right Side Panel Redesign (Option O + S2 + C1)

## Starting point

- **Issues:** [#1403](https://github.com/yicheng47/runner/issues/1403) (folds [#1331](https://github.com/yicheng47/runner/issues/1331) and [#1360](https://github.com/yicheng47/runner/issues/1360)).
- **Branch / worktree:** `feat/1403-right-side-panel` at `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1403-right-side-panel`.
- **Base commit:** `8012ecb7dea011bf99811fd7450500afeee5a45c` (`origin/main`).
- **Parallel isolation:** Sibling worktrees exist at `.worktrees/feat-1213-worktrees-app`, `.worktrees/feat-1332-session-runtime-rows` (PR #1411), and `.worktrees/feat-1374-appearance-theme-controls` (PR #1410). Stay strictly inside this worktree, never assume unmerged peer branches landed, and never share `CARGO_TARGET_DIR` (if `coder` builds while `qa` holds a candidate build, use `CARGO_TARGET_DIR=target/coder` inside this worktree).

## Problem and outcome

Today's right side panel mixes lightweight session **Info** into the same tab/`+`-menu container as heavy workspace tools (**Files**, **Changes**), opens new chats to an empty "Open a surface" view, mutates a global open flag across tabs, opens **Files** immediately into a wide blank viewer, and repeats `cmd`/`cwd`/`branch` in worktree Info.

Implement the signed-off **Option O + S2 + C1** redesign:
- **Authoritative spec:** `docs/features/1403-right-side-panel.md` on this branch (wins on behavioral detail).
- **Signed-off design:** `~/repos/yeelinks/yeelinks_design/runner/desktop/specs/1403-right-side-panel.pen` (commit `646a913`): boards `t4Our` (Sign-off — Option O + S2), `i7bs0w` (Files & Changes deep-dive, C1), and reference `o0nOb` (split S1/S2/S3 comparison).

## Read first

1. `docs/features/1403-right-side-panel.md` (full contract, states, and live checks).
2. `crates/runner-app/src/surfaces/side_panel.rs` (surface model, `header_tabs`, `+` menu, `widened(...)`, `SavedPanel`).
3. `crates/runner-app/src/surfaces/panes/header.rs`, `crates/runner-app/src/surfaces/panes/panel.rs`, `crates/runner-app/src/surfaces/panes/pane.rs`, and `crates/runner-app/src/surfaces/panes/tree.rs` (`pane_title_menu`, chat panel layout, `chat_panel_content`, `info_branch_row_from_checkout`).
4. `crates/runner-app/src/surfaces/files/panel_view.rs`, `crates/runner-app/src/surfaces/files/open_file/render.rs`, and `crates/runner-app/src/surfaces/changes/render.rs` (tree-only default vs widened viewer, Changes C1 list-only vs diff viewer).
5. `crates/runner-app/src/surfaces/mission_workspace/view.rs` and `crates/runner-app/src/surfaces/mission_workspace/rail.rs` (mission header, rail, `render_mission_meta_panel`).
6. `crates/runner-app/src/surfaces/settings/page/side_panel.rs`, `crates/runner-app/src/app_settings.rs`, and `crates/runner-app/src/keymap.rs` (Side panel settings, `toggle-session-info` shortcut).

## Deliverables and non-goals

1. **Orca 40 px top icon activity bar:** Replace surface tabs, `+` menu, and empty "Open a surface" state with the fixed icon switcher (`Files`, `Changes` with dirty dot; `Sessions` first on missions) with active raised tile + 2 px bottom accent bar, plus `Maximize` (only when widened/maximized) and `Collapse`.
2. **Default 300 px Project Tree & C1 Changes progressive widening:** Wide chats/terminals default open on `Files` (300 px Project Tree only with `All files | Changed (N)` toggle and `Find files ⇧⌘O`, zero blank file viewer); missions default open on `Sessions` (300 px). Clicking `Changes` stays compact at 300 px (full `#1345` Checkout/Base/Commits/Files rail). Clicking a file or changed file widens smoothly to ~660–740 px; closing the last file tab or clicking `×` on the diff header shrinks back to 300 px.
3. **S2 Title `▾` Info popover (`⌘I` / `Ctrl+Shift+I`) & `#1360` de-duplication:** Remove `Info` from the right panel and add zero `(i)` header icons. Embed the compact `#1360` Info card (`worktree` replacing redundant `cwd`/`branch`, `cwd` only outside worktree or relative below worktree root, default `cmd` hidden, collapsible scrollable `System prompt`, `session_key` copy) at the top of chat `pane_title_menu` (single, split, archived, remote) and Mission header title menu. Wire `toggle-session-info` in `keymap.rs`.
4. **Per-tab isolation, split "Following" cue, & narrow/split overlay sheet (#1331):** Opening/collapsing a tab never mutates future tabs' default. Narrow windows (`available - 300 < 420`) start collapsed and open as an elevated overlay sheet with scrim; widening a file/diff in a split auto-sheets if any split column would drop below `340 px`, snapping back to 300 px tree on close. Show `Following <pane> · <branch/folder>` in the rail header when split panes differ.
5. **Settings, i18n, and Docs:** Simplify `Settings → Side panel` (remove `Info` toggle, keep legacy settings JSON backward-compatible), update EN + zh-CN i18n, and update EN + zh-CN guides (`chats-and-windows.md`, `missions.md`, `sessions.md`, `keyboard-shortcuts.md`; verify with `python3 script/check-guide-translations.py`).

**Non-goals:** Embedded browser implementation (#1260 — do not add a dead placeholder button in shipped UI), full-text content search (`Names | Contents`), daemon protocol/database migrations (keep `frontend-only`), or modifying sibling worktrees.

## Validation and handoffs

- **Automated (`@coder` before review):** Cover activity-bar switching, 300 px ↔ widened transitions for Files and Changes (C1), per-tab open isolation, narrow/split overlay sheet geometry (`420 px` / `340 px`), S2 Title `▾` Info popover (role collapsed/expanded prompt, runtime, remote, split per-pane, mission) + `#1360` row rules, `⌘I` shortcut, and Settings backward compatibility. Loop any new async/daemon-backed app tests 20x via `cargo nextest run -p runner-app --stress-count 20 -E '<filter>'` and report the count. Run `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, `python3 script/check-guide-translations.py`, and `git diff --check`.
- **Review (`@reviewer`):** Reviews frozen commits against `docs/features/1403-right-side-panel.md` and `1403-right-side-panel.pen` (`t4Our`, `i7bs0w`), verifies test determinism, confirms the 5 live QA checks, and applies `reviewed` on the final PR head.
- **Live QA (`@qa` after clean review, macOS dev app):** Acquire `script/qa-lock acquire --timeout 540` (preserve any pre-existing user dev sessions), run all 5 live checks in `docs/features/1403-right-side-panel.md#live-checks-macos-development-app-qa-with-scriptqa-lock` (including the same-Mac two-host relay check for remote chat S2 Info + Files unavailable), record evidence in `docs/tests/runs/1403-right-side-panel-mac.md` and the PR description, and release `script/qa-lock`. Keep `needs-live-test` if any required check fails/blocks or for untested Windows/Linux native checks per `AGENTS.md`.

## Authorization

Approved crew: `neon trio` (`coder`, `reviewer`, `qa`). Authorized to commit, fold this brief and `docs/features/1403-right-side-panel.md` into clean implementation commit(s) before final push, push `feat/1403-right-side-panel`, open a PR against `main` (`Fixes #1403`, `Fixes #1331`, `Fixes #1360`, `frontend-only`, accurate `risk-*`), and drive CI green. Stop at the open, reviewed, QA-verified, CI-green PR; do not merge.
