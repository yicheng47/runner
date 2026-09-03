# 469 — Terminal drawer: a shell beneath the panes

Tracking issue: [#469](https://github.com/yicheng47/runner/issues/469). Feature, P2. Spec: [`docs/features/469-terminal-drawer.md`](../features/469-terminal-drawer.md) — the spec wins on any detail this plan leaves out. Design: `design/runner.pen`, the `(469)` frames and `Spec — Terminal drawer (469) · v1`.

## What ships

Inside a chat tab the terminal stops being a pane and becomes a **drawer** docked under the pane split: toggled by ⌘J or a `panel-bottom` glyph in the header's trailing group, resized by dragging the hairline above it, holding one or more `"shell"` direct sessions shown as chips (icon · name · `×`), with `+` to add one and a chevron to hide. Hiding keeps shells running; a chip's `×` kills that shell. The header's trailing group becomes surface toggles only (split · drawer · side panel) and Fork moves into the title cluster beside Stop. The empty-pane placeholder drops **New terminal**, keeps **New chat** with a `⌘N` pill, and ⌘N fills a focused empty pane before it opens a new tab. The sidebar's **New terminal** (terminal-only tab, terminal in the pane) is untouched.

## Where it touches

- `crates/runner-app/src/pane_layout.rs` — `PersistedLayout` (`:174`) gains `drawer: { open, height, shells: Vec<String>, active }` with serde defaults; `PaneLayout` gains accessors / mutators; `TabSet::from_rows` (`:487`) and the app-store node refresh (`main.rs:773`) must treat a session listed in any tab's `drawer.shells` as placed, so it is never wrapped into a tab of its own.
- `crates/runner-app/src/surfaces/panes.rs` — the tab body becomes a vertical stack (split · handle · drawer); `title_actions` (`:241`) gains Fork, `trailing_actions` (`:315`) becomes split · drawer · side panel; `empty_pane_action_labels` (`:1908`) drops New terminal and the placeholder gains the `⌘N` pill; the drawer element itself (strip, chips, handle, body) lives here or in a sibling module.
- `crates/runner-app/src/main.rs` — `AttachedChat` (`:113`) bundles are created per drawer shell exactly as per pane; `request_close_terminal_pane` (`:213`) confirmation reused for a chip's `×`; the `new-chat` action fills a focused empty pane first.
- `crates/runner-app/src/keymap.rs` — `toggle-terminal-drawer`, global, default `KeyJ` + meta (`:97` is the entry shape; the binding table at `:849`); `⌘W` from a focused drawer hides it.
- `crates/runner-app/src/surfaces/command_palette.rs` — `PaletteDestination::NewTerminal` (`:114`, dispatched at `:347`) opens the drawer and adds a shell on a chat tab; unchanged on a terminal-only tab.
- `crates/runner-app/src/assets.rs` — `panel-bottom-hollow.svg` / `panel-bottom-filled.svg` in the style of the existing `PANEL_RIGHT_*` pair.
- `crates/runner-app/src/bootstrap.rs` — `consume_launch_claims` (`:205`) already exempts `claim.shell`; the app side re-attaches a respawned drawer shell to its tab's drawer (chip order, active chip, open state, height) instead of a pane.
- `crates/runner-backend/src/ops/session.rs` — `session_start_shell` (`:892`, takes `project_id` and an explicit `cwd`) and `session_close` (`:518`) reused as they are; the focused-sibling cwd is resolved in the app before the call.
- `crates/runner-app/src/ui/session_overlay.rs` — `SessionOverlay::shell_exited` (`:71`) reused for the drawer body's exited state.
- `docs/features/README.md` index line and `docs/features/archive/64-native-terminal.md` are not edited by the crew; the landing commit handles docs.

## Plan

1. **Model and persistence** — `PersistedLayout.drawer`, `PaneLayout` API, coverage in the reconciler, exclusion from sidebar rows / composed title / `⌘1–9` / drag-drop, inclusion in Archive all and tab close. Tests first: round-trip, old-layout default, coverage.
2. **Sessions and relaunch** — spawn on first open with the cwd rule (focused pane's session cwd → project dir → `default_working_dir` → `$HOME`), close via `session_close`, re-attach on launch with restored chip order / active / open / height.
3. **Drawer element and header** — vertical stack, drag handle (120–600 px clamp, default 280, row-resize cursor, two-pixel `line_strong` bar while dragging), 32 px strip with chips / `+` / chevron, `TerminalElement` reuse, Shell exited card with Restart / Close, `panel-bottom` toggle (hollow ghost hidden, filled `Secondary` open, tooltips "Show / Hide terminal drawer · ⌘J"), Fork moved left. No toggle on a terminal-only tab or in the mission workspace.
4. **Entry points and keys** — ⌘J keymap entry (shows in Settings → Keyboard shortcuts), palette New terminal → add a shell, placeholder New chat + `⌘N` pill, ⌘N fills a focused empty pane, `⌘W` from the drawer hides.

Land phases in order; each phase should leave `cargo test -p runner-app` green so review can happen per phase if the reviewer prefers.

## Rules of the road

- Do not launch the Runner app (`make run`) — the human smoke-tests. Verify with tests, `make clippy`, `make fmt`.
- No new abstractions beyond what the drawer needs; follow the pane code's patterns (`AttachedChat`, `SessionOverlay`, `IconButton`, `TerminalElement`).
- Keep the `"shell"` runtime out of the catalog and out of attention, as 64 decided.
- A session id in `drawer.shells` must never appear in the sidebar, the composed tab title, or the fork / split / pane-navigation paths.

## Non-goals

A split inside the drawer, chip reorder or rename, a drawer in the mission workspace, changing `⌘D` / `⇧⌘D`, retiring terminal-as-a-pane for terminal-only tabs, a "move to drawer" action, respawn-only-open-drawers, shell profiles.

## Verification

See the spec's Verification section for the test list; the crew runs `cargo test -p runner-app`, `cargo test -p runner-backend`, `make clippy`, `make fmt`, and reports which it ran. Manual smoke is the human's.
