# 347 — Slice 1: hook-driven status for Claude Code

Tracking issue: [#347](https://github.com/yicheng47/runner/issues/347). Spec: [347](../features/347-hook-based-session-status.md).

**Slice 1 only.** Replace the *source* of agent status for Claude Code sessions with lifecycle hooks, behind the status vocabulary that ships today. No new states, no UI change, no delivery-gate change, no Codex or TRAE adapter. Those are later slices and are explicitly out of scope — do not start them.

## Why this slice is small

The spec's target vocabulary (Working / Needs you / Ready, with lifecycle and observability states) needs UI that is designed but not built. This slice delivers the plumbing underneath it and proves the transport, exactly as #584 did: same `SessionActivityState::{Busy, Idle}`, same dot, better source.

## The mechanism already half exists

`claude_settings_args` (`crates/runner-backend/src/router/runtime.rs:171`) already injects a per-spawn hook into Claude Code through `--settings`:

- per-invocation, so nothing is written into the user's `~/.claude/settings.json` and nothing needs trusting;
- it bails out entirely if the user passed their own `--settings`;
- its `SessionStart` hook is a shell command that captures stdin atomically (`cat > tmp && mv tmp drop`) into an app-data path, for `claude_rekey`.

Extend that, do not replace it. The existing rekey hook must keep working — `claude_settings_injects_fullscreen_and_per_spawn_session_start_hook` must still pass.

## What to build

1. **Inject the status events** alongside the existing `SessionStart` entry: `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Notification`, `Stop`. Every injected hook carries an explicit short `timeout` — hooks run synchronously, and an unbounded one stalls the agent.
2. **Each hook appends one NDJSON line** to a per-session status file under the app data dir, keyed by the Runner session id. Appending is fire-and-forget: the command must return immediately and must never print JSON or exit 2, so it can never alter a turn. Reuse `shell_quote` for every path.
3. **Watch that file with `notify`**, the same machinery the event log already uses, and parse each line into a transition.
4. **Add a `"hook"` source** to `note_forwarder_transition` (`crates/runner-backend/src/session/manager/mod.rs:1120`) with top precedence, plus a `hook_status_armed` latch on `SessionState`. It suppresses the `forwarder` source the way `title_status_armed` already does at `:1128` — that guard is the precedent to copy. Once armed, silence must never demote the session: a long quiet turn is normal.
5. **Map only what this slice needs.** `UserPromptSubmit`, `PreToolUse`, `PostToolUse` → `Busy`. `Notification` with `notification_type == "idle_prompt"` → `Idle`. **Do not key Idle on `Stop`** — a `Stop` hook that exits 2, or returns `decision: "block"`, makes the agent continue, so `Stop` does not mean the turn ended. `idle_prompt` is emitted only after competing stop hooks have had their say. Ignore every other event and notification type for now.

## Non-goals

- The Working / Needs you / Ready vocabulary, the amber needs-you states, and every UI surface on the canvas.
- Any change to `reserve_delivery` or inbox delivery behaviour.
- Codex and TRAE adapters. Codex additionally needs `--enable hooks --dangerously-bypass-hook-trust`; that is a later slice.
- Removing or weakening the byte and title-spinner baseline. Both keep running underneath, untouched.

## Verification

- The existing `claude_settings_args` tests still pass, including the rekey `SessionStart` hook.
- New tests: the injected settings contain each status event with a timeout; a user-supplied `--settings` still suppresses injection entirely; each event maps to the expected state; `Stop` maps to nothing.
- A precedence test: once `hook_status_armed` is set, a `forwarder` transition is ignored, including an Idle one after silence.
- `make verify`.

## Out of the working tree

Leave all changes uncommitted in the working tree on the feature branch. Do not commit, push, open a PR, or merge.
