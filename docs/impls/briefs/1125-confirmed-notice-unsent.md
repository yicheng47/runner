# #1125 — A confirmed inbox notice later reads as Unsent

Jason requested a claude trio mission (coder, reviewer, qa) on 2026-11-02. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1125-confirmed-notice-unsent` on `fix/1125-confirmed-notice-unsent`, based on `origin/main` at `51c91c7e1836c04b817a762ebd96cdd95dd8dd6d`. The root checkout stays on main. Use this worktree's own `target/`. The #1025 mission runs in parallel in its own worktree; treat it as another machine's checkout.

## Problem

[#1125](https://github.com/yicheng47/runner/issues/1125) has the full timeline and evidence. In short: on Runner 0.19.0, a Claude Code mission member's delivery was held as `Unsent` for 64 minutes. The router's previous notice had been `Confirmed` and Claude Code ran a turn on it, yet the input tracker kept reporting that notice as unsubmitted in the composer. No unconfirmed attempt backed the hold, so the dispatcher never resubmitted or re-checked. The idle agent printed nothing, so the tracker never re-read the screen. The hold cleared only when the mission view was opened and the pane redrew, which put the cursor on the empty composer.

The code reading in the issue is unverified on screen. The most likely trigger is the tracker's composer row landing on a history row that shows the submitted prompt with the same `> ` prefix as the live composer. Confirm or replace that explanation with a test before fixing.

## Read first

The issue; `runner-terminal/src/input_state.rs` (`InputTracker`: `current_reported_state`, `delivery_owned`, `unsent_notice`, `observe_output`, `observe_composer`, `cursor_at_empty_composer`); `runner-daemon/src/session/state/mod.rs` (`draft_hold`, `unsent_notice`); `session/manager/{mod.rs,bookkeeping.rs}` (`delivery_readiness`, `delivery_snapshot`, `resubmit`); `router/dispatch.rs` (attempt outcomes, `Unsent`, `Hold`). `dispatch.rs` changed after 0.19.0 in #1102 and #1103; build on `main`. Also #1075 and #1018 for the earlier stale-hold fixes and their tests.

## Deliverables

Start with a failing test that reproduces the hold from the Claude Code screen shape: a router notice typed and submitted, a confirmed turn, the submitted prompt left in history at the tracker's composer row, then an idle agent with no output. Readiness must currently report `Unsent` with no unconfirmed attempt.

1. **Confirmation ends delivery ownership.** When the router confirms a delivery, the tracker must stop treating that notice as the router's unsent text. A history row that repeats it must not produce `Unsent`.
2. **No indefinite hold without an unconfirmed attempt.** When the session reports a finished turn or idle with unread messages, and an `Unsent` hold has no unconfirmed router attempt behind it, re-check the composer or drop the hold so the router delivers. A genuinely unsubmitted router notice (#753, #1075) must still be detected and resubmitted, never stacked.
3. **Traceable.** Log at info level when the tracker starts reporting `Unsent` and when it stops, with session and the reason (row match, ownership cleared, re-check). Do not log screen contents.

Non-goals: no change to Codex or other runtimes' delivery behavior beyond what the shared tracker fix requires, no new settings, no UI change. The issue's visibility request (`mission show` and repeated warnings) stays in the issue.

## Validation

Tie each check to the behavior it covers. In-process: the reproduction above passes with the fix; a confirmed delivery followed by a history row equal to the notice reports `Idle`; an idle session with unread messages and a stale `Unsent` hold is delivered; an unconfirmed attempt whose text really stays in the composer is still `Unsent` and resubmitted once; the existing #753, #1018 and #1075 regression tests still pass. Run the affected `runner-terminal` and `runner-daemon` tests with locked dependencies, `cargo fmt --all --check`, workspace Clippy with warnings denied, and `git diff --check`. Drive the required CI green.

Live check (qa, after the clean code review, on the same candidate), in the isolated development app under Test Scope and driven through `runner-dev` where possible: start a bounded test mission with one Claude Code member on an inexpensive model and low effort in a fresh `/tmp` folder. Send several messages that each start a short turn while the member's pane is not shown, then post again and confirm delivery happens without opening the pane, and no `Unsent` hold appears in the daemon log. Also confirm ordinary delivery to a Codex member is unchanged. Record results in `docs/tests/1125-confirmed-notice-unsent.md`. If the original screen shape cannot be reproduced live, say so and rely on the in-process reproduction; do not mark it passed.

## Authorization and handoff

Jason authorized this three-agent claude trio mission: implementation, review, qa live checks in the development app, commits, pushing this branch, and opening one PR against main. coder implements and owns all commits; reviewer reviews the working-tree diff first; qa runs the live check after a clean review. Fold this brief into an implementation commit. Fold review and CI fixes into the commits they correct and use `git push --force-with-lease` when rewriting pushed history. Do not add tool-specific co-author trailers. The PR may close #1125 with a closing keyword only if qa's live check passes; otherwise link it without one.

Stop at an open PR with green CI and qa's verdict. Report through Runner with the PR URL, final SHA, changes, validation and any live checks not run. Do not merge, release, cut a nightly, archive the mission, delete branches or remove worktrees. When waiting for a teammate or Jason, finish your turn and rely on the pushed inbox notification.
