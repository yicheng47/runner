# v0 MVP — Implementation Log

> Dated implementation notes and validation follow-ups for the v0 MVP.
> Keep this file chronological and lightweight. The stable implementation
> reference lives in `docs/impls/v0-mvp.md`.

## 2026-04-30

**Review-driven fixes (PR #25).**
- Tests green again. The lead's launch prompt is async in production
  (`Router::inject_and_submit_delayed` 2.5s) but synchronous under
  `cfg(test)` via a zeroed `LEAD_LAUNCH_PROMPT_DELAY` constant + an
  inline branch in `inject_and_submit_delayed` when delay is zero.
  Production keeps the body+`\r` chord; tests skip the `\r` so push
  counts match the pre-async assertions. The
  `claude_code_conversation_exists` fs check short-circuits to `true`
  under `cfg(test)` so resume tests don't have to fake out
  `~/.claude/projects/...` fixtures.
- `mission_reset` is now all-or-nothing. Spawn-loop and bus-mount
  failures both roll back: kill any live PTYs, stamp `archived_at` on
  the freshly-inserted session rows, flip the mission to `aborted`.
  Same shape as `mission_start`'s rollback — no half-reset states.
- Partial PTY failure no longer pauses the whole mission. Pause
  overlay + input disable now gate on `!anySessionLive` (zero alive)
  instead of `!allSessionsLive` (any dead). One worker crashing while
  the lead is still up keeps human-to-lead messaging working.
- Codex resume no longer re-replays `runner.system_prompt` as a new
  positional turn. `SessionManager::resume` now mirrors the spawn
  guard (`runtime == "codex" && plan.resuming → None`).
- Live `runner/activity` events use `slots` for `crew_count` instead
  of the removed `crew_runners` table. Live + cold-path queries
  (`commands::runner::runner_activity`) now agree.

**Next session — follow-ups.**
- Rename "direct session" → "chat" everywhere (UI copy, sidebar
  section header, type names where reasonable). The current term is a
  carryover from the backend `sessions` table; users read these as
  "chats" and the mismatch is confusing.
- Runner Detail's "Chat" button fires two sessions instead of one. Likely
  a StrictMode / mount-effect double-trigger that slipped past the prior
  spawn-mode dedupe. Repro: open Runners → click any runner → click Chat
  once → sidebar shows two new entries.
- Crew list page does not match the Pencil design. Audit against the
  design's crew-list frame and bring layout / cards / empty state into
  parity.
- Runner template needs a per-runner default **model** + **effort**
  selection (claude-code: `--model` / thinking effort flag; codex:
  equivalent). Today every spawn inherits whatever the agent CLI's own
  default is, so users can't pin a runner to e.g. Opus + xhigh effort.
  Surface as fields on the runner editor; thread through to argv via
  `runner.args` or dedicated columns + the runtime adapter.

**Workspace input gating + Mission paused overlay.** When a mission row
is `running` but every PTY is dead (the derived "stopped" display
state), the feed input is no longer interactive — replaced by a
bottom-anchored Resume card that mirrors `SessionEndedOverlay`'s
inline variant on the slot panes, so feed and PTY tabs share one
recovery affordance. `SessionEndedOverlay` gained optional
`title` / `subtitle` / `resumeLabel` overrides so mission-level copy
("Mission paused") reuses the same visual contract.

**Reset cleanup leaves no ghost sessions.** `mission_reset` already
stamped `archived_at` on the rows it superseded, but `session_list`
wasn't filtering on it — the sidebar stacked the old stopped row
alongside the freshly-spawned one for every slot. Added
`AND s.archived_at IS NULL` to the query, matching the predicate
`mission_attach`'s slot lookup already uses.

**Lead launch prompt deferred 2.5s.** The bus's initial replay fires
`mission_goal` milliseconds after the lead PTY spawns. On a warm app
(mission_reset, fast mission_start) claude-code's TUI hasn't drawn yet
and the synchronous bytes get swallowed by the boot / trust-folder
screen, leaving the lead with no system prompt. New
`Router::inject_and_submit_delayed` defers the body+`\r` chord by the
same 2.5s budget `SessionManager::schedule_first_prompt` already uses
for non-lead workers; the `mission_goal` handler now routes through it.

**Resume: fresh-fallback for missing claude-code conversations.**
`claude --resume <uuid>` against a missing conversation file leaves
the TUI half-broken with `No conversation found with session ID`.
Trips most often when a lead PTY never persisted a turn (reset before
its first message landed). New
`router::runtime::claude_code_conversation_exists(cwd, uuid)` checks
`$HOME/.claude/projects/<encoded-cwd>/<uuid>.jsonl` (encoding maps
both `/` and `.` to `-` — claude-code's actual scheme — without the
`.` swap, every cwd containing a dot would spuriously fall back). On
miss, `SessionManager::resume` swaps `--resume` for `--session-id
<existing-uuid>`, keeping the row's UUID bound to the new conversation
via the existing `COALESCE` write.

**Lead recovery prompt on fresh-fallback resume.** `mission_attach`
sets a watermark that suppresses bus replay of `mission_goal`, so the
lead would come up with no context after a fresh-fallback. Added
`Router::fire_lead_launch_prompt` which reads the latest
`mission_goal` text from the event log, runs the same
`compose_launch_prompt` builder the bus handler uses, and injects via
`inject_and_submit_delayed`. `SessionManager::resume` surfaces a
`fresh_fallback_lead` flag on `SpawnedSession` (serde-skipped — not
actionable from the UI); the `session_resume` command sees it and
calls `fire_lead_launch_prompt` after the resume returns. The lead
gets the rich launch prompt — system_prompt + mission goal + roster +
crew context — not the worker coordination preamble.

## 2026-04-29

**Mission lifecycle redesign + workspace polish.** Stop / Resume /
Archive split: `mission_stop` is now reversible (kills PTYs, mission row
stays `running`, router/bus stay mounted). New `mission_archive` is the
destructive end-of-mission path that flips status to `completed`, writes
the terminal `mission_stopped` event, and unmounts. `mission_attach`
rebuilds Router + Bus on workspace mount after app restart so resumed
slot PTYs land on a live router. Per-slot Resume button on each tab,
top-level Resume button (visible when ≥1 slot is stopped), and a
client-side iteration over `session_resume`. Also: PTY classification
fix — user-initiated kills are tracked in a `killed` set so SIGTERM
maps to `stopped`, not `crashed`.

**Inbox-arrival nudges.** Router now pushes a one-line stdin nudge when
a directed message lands so workers wake up to call `runner msg read`.
Broadcasts nudge every slot except the sender; self-targeted messages
are ignored. Without this, pull-based inbox routing strands the worker
— they have no clock to poll on.

**Mission session resume + boot rehydration.** Lifted the `mission_id`
guard in `SessionManager::resume`; mission rows now respawn through the
same path as direct chats with `RUNNER_HANDLE = slot.slot_handle` and
the full crew/mission env block. `mission_attach` (idempotent) rebuilds
the in-memory router + bus from persisted rows after restart and
registers existing slot_handle → session_id pairs.

**Pin + Rename for missions.** Migration `0007_mission_pin.sql` adds
`pinned_at`. `mission_pin` and `mission_rename` Tauri commands wire the
actions. Pinned missions float to the top of the sidebar (sort key:
`pinned_at IS NULL, pinned_at DESC, started_at DESC`). Inline rename in
the sidebar; `prompt()` from the topbar kebab.

**Workspace tab strip + collapsible right rail.** Feed tab is permanent;
PTY tabs open on demand from the right rail (each runner card has a
terminal-icon button) and close via per-tab `×`. The Runners rail moved
out of the body row to a top-level sibling so it spans the full
workspace height with its own header lined up across the topbar
divider, mirroring `RunnerChat`'s layout. Topbar gained a flag glyph
(matching the Pencil's `nEpyL`/`Wopzz`), stacked title/subtitle,
smaller status pill (derived from PTY liveness so a running-but-all-
stopped mission reads as "stopped"), Stop with icon, and a kebab
dropdown for Pin / Rename / Archive.

**Sidebar parity with design.** Search input replaced by a click-to-
callout `search` nav row — the inline input never made sense
(interaction is open-a-palette, not type-in-place). All five sidebars
in the Pencil design now share `nav_r` / `nav_c` / `nav_s` naming and
layout; tab order is `runner → crew → search` everywhere. Right-click
on a mission row in the sidebar opens the same Pin / Rename / Archive
popover as the topbar kebab.

**Direct-chat double-spawn fix.** Spawn-mode StrictMode (and any
mid-spawn cancel) now reaps both the PTY child AND archives the
orphaned `sessions` row, so a single Chat click no longer leaves two
visible entries in the sidebar.

**claude-code system prompt delivery.** `--append-system-prompt` /
`--system-prompt` are SDK-only — the interactive TUI silently drops
them. New `SessionManager::schedule_first_prompt` injects
`runner.system_prompt` as the first user turn via stdin ~1.5s after
spawn. Skipped on resume, on non-claude-code runtimes, and for the
mission lead (the `mission_goal` handler already injects a richer
launch prompt that embeds system_prompt). This also fixes the resume
"Session ID … is already in use" error: claude-code's `--session-id`
flag is fresh-only, so resume reverts to `--resume <uuid>`. The
"conversation file never persisted" failure mode `--session-id` was
introduced to mask is now masked instead by `schedule_first_prompt`,
which forces claude-code to write the conversation file on first
spawn.

**Test fixtures.** Default system prompts (`tests/fixtures/system-prompts/{architect,impl,reviewer}.md`)
and a sample crew (`tests/fixtures/crews/feature-delivery.md`) for
end-to-end testing.

---

**PR #24 merged — crew slots / runner-as-template.** Crew composition now
uses `slots` instead of `crew_runners`: one Runner is a reusable config
template, and one crew can place that template in multiple slots with distinct
`slot_handle`s. Migration `0006_slots.sql` drops `crew_runners`, drops
`runners.role`, creates `slots`, and adds `sessions.slot_id`. Mission spawn,
the router roster, `RUNNER_HANDLE`, `roster.json`, MissionWorkspace tabs, and
Crew Detail now use `slot_handle` as the in-mission identity. Per-slot role was
cut from v0; the only per-slot identity in the shipped model is `slot_handle`.

Post-merge follow-ups from review:
- **Running mission identity snapshot.** `session_list` still resolves
  mission session labels and lead status from the live `slots` table. If a user
  edits/removes/reassigns slots while a mission is running, the workspace can
  relabel or default-target using the new crew state while the router and
  child PTYs still use the start-time `slot_handle`. Fix by snapshotting
  `slot_handle`/`lead` onto the session row or by blocking slot mutations for
  crews with running missions.
- **Codex resume prompt replay.** Codex has no real system-prompt flag, so
  fresh Codex sessions receive `runner.system_prompt` as the positional initial
  prompt. The direct-chat resume path must not append that positional prompt
  when using `codex resume <uuid>`, or every resume adds the runner brief as a
  new user turn.
- **Live activity crew count.** The spawn/reap `runner/activity` emitter still
  queries the removed `crew_runners` table and falls back to `crew_count = 0`;
  switch it to `slots` so Runners and Runner Detail do not temporarily show
  incorrect usage counts after live activity changes.
- **Runner Detail duplicate slot rows.** `runner_crews_list` is now one row per
  slot, but Runner Detail keys rows by `crew_id`. Use `slot_id` and show
  `@slot_handle` so a template used twice in the same crew renders correctly.

Validation for the reviewed PR head passed: `pnpm exec tsc --noEmit`,
`pnpm run lint`, `cargo test --workspace`, and
`git diff --check origin/main...HEAD`.

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
