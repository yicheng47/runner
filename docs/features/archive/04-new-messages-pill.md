# 04 — New-messages pill

> Tracking issue: [#57](https://github.com/yicheng47/runner/issues/57)

## Motivation

The mission workspace's `EventFeed` auto-sticks to the bottom while the
user is parked there. The moment the user scrolls up to read older
context, that auto-stick disengages — appended events keep landing
offscreen and the user has no signal that *anything* new arrived. They
either scroll back down speculatively, or they miss new messages
entirely until they happen to scroll.

Every chat surface the user touches daily (Slack, iMessage, Discord,
Linear comments) solves this with the same pattern: a small floating
pill above the input dock that says "New messages ↓", and clicking it
snaps the feed to the bottom. Adopt the same pattern in
`EventFeed`.

## Scope

### In scope (v1)

- **Pill surface in `EventFeed`**, positioned over the bottom of the
  scroll container (above the `MissionInput` dock). Centered
  horizontally; floats with a soft drop shadow; matches the existing
  workspace dark palette.
- **Visibility rule**: the pill appears when **both** are true:
  - the user is not near the bottom (the existing
    `wasNearBottomRef` already tracks "within 80px"), and
  - at least one new event has been appended since the user last
    left the bottom.
- **Label**: short fixed copy — "New messages ↓". No counter in v1
  (keeps the pill width stable and avoids jitter as events stream
  in). Counter is a possible v2.
- **Click behavior**: smooth scroll to the bottom, mark "near
  bottom" so subsequent events resume auto-stick, and dismiss the
  pill.
- **Auto-dismiss**: if the user scrolls themselves back to the
  bottom, the pill hides. If they scroll partway down but not to
  the bottom, the pill stays (still has unread).
- **Keyboard**: focused pill responds to Enter / Space (it's a real
  `<button>`, so this is free).

### Out of scope (deferred)

- **Per-handle filtering** ("3 new from `@architect`"). Useful but
  needs UI for filtered states; defer until the workspace gets per-handle
  filtering at all.
- **Sticky during streaming output** for the embedded `RunnerTerminal`
  (xterm.js). Different surface, different bottom-detection (xterm has
  its own scrollback API). Out of scope; this spec is `EventFeed`-only.
- **New-events sound or notification.** The notification feature
  (separate spec) is the right home for that.
- **Counter on the pill.** Considered for v1 but deferred — counts
  would either include muted rows (`runner_status`, `inbox_read`) and
  feel noisy, or exclude them and need a per-row "is meaningful"
  predicate that isn't worth the complexity yet.

### Key decisions

1. **Position is inside the `EventFeed` container, not in
   `MissionWorkspace`.** Keeps the affordance with the feed it
   describes; if we later expose `EventFeed` in another surface
   (archived missions, search results) the pill goes with it.
2. **Visibility ties to "scrolled away" + "new event arrived since
   then,"** not just "scrolled away." A user who has scrolled up to
   read older context but has nothing new yet doesn't need the pill —
   showing it would be permanent visual noise.
3. **No counter**, per the deferred-decision rationale above. The pill
   is a binary signal in v1: there is something new below.
4. **Smooth scroll, not instant jump.** A 150–250ms `behavior:
   "smooth"` scrollIntoView is enough orientation for the user to see
   roughly where they came from in the feed; an instant snap loses that.

## Implementation phases

### Phase 1 — visibility state

- Add `hasNewSinceLeftBottom` state to `EventFeed`.
- The existing `useEffect` on `events.length` already branches on
  `wasNearBottomRef`. In the `false` branch (user is up the feed),
  flip the new state to `true`.
- The `onScroll` handler already resolves "near bottom"; when it
  becomes near-bottom, clear the new state.

### Phase 2 — pill render + click

- New `<NewMessagesPill>` rendered absolutely-positioned inside the
  scroll container's parent (so it floats over the feed). Conditional
  on `!wasNearBottom && hasNewSinceLeftBottom`.
- Click handler: `scrollRef.current.scrollTo({ top: scrollHeight,
  behavior: "smooth" })`, then clear the new-state flag and set
  `wasNearBottomRef.current = true` so the next append re-auto-sticks.
- Style: matches the existing toast / overlay pill styling — rounded
  full, accent-tinted, drop shadow, `text-[12px]`.

### Phase 3 — design pass + edge cases

- Mock the pill in `design/runners-design.pen` against the existing
  workspace dark palette.
- Verify the pill doesn't overlap the `AskHumanCard` floating affordances
  when both want bottom space.
- Handle the empty-feed → first-event case (don't flash the pill if
  the user lands on the workspace and the first event arrives within
  the same render tick).
- Tab away / tab back: `useEffect` already handles visibility via the
  document visibility listener for terminals; confirm the feed's
  near-bottom detection is correct after a long backgrounded window.

## Verification

- [ ] With the feed scrolled to the bottom, new events do not show
      the pill (auto-stick keeps user pinned).
- [ ] Scroll up 200px; append a new event; pill appears within one
      paint.
- [ ] Click the pill; feed smooth-scrolls to bottom; pill disappears.
- [ ] Scroll back to bottom manually; pill disappears without click.
- [ ] Scroll up but no new events; no pill.
- [ ] Pill is keyboard-focusable and Enter / Space activates it.
- [ ] `pnpm tsc --noEmit` and `pnpm lint` clean.
