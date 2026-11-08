# #1183 — A mission does not settle while a member's background work is pending

Jason approved the claude trio for [#1183](https://github.com/yicheng47/runner/issues/1183) on 2026-11-08. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1183-background-settle` on `fix/1183-background-settle`, based on `origin/main` at `47cd529012f1081362291cfe6b12d6e9bde53a85`.

## Problem

`derive` in `crates/runner-daemon/src/ops/mission/report.rs` settles a mission once every live member has been idle for `SETTLE_WINDOW` (30 s) with nothing pending, and Runner then reports "settled" to the linked chat. A Claude Code member that ends its turn while a background task it will wake from is still running (background Bash, Monitor, a background subagent, a scheduled wakeup) looks idle, so the mission settles while work is still in flight. Operators treat Settled as "the crew is done" (AGENTS.md adds `qa` once a mission settles), so a false settle can start the next step early. Settled must mean: nothing more happens without the person.

## Signal

Claude Code documents this in its Stop hook input ([hooks reference, Stop input](https://code.claude.com/docs/en/hooks#stop-input)): "The `background_tasks` and `session_crons` arrays let hooks distinguish 'session is done' from 'session is paused waiting for background work to wake it back up'. Both arrays are present when the task registry is reachable and are empty when nothing is in flight or scheduled." `runner hook report` already forwards the whole Stop payload to runnerd. Use only these documented fields. Never parse terminal output or inspect process trees (Jason, 2026-11-08).

A task notification starts the woken turn with `UserPromptSubmit`, so the member returns to Working through the existing path. Live check L1 confirms this.

## Read first

- `crates/runner-daemon/src/runtimes/claude_code/claude_status.rs`: `StatusReport`, `ClaudeParser::hook`, the `"Stop" | "StopFailure"` arm.
- `crates/runner-daemon/src/session/state/agent/mod.rs`: `AgentEvent`, `TurnState`, `AgentModel::apply`, `clear_turn`.
- `crates/runner-core/src/protocol/status.rs`: `AgentObservation` and how `program_status` stays additive.
- `crates/runner-daemon/src/ops/mission/report.rs`: `Member`, `derive`, `quiet`, and its tests.

## Deliverables

1. **Parser.** `StatusReport` gains optional `background_tasks` and `session_crons`. On `Stop` only, background work is pending when a `background_tasks` entry has one of the documented `type` labels (`shell`, `subagent`, `monitor`, `workflow`, `teammate`, `cloud session`, `MCP task`), or `session_crons` has any entry, recurring included. Any other label is ignored. When the fields are absent, as in older Claude Code versions, nothing is pending. Emit a new `AgentEvent::BackgroundWork { pending }` after `TurnEnded` in the same batch, so the other runtimes' `TurnEnded` stays unchanged.
2. **Model.** `TurnState` carries `background: bool`. `BackgroundWork` sets it. A new turn (`TurnStarted`), `StartupReady`, `SessionEnded` and `ConversationRecovered` clear it. Make sure the existing reject guards in `apply` cannot drop or strand it.
3. **Protocol.** `AgentObservation` gains `background: bool` with `#[serde(default, skip_serializing_if = ...)]`, so older clients and existing goldens are unaffected. Add a compatibility assertion beside the `program_status` one.
4. **Mission state.** `Member` gains `background`, and `derive` counts a live member with it as working. The settle clock therefore starts only after the woken turn ends, and `quiet` follows.
5. **Docs.** Update the mission-state sentence in `docs/guide/concepts/missions.md` and `docs/guide/zh-CN/concepts/missions.md`: Settled waits while a Claude Code member's background tasks or scheduled wakeups are pending. Also update the matching clause in `docs/arch/arch.md`'s "Reports to a linked chat" paragraph. Run `python3 script/check-guide-translations.py`.

**Decisions:**
- Scheduled wakeups count, including recurring ones, so a member running `/loop` keeps its mission Working until the loop stops.
- There is no time cap. #1216 will add a report for a mission with no activity for 30 minutes.

**Non-goals:**
- No change to member display in the app or `mission show`.
- No stall report (that is #1216).
- No other runtimes.
- No `StopFailure` handling: its payload has no such fields.

A turn the person cancels with Esc gets no Stop hook, so it keeps today's behaviour.

## Tests

- **Parser fixtures:**
  - A running `shell` task: pending.
  - Only an unknown label: not pending.
  - Only a cron: pending.
  - Fields absent: not pending.
  - `StopFailure`: not pending.
- **Model:** the flag is set and cleared on each event above.
- **Report:**
  - An idle member with background work keeps the mission Working past `SETTLE_WINDOW`.
  - The mission settles 30 s after the flag clears.
  - Existing tests are unchanged.
- **Gates:** run the affected `runner-daemon` and `runner-core` tests, workspace Clippy with warnings denied, and `cargo fmt --all --check`. Drive macOS and Windows CI green.

## Live checks (qa)

Use the development app from this worktree. Each test mission has one Claude Code member on an inexpensive model at low effort, with a tiny prompt, in a fresh folder under `/tmp`. Link each to a test chat qa creates (`runner-dev mission link <mission> --chat <session>`).

- **L1, the reported bug:** ask the member to start `sleep 90` with `run_in_background` and end its turn without waiting.
  - While the job runs, about 40 s and 80 s after the turn ends, `runner-dev mission show` prints `STATE working` and the chat has no `[runner] … settled` line.
  - After the job ends, the member runs a woken turn (Working, then idle).
  - About 30 s later the state is `settled`, and the chat gets exactly one settled line.
- **L2, no regression:** a member that answers a one-line prompt with no background work reaches `STATE settled` about 30 s after its turn, and the chat gets one settled line.

The verdict gives pass or fail per check, with screenshot or command-output evidence and the timestamps. The PR description records it.

## Handoffs

The coder implements and commits, then hands the candidate to `reviewer`. Must-fix findings are folded into their commits, and the coder asks for review again until it is clean. The clean review names L1 and L2. The coder then hands the reviewed commit to `qa`. A failure goes back to the coder, and the fixed candidate is reviewed again before qa reruns the affected checks.

The PR carries `risk-medium`, plus `core-review` (added automatically for session state). It keeps `needs-live-test` until L1 and L2 pass.

## Authorization

The claude trio may commit, push this branch and open one PR against `main`. It does not merge.
