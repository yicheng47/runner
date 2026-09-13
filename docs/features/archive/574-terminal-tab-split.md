# Terminal tab split

Tracking issue: [#574](https://github.com/yicheng47/runner/issues/574). Status: shipped 2026-09-13 in [#576](https://github.com/yicheng47/runner/pull/576) (brief [574](../../impls/archive/574-terminal-tab-split.md), mission `01M2CEE7QAZSAA65ATQQ3XRQWN` on codex-crew). Priority P2.

## Motivation

[570](./570-split-panes-redesign.md) wrote "a split holds chats only", and the code enforces it: a terminal-only tab hides the split icon, `split_decision` answers NotSplittable, and `⌘D` / `⇧⌘D` return early. The rule was about mixing — the empty pane a split creates offers New chat alone, so a split on a terminal tab would have produced a pane that could only hold a chat. It is narrower than that intent: a terminal tab cannot become two terminals at all. Meanwhile **New terminal** with a terminal tab active does split the focused pane through `prepare_new_pane`, without the size floor — the follow-up 570 left unfiled.

Ghostty is the reference. A split there means "another shell here, now": no empty pane, no picker, the new surface is a shell in the directory you were in. A terminal split has no choice to make, which is exactly why the chat split's stop at the empty stub does not apply.

## Behavior

- **A split is a split.** A terminal-only tab shows the same split icon — in the header while single-pane, on every identity line once grouped — with the same Split Right `⌘D` / Split Down `⇧⌘D`, the same 50 / 50 tree, and the same 240 × 160 px floor with the same **Too small to split** tooltip.
- **The new pane is never empty.** A shell spawns in it at once and takes focus, in the working directory the split-from pane's shell was spawned with; Runner stores that per session. The empty-pane stub never appears on a terminal tab. Inheriting the shell's *live* directory needs OSC 7 and is [#575](https://github.com/yicheng47/runner/issues/575).
- **No mixing, by tab kind.** A terminal tab's split makes a terminal; a chat tab's split makes the chat stub as today; the drawer stays the terminal home beside a chat. The header's drawer icon stays hidden on terminal tabs.
- **New terminal on a terminal tab is Split Right.** Same gate, same floor, instead of `prepare_new_pane`.
- Grip and drag ([568](./568-pane-drag-reorder.md)), `×` with its foreground-process confirmation, `⋯` (Stop · Rename…), rename, the sidebar row's terminal glyph and title: unchanged.

## Non-goals

- Chat and terminal in one split.
- Live cwd (OSC 7): #575.
- A terminal split from inside a chat tab; the drawer covers that.

## Design

`design/runner.pen`, frame `Spec — Terminal tab split (574) · v1`: the terminal tab header before (no icon) and after (the icon and its two-item menu), the split surface after Split Right with the new shell focused and the old pane faded, a terminal identity line with the menu open, and the rule as notes.

## Implementation Phases

1. **Gate.** `split_decision` drops the terminal-only → NotSplittable arm; the header split icon renders on a terminal-only single-pane tab; the drawer icon still does not. Tests: a terminal-only tab is allowed or blocked by size like a chat tab.
2. **Split.** In `split_pane`, after `layout.split`, when the tab is terminal-only spawn a shell into the new pane with the existing spawn-into-pane helper, using the split-from session's cwd, then the same persist → reload → attach → focus; otherwise the empty stub as today. `new_terminal` with a terminal-only tab active calls the same split path on the focused pane.
3. **Docs.** `docs/arch/arch.md` pane paragraph; `docs/tests/64-terminal-as-pane-option-smoke.md` §1 and §4; the 570 archived spec gets a superseded note on its "chats only" line.

## Verification

- Single terminal tab: the header shows the split icon; Split Right and `⌘D` give a second shell on the right, focused, at the same cwd, with no stub; Split Down the same below. Both identity lines carry the terminal glyph, grip, split icon and `×`, no status dot.
- Keep splitting until the item disables with the tooltip; widen the window and it re-enables.
- New terminal (palette, sidebar `+`) with the terminal tab active splits right through the same floor.
- A chat tab still gets the New chat stub on split and its drawer icon; a terminal tab still has no drawer icon.
- Quit and relaunch with a three-shell tab: shape and cwds come back.
- `cargo test -p runner-app` green.
