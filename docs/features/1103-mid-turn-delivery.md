# 1103 — Deliver inbox notices to working members on runtimes that queue mid-turn input

> Tracking issue: [#1103](https://github.com/yicheng47/runner/issues/1103). Priority: P1. Platforms: macOS, Windows and Linux.
> Status: in development with [#1102](https://github.com/yicheng47/runner/issues/1102) on `fix/1102-submit-without-turn`; one pull request closes both.
> Related: #1046, #753, #1075, #1018.

## Motivation

Since 0.19.0 ([#1085](https://github.com/yicheng47/runner/pull/1085), closing #1046) every inbox notice waits until the recipient is idle: an attempt requires `DeliveryReadiness::Ready { idle: true }`. The change targeted Codex, where a notice typed during a turn can sit unsubmitted in its composer (#753, #1075). The cost lands on every runtime: a Claude Code coder implementing for half an hour sees no correction sent five minutes in, although Claude Code queues input typed during a turn and shows it to the agent at its next tool call.

## Behavior

- Each runtime records whether its composer reliably accepts and submits input during a turn (`RuntimeMetadata::queues_input_mid_turn` in `crates/runner-core/src/protocol/runtime_metadata.rs`). Claude Code does. Codex and every runtime not shown to do so (pi, Copilot, Antigravity, Grok, Cursor, OpenCode, Trae, shells) keep idle-only delivery.
- For a runtime that does, a Busy member is `DeliveryReadiness::Working` and a new message is attempted at once. Startup, an unsent draft, recent typing, Needs you and an in-flight delivery still hold, so no notice is typed into a composer the person is using.
- Redelivery of an unacknowledged backlog stays idle-only for every runtime: the 30-second reconcile with its two-minute backoff sets `wanted` only for an idle member.
- The attempt is unchanged. Confirmation is a turn-start report or a composer that no longer shows the notice; an Enter the agent ignored leaves Runner's own notice tracked as unsent, and the usual resubmit rules apply.
- A working member of such a runtime is not a hold, so the busy hold logging and warning from #1102 apply only to idle-only runtimes.

## Out of scope

- A Settings override ("Deliver messages to working agents"), which needs a design first.
- Hook-based mid-turn delivery without the composer (#1046's later work).
- Qualifying further runtimes. A runtime qualifies only after a live run shows its composer submitting input mid-turn reliably.

## Guide pages affected

`docs/guide/concepts/missions.md` and its zh-CN counterpart.

## Validation

In-process: readiness mapping per runtime with every hold still blocking, the dispatcher attempting a Working member once and leaving redelivery idle-only, and the confirm and unsent outcomes under a Working target. Live: the MIS-MSG cases on Claude Code and Codex in `docs/tests/regression/missions.md`, recorded in `docs/tests/1102-submit-without-turn.md`.
