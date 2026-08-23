# Chat-style mission feed: Discord-like rows and identicon identity

## Status

Implemented (`b5f7fd5`, shipped in v0.5.0; the native feed carries the same rows and identicons, reworked by M6.13). Design: `design/mission-feed-composer.pen` — frame "Feed · chat-style messages" (`rAJ00`), updated "Runners rail" (`LMQ7c`), decision notes (`Rauwg`). Builds on the #392 working-tree fixes already present in `MessageBody.tsx` / `EventFeed.tsx` (list gutter, mission-goal row, spacing). Revives the identicon idea from `docs/features/archive/11-runner-avatar.md`, which was specced but never shipped.

## Problem

The feed renders three different anatomies for what is conversationally the same thing: runner/human messages are bare header+body rows, the mission goal was (until the #392 fix) a bordered mono payload block, and signals are JSON boxes — including the raw `ask_human` signal, which renders its full JSON right above the `human_question` card that presents the same prompt, choices, and attribution properly. Identity is a monospace handle with a single accent color for every runner, so multi-runner missions have nothing for the eye to lock onto, and the rail's busy/idle state lives in a separate dot glyph next to the handle.

## Key Decisions

1. **Discord/Slack row anatomy.** Every conversational event renders as avatar + header (author, context, time) + body. Consecutive message-like events from the same author within a 5-minute window collapse into one group: avatar and header render once, subsequent bodies stack under them. Any non-message block (divider, signal line, ask card) breaks the group. Grouping is a pure function over the filtered event list so it is unit-testable.

2. **`RunnerAvatar` identicon, deterministic from the handle.** 5×5 symmetric pixel grid (columns mirrored) generated from a handle hash, rendered on a raised rounded square — 35px in the feed, 25px in the rail. The same hash picks the runner's hue from a fixed palette of carbon-legible colors (accent green, cyan, violet, orange, …); the hue also colors the handle text wherever it appears. Amber (`warn`) is reserved for the human — "you" always renders amber with a fixed pattern and is never assigned to a runner. The LEAD marker stays a badge, not a color.

3. **Message-like events share one renderer.** `message`, `human_said`, `human_response`, and `mission_goal` all go through the same message-row path. `mission_goal` keeps its small `GOAL` chip in the header (from the #392 fix) and an optional `→ @target`. `mission_start` becomes a thin centered divider (`MISSION STARTED · time`), Discord date-divider style.

4. **Raw `ask_human` signals are hidden.** They are worker→router plumbing fully duplicated by the router's `human_question`; `isHiddenSystemSignal` grows to cover them. The `AskHumanCard` absorbs the identity: it renders inside the asker's avatar row with a `NEEDS YOUR INPUT` chip and the `→ you` chain in the header; card internals (prompt, choice buttons, resolved state) are unchanged.

5. **Remaining signals become one-line rows.** `ask_lead`, `mission_warning`, and unknown types render as a single indented line — zap icon, author handle, `signal · type`, time — with a `payload ▾` disclosure that expands the existing payload rendering (the current JSON box body) inline. `mission_warning` keeps a danger tint on the line so diagnostics still stand out. Answered `ask_lead` lines stay visible; the feed remains a faithful log, no fading.

6. **Rail cards adopt the same identity.** In `RunnersRail`, the avatar (25px) with the runner hue replaces the inline status dot; PTY/runner status moves onto the avatar corner as a presence dot (busy accent, idle dim accent, stopped gray, crashed danger — same priority order as today's `dotClass`). Handle text takes the hue. The rest of the card (LEAD badge, Open-PTY button, status subtitle, `session_key` row) is already aligned between code and the updated design frame and does not change structurally.

## Goals

- One visual grammar for every conversational turn; system rows visibly distinct but quiet.
- A three-runner mission is scannable by color and pattern without reading handles.
- Pure presentation: no event-schema, router, log, or backend changes; archived missions replay identically.

## Non-Goals

- Reactions, replies, hover action toolbars, message editing — Discord look, not Discord features.
- Composer changes (shipped in impl 0042) or `AskHumanCard` flow/button changes.
- Avatar images/uploads or per-runner color configuration; the mapping is deterministic.
- Persisting payload-disclosure open state.

## Implementation Notes

- `src/components/ui/RunnerAvatar.tsx` — new. Props: `seed`, `size`, optional `presence`. Exports `hueForSeed(seed)` for handle coloring; special-cases the human seed to `warn`.
- `src/lib/eventFeed.ts` — add `groupFeedBlocks(events)` returning typed blocks (`divider` | `message-group` | `signal` | `ask-card`); extend the hidden-signal predicate with `ask_human`.
- `src/components/EventFeed.tsx` — render from `groupFeedBlocks`; message-group renderer (avatar column + header + stacked bodies via `MessageBody`); divider renderer; signal one-liner with payload disclosure reusing `renderPayload` as the expanded body.
- `src/components/AskHumanCard.tsx` — header row becomes avatar + chip + chain per design; accepts the asker handle it already receives.
- `src/components/RunnersRail.tsx` — swap dot for `RunnerAvatar` with presence; hue on handle text.
- `src/components/MessageBody.tsx` — no changes beyond the #392 fixes already in the tree.

## Validation

- Vitest: `groupFeedBlocks` — same-author grouping inside/outside the 5-minute window, group break on interleaved signal, `mission_goal`/`human_said` classified message-like, `ask_human` hidden, `mission_start` → divider; `hueForSeed` determinism and human reservation.
- Manual: live 2-slot mission — grouped runner turns, goal row, divider, signal disclosure toggle, ask card with avatar; archived mission replay renders identically; rail presence dot tracks busy/idle/stopped.
- `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.
