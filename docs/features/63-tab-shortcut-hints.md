# ⌘1–9 tab switching with hold-to-reveal shortcut pills

Status: designed 2026-08-23 (`design/runner.pen` `cmp/TabShortcutPill` `gpQTy`, frame `BOqhO`), in implementation as M6.15. Program slot: M6.15 in [m6-remainder.md](../impls/gpui-rewrite/m6-remainder.md).

## Motivation

Switching between open tabs is a click in the sidebar today; `⌘[` / `⌘]` only move between panes and between a mission's Feed and slot tabs. Browsers settled this long ago: `⌘1`…`⌘9` jump to the Nth tab. Arc adds the part that makes it learnable — hold `⌘` and each tab shows its number — so nobody has to memorize positions. Runner wants both: the jump, and the pills that appear only while `⌘` is held.

## Behavior

- **What counts as a tab**: the sidebar's tab rows — the unit the sidebar highlights (`AGENTS.md` surface hierarchy: window → tab → pane). `1`…`9` are assigned top to bottom to the rows **currently visible** in the sidebar, across PINNED and RECENT (the section labelled CHATS & MISSIONS until M6.18) in display order, skipping rows hidden inside a collapsed project. Mission rows count too — they are rows the user switches to the same way. A mission's own Feed/slot tabs are not part of this (they keep `⌘[` / `⌘]`).
- **Jump**: `⌘N` activates the Nth visible row exactly as a click would (route to Chat and activate the tab; route to the mission for a mission row). No row at N → no-op. Ten or more rows: 1–9 only; no `⌘0`.
- **Reveal**: while `⌘` is held with no other key, each numbered row shows a small pill at its right edge with its digit. The pills appear after a short hold (~150 ms) so ordinary chords (`⌘C`, `⌘N`, `⌘K`) never flash them, and disappear the moment `⌘` is released or another modifier joins. They render in place of the row's hover actions (kebab, watermark) for the duration — the pill is the only thing at that edge while `⌘` is down.
- **Scope**: per window, whichever window has focus. Collapsed sidebar: the jump still works, nothing is shown. Text fields and terminals: `⌘`+digit is an app-level chord (macOS never delivers it to the PTY), so the binding is global; the keymap registry entries are `select-tab-1` … `select-tab-9` (`cmd-1` … `cmd-9`), user-rebindable like the rest.

## Design (settled on the canvas 2026-08-23)

- A `cmp/TabShortcutPill` component: mono digit, ~16 × 16, `$raised` fill, `$text-mid` text, radius 4 — sized to sit in the row's trailing 24 px slot where the kebab lives.
- A "Sidebar — ⌘ held" state frame: `cmp/SidebarC` with pills on the first nine visible rows, kebabs hidden, to settle the pill's colour against selected / unread / busy rows.
- Decided: the selected row's pill stays neutral (`$raised` chip, digit in `$text-hi`); no accent inversion — accent stays reserved for attention and busy. The pill replaces the trailing-slot content (pin, unread dot, status glyph, kebab) while ⌘ is held; numbering runs PINNED → expanded projects' children → RECENT.

## Implementation notes (for the brief)

- Modifier tracking: `window.on_modifiers_changed` is already used for ⌘-hover links in `terminal/element.rs:648`; the sidebar entity registers the same way, keeps a `cmd_held_since: Option<Instant>` and a deferred 150 ms timer that flips a `show_shortcut_pills` flag, clearing it on any modifier change that is not "⌘ only" and on window blur. Notify only the sidebar entity.
- Numbering comes from the same visible-row walk `render_sidebar_contents` already does; compute once per render into a `tab_index_by_row` map so the pills and the `select-tab-N` actions agree by construction.
- Actions: nine `gpui::actions!` entries dispatched at the root, handled by the shell (route + `TabSet` activation — `pane_layout.rs:398`), mirrored in the Shortcuts pane listing.

## Non-goals

- Reordering tabs with `⌘⇧[`/`]` or drag-to-renumber.
- `⌘0` / "last tab" semantics.
- Pills in the mission workspace's tab strip.
