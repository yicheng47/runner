# Switch sidebar view

Tracking issue: [#1266](https://github.com/yicheng47/runner/issues/1266).

## Motivation

Switching between Projects and Active currently requires the list-filter button beside Search. A rebindable keyboard shortcut makes frequent switching accessible from the keyboard.

## Scope

- Add `switch-sidebar-view`, titled **Switch sidebar view**, with ⌘. on macOS and Ctrl+. on Windows/Linux. Use the existing Keyboard shortcuts row and command palette patterns, with English and Simplified Chinese strings. The issue is `agent-ready`: no new layout or visual component needs drawing.
- Switch Projects ↔ Active exactly as the list-filter button does. Share the existing behavior, including clearing sidebar drag state, the remote computer's `toggle_active_only` path, local persistence of `sidebar_active_only`, and `sync_sidebar_shortcut_rows`.
- When collapsed, expand the sidebar and show the newly selected view. Keep the existing list-filter button's behavior intact.
- On Windows/Linux, respect **Ctrl keys in terminals**: Runner first handles Ctrl+.; Terminal first passes it to a focused terminal and shows **Outside terminals** for the shortcut. Rebinding, unbinding and restoring defaults follow the existing keymap behavior.
- Ctrl+. terminal delivery preserves the modifier as `CSI 46;5u` when the terminal program enables Kitty disambiguation (`CSI >1u`). Legacy mode has no distinct C0 code for Ctrl+Period and sends a literal period. Test actual emitted input, including return to legacy mode after the program pops its keyboard flags.
- Keep ⌘1–9 / Ctrl+1–9 tied to the visible view; do not change their selection rules.
- Add the Navigation and creation row in `docs/guide/reference/keyboard-shortcuts.md` and mention the shortcut in the Projects/Active paragraph of `docs/guide/guides/chats-and-windows.md`. Keep the corresponding `docs/guide/zh-CN/` pages consistent with the existing translation conventions. No CLI reference generation is needed.

The issue chooses ⌘. over ⇧⌘S for speed and accepts macOS's cancel convention. Stopping a session remains on ⇧⌘X. Do not broaden the keymap or redesign the sidebar.

## Implementation Phases

1. Register the action, configurable keymap entry, action handler and palette metadata using existing patterns; route the button and shortcut through shared view-switch behavior where appropriate.
2. Add translations and guide changes; extend focused tests for key dispatch, effective bindings, terminal priority, collapsed-sidebar expansion and local/remote state updates.
3. Review the frozen candidate, run the selected native checks, record evidence, and open the pull request with its actual validation status.

## Verification

Before running checks, list the changed behaviors and why each selected check covers them. Run `runner-app` tests, workspace Clippy and formatting checks under the repository's required native-UI gates; CI must pass its platform checks.

| Check | Reason |
|---|---|
| Focused keymap/action/settings tests: default Period binding on each platform, configurable/unbound bindings, command palette registration, terminal-priority dispatch and Outside terminals tag | The new action must participate in all existing shortcut mechanisms. |
| Focused local and remote sidebar tests, including visible shortcut rows after a switch | The shortcut must preserve the button's persistence and per-computer behavior without stale number mappings. |
| Native: press the shortcut in each view with the sidebar open, then with it collapsed; compare the button and verify local preference survives reopening | Covers visible switching, expansion and persistence. Select the switching/persistence portion of WS-ACTIVE-04 only. |
| Native: use owned rows with distinguishable ordering and press number shortcuts after switching views | Verifies row synchronization, the affected portion of WS-ACTIVE-02; the full attention-state matrix is outside this change. |
| Native: find the Settings row, rebind the shortcut, prove the new key works and the default no longer switches, restore the original binding, then execute Switch sidebar view from the command palette | Covers discoverability, rebinding and the palette's action path. |
| Windows native: with an owned raw-input terminal that enables Kitty disambiguation, verify Ctrl+. switches without terminal input under Runner first, emits `CSI 46;5u` without switching under Terminal first, still switches outside terminals, and shows Outside terminals; pop the keyboard flags and verify legacy period output | Covers the new bare-Ctrl binding; use only the relevant routing portion of TERM-INPUT-06. |

QA owns computer use, the development-app lock and `docs/tests/1266-switch-sidebar-view.md`, including candidate identity, screenshots, results and restored settings. Check prerequisites early. A missing Windows host or other required prerequisite is a named blocked check, never a pass; preserve `needs-live-test` while any required native check remains unpassed. The remote path requires in-process coverage; a new remote setup or full regression run is outside this task.
