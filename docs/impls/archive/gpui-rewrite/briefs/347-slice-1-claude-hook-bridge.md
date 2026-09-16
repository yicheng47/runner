# 347 — Slice 1: hook-driven status for Claude Code

Tracking issue: [#347](https://github.com/yicheng47/runner/issues/347). Spec: [347](../../../../features/archive/347-hook-based-session-status.md).

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

1. **Inject the status events** alongside the existing `SessionStart` entry: `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Notification`, `Stop`, `StopFailure`. Every injected hook carries an explicit short `timeout` — hooks run synchronously, and an unbounded one stalls the agent.
2. **Each status hook appends one NDJSON line** to a per-session status file under the app data dir, keyed by the Runner session id. Appending is fire-and-forget: the command must return immediately and must never print JSON or exit 2, so it can never alter a turn. Reuse `shell_quote` for every path.
3. **Watch that file with `notify`**, the same machinery the event log already uses, and parse each line into a transition.
4. **Add a `"hook"` source** to `note_forwarder_transition` (`crates/runner-backend/src/session/manager/mod.rs:1120`) with top precedence, plus a `hook_status_armed` latch on `SessionState`. It suppresses the `forwarder` source the way `title_status_armed` already does at `:1128` — that guard is the precedent to copy. Once armed, silence must never demote the session: a long quiet turn is normal.
5. **Map only what this slice needs.** `UserPromptSubmit`, `PreToolUse`, `PostToolUse` → `Busy`. `Stop` and `StopFailure` → `Idle`. `Notification` with a real stdin `notification_type == "idle_prompt"` → `Idle` as secondary confirmation. Keep the `^idle_prompt$` matcher as a cheap first filter, but never fabricate the type: Claude skips that matcher when a notification has no type. Ignore every other event and notification type for now. This mapping supersedes the original brief and mission prohibition on `Stop`, following Jason’s review decision on 2026-09-14.

## Non-goals

- The Working / Needs you / Ready vocabulary, the amber needs-you states, and every UI surface on the canvas.
- Any change to `reserve_delivery` or inbox delivery behaviour.
- Codex and TRAE adapters. Codex additionally needs `--enable hooks --dangerously-bypass-hook-trust`; that is a later slice.
- Removing or weakening the byte and title-spinner baseline. Both keep running underneath, untouched.

## Verification

- The existing `claude_settings_args` tests still pass, including the rekey `SessionStart` hook.
- New tests: the injected settings contain each status event with a timeout; a user-supplied `--settings` still suppresses injection entirely; each event maps to the expected state; `Stop` and `StopFailure` map to Idle; a `Stop` followed by `PreToolUse` ends Busy.
- Exercise the real hook scripts with missing, empty, and non-idle notification types, misleading notification text, and large stdin payloads. Every event drains stdin; no status hook prints JSON or returns exit 2.
- Status files and scripts are cleaned on session teardown, failed setup, and app startup after a crash. Preserve the rekey hook’s original failure signal, with an explicit short timeout.
- A precedence test: once `hook_status_armed` is set, a `forwarder` transition is ignored, including an Idle one after silence.
- Interrupt recovery: Ctrl+C and exact bare Escape make a running hook turn reach Idle without a turn-end hook; Escape cancelling a dialog returns to Busy on the following `PreToolUse`; buffered tool records drain before the interrupt Idle; a later `PreToolUse` can restore Busy. Baseline sessions and arrow/paste escape sequences remain unaffected.
- `make verify`.

## Out of the working tree

Leave all changes uncommitted in the working tree on the feature branch. Do not commit, push, open a PR, or merge.

## Review decision: turn boundaries

Review against Claude Code 2.1.270 confirmed that `Notification(idle_prompt)` waits for `messageIdleNotifThresholdMs` without interaction (60,000 ms by default), is suppressed while loading or showing a dialog, and is disabled when that setting is zero. It is an idle notification, not a reliable immediate turn boundary. Depending on it alone would delay every Idle transition and could leave Busy latched for a whole session.

Jason explicitly changed the slice’s mapping after that finding: `Stop` and `StopFailure` enter Idle, with `idle_prompt` retained as secondary confirmation. Another stop hook can continue the turn; a following `PreToolUse` restores Busy. Slice 1 accepts that temporary wrong dot because Busy/Idle gates no delivery behavior. It adds no error state, so a failed turn also ends Idle. Silence still never demotes an armed hook source.

Claude publishes no interrupt hook and does not emit `Stop` for interrupted turns. A successful Ctrl+C or bare Escape PTY write sets a flag shared with the hook watcher. The monitor drains buffered hook records first, then emits `input-interrupt` (Ctrl+C) or `input-escape` (Escape) Idle on the same output channel, avoiding a buffered tool Busy arriving after Idle. The signal belongs in the common PTY write path: terminal input currently arrives through `send_bytes`; named `C-c`/`Escape` and programmatic interrupts would use the same path if introduced, although production named-key callers currently send only Enter. Detection requires exactly one `b"\x03"` or `b"\x1b"` write, consistent with the existing exact Enter detection. Bracketed paste and larger writes containing those bytes do not count; if terminal input later batches an interrupt with adjacent bytes, the bridge will miss it.

Jason explicitly approved bare Escape as a deliberate, bounded display inference on 2026-09-14. The manager accepts it only for an armed hook session currently Busy. It is not a proven turn end: Escape can cancel an `AskUserQuestion` dialog, briefly showing Idle before the next `PreToolUse` restores Busy. The signal arms nothing and does not clear unresolved input or interaction state. The byte/title baseline is unchanged. Slice 2’s `Answer needed` state is what will represent the dialog correctly.

Ctrl+C clears the completion notification latch, preventing a turn-finished badge for a turn the user interrupted, including Ctrl+C after provisional Escape Idle. For direct chats, Escape does not trigger completion processing and preserves its own latch while Idle is provisional; the next accepted status resolves it. A runtime Idle following provisional Idle is emitted even though the dot stays the same, so a real completion still reaches the notification path. Other sessions can record their own completions without consuming this provisional session’s latch. Completion consumption keeps one session lock at a time; holding multiple member locks would introduce a new lock-ordering requirement. The existing two-state tab snapshot can therefore report another member’s completion while an Escape-cancelled dialog turn is continuing; it must not indefinitely withhold that member’s badge when Escape really interrupted the turn. Mission status goes to the event log and does not run this direct-chat completion path.

Static inspection of Claude Code 2.1.270 indicates interrupted tools emit `PostToolUseFailure`, whose documented payload includes `is_interrupt`; Runner does not inject that event. This is an inference from the installed binary’s event catalog, not an executed interrupt trace. No trailing-tool suppression is implemented: already-buffered events are ordered before interrupt Idle, and new `PreToolUse` still restores Busy.

## Later

- Consider Claude command-hook exec form to avoid an extra shell per event, after settling the minimum supported Claude version and verifying native Windows shell behavior. Keep the existing shell-form mechanism in this slice.
- Consider replacing the status path environment variable with an explicit `SpawnSpec` field when that API next changes.
- Verify TRAE’s `idle_prompt` against its installed binary before building its adapter. Its documentation claims immediate turn completion without a threshold; the Claude finding means that claim needs the same implementation-level check.
