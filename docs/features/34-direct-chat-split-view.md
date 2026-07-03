# 34 — Direct-chat split view (layout picker)

> Tracking issue: [#245](https://github.com/yicheng47/runner/issues/245)
> Priority: P2.

## Motivation

Direct chats are one-at-a-time: `/chats/:sessionId` shows a single chat and you switch via the sidebar. Watching or driving two runners at once — an implementer next to a reviewer, two parallel explorations — means flipping back and forth and losing whatever the other chat printed meanwhile. Splitting the direct-chat surface into 2–3 side-by-side panes removes the flip. This supersedes the near-term need behind mission split (#166, closed as deferred): the chat surface is simpler (PTY panes only, no feed) and is the more common multi-view case.

## Scope

### In scope

- A layout button left of Stop in the chat topbar opens a TradingView-style preset picker: 1 · 2 side-by-side · 2 stacked · 1-big+2-stacked · 3 columns · 3 rows.
- Each pane is a self-contained chat: per-pane header (name, CHAT chip, status dot) above the terminal; the focused pane shows an accent ring.
- Resizable gutters between panes (`react-resizable-panels`).
- A chat lives in exactly one pane (move-not-copy); layout changes never remount terminals, so xterm state and the single-writer stdin invariant are preserved.
- Empty panes (preset has more slots than open chats): focus the first empty pane and auto-open `StartChatModal` with the focused chat's runner preselected; cancel leaves an empty state with a New chat button and a sidebar hint.
- Sidebar reflects the layout: pane-open rows get the selected-row fill, the focused pane's row gets a left accent bar; clicking a chat row loads it into the focused pane.
- Topbar, URL, and the right-hand runner panel follow the focused pane.
- `Cmd+W` while split closes the focused pane (the session keeps running); the sibling pane reflows.
- Layout is in-memory and resets when leaving the chat surface.

### Out of scope

- Mission-workspace split (#166) and cross-window layouts.
- Synchronized input across panes.
- Layout persistence across restart; saved layout templates.
- Per-pane runner panels.

## Implementation Notes

- `RunnerChat` already mounts every direct chat in a hidden stack; split view shows 2–3 of those already-mounted panes at once by geometry-syncing each visible terminal's wrapper onto its pane's body rect — the terminals' React tree position never changes, so no remount by construction. (Portal re-parenting was rejected: React remounts portal children when the container changes.) See impl plan [0020](../impls/0020-direct-chat-split-view.md), decision 4.
- Layout state lives in a small module store shared by `RunnerChat` and `Sidebar`; cleared when the chat surface unmounts.
- The multi-window duplicate-subject gate (impl 0018) applies per pane: the window registry stores a list of subjects per window and `RunnerChat` reports every visible pane's session, so a session owned by another window shows the overlay in its pane only (impl 0020, decision 9).
- Design: `design/runners-design.pen` — `Layout picker popup`, `Runner direct chat — 2-pane split`, `— 3-pane split`, `— 2-pane split, empty pane`.

## Verification

- [ ] Picking a 2-pane preset shows both chats live; typing in one pane never echoes in the other.
- [ ] Layout changes and gutter resizes preserve terminal scrollback (no remount) and refit dimensions.
- [ ] Loading a chat already visible in another pane moves it (no duplicate terminals).
- [ ] Empty pane auto-opens `StartChatModal` with the focused chat's runner preselected; cancel shows the empty state; both fill paths (modal, sidebar click) work.
- [ ] Sidebar shows pane-open fills and the focused accent bar; row clicks target the focused pane.
- [ ] Topbar, URL, and runner panel track the focused pane; refresh on `/chats/:sessionId` lands single-pane on that chat.
- [ ] `Cmd+W` collapses the focused pane without stopping the session; single-pane `Cmd+W` unchanged.
- [ ] Leaving the chat surface and returning resets to single pane.
- [ ] Secondary window overlay appears per pane, input gated, regardless of the primary window's layout.
- [ ] `pnpm exec tsc --noEmit` passes.
- [ ] `pnpm run lint` passes.
