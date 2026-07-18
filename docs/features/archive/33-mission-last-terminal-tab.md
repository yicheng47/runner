# 33 — Mission last terminal tab

> Tracking issue: [#236](https://github.com/yicheng47/runner/issues/236)
> Priority: P2.

## Motivation

Mission workspaces currently reopen on the feed even when the operator was actively working in a runner PTY before navigating away. Returning to a mission should restore the last runner terminal used for that mission so context switching does not force an extra tab selection.

## Scope

### In scope

- Remember the last selected mission PTY session id per mission.
- Restore that PTY tab after mission and session rows load.
- Reopen the remembered PTY tab in the tab strip before selecting it.
- Fall back to feed when the remembered session id is missing, archived, no longer belongs to the mission, or the mission is archived.
- Keep explicit tab actions unchanged: feed click shows feed, terminal click selects and remembers the PTY, and closing the active PTY returns to feed.

### Out of scope

- Backend persistence for tab state.
- Remembering arbitrary split-view or future pane layouts.
- Restoring archived mission PTYs.

## Implementation Notes

- Persistence is mission-scoped and local to the frontend through `localStorage`.
- Session ids are validated against `session_list` rows before restore; rows hidden by archive/reset naturally fail validation.
- Reset and archive clear the stored terminal id because they invalidate the prior session rows.

## Verification

- [ ] Returning to a mission restores the last selected runner terminal tab.
- [ ] The restored terminal tab exists in the tab strip before selection.
- [ ] Stale stored session ids fall back to feed without selecting another session.
- [ ] Archived missions render feed/read-only only.
- [ ] Feed click, terminal click, and active terminal close behavior remain unchanged.
- [ ] `pnpm exec tsc --noEmit` passes.
- [ ] `pnpm run lint` passes.
