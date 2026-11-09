# #1224 — Claude Code leaves Working when a compaction ends

Jason asked for a claude trio to fix [#1224](https://github.com/yicheng47/runner/issues/1224) on 2026-11-09. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1224-claude-compaction-working` on `fix/1224-claude-compaction-working`, based on `origin/main` at `0c4d2e479d3226c4d416e560cf42fa93265574c8`. A separate trio is fixing #1225 at the same time in its own worktree; the two share the QA lock.

## Problem

Runner enters Working on Claude Code's `PreCompact` hook and leaves it only on `PostCompact`, a turn boundary or the once-per-turn `idle_prompt` notification. Claude Code 2.1.319 compacts idle sessions on its own about 55 minutes after a turn's `Stop` (`idleCompaction`, on by default), and Claude's compaction function fires `PreCompact` before it can fail (`too_few_groups`, `aborted`, `exhausted`, `media_unstrippable`, `error`), be skipped or be blocked by a hook, returning before `PostCompact` and `SessionStart(compact)`. An idle compaction that does not complete therefore leaves the session Working · Compacting context with nothing to clear it, and a mission waiting on that member never settles. Two mission incidents in the issue's comment match this: Working about 54 minutes after `Stop`, detail `compacting_context`, no later hook.

A second defect: on at least one path Claude fires `PostCompact` before `SessionStart(compact)`. The parser has cleared `compacting` by then, so it treats that `SessionStart` as a fresh startup (`StartupReady`, then Idle), dropping the turn's Completed outcome.

runnerd does not log hook events, so neither sequence has been captured from a real session. The issue has the full trace and Claude Code references.

## Outcome

When a compaction ends, by success, failure, skip or block, the session returns to the state it had before: Ready or Completed for an idle compaction, Working for one inside a turn. A compaction that starts outside a turn never leaves the session Working with nothing to clear it. `SessionStart(compact)` never resets the session, in either order. The daemon log shows which hook moved a session, so the next incident is diagnosable.

## Read first

- `crates/runner-daemon/src/runtimes/claude_code/claude_status.rs`: `ClaudeParser::hook` (the `SessionStart` branch and the `PreCompact`/`PostCompact` mapping), the transcript tail handling, and the test `compact_session_start_restores_manual_idle_and_in_turn_work`.
- `crates/runner-daemon/src/session/state/agent/mod.rs`: `CompactionStarted`, `CompactionEnded`, `IdlePrompt`, `Settled`, `finish_compaction` and `compaction_resume`.
- `crates/runner-daemon/src/runtimes/claude_code/mod.rs`: Claude's `StatusSemantics`.
- `crates/runner-daemon/src/runtimes/status.rs`: the `StatusSemantics` fields, and how Codex, Copilot and pi use the compaction policies, which must not change.

## Deliverables

1. **Log the hooks.** Write one line to `runnerd.log` for each hook event Runner admits for a session at a level the default log keeps: the session, the hook name, its `source`/`trigger` when present, and the activity it produced. Do not log prompts, tool input or other payload content. Keep it quiet enough for a busy mission; dropped hooks (subagent, stale session, stale prompt) may log at debug.
2. **`SessionStart(compact)` is never a startup.** Treat it as part of a compaction whether it comes before or after `PostCompact`, keeping the session id and transcript path updates. Cover both orders, idle and inside a turn, and keep the turn's Completed outcome.
3. **A compaction that never ends.** When `PreCompact` arrives and no end follows, the session must still return to its prior state. Prefer a positive end signal Claude already gives (a transcript record the existing tail can read, such as `compact_boundary` or an error/system entry, or an existing hook); if none covers the failure paths, add a bounded fallback that restores `compaction_resume` once a compaction has gone well past a real one's duration without an end event, and logs a warning naming the session. Pick the bound from evidence and justify it in the PR. Cover a failed compaction inside a turn too: a later `Stop` must still settle it.
4. **Confirm the order.** If you can do it without touching global configuration, capture Claude Code's real hook order with a throwaway run in a fresh `/tmp` folder, an inexpensive model and `--settings` pointing at a temporary file whose hooks only append the event name and source to a log: a successful `/compact` after two tiny turns, and `/compact` on a near-empty conversation, which should take a failure path. Record what you saw in the PR description. qa's live checks also capture the sequence through deliverable 1.

**Non-goals:** no change to other runtimes' compaction behavior; no UI change; no change to Claude's `idleCompaction` setting or any agent's global configuration; no mission-level warning surface unless the fallback needs one to be honest.

No guide change is expected; check `docs/guide/concepts/sessions.md` and the status pages and update them only if they describe compaction status in a way the fix changes.

## Tests

- Parser and state tests for: idle `PreCompact` with no end (fallback restores Ready/Completed); idle `PreCompact` → `PostCompact` → `SessionStart(compact)` and the reverse order (both restore Ready/Completed, no `StartupReady`); in-turn compaction in both orders (stays Working, then `Stop` completes); in-turn `PreCompact` with no end followed by `Stop` (Completed); a real `SessionStart(startup|resume|clear)` still resets.
- Each new test must fail without the fix; say which ones in the handoff.
- Gates: `runner-daemon` tests, workspace Clippy with warnings denied, `cargo fmt --all --check`. Drive macOS and Windows CI green.

## Live checks (qa)

Use the macOS development app from this worktree, with an inexpensive Claude model at low effort, tiny prompts and a fresh folder under `/tmp` for each chat. Take the QA lock as AGENTS.md describes; the #1225 trio's qa shares it.

- **L1, failed manual compaction:** in a new Claude chat with one tiny turn finished (Ready), run `/compact`. If Claude reports it could not compact, the session returns to Ready/Completed, by the end signal or within the fallback bound, and is not left Working. `runnerd.log` shows the hook lines. Reason: the failure path that sends `PreCompact` without `PostCompact`.
- **L2, successful manual compaction:** after a few tiny turns, run `/compact` and let it finish. The session shows Working · Compacting context while it runs and returns to Ready with the earlier outcome; it does not flash a fresh startup. Record the hook order from the log. Reason: the reordered `SessionStart(compact)` path.
- **L3, mission settles:** in a bounded test mission with one Claude member, have that member compact as in L1 after its task, then confirm the mission settles. Reason: the reported mission impact.
- **I1, idle compaction (not for qa):** about 55 minutes after a turn ends, an idle chat that auto-compacts returns to Ready. Jason observes this in normal use; it is not a merge gate.

The verdict gives pass or fail per check, with screenshot evidence and the captured hook order, and the PR description records it.

## Handoffs

The coder implements and commits, then hands the candidate to `reviewer`. When the review is clean, the coder hands the reviewed commit to `qa` with L1–L3. Any failure, and any change after review, goes back through `reviewer` before qa reruns the affected checks.

The PR uses `Fixes #1224`, carries `risk-medium` and `needs-live-test`; `core-review` is added automatically if the paths match.

## Authorization

The claude trio may commit, push this branch and open one PR against `main`. It does not merge.
