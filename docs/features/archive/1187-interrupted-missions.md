# 1187 — Interrupted missions

Tracking issue: [#1187](https://github.com/yicheng47/runner/issues/1187). Design: `runner/desktop/specs/1187-interrupted-missions.pen` in the design repo, signed off by Jason on 2026-11-05: option A (amber pause) in frame `g1uiM1`, the feed line without a Continue button (variant 1) in frame `YoQDv`, and the report and `runner mission show` in frame `jSj1Z`. Option B, option C and the Continue button were not chosen and are not built. Where the design and this spec disagree on visual detail, the design wins; on behaviour, this spec wins.

## Motivation

When runnerd stops while missions run (an update replacing it, ⌥⌘Q, or `runner daemon stop`), it stamps every running session for launch resume and kills it. The next daemon resumes each member, and the resumed agents come back idle at their prompts: any turn in flight is gone. With every member idle and nothing pending, the mission settles after the 30-second settle window, and its linked chat is told "settled". On 2026-11-05 this happened to all three running missions at once, with a member mid-turn in each. The operator chats had to work out from the feeds that nothing had finished.

## Scope

### The Interrupted state

`MissionPhase` gains `Interrupted`: at least one member was mid-turn when runnerd stopped and has not started a turn since. In `report::derive`, Interrupted comes after Ended, Needs you and Failed and before Working and Settled. If one interrupted member is re-prompted while another still waits, the mission stays Interrupted, so the second member isn't forgotten. While any member is interrupted, the mission cannot settle.

### Marking and clearing a member

- **Marking.** Before launch resume respawns a mission session, the daemon reads that member's last `session_status` in the mission's event log. If it was `busy`, the daemon appends a `session_interrupted` signal from that handle, carrying the time runnerd stopped if known, otherwise the time of the restart. Because the mark lives in the log, it doesn't depend on the build of the daemon that stopped, needs no migration, and survives another restart before anyone re-prompts the member. A member that was idle, ready or waiting on input gets no mark. A crash isn't resumed automatically and keeps today's handling.
- **Clearing.** A member is interrupted from its `session_interrupted` signal until its next `busy` status. Restarting, stopping or archiving the session clears it too, and stopping or archiving the mission ends it (Ended wins).
- **Member status.** Each interrupted member's `AgentStatus` in the mission summary, `runner mission show` and the app carries the interrupted mark, so the UI can name the member. Pick the smallest protocol shape that carries it, and keep older clients decoding (see Compatibility).

### Reports

`MissionPhase::reported()` includes Interrupted. Entering Interrupted queues one `State` report for the linked chat. As with the other reports, the line names only the mission:

```text
[runner] Mission <id> "<title>" was interrupted by a Runner restart. Read it with runner mission show <id>.
```

The first refresh after boot baselines the mission without reporting it, so the mark must be in place before that baseline, or the Interrupted transition must still be reported after it. Either way the chat gets exactly one Interrupted report per restart and no Settled report until every interrupted member has run a turn and the mission goes quiet again. A mission that had already settled before the restart stays Settled and gets no report. A `Linked` report for an interrupted mission reads "it was interrupted by a Runner restart".

### App

- **Mission row.** The trailing slot shows a lucide `circle-pause` (12 px) in the warning colour, in Projects and Active.
- **Collapsed group.** The summary reads "N missions · M interrupted" (`format.link_group_interrupted`) in the warning colour, and the collapsed row's trailing glyph is the pause. In `URGENCY`, Interrupted comes after Needs you and Failed.
- **Feed.** Each `session_interrupted` event renders as a system warning row like the other feed warnings, with the pause glyph and amber text: "Runner restarted while @coder was mid-turn. That turn was lost; message @coder to continue." It stays in the feed as history after the member resumes. It doesn't address any member and isn't delivered to agents.
- **Member surfaces.** Until the member starts a turn, its member tab and its Sessions card show the pause glyph and "Interrupted" in place of Idle, in the warning colour. The label, its tooltip ("Interrupted by a Runner restart at <time>; message it to continue.") and the Stop chat offered on hovering a running session's status live in the shared status presentation and are covered by unit tests, since a mission member has no island pane header of its own.
- **Companion.** The companion overview (`runner-core/src/companion/overview.rs`) shows the phase too, styled like Needs you.

### CLI

`runner mission show` prints `STATE interrupted (Runner restarted at <time>)` for the mission and `interrupted` in the member table's STATE column. `--json` carries `interrupted` as the phase and the member's mark in its status. `runner mission list` is unchanged. Regenerate the CLI reference with `make guide-cli` if help text changes.

### Compatibility

`MissionPhase` has no catch-all variant, so a client on an older build would fail to decode `interrupted`. That includes a companion (iPhone), a remote viewer, or an app connected during the in-use grace. Add a `#[serde(other)]` fallback on the client side of every type that gains a value, and make sure today's builds don't fail on the new value. If that isn't possible, send the new value only to clients that announce support for it. Say in the PR which approach was taken.

### Guide pages

Update `docs/guide/concepts/missions.md` (the state list and the report example), `docs/guide/guides/chats-and-windows.md` (row glyphs and the collapsed summary order) and `docs/guide/guides/drive-runner-from-agents.md` (when reports arrive), with their `zh-CN` copies passing `python3 script/check-guide-translations.py`.

### Out of scope

- Keeping agents alive across a restart ([#1136](https://github.com/yicheng47/runner/issues/1136)), which would remove the interruption itself.
- Runner typing anything into an interrupted member (the Continue button or an automatic "continue").
- Interrupted direct chats outside missions.

## Implementation notes

- State, derivation and reports: `crates/runner-daemon/src/ops/mission/report.rs` (`derive`, `PhaseInput`, `Member`, `Reports::apply`, `line`).
- Startup order: `crates/runner-daemon/src/daemon/boot.rs` (`cleanup_stale_running_rows_on_startup`, `Reports::start`). Launch resume: `crates/runner-daemon/src/daemon/resume.rs` and `crates/runner-daemon/src/session/manager/resume.rs`. Stamp: `mark_running_for_resume_on_launch` from `crates/runner-daemon/src/daemon/server.rs`.
- Last busy/idle per handle: `latest_session_statuses_from_log` in `crates/runner-daemon/src/ops/mission.rs`. Signal types: `crates/runner-core/src/model.rs`.
- App: `crates/runner-app/src/surfaces/sidebar/link.rs` (`phase_attention`, `mission_attention_indicator`, `URGENCY`, `collapsed_summary`, `GroupState::summary`), the mission feed in `crates/runner-app/src/surfaces/mission_workspace/feed.rs`, member tabs and the Sessions card in `mission_workspace`, and status labels in `crates/runner-app/src/ui/agent_status.rs`.
- This touches session lifecycle and the client protocol, so the PR takes `core-review`.

## Validation

Automated:

- Derivation: a busy-then-interrupted member gives Interrupted, and Interrupted outranks Working; it clears on the member's next busy; the mission can't settle while a member is interrupted; Ended still wins.
- Boot: a mission with a busy member and a stamped session gets one `session_interrupted` signal and one Interrupted report, and no Settled report after the settle window. An idle or settled mission gets no mark and no report. A second restart before a re-prompt doesn't add a second report.
- Serde: an older-shaped client decodes the new phase without failing.
- App: `phase_attention`, `most_urgent` and `collapsed_summary` cover Interrupted.

Live checks (qa, development app on macOS), each with its reason:

1. Start a linked mission with a member given a long task. While it works, replace the development daemon (rebuild with a daemon change and `make run`, or `runner-dev daemon stop` and relaunch). After resume, the mission row shows the amber pause, the feed shows the warning row naming the member, and the member's tab and Sessions card read Interrupted. *Reason: the new state end to end, after a real restart.*
2. The linked chat gets one Interrupted `[runner]` line and no Settled line within two minutes. *Reason: the reported bug.*
3. Message the interrupted member. It goes back to Working, and the mission leaves Interrupted. *Reason: clearing.*
4. With a second mission whose members were all idle before the restart, nothing reports, and it stays Settled. *Reason: no false positives.*
5. Collapse the linked group while one mission is interrupted. The summary reads "… · 1 interrupted" in amber with the pause glyph. *Reason: the collapsed summary.*
6. `runner-dev mission show <id>` prints `STATE interrupted` and the member row's `interrupted`. *Reason: the CLI surface.*
