# 48 — Mission inbox reconciliation tick

> Tracking issue: [#332](https://github.com/yicheng47/runner/issues/332)

## Motivation

Message *bodies* in a mission are never lost — they live in the append-only event log, and each runner's read position is event-sourced: `runner msg read` emits an `inbox_read` signal with `payload.up_to`, which the bus projects into a per-handle `read_idx` / `unread_count` (`event_bus/mod.rs:250-268`, `cli/src/msg.rs`). What can be lost is the **wake**: nudge delivery is a fire-and-forget stdin injection, and a session that crashes before the Enter lands, a respawn that races the #328 outbox (which drops on session exit by design), or a turn that swallows the nudge text leaves an agent idling forever next to a non-empty inbox. The mission stalls silently until the human notices.

Push delivery is the latency path; it needs a correctness net. A per-mission system clock closes the loop: because unread state is already durable and queryable, "session X missed its wake" is a decidable predicate the system can check and repair — without agents ever polling.

## Scope

- **Per-mission clock in the runner system** — an in-process timer mounted alongside the mission's bus/router (started on mission start / router mount, stopped on mission stop). Not an agent behavior: the crew protocol's "delivery is push, not pull; never poll" stands unchanged.
- **Tick = pure in-memory check, no messages.** On each tick, for every live session in the mission, read the handle's `unread_count` from the existing bus projection. An empty inbox costs nothing — no injection, no agent wake, no tokens.
- **Nudge only when non-empty**: if `unread_count > 0` AND the session is idle (activity state) AND no delivery for that handle is in flight, parked in the #328 outbox, or within the backoff window, attempt the standard inbox nudge through the normal atomic reservation/injection path. A clock nudge injects only when the reservation is immediately ready; typing, another in-flight delivery, or an outbox backlog makes that tick a no-op so the next tick can re-evaluate idle and unread state. Existing push delivery still uses #328's pending-input deferral and coalescing unchanged.
- **Self-quieting**: the agent's `runner msg read` advances the `inbox_read` watermark, `unread_count` drops to zero, and the tick goes silent. Redelivery is idempotent by construction — nudges are wake-only, bodies live in the inbox projection, a redundant nudge at worst shows an agent an empty unread tail.
- **Re-nudge backoff** so a stuck agent isn't nagged every tick: clock-initiated re-nudges for the same handle are at least two minutes apart.

This retroactively closes #328's drop-on-exit window: an outbox lost at session exit leaves `unread_count > 0`, and the first tick after respawn re-covers it. Together with #328, wake delivery becomes at-least-once.

## Flush grace after input-clear

The #328 outbox flush on `InputCleared` currently uses `SUBMIT_DELAY` (80ms), but that constant exists for the injected body/Enter chord rather than flush pacing. A parked nudge can therefore land as the user hits Enter or Ctrl+C and begins the next draft.

Input-cleared flushes use a separate `INPUT_CLEAR_FLUSH_GRACE` of 500ms. `SUBMIT_DELAY` remains 80ms everywhere else. When the grace expires, the existing `reserve_delivery` / `inject_reserved` path re-checks pending input; if typing resumed during the grace window, the delivery stays parked. Ctrl+U is not treated as input-cleared because the host cannot distinguish an empty single-line draft from one removed line with multiline content remaining. The reconciliation tick is the backstop after a parked delivery is lost with its exiting session, so no additional retry mechanism is needed.

## Timing

- Reconciliation tick interval: 30 seconds.
- Per-handle backoff between clock-initiated re-nudges: at least two minutes.
- Input-clear flush grace: 500ms.
- All timing values are module constants; none are persisted or configurable.

## Out of scope

- Agent-side inbox polling in any form — rejected; it wakes every LLM per tick to usually find nothing and contradicts the push-not-pull crew protocol.
- Changes to `inbox_read` / watermark semantics or the inbox projection — the tick is a pure consumer of existing state.
- Direct chats — no inbox, no router; mission-only.
- Busy sessions — never re-nudge a busy agent; if it finishes its turn without reading, the next tick catches it.
- UI warnings for an agent that repeatedly wakes without reading.

## Implementation phases

1. **Tick loop** — per-mission timer mounted with the router/bus; per-live-session check of `unread_count` × activity state × nudge-recency; re-nudge through an immediately ready delivery reservation without parking clock nudges. Unit tests with a fake clock cover no-op on empty inbox, no-op on busy sessions, re-nudge on idle sessions with unread mail, pending-input deferral to a later tick, outbox suppression, backoff, and quiescence after a watermark advance.
2. **Lifecycle wiring** — start/stop with mission mount/stop; no ticking for stopped missions or reaped sessions; tick state drops with the mission.
3. **Input-clear flush grace** — schedule the existing outbox flush after 500ms and rely on its delivery reservation to re-check pending input.
4. **Validation** — `cargo test --workspace`; manually send mail to a runner, kill its session before it reads, resume it, and confirm the nudge re-arrives on the next tick without any human message.

## Verification

- [ ] Kill a recipient session between mail arrival and read; resume it; the nudge re-arrives within one tick.
- [ ] An idle session with an empty inbox is never nudged by the clock.
- [ ] A busy session with unread mail is not nudged until it goes idle.
- [ ] After the agent reads (`inbox_read` advances), ticks go silent.
- [ ] Backoff: clock-initiated re-nudges for a stuck agent are at least two minutes apart.
- [ ] Mission stop halts the clock; no ticks against stopped sessions.
- [ ] A parked delivery does not flush when typing resumes during the 500ms input-clear grace.
- [ ] A parked delivery flushes after a quiet 500ms input-clear grace.
- [ ] `cargo fmt`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` pass.
