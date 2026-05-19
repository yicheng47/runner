# 08 — Hide system signals from the mission feed

> Tracking issue: [#97](https://github.com/yicheng47/runner/issues/97)

## Motivation

The mission workspace `EventFeed` currently renders **every** appended
event — including router-internal/system signals — and merely
de-emphasises a handful by lowering opacity:

```ts
// src/components/EventFeed.tsx:142
const isQuiet =
  event.type === "inbox_read" ||
  event.type === "mission_warning" ||
  event.type === "runner_status";
```

In practice this means a healthy mission's feed is **dominated** by
`signal · runner_status` (busy / idle flips on every turn) and
`signal · inbox_read` watermark advances. These rows carry no
information a human needs to read — they're plumbing the router uses
to project state and watermarks. The screenshot the user attached
shows two consecutive rows being `signal · runner_status` and
`signal · inbox_read`, sandwiched between the actual conversation
they wanted to follow.

`mission_warning` looks superficially similar (it's in the same
`isQuiet` set today) but it's a different category: warnings are the
router's way of telling the **user** that something is off but
recoverable. Bundling it with plumbing would hide a diagnostic we
deliberately surfaced. v1 keeps `mission_warning` rendering — and
renders it at full strength, dropping the opacity dim that lumped it
in with plumbing.

The original "mute, don't drop" rationale (preserved in the file
header) was the audit-trail invariant: every line in the log should
surface somewhere. That's still a good invariant — but the
authoritative audit trail is the NDJSON event log on disk, not the
workspace feed. The feed is a **reading** surface for humans
collaborating with runners, and the right default for plumbing rows
is to hide them.

## Scope

### In scope (v1)

- **Hide two router-internal signal types from the feed**:
  - `inbox_read` — router-only watermark; never relevant to a reader.
  - `runner_status` — busy/idle; already projected onto the
    `RunnersRail` badge, which is the right surface for "who's
    working right now".
- **Keep `mission_warning` rendering, at full strength**. It's a
  diagnostic intended for the user; hiding it would defeat its
  purpose. Drop the `opacity-60` that previously lumped it in with
  plumbing so when one does fire it reads clearly.
- **Implementation site**: a single filter in `EventFeed` (the same
  component that owns the `isQuiet` predicate today). The filter is
  applied where the feed maps `events.map((ev) => …)`; the parent
  `MissionWorkspace` keeps receiving and projecting the full event
  stream so `RunnersRail`, watermark logic, and the new-messages
  pill (spec 04) keep working unchanged.
- **No new persistence**. The hide rule is a hard-coded predicate in
  v1; no setting, no toggle UI.
- **Audit-trail note in the file header**. Update the EventFeed
  comment so the next reader doesn't reintroduce the old "we never
  drop" claim by mistake. The replacement note records *why* these
  rows are hidden and where the audit trail still lives (NDJSON
  log).

### Out of scope (deferred)

- **Show-system-signals toggle**. A "show plumbing" affordance is the
  obvious follow-up if a power user wants the rows back without
  tailing the log file. Skip until someone asks. If we add it later
  it lives on the workspace header or as a filter chip, *not* in
  Settings — it's a per-view affordance, not a global preference.
- **Per-type granularity beyond the v1 split**. v1 hides
  `inbox_read` + `runner_status` and keeps `mission_warning`. If a
  user later wants a different cut (e.g., hide `mission_warning`
  too), that's the toggle work above.
- **Visual indicator that hidden rows exist** (e.g., "3 system events
  hidden ▾"). Adds chrome the v1 audience doesn't need; the rail
  badge already tells the user the runners are alive.
- **Hiding `human_question` resolved acks or other non-plumbing
  signals**. Out of scope by design: those are part of the human ↔
  runner conversation.
- **Backfill / migration**. The hide is a render-time filter; no
  stored data changes.

### Key decisions

1. **Render-time filter, not projection-time drop.** The full event
   stream still lives in `MissionWorkspace.events` so projections
   (status map, watermark, ask resolution) keep their inputs
   intact. Only the feed view filters.
2. **Hidden set is a strict subset of today's `isQuiet` set.**
   `isQuiet` conflates "plumbing the user shouldn't read" with
   "warning the user *should* read, just quietly." v1 splits the
   two: `inbox_read` + `runner_status` are plumbing and get hidden;
   `mission_warning` is a diagnostic and stays. If a new
   router-internal signal type lands later, it joins the hidden
   predicate.
3. **No toggle in v1.** Adding a toggle now means designing the
   surface, the persistence key, and the empty-state copy ("0
   hidden events"). All of that is premature when nobody has asked
   to see the rows back yet. The log file is the escape hatch.
4. **Audit-trail invariant is satisfied by the log, not the feed.**
   The NDJSON at `<mission_dir>/events.ndjson` (see
   `runner_core::event_log::EventLog`) is the source of truth. The
   feed is a curated view, and curating it is consistent with how
   `RunnersRail` already projects `runner_status` into a badge
   instead of a row.

## Implementation phases

### Phase 1 — single filter in `EventFeed`

- In `src/components/EventFeed.tsx`, add a top-level predicate
  `isHiddenSystemSignal(event)` that returns true for `kind ===
  "signal"` && `type ∈ { inbox_read, runner_status }`.
- Apply it to the `events.map(…)` call so hidden rows produce no
  DOM. The empty-state check (`events.length === 0`) should
  continue to look at the **unfiltered** length so a mission that
  has only had plumbing events still shows "No events yet."
  rather than the empty-feed placeholder vanishing the moment a
  `runner_status` lands.
- Remove the `isQuiet` branch entirely (the two hidden types no
  longer render; `mission_warning` renders at full strength via the
  default signal-row path).
- Update the file-header comment: replace the "we never silently
  drop events" paragraph with one that says the feed hides two
  router-internal types (`inbox_read`, `runner_status`) by default,
  notes that `mission_warning` is intentionally kept, and points
  to the NDJSON log as the audit trail.

### Phase 2 — verify projections + pill still work

- `MissionWorkspace` keeps receiving the full event stream from
  `eventsReplay` + the bus listener; nothing in the filter touches
  the parent state. Confirm by reading the workspace's
  `runner_status` projection (`MissionWorkspace.tsx:417`) and
  watermark logic — both should be untouched.
- The new-messages pill (spec 04) counts "events arrived while
  scrolled up". v1 of the pill uses a binary "something new"
  signal, not a counter, so a stream of hidden `runner_status`
  flips will still trigger the pill even though nothing visible
  appears. That's a known minor wart; left for the pill spec's
  counter follow-up to address by sharing the same
  `isHiddenSystemSignal` predicate.

## Verification

- [ ] Start a mission with the `feature-delivery` crew. Drive the
      lead and a worker through a few busy/idle transitions plus a
      `runner msg` exchange that triggers `inbox_read`. The feed
      shows only the `mission_goal`, `human_said`, `ask_*` /
      `human_question` / `human_response`, and `message` rows. No
      `runner_status` or `inbox_read` rows.
- [ ] Trigger a `mission_warning` (e.g., a router path that
      currently emits one — or stub one into a test fixture). It
      renders in the feed as a standard signal row at full
      strength, *not* hidden and *not* opacity-dimmed.
- [ ] `RunnersRail` continues to flip the busy/idle badge as the
      runners report status — the projection is intact even though
      the row is hidden.
- [ ] Open `<mission_dir>/events.ndjson`: every event still on
      disk, in append order. Audit trail intact.
- [ ] Empty-state check: a fresh mission with zero appended events
      still shows "No events yet." (not blank).
- [ ] Replay-on-mount: close the workspace, reopen it. The feed
      reconstructs from `events_replay` and applies the same
      filter; user-visible rows look identical to the pre-close
      state.
- [ ] `pnpm tsc --noEmit` and `pnpm lint` clean.
