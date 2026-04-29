# v0 MVP — Implementation Log

> Dated implementation notes and validation follow-ups for the v0 MVP.
> Keep this file chronological and lightweight. The stable implementation
> reference lives in `docs/impls/v0-mvp.md`.

## 2026-04-29

**Post-MVP follow-up planning.** PR #23 kept the runtime sidebar aligned with
the Pencil design: live runtime entries only, with no "mission history" row
forced into the MISSION tray. A discoverable home for completed/past work is
deferred to a separately designed workspace-level **Archived** affordance. That
surface should list both archived missions and archived direct-chat sessions.
Design it in Pencil first; ship it in its own PR.

**Agent-native session resume.** Persist each agent CLI's own resumable
session/conversation id alongside Runner's `sessions` row. Direct chats should
resume the prior Codex/Claude Code conversation when available; mission PTYs
should be able to resume each runner's stored native id after app restart. If
the native id is missing or resume fails, fall back to fresh spawn and surface
a clear warning. The detailed direct-chat lifecycle plan now lives in
`docs/impls/direct-chats.md`.

## 2026-04-28

**Validation update.** PR #22 is the post-MVP validation/fix branch after C11
merged. It aligns the Crew Detail slot action menu with Pencil node `CUKjM`,
rewrites Add Slot around existing-runner selection per node `sYprG`, restyles
`StartMissionModal` per node `rMw15`, hardens hidden-pane terminal fitting so
Codex/Claude do not paint into a tiny grid, and experiments with a collapsible
runtime sidebar for live missions + direct sessions.

Review follow-ups before merge:
- The sidebar SESSION tray must only show direct chats (`direct_session_id`),
  not mission PTYs counted by `active_sessions`.
- Past/completed missions need a discoverable route after removing `/missions`.
- Blank crew-name drafts should not render the `Saved` badge because the draft
  differs from persisted state but is invalid.

## 2026-04-27

**C11 night update.** C11 (Missions list + Start Mission modal) merged via
PR #21. The v0 MVP entrypoint is in place. `/missions` renders Active/Past
tabs over `mission_list_summary`, joined with crew name and pending ask count.
`StartMissionModal` provides the crew picker, title, goal textarea, cwd
`Browse...` via `@tauri-apps/plugin-dialog`, and a stubbed Advanced disclosure.
Clicking **Start** invokes `mission_start` and routes to `/missions/:id`.
Sidebar Mission link is enabled. With this merged, every chunk of v0 ships from
the UI alone with no DevTools required for the demo path.

**C10 evening update.** C10 (mission workspace UI) merged via PR #20. The
`/missions/:id` page subscribes to `event/appended` before its initial replay,
dedupes merged streams on ULID, and renders Feed plus one xterm-backed PTY tab
per runner. Terminal panes remain mounted while hidden so xterm scrollback
survives switching. `SessionManager` now keeps a bounded output snapshot, and
`OutputEvent.seq` lets late attachers merge snapshot and live output without
duplicating chunks. AskHumanCard renders `human_question` attribution chains
and posts `human_response` through `mission_post_human_signal`. RunnerChat was
refactored onto the same per-session-pane model. Tauri config sets
`acceptFirstMouse: true`.

**C8/C9 update.** C8 (signal router v0) and C9 (`runner` CLI binary) are both
merged. The router observes the bus, dispatches built-in signals to handlers,
and reconstructs pending-ask + status state from the log on reopen via a
high-water mark. The CLI exposes `signal`, `msg post`, `msg read`, `status`,
and `help`; validates against per-crew signal types and per-mission roster
sidecars; and suppresses `inbox_read` on `--from` filtered reads.

## 2026-04-26

**Plan revision.** C8 was reframed from "orchestrator v0" to "signal router
v0": a flat parent-process dispatcher, not a rule engine. The dispatch ledger,
replay idempotence, inbox-summary enrichment, and policy loader were descoped
because the lead runner owns coordination judgment. C8 only owns the plumbing:
bootstrap, cross-process stdin push, UI bridge, and availability bridge. The
cross-cutting prompt/runtime adapter moved into C8.
