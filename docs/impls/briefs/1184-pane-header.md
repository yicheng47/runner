# #1184 — Pane header: chat menu at the title, stop beside the status, maximize toggle

Jason requested this mission with the claude trio on 2026-11-05. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1184-pane-header` on `feat/1184-pane-header`, based on `origin/main` at `6ee39a05836ff396ec6637e0166786470f589767`. The root checkout stays on main.

## Problem and outcome

Issue [#1184](https://github.com/yicheng47/runner/issues/1184). The pane header's `⋯` menu repeats the Stop/Resume button beside it, Rename hides in `⋯` or behind an unannounced double-click, the sidebar row menu lists the same chat operations in another order and with other labels, and split tabs cannot maximize a pane. The spec, [`docs/features/1184-pane-header.md`](../../features/1184-pane-header.md), is authoritative on behavior; the signed-off design is authoritative on layout and spacing.

The design is exported as PNGs in this worktree's ignored `target/design-ref/`: `pane-header-final.png` (the final header in six states), `maximize-pane.png`, `sidebar-menu.png` (today vs proposed; implement the Proposed column) and `today.png` (the current header, for reference only). Match them with the existing theme tokens; sample text is illustrative. Do not delete them, and do not open or edit `.pen` files.

## Read first

- `crates/runner-app/src/surfaces/panes/pane.rs`: the pane header (identity, title, rename input, `pane_lifecycle_control` placement, the `⋯` menu) and its width budget in the `container_query`.
- `crates/runner-app/src/surfaces/panes/mod.rs`: `pane_action_items_for`, `pane_action_indexes`, `pane_lifecycle_action`, `split_menu_items`, `top_right_pane`, `pane_close_behavior`.
- `crates/runner-app/src/surfaces/panes/tree.rs`: `pane_action_menu`, `activate_primary_pane_action`; `surfaces/chat/actions.rs`: `begin_pane_rename`.
- `crates/runner-app/src/ui/agent_status.rs`: `pane_lifecycle_control`, status labels.
- `crates/runner-app/src/surfaces/sidebar/menus.rs`: `tab_menu_entries`, `open_tab_menu`; `sidebar/elements.rs`: `sidebar_fork_menu_target`.
- `crates/runner-app/src/keymap.rs` for the new `toggle-maximize-pane` entry; `crates/runner-core/src/pane_layout.rs` only to read: maximize state must not be added to the persisted `PaneLayout`.

## Deliverables

1. **Pane header** per the spec: remove `⋯`; the title and its chevron open the chat menu (Rename… · Fork chat · Pin/Unpin; terminal panes Rename… · Pin) with today's capability gating, fork rules and secondary-pane disabling; double-click still renames. Status moves to the right with the lifecycle control right after it, its slot kept during Starting/Resuming; the narrow-pane truncation order from the spec.
2. **Maximize pane**: the toggle next to split in split tabs only, ⤢/⤡ with the selected background while maximized, `toggle-maximize-pane` defaulting to ⇧⌘↩ / Ctrl+Shift+Enter (another default only on a conflict, stated in the PR), transient per-tab app state, and the exit rules in the spec (navigation to another pane, split, close, single pane). Hidden panes keep running and keep their last PTY size.
3. **Sidebar row menu**: the order, labels and separator from the spec for chat, split-tab and terminal rows; actions unchanged.
4. **Guide** pages named in the spec, mirrored in `docs/guide/zh-CN/` and passing `python3 script/check-guide-translations.py`; i18n strings for new labels in `en.toml` and `zh-hans.toml`.

Non-goals are listed in the spec. Fold this brief and the spec into the implementation commit.

## Validation and handoffs

Before testing, list the changed behaviors and the check covering each. Automated: the spec's Verification list as unit tests, then `runner-app` tests, workspace Clippy with warnings denied, `cargo fmt --all --check`, `git diff --check` and the guide translation check, all with this worktree's own target directory. CI green on macOS, Windows and Linux.

The coder implements and hands off through Runner to the reviewer, who reviews the whole diff against the spec, the PNGs and this brief, must-fix findings first with file:line pointers, and checks in particular that maximize state stays out of `PaneLayout`, that the lifecycle control cannot shift as the status changes, and that capability gating for remote hosts is preserved. After a clean review, qa takes the reviewed candidate commit and runs the six live checks in the spec's Verification section in the development app on macOS, with screenshot evidence, and reports pass or fail per check. A failure returns to the coder; the fix is reviewed again before qa reruns the affected checks.

## Authorization

Jason approved the claude trio (coder, reviewer, qa). The crew may commit, push this branch and open one PR against main labelled `frontend-only`, a risk label picked from the diff, and `needs-live-test` until qa passes every live check. Stop at an open PR with green CI, reviewed and live-tested, and report the PR URL, final SHA, changes and validation through Runner.
