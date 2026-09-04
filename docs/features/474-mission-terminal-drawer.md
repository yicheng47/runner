# Mission terminal drawer — a shell beneath the mission

Tracking issue: [#474](https://github.com/yicheng47/runner/issues/474). Status: planned. Priority P2.

## Motivation

[469](./469-terminal-drawer.md) gives a direct-chat tab a terminal drawer so a companion shell no longer consumes one of the tab's pane slots. The mission workspace still has no equivalent: running a mission-wide command means leaving the mission for a terminal-only tab or using another app, even though the Feed and every runner tab share one mission working directory.

The mission should have the same bottom-shell affordance, owned by the mission rather than by whichever runner tab happens to be selected. A build, test watcher, or quick repository command should remain available while the user moves between Feed and runner terminals, without becoming a crew slot or entering the mission's coordination traffic.

## Vocabulary

- **Mission drawer**: one bottom drawer owned by a mission and shown beneath the mission's central Feed / runner-tab surface. It spans the center column; the mission rail remains a separate surface on the right.
- **Mission drawer shell**: a `"shell"` direct session placed in that drawer. It is not a mission slot, runner, feed participant, sidebar session, or source of mission attention.

## Scope

### Placement and header

- A non-archived mission's primary workspace gains one drawer beneath the currently selected Feed or runner tab. Switching mission tabs changes the surface above it, not the drawer; the same active shell remains visible.
- The drawer occupies only the mission center column. The window sidebar and mission rail keep their current height and width behavior.
- The mission header's trailing surface controls gain a `panel-bottom` toggle before the mission-rail control. It uses the same hollow / filled icons, hidden / raised states, effective configured shortcut text, and Show / Hide wording as the chat-tab drawer from 469.
- The control is absent from archived missions and secondary duplicate mission workspaces. The primary workspace owns shell interaction and persisted state.

### Drawer behavior

- The visual and interaction contract follows 469: a 120–600 px drag-resized body with a 280 px default, a two-pixel strong line while resizing, a 32 px strip, terminal-icon shell chips, `+`, and a chevron that hides the drawer.
- Opening an empty drawer starts one shell. `+` and command-palette **New terminal** each add a shell and make it active. A chip switches the active terminal without stopping the others.
- The drawer renders the active shell through the existing `AttachedChat` / `TerminalElement` path. Selection, links, scrollback, IME input, copy / paste, and terminal resize behavior match a shell pane and the chat-tab drawer.
- A chip's `×` closes that shell through the existing foreground-process confirmation. Closing the active chip selects its left neighbor; closing the last chip hides the drawer. A shell that exits on its own keeps its chip and shows the existing **Shell exited** card with **Restart** and **Close**.
- Hiding the drawer does not stop its shells. The chevron, header toggle, configured `toggle-terminal-drawer` shortcut (default ⌥F12), and `⌘W` while a drawer terminal is focused all hide it.

### Shell ownership and cwd

- Mission drawer shells use the existing `"shell"` direct runtime and `session_start_shell`; they do not acquire a `mission_id` or `slot_id` and are never treated as crew sessions.
- A new shell starts with the mission's `project_id` and cwd resolved as mission cwd → project directory → `settings.default_working_dir` → `$HOME`. Switching Feed / runner tabs does not change the cwd of a running shell.
- Drawer shells stay out of mission routing, Runner messages, Feed events, runner status, attention, completion notifications, sidebar rows, direct-chat tab composition, pane navigation, split / fork destinations, and drag-drop.
- Mission **Stop** and **Resume** continue to act on crew slots only; they do not kill or respawn drawer shells. Header help text must describe runner slots rather than all PTYs once a drawer can exist.

### Persistence and lifecycle

- The mission node's layout data gains a serde-defaulted drawer object with `open`, `height`, `shells` in chip order, and `active`. An older mission node with no drawer data reads as hidden, 280 px, and empty.
- Any node/session reconciliation that wraps uncovered direct sessions into chat tabs treats ids placed in a mission drawer as covered. A mission drawer shell must never gain a second placement or appear as a terminal-only tab.
- Switching to another mission, chat, or settings surface preserves the drawer state and leaves its shells running. Returning to the mission restores open state, height, chip order, and the active chip.
- Normal app relaunch reattaches shells using the same launch-claim behavior as the chat-tab drawer and restores their mission placement. This feature does not broaden the separate direct-shell recovery policy after an abrupt app termination.
- Archiving or deleting a mission closes all of its drawer shells and removes the persisted drawer state. Closing an individual runner tab does not affect the mission drawer.

### Entry points

- The existing global `toggle-terminal-drawer` action dispatches to the active surface: a chat tab toggles its tab drawer and a primary, non-archived mission toggles its mission drawer. No second shortcut setting is added.
- Command-palette **New terminal** adds a shell to the mission drawer when a primary, non-archived mission is active. Its behavior in chat tabs and terminal-only tabs remains defined by 469.
- The sidebar **New terminal** action remains a terminal-only tab and never targets a mission drawer.

## Non-goals

- A separate drawer per runner, a drawer inside the mission rail, or a shell that follows the selected runner.
- Turning a drawer shell into a crew slot or exposing it to mission messages, events, status, or attention.
- Splits inside the drawer, chip drag-reorder, chip rename, shell profiles, or a terminal picker.
- Sharing one drawer across missions or moving a shell between a mission drawer and a chat tab.
- Making a secondary duplicate mission workspace interactive.
- Changing the general direct-shell recovery policy after an abrupt app termination.

## Design

Before implementation, add a `(474)` row to `design/runner.pen` with at least: mission Feed with the drawer closed, mission Feed with the drawer open, a runner tab with the same drawer preserved, the open / hidden header toggle states, the strip and chip anatomy, resize treatment, and the Shell exited state. Add `Spec — Mission terminal drawer (474) · v1` for the ownership, lifecycle, and exclusion rules.

## Implementation phases

1. **Design and state.** Add and review the `(474)` Pencil frames; define the serde-defaulted mission-node drawer layout; expose mission drawer accessors; make drawer shell ids covered by node/session reconciliation and excluded from direct-chat placement.
2. **Shell lifecycle.** Start shells at the mission cwd, attach one terminal bundle per drawer shell, close with foreground-process confirmation, restart exited shells in place, restore normal relaunch claims, and close all drawer shells on mission archive/delete.
3. **Mission workspace UI.** Turn the mission center body into the active Feed/runner surface above the resize handle and drawer; add the strip, chips, `+`, hide chevron, terminal body, exited state, height clamp, and durable resize completion.
4. **Entry points and exclusions.** Add the mission header toggle, route the existing drawer action and command-palette New terminal to the mission drawer, hide with `⌘W` from drawer focus, update mission Stop copy, and cover archived/secondary/sidebar/navigation exclusions.

Each implementation phase must keep `cargo test -p runner-app` green. Backend reconciliation or lifecycle changes also require `cargo test -p runner-backend` before the phase is handed off.

## Verification

- `cargo test -p runner-app`: old mission-node data defaults to a hidden empty drawer at 280 px; drawer state round-trips; the toggle is present on primary non-archived missions and absent on archived/secondary workspaces; Feed and runner-tab switches preserve one active drawer; height clamps at 120 and 600 px and persists when a drag ends outside the handle; default, rebound, and unbound shortcut text stays honest; `⌘W` from drawer focus hides without closing; closing the active and last chips follows the selection/hide rules; command-palette New terminal targets the mission drawer while sidebar New terminal remains a terminal-only tab.
- `cargo test -p runner-backend`: mission drawer shell ids count as covered and never receive a generated chat-tab placement; archiving/deleting a mission closes its drawer shells and removes placement state without changing crew-slot behavior.
- `make clippy` and `make fmt`.
- Manual smoke (human): open a running mission and use ⌥F12; confirm one shell starts at the mission cwd below Feed; run a long command, switch across Feed and several runner tabs, hide/reopen the drawer, and confirm the same shell stays live; add and switch chips; resize and revisit the mission; verify `⌘W` hides; verify foreground-process close confirmation and Shell exited → Restart / Close; confirm mission Stop does not kill drawer shells; confirm archive closes them; confirm no drawer shell appears in the sidebar, Feed, runner statuses, direct tabs, or mission attention.
