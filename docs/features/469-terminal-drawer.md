# Terminal drawer — a shell beneath the panes

Tracking issue: [#469](https://github.com/yicheng47/runner/issues/469). Status: designed 2026-09-03 in `design/runner.pen`, in implementation. Priority P2.

## Motivation

[64](./archive/64-native-terminal.md) made a terminal a pane: **New terminal** from an empty pane or the command palette fills or splits the focused pane, and the shell then competes with the chats for the tab's two or three cells. In practice the terminal is a companion to the chat beside it — a quick `git status`, a `pnpm dev` you glance at — and giving it a full pane slot spends the tab's scarcest resource on it, forces a layout decision before the first command, and leaves a pane in the split after the command is done. The pane-as-terminal shape was the smallest one at the time; it is not the right one now.

The shape that fits is the one editors converged on: **a terminal drawer that slides up from the bottom of the tab**, toggled with a key, sized by a drag handle, hidden when it is not needed. The chats keep the whole split; the shell appears beneath them at the focused chat's cwd and goes away without re-flowing anything. Codex's desktop app is the reference: a panel-bottom toggle beside the side-panel toggle, and a strip of terminal chips with a `+`.

The sidebar path is fine as it is: **New terminal** from the Recent / project `+` menus keeps creating a terminal-only tab with the terminal in its pane, and that row keeps its icon and **Close terminal** menu. This spec changes only what happens *inside* a chat tab.

## Vocabulary

The hierarchy `AGENTS.md` pins — **window → tab → pane** — does not change. Two words are added:

- **Drawer**: a tab-level surface docked under the tab's pane split, spanning its full width. A tab has at most one; it is open or hidden.
- **Drawer shell**: a `"shell"` direct session (the same runtime 64 introduced) that belongs to the tab, not to a pane. The drawer shows one drawer shell at a time and lists them as chips.

A drawer shell is never a pane, never a sidebar row, and never part of a tab's composed title.

## Behavior

### Header

The chat header's two clusters get a rule: **the left cluster acts on the session, the right cluster toggles a surface.**

- Left (`title_actions`, `crates/runner-app/src/surfaces/panes.rs:241`): icon · title · ⋯ · Stop · **⑂ Fork**. Fork moves here from the trailing group where [60](./60-fork-chat-to-pane-or-tab.md) placed it (`trailing_actions`, `panes.rs:315`); its enabled / disabled / pending states and the confirm dialog are unchanged.
- Right (`trailing_actions`): **split** (the layout picker, `panes.rs:271`) · **drawer** · **side panel** (`panes.rs:286`). Codex's order.

The drawer toggle is a lucide `panel-bottom` glyph in the same hollow / filled pair the sidebar and side panel already use (`assets.rs` `PANEL_LEFT_*` / `PANEL_RIGHT_*`). Hidden: hollow, ghost variant, tooltip "Show terminal drawer · ⌥F12". Open: filled, `ButtonVariant::Secondary` — the raised pill the layout picker gets while it is open (`panes.rs:273`) — tooltip "Hide terminal drawer · ⌥F12". The shortcut hint follows the configured binding and disappears when the action is unbound. The toggle renders on every chat tab, including a single-pane one; it does not render on a terminal-only tab (`focused_shell`, a single shell pane and nothing else) or in the mission workspace.

### The drawer

- **Placement.** The tab body becomes a vertical stack: the existing pane split (which keeps its own horizontal / vertical layout, presets, and dividers) above, then a one-pixel hairline, then the drawer. The split reflows to the remaining height, exactly as it does when the window resizes.
- **Height.** Default 280 px, clamped to 120–600 px, in the same logical pixels the pane dividers use. The hairline between the split and the strip is the drag handle: row-resize cursor on hover, a two-pixel `line_strong` bar while dragging. The height is remembered per tab.
- **Strip.** 32 px, hairline below, `[0, 8, 0, 10]` padding, 6 px gap: one **chip per drawer shell** (terminal glyph 12 · name · `×` 12), then `+`, a spacer, then a `chevron-down`. The active chip is raised (`raised` fill, radius 5); inactive chips are plain `text_mid` labels that raise on hover. A chip's name is the shell's basename (`zsh`, `fish`), the same default a shell pane gets.
- **Body.** The active drawer shell's terminal, rendered by the same `TerminalElement` bundle a pane uses (`AttachedChat`, `crates/runner-app/src/main.rs:113`: session, view, interaction, scrollbar, IME input, focus). Links (458), selection, scrollback, and the mouse work as in a pane. Switching chips swaps which bundle is shown; the others stay attached and keep scrolling.
- **Focus.** Clicking in the drawer focuses its terminal; `⌘W` while the drawer has focus hides it (not close, see below). ⌘1–9, pane-next / pane-previous, and `⌘D` / `⇧⌘D` keep acting on the pane split, never on the drawer.

### Opening it

- **⌥F12** toggles the drawer of the active tab. New keymap entry `toggle-terminal-drawer`, global scope, default `F12` + alt (`crates/runner-app/src/keymap.rs:97` is the entry shape). It appears in Settings → Keyboard shortcuts like every other entry.
- The **header toggle** does the same.
- The **command palette's New terminal** (`crates/runner-app/src/surfaces/command_palette.rs:114`) opens the drawer and adds a shell to it — the same thing `+` does — instead of filling or splitting a pane. On a terminal-only tab it keeps today's behavior.
- **First open spawns.** A drawer with no shells spawns one on open. The cwd follows the 64 rule: the focused pane's session cwd → the tab's project directory → `settings.default_working_dir` → `$HOME`. `ops::session::session_start_shell` (`crates/runner-backend/src/ops/session.rs:892`) already takes `project_id` and an explicit `cwd`; the app resolves the focused sibling and passes it.
- **The empty-pane placeholder drops New terminal.** It keeps one primary button with the configured shortcut before its label — **⌘N New chat** by default (`empty_pane_action_label`, `panes.rs`). The shortcut prefix follows a rebound action and disappears when `new-chat` is unbound. To make the hint honest, **⌘N fills a focused empty pane** with a new chat before it falls back to opening a new tab (today `new-chat`, `keymap.rs:98`, always opens a tab).

### Hiding, closing, exiting

- **Hide is not close.** The chevron, ⌥F12, the header toggle, and `⌘W` from the drawer hide it. Every drawer shell keeps running; the strip and chips come back exactly as they were when the drawer reopens.
- **A chip's `×` closes that shell** through `ops::session::session_close` (`ops/session.rs:518`), with the same foreground-process confirmation a shell pane's `×` has today (`request_close_terminal_pane`, `main.rs:213`); a shell at its prompt closes silently. Closing the active chip activates its left neighbour. Closing the last chip hides the drawer; the next open spawns a fresh shell.
- **A shell that exits on its own** keeps its chip; the body shows the **Shell exited** card (`SessionOverlay::shell_exited`, `crates/runner-app/src/ui/session_overlay.rs:71`) with exit code · shell · cwd. **Restart** respawns in place at the recorded cwd (a `"shell"` resume is a fresh spawn); **Close** drops the chip.
- **Closing or archiving the tab closes all of its drawer shells.** Archiving the last remaining chat also closes the drawer shells and removes the tab rather than leaving a drawer-only tab. Archive all on a tab that has drawer shells always confirms and names the count, the rule 64 set for pane terminals.
- **Attention.** Drawer shells never contribute to `working` / unread / needs-you and never stamp `record_session_completion` — already true for the `"shell"` runtime, and the drawer does not change it.

### Persistence and relaunch

- The tab's layout JSON (`PersistedLayout`, `crates/runner-app/src/pane_layout.rs:174`, written through `node_tab_upsert`, `crates/runner-backend/src/ops/node.rs:75`) gains a `drawer` object with serde defaults: `open`, `height`, `shells` (session ids in chip order), `active` (index). A tab written by an older build reads as "no drawer, hidden, default height".
- Drawer shells are **covered** sessions: whatever wraps an uncovered direct session into its own single-pane tab (`TabSet::from_rows`, `pane_layout.rs:487`, and the app-store node refresh at `main.rs:773`) must treat a session listed in any tab's `drawer.shells` as placed. They are excluded from the sidebar's rows, from the composed tab title, from `⌘1–9`, and from drag-drop.
- **Relaunch** follows pane shells: drawer shells come back live (a fresh shell at the recorded cwd, no command replay, no scrollback), the launch-claim path already exempts `claim.shell` from the resume-on-launch gate (`consume_launch_claims`, `crates/runner-app/src/bootstrap.rs:205`); the app re-attaches each respawned shell to its tab's drawer instead of a pane, restores chip order, the active chip, the open state, and the height. A missing cwd resolves to the nearest existing ancestor, then the project, then `$HOME`, with the substitution written into the terminal — the 64 rule.
- The window-layout checkpoint (`crates/runner-app/src/window_state.rs`) is unchanged; the drawer lives with the tab it belongs to.

### Migration

A split that already holds a shell pane keeps working until that pane is closed. Nothing moves automatically, and there is no "move to drawer" action.

## Out of scope

- A split inside the drawer, chip drag-reorder, per-chip rename, a terminal picker.
- A drawer in the mission workspace.
- Changing `⌘D` / `⇧⌘D`; they keep growing the chat split to its three-pane cap.
- Retiring terminal-as-a-pane for the sidebar-created terminal-only tab, or a "move to drawer" action for existing shell panes.
- Respawning only open drawers on relaunch (a process-count optimisation; revisit if it matters).
- Per-tab shell-path or profile settings.

## Decisions

- **Per tab, not per window.** In editors the terminal panel is per window, but a VS Code or Zed window is one project. A Runner window holds tabs across many projects, so one global drawer would show a shell in the wrong repo on every tab switch, and a "follow the focused tab's cwd" rule is a `cd` inside a live shell, which breaks a running `pnpm dev`. Per tab keeps the shell where the chat is, for free, through the cwd rule.
- **The drawer shell is a session, not a widget.** It is a `"shell"` direct session with a node placement, so stop / resume, the exited card, relaunch, Archive all, and the terminal element all come from 64 unchanged. Only its placement is new — the tab, not a pane.
- **Hide ≠ close, and the strip says which is which.** The chevron only hides; a chip's `×` kills. That is the `×`-is-destructive convention the identity line established, and it is why the strip's far-right control is a chevron rather than Codex's `×`.
- **Fork moves left.** Every glyph on the right opens or closes a surface; Fork acts on the focused session, so it belongs with Stop. This corrects 60's placement rather than adding a fourth trailing glyph.
- **The empty pane offers only New chat.** With a drawer in the tab there is no reason to spend a pane on a shell, so the placeholder stops offering it; the palette keeps **New terminal** and points it at the drawer.
- **⌘N fills a focused empty pane.** The hint under New chat would be a lie otherwise. When no empty pane is focused, ⌘N keeps opening a new tab.
- **Palette New terminal adds a shell.** "New terminal" with a drawer already open and populated should produce a new terminal, not just reveal the drawer; it is `+` with a keyboard.
- **280 px default, 120–600 clamp.** Enough for a prompt and a screen of output without halving a chat; the clamp keeps the split from collapsing on a small window.

## Design

Settled in `design/runner.pen`, row label "terminal drawer — a shell beneath the panes (469)":

- `Runner chat — terminal drawer closed (469)` — the hollow toggle in the trailing group, Fork beside Stop.
- `Runner chat — terminal drawer open (469)` — single pane above a 280 px drawer: strip (chip · `+` · chevron) over the terminal, toggle as a raised pill.
- `Runner chat — terminal drawer open · 2-pane split (469)` — the same under a two-pane split.
- `Runner chat — empty pane · New chat only (469)` — the placeholder without New terminal, `⌘N` pill under New chat.
- `Spec — Terminal drawer (469) · v1` — toggle states, the two header clusters, strip anatomy with callouts, the Shell exited card inside the drawer, and the decisions above.

## Implementation phases

1. **Model and persistence.** `PersistedLayout.drawer` with defaults; `PaneLayout` accessors and mutators (`drawer_open`, `drawer_height`, `drawer_shells`, `add_drawer_shell`, `remove_drawer_shell`, `activate_drawer_shell`); drawer shells count as covered wherever uncovered sessions get wrapped into tabs; excluded from sidebar rows, composed titles, `⌘1–9`, and drag-drop; Archive all and tab close include them.
2. **Sessions and relaunch.** Spawn through `session_start_shell` with the resolved cwd; close through `session_close`; on launch, re-attach respawned drawer shells to their tab's drawer with chip order, active chip, open state, and height restored.
3. **Drawer element and header.** The vertical stack in the tab body; the drag handle with the clamp; the strip with chips, `+`, chevron; `TerminalElement` reuse for the active shell; the Shell exited card with Restart / Close; the `panel-bottom` toggle in the trailing group; Fork moved to the title cluster.
4. **Entry points and keys.** `toggle-terminal-drawer` (⌥F12) in the keymap and Settings → Keyboard shortcuts; the palette's New terminal adds a shell to the drawer; the placeholder drops New terminal and prefixes New chat with its configured shortcut; `new-chat` fills a focused empty pane first; `⌘W` from the drawer hides it.

## Verification

- `cargo test -p runner-app`: `PersistedLayout` round-trips the drawer object and reads an older layout as hidden / default height / no shells; a session listed in `drawer.shells` is covered and never wrapped into a tab of its own, never appears in the sidebar rows or the composed title; the header's trailing group renders split · drawer · side panel and the title cluster ends with Fork; the drawer toggle is absent on a terminal-only tab; the placeholder renders **New chat** with its configured shortcut prefix and no New terminal; the palette still lists **New terminal**; the keymap lists `toggle-terminal-drawer` defaulting to ⌥F12; closing the active chip activates its neighbour and closing the last chip hides the drawer; Archive all on a tab with drawer shells always confirms with the live count; the height clamp holds at both ends.
- `cargo test -p runner-backend`: nothing new is expected — `session_start_shell` and `session_close` are reused as they are; add a test only if the cwd resolution for the focused sibling lands in the backend.
- `make clippy` and `make fmt`.
- Manual smoke (Jason, per the crews-never-run-the-dev-app rule): ⌥F12 in a chat tab opens a drawer with a shell at the chat's cwd; the header pill lights; drag the handle and the height survives a tab switch and a relaunch; `+` adds a second chip and chips switch the terminal; the chevron hides and the shells keep running (`sleep 100` in one, reopen, it is still running); a chip's `×` confirms while `sleep` runs and closes silently at a prompt; `exit` shows Shell exited → Restart brings a fresh shell in place; palette New terminal adds a chip; an empty pane shows only New chat with the configured shortcut prefix and the shortcut fills it; Fork sits beside Stop and still forks; Archive all on a mixed tab confirms and closes the drawer shells; quit with the drawer open, relaunch, and it is back with the same chips and height.
