# #1283 — Focus the surviving pane after closing a split

## Starting point

- Issue: https://github.com/yicheng47/runner/issues/1283
- Branch: `fix/1283-focus-surviving-pane`
- Worktree: `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1283-focus-surviving-pane`
- Base: `origin/main` at `16fdc7145321a7c5da0bf10830cb0cb5074c21f2`.
- Crew: codex trio (`coder`, `reviewer`, `qa`).

## Problem and outcome

Jason reports that closing a focused pane in a split tab leaves the surviving pane visible without keyboard focus. Typing should continue in the survivor immediately, without another click. The report does not establish the close entry point, pane content, operating system, or local/remote context; do not present a suspected cause as confirmed.

`PaneLayout::close_pane` already selects a surviving focused pane ID. The app's local `NativeRoot::close_pane` reloads/attaches and calls `focus_active_terminal`, but its remote path returns after saving. Terminal closure and chat archive have asynchronous completion paths. Investigate actual GPUI focus and completion ordering rather than assuming the model selection is wrong.

## Read first

- `AGENTS.md`, especially Test Scope and Crew Missions.
- `crates/runner-core/src/pane_layout.rs`: `PaneLayout::close_pane`.
- `crates/runner-app/src/surfaces/chat/close.rs`: pane closure and terminal completion.
- `crates/runner-app/src/surfaces/chat/attachment.rs`: `focus_active_terminal`.
- `crates/runner-app/src/surfaces/sidebar/archive.rs`: archive completion and pane collapse.
- `crates/runner-app/src/surfaces/front_page/actions.rs`: layout persistence and workspace completion.
- `docs/tests/regression/workspace.md`, `docs/tests/full-smoke-test.md` for the selected checks' isolation and evidence rules.

## Deliverables and boundaries

1. Identify the failing close path and fix focus restoration with the smallest change consistent with existing focus conventions. Handle the actual async completion point where applicable.
2. Closing the focused pane gives its surviving pane keyboard focus. Closing an unfocused sibling preserves the current pane's focus. A delayed completion must respect a user who has switched to another tab or pane while the close was pending.
3. Preserve current close/archive confirmation and layout semantics, including whether a chat archive leaves an empty launcher or collapses a pane. Use the established focus target for empty, stopped, local terminal and remote panes.
4. Add a meaningful regression for the failing focus/completion behavior using existing test facilities; a layout-ID-only assertion does not prove native keyboard focus. Cover the asynchronous close/archive path when it is involved.
5. Update `docs/guide/guides/chats-and-windows.md` and its Chinese counterpart with the resulting focus behavior. Update remote-access guidance only if the remote user contract changes.
6. Record the selected native checks and their evidence in `docs/tests/runs/2026-11-12-mac.md`, or the actual run date's existing record. Identify candidate SHA and artifacts, and distinguish passes from platform/fixture gaps.

This restores existing keyboard behavior and needs no new visual design. Do not redesign pane controls or expand into a focus-system refactor.

## Validation and handoffs

Before running checks, list changed behaviors and the checks that cover them. `coder` runs the relevant `runner-app` tests, workspace clippy and formatting checks; run relevant `runner-core` tests if model code changes. `reviewer` reviews the frozen candidate and names any additional live checks the diff requires. Only `qa` launches the development app, after clean review, using the shared QA lock and computer use.

Selected native checks on macOS:

- **FOCUS-01 — Focused shell close:** In an owned two-shell split, close the focused pane with its header ×, then immediately type a unique harmless marker into the survivor without clicking it. Repeat through the close-pane shortcut, in both pane positions. This exercises asynchronous terminal close and actual input delivery. These are the relevant split-close variants of `WS-SHELL-02`, not a claim to its whole lifecycle matrix.
- **FOCUS-02 — Unfocused sibling:** Close an unfocused sibling using its × while the other owned pane is the active input target. Immediate typing still reaches the original pane. This detects incorrect focus reassignment.
- **FOCUS-03 — Empty and chat close:** Close a focused empty split launcher with × and the shortcut, and close/archive an owned chat pane through its normal confirmation/completion path. Verify focus reaches the surviving input target after completion and cancellation retains the original usable pane. These select the split-close part of `WS-CREATE-18` and archive-confirmation part of `WS-PANE-05`; they do not require unrelated launcher, lifecycle, styling or restore checks.
- **FOCUS-04 — Async navigation:** Where completion is asynchronous, exercise or meaningfully simulate switching to another owned tab while closure is pending; completion must not steal focus back. Use an in-process regression if a safe native timing fixture cannot make this deterministic, and disclose the native gap.

Inspect local and remote completion paths. If the fix changes remote behavior or the reproduced bug is remote-specific, add the corresponding focused/unfocused remote split-close checks using an existing authorized safe fixture; missing fixture access is a disclosed required-check gap, not a pass. Do not alter accounts or operate unrelated sessions to obtain coverage. Windows/Linux native checks are not an automatic full-platform matrix for this macOS mission; disclose what was actually tested.

QA uses owned disposable shells and, only where needed for chat-specific behavior, bounded cheap agent chats in fresh `/tmp` folders. Preserve existing development sessions. Retain screenshots and marker/input evidence; a focus border or CLI state alone does not prove typing works. Recheck only affected cases after fixes.

## Authorization

Jason requested the codex trio to fix #1283. This mission may commit, push and open a pull request, including its initial brief commit; finish at an open PR under the repository's Crew Missions rules.
