# 659 / 670 — a session start must not read "Status unavailable"

[#659](https://github.com/yicheng47/runner/issues/659) and [#670](https://github.com/yicheng47/runner/issues/670), both P1, milestone 0.11. They are one bug seen from two sides, so they are fixed together and closed together. Work in `/Users/jason/repos/yicheng47/runner` on the existing branch `fix/659-session-start-status`, cut from `main` at `509e493`; its first commit carries this brief. Build on it; do not rebase, squash or create another branch.

Read first: this brief; both issues end to end, including the comment on #670 recording a live measurement; `crates/runner-backend/src/session/{claude_status,codex_status,copilot_status,pi_status}.rs`; `crates/runner-backend/src/session/manager/mod.rs` around `apply_activity` and `apply_observation`; `AGENTS.md` for the conventions.

## The bug

Every hook adapter treats a lifecycle session start as *"we no longer know what this agent is doing"* and writes `Activity::Unavailable`. That is backwards. A session start is the one moment we do know: the agent has come up and is sitting at its prompt, which is `Activity::Idle`.

That single wrong value produces both reports, because the adapters only publish an observation once they are already hook-sourced:

- **#659, on an armed session.** `/clear` in Claude Code fires `SessionStart{source:"clear"}`. `self.value.source` is already `Hook`, so the `Unavailable` is published, and `hook_status_armed` then bars the PTY baseline from correcting it (`manager/mod.rs:1182`). The row is stuck until the next `UserPromptSubmit`.
- **#670, on a fresh watcher.** After an app relaunch the watcher is rebuilt per PTY spawn, so `self.value.source` is still the default and the observation is *swallowed*: nothing publishes and nothing arms. The row waits on the PTY baseline, measured at roughly twelve seconds in the #670 comment, and stays `Unavailable` forever if the resumed agent never paints anything the idle detector scores.

Two of the four adapters already hold part of the answer, which is what makes this cheap: `pi_status.rs:211` writes `Activity::Ready` on `session_start`, and `copilot_status.rs:197` ignores a `SessionStart` for a session it already owns. `claude_status.rs` and `codex_status.rs` have neither.

## Deliverable

One rule, applied to all four adapters: **a lifecycle session start publishes an idle, hook-sourced observation.**

1. **Activity.** In `claude_status.rs:109` and `codex_status.rs:118`, the session-start reset writes `Activity::Idle` instead of `Activity::Unavailable`; `outcome` and `detail` still clear. Same in `copilot_status.rs:206`. The compaction special-case above each of them is unchanged and still returns `None`.
2. **Publish and arm.** Each adapter's session-start arm currently ends in a gate equivalent to `(self.value.source == ObservationSource::Hook).then(...)` — `claude_status.rs:104,111`, `codex_status.rs:121`, `copilot_status.rs:202,208`, `pi_status.rs:215`. Replace it: set `self.value.source = ObservationSource::Hook` and publish unconditionally. Receiving the hook at all is the proof that the bridge is wired, and `manager/mod.rs:1436` turns that published source into `hook_status_armed`, so the row goes idle at once instead of waiting on the baseline.

Decide and record one thing in the handoff: whether arming this early can strand a session as idle while it works, if a runtime's session-start hook can fire when its turn hooks cannot. Read how each runtime's hook config is written by Runner before answering; if they are written in one shot, say so and accept the arming.

**pi's `Activity::Ready` stays `Ready`** — a decided non-goal, not an oversight. `Ready` and `Idle` both render as `StatusKind::Idle` (`runner-app/src/ui/agent_status.rs:77`), so changing it moves no pixel while touching a working adapter. pi changes only its publish gate.

### The traps

- **A session start must never reset a live turn.** `codex_status.rs:114` returns `None` when `self.turn_id.is_some() || self.ended`, for delayed startup and resume hooks arriving mid-turn. That guard and its Claude/Copilot equivalents stay exactly as they are; the new publish must sit after them, not before.
- **Compaction is not a session start.** Each adapter matches `source == "compact"` against its own `compacting` flag and returns `None`, preserving the turn. Untouched, and its tests must still pass unedited.
- **The existing tests encode the old value on purpose.** `codex_status.rs`'s `root_session_start_handover_is_independent_of_source_and_can_resume_an_old_session` asserts `Activity::Unavailable` across five sources, and the Claude and Copilot modules have counterparts. These assertions change to `Idle`; the handover *structure* they test — a new session id takes over, the old one's events are ignored — does not change and must keep passing.
- **`/clear` may hand over a new session id.** With this fix both paths end at `Idle`, so it stops mattering for what the user sees. Do not add session-id matching to Claude to chase it, and leave Copilot's same-session guard alone.
- **Nothing keys off the pairing of a hook source with a non-`Ready` idle.** Before settling on `Idle`, grep for readers that assume a hook-sourced observation is `Working`, `Ready` or `Unavailable` — the sidebar attention state and `completion_armed` in particular — and report what you found.

New tests, one per adapter: a fresh watcher's first session start publishes `Idle` with `ObservationSource::Hook`; a session start on an armed session publishes `Idle` rather than `Unavailable`; the compaction and mid-turn guards still publish nothing.

Out of this mission: the manager's `hook_status_armed` gate and the baseline lockout at `manager/mod.rs:1182`; the PTY idle detector's arming latency; the UI; pi's activity value; new abstractions; any change to hook installation or the hook feed; `docs/`.

## Ownership and authorization

The coder owns the change, its tests and the checks; the reviewer waits for an explicit Runner handoff, then audits the working-tree diff with one question in front: **does every changed assertion describe behaviour we want, or was it edited to make a test pass?** Iterate through Runner until no must-fix findings remain, and post the final handoff on the feed. No additional crew, nested subagents, new checkout or worktree. Commits, push, PR, merge, and launching or restarting Runner are not authorized; leave the work uncommitted on the branch. `target/debug/runner` is the GUI app on this case-insensitive volume; the CLI is `target/debug/runner-agent-cli`. Do not touch any real Runner data directory, database or agent config; the adapter tests are pure unit tests over JSON reports.

## Verification

`make verify` green, plus `cargo test --locked --workspace --no-fail-fast --profile ci`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check` and `git diff --check`. `main` is 1459 passed / 3 ignored on macOS at `509e493`; the count may only go up, and by the number of tests you added.

The live check is the user's, not yours, and the handoff must state both repro steps for them: `/clear` in a Claude session shows Idle rather than Status unavailable, and a Codex slot is idle immediately after the app relaunches instead of twelve seconds later.

## Handoff

Final Runner handoff, posted on the feed: branch and base commit; every file and function changed; the arming decision from the Deliverable with the evidence behind it; what the grep for hook-source readers turned up; every existing assertion you edited and why each new value is correct; before and after test counts; checks with results; and the reviewer's explicit no-remaining-must-fix verdict. Leave the work uncommitted.
