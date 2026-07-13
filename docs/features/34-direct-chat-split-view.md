# 34 — Direct-chat split view (layout picker)

> Tracking issue: [#245](https://github.com/yicheng47/runner/issues/245)
> Priority: P2.

## Motivation

Direct chats are one-at-a-time: `/chats/:sessionId` shows a single chat and you switch via the sidebar. Watching or driving two runners at once — an implementer next to a reviewer, two parallel explorations — means flipping back and forth and losing whatever the other chat printed meanwhile. Splitting the direct-chat surface into 2–3 side-by-side panes removes the flip. This supersedes the near-term need behind mission split (#166, closed as deferred): the chat surface is simpler (PTY panes only, no feed) and is the more common multi-view case.

## Scope

### In scope

- A layout button left of Stop in the chat topbar opens a TradingView-style preset picker: 1 · 2 side-by-side · 2 stacked · 1-big+2-stacked · 3 columns · 3 rows.
- Each pane is a self-contained chat: per-pane header (name, CHAT chip, status dot, its own Stop/Resume control, and a `…` menu with Archive) above the terminal; the focused pane shows an accent ring. While split, the topbar controls aggregate: Stop all / Resume all, and the kebab's Archive becomes Archive all.
- Panes run edge-to-edge as one connected surface, separated by a single 1px resizable divider (`react-resizable-panels`).
- `Cmd+[` / `Cmd+]` cycle pane focus while split; sidebar page navigation moves to `Cmd+Shift+[` / `Cmd+Shift+]`.
- A chat lives in exactly one pane (move-not-copy); layout changes never remount terminals, so xterm state and the single-writer stdin invariant are preserved.
- Empty panes (preset has more slots than open chats): focus the first empty pane and auto-open `StartChatModal` with the focused chat's runner preselected; cancel leaves an empty state with a New chat button and a sidebar hint.
- The split is a **tab** — one group of panes on the chat surface, bound to specific sessions (arch §3.6, Window → Tab → Pane; earlier wording here called it the "chat group"): it renders while the open chat is a member; other chats open single-pane and pane tabs stay intact in the background. Navigation never mutates a tab — members are added only via the empty-pane modal or a sidebar pick while an empty pane is focused, and building panes from a non-member chat creates another pane tab.
- Sidebar reflects the on-screen tab: pane-open rows get the selected-row fill, the focused pane's row gets a left accent bar; clicking a member's row focuses its pane.
- While split, the topbar carries the tab identity: split-icon avatar, tab name (user-given via kebab Rename, else derived from member chat names; persisted with the layout), a TAB chip (UI copy still reads GROUP until the code rename pass lands), an aggregate "n/m running" status, and a pane-count meta line. URL and the right-hand runner panel still follow the focused pane.
- `Cmd+W` while split closes the focused pane (the session keeps running); the sibling pane reflows.
- Layout is sticky per window: leaving the chat surface keeps pane tabs, and the main window persists them across restarts (restored chats come back stopped, in their panes, resumable).

### Out of scope

- Mission-workspace split (#166) and cross-window layouts.
- Synchronized input across panes.
- Saved layout templates.
- Per-pane runner panels.

## Implementation Notes

- `RunnerChat` already mounts every direct chat in a hidden stack; split view shows 2–3 of those already-mounted panes at once by geometry-syncing each visible terminal's wrapper onto its pane's body rect — the terminals' React tree position never changes, so no remount by construction. (Portal re-parenting was rejected: React remounts portal children when the container changes.) See impl plan [0020](../impls/0020-direct-chat-split-view.md), decision 4.
- Layout state lives in a small module store shared by `RunnerChat` and `Sidebar`; the main window mirrors it to localStorage and rehydrates pane sessions from the recent-direct list on restore (impl 0020, decision 6).
- The multi-window duplicate-subject gate (impl 0018) applies per pane: the window registry stores a list of subjects per window and `RunnerChat` reports every visible pane's session, so a session owned by another window shows the overlay in its pane only (impl 0020, decision 9).
- Design: `design/runner-mvp-design.pen` — `Layout picker popup`, `Runner direct chat — 2-pane split`, `— 3-pane split`, `— 2-pane split, empty pane`.

## Verification

- [ ] Picking a 2-pane preset shows both chats live; typing in one pane never echoes in the other.
- [ ] Layout changes and gutter resizes preserve terminal scrollback (no remount) and refit dimensions.
- [ ] Clicking a sidebar row for a chat already visible in another pane focuses that pane (no duplicate terminals, no pane emptied).
- [ ] Opening a non-member chat renders it single-pane with pane tabs untouched; opening a member re-activates that member's tab, and creating panes from the non-member preserves the previous pane tabs.
- [ ] Pane-header Stop/Resume act on that pane's session only (no focus/URL jump); topbar Stop all / Resume all act on every visible pane, and concurrent resumes settle independently.
- [ ] After Stop all, every stopped pane dims and shows its own scrim + Chat-paused card with per-pane Resume and Archive.
- [ ] Archiving a pane's chat (card, kebab, or sidebar) shows the amber pill in that pane and empties it in place; archiving the focused chat hands the URL to a surviving member, leaving the surface only when none remains.
- [ ] While split, the topbar kebab reads "Archive all" and archives every visible pane's chat (background panes first, one final navigation).
- [ ] Starting/resuming a sibling pane never corrupts another pane's glyphs (WebGL atlas peers rebuild on context churn).
- [ ] Empty pane auto-opens `StartChatModal` with the focused chat's runner preselected; cancel shows the empty state; both fill paths (modal, sidebar click) work.
- [ ] Sidebar shows pane-open fills and the focused accent bar; row clicks target the focused pane.
- [ ] Topbar, URL, and runner panel track the focused pane; refresh on `/chats/:sessionId` lands single-pane on that chat.
- [ ] `Cmd+W` collapses the focused pane without stopping the session; single-pane `Cmd+W` unchanged.
- [ ] Leaving the chat surface and returning keeps the layout; relaunching the app restores the same panes (main window), with their sessions stopped and resumable. Stale (archived-while-closed) sessions restore as empty panes.
- [ ] Secondary window overlay appears per pane, input gated, regardless of the primary window's layout.
- [ ] `pnpm exec tsc --noEmit` passes.
- [ ] `pnpm run lint` passes.
