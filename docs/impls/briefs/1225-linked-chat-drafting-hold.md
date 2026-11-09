# #1225 — A delivered report must not leave a linked chat held as Drafting

Jason asked for a claude trio to fix [#1225](https://github.com/yicheng47/runner/issues/1225) on 2026-11-09. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1225-linked-chat-drafting-hold` on `fix/1225-linked-chat-drafting-hold`, based on `origin/main` at `0c4d2e479d3226c4d416e560cf42fa93265574c8`. A separate trio is fixing #1224 at the same time in its own worktree; the two share the QA lock.

## Problem

Runner typed a mission's `[runner] … is now linked to this chat` line into a linked Claude Code chat, and Claude Code received it and started a turn. 9 ms after the Enter the input tracker reported the line unsent (`row match`), and the attempt ended `Unverified`, so the router held the chat as `OwnLine`. When the turn ended the tracker switched to `Drafting`, and that hold never cleared on the idle chat: the mission's `NeedsYou` and `Settled` reports were never delivered, and the person missed a pending question. The issue has the full daemon-log timeline; #1203 shows the same `Drafting` hold on a Codex chat, which eventually released.

Probable cause, from the code and not yet confirmed by a test:

- `input_state.rs`, `observe_composer`: with `submitted_form` set, a row that still equals it returns `Drafting`. The line Claude just accepted stays on screen as a transcript row at the tracked row, so `delivery_owned()` reports `Unsent` (`row match`); the `cursor_at_empty_composer` guard in `observe_output` did not clear it.
- `report.rs`, `advance_chat`: the `!unchanged` branch returns `Unverified` before `turn_started` is consulted. `input_revision` differed between the pre-inject snapshot and the confirm check; `InputTracker::observe_input` bumps it on every event, including `Composing` and navigation keys, and the person had just linked the mission in the app.
- `observe_composer`, the `delivery_revision != input_revision` branch: once the tracked row changed at turn end, that bump made the tracker adopt the row as the person's own text (`submitted_form = None`, `Drafting`), which stays until the row reads empty, and an idle chat's row never does.
- `report.rs`, `readiness`: only `Unsent` is re-checked (`recheck_unsent`); `Drafting` is trusted, so nothing releases a stale draft hold.

## Outcome

- A report line the agent received and that started its turn counts as delivered (`Confirmed`, not `Unverified`), even if unrelated input events moved `input_revision` in between.
- A submitted line still visible as a transcript row at the tracked composer row is not read as unsent.
- A `Drafting` hold that no real draft supports does not hold an idle chat's reports forever: like `recheck_unsent`, it is re-checked against the composer, at least when the chat's turn ends.
- A genuine half-typed draft by the person still holds reports and inbox deliveries, as today.

## Read first

- `crates/runner-terminal/src/input_state.rs`: `InputTracker::observe_input`, `observe_output`, `observe_composer`, `delivery_owned`, `cursor_at_empty_composer`, `submitted_form`, `delivery_revision`, `input_revision`.
- `crates/runner-daemon/src/router/report.rs`: `advance_chat`, `readiness`, the hold reasons and their release.
- The inbox dispatcher's use of the same tracker and `recheck_unsent` (find it from `recheck_unsent`'s callers); keep its behavior consistent with reports.
- `crates/runner-daemon/src/ops/mission/report.rs`: queuing, which the issue found correct.
- The tests and fixtures beside these files, and #1203's discussion for the Codex variant.

## Deliverables

1. **Confirmation.** Decide whether `input_revision` should move only for events that can change the composer's content, or whether `advance_chat` should accept `turn_started` evidence despite a revision change; pick the narrower change that makes the reported sequence `Confirmed` without confirming a line the person actually edited. Explain the choice in the PR.
2. **No false `row match`.** A submitted report line that Claude has moved into the transcript is not treated as the unsent composer; keep a line genuinely left in the composer detected as unsent.
3. **Stale `Drafting` re-check.** Re-check a `Drafting` readiness against the live composer before it holds a report, at least on the chat's turn end and while the chat is idle, and release the hold (logging `report delivery released`) when the composer is empty. Apply the same to the inbox dispatcher if it shares the gap.
4. **Logging.** Keep the existing hold/release lines; add a reason to the release only if it helps the next diagnosis.

**Non-goals:** no change to report queuing or which mission states report; no fix for `runner mission link` refusing a chat outside the mission's project (the repro's step 1 is only how the link came about); no UI change.

No guide change is expected; check `docs/guide/` pages on linked chats and reports and update one only if it describes the hold behavior the fix changes.

## Tests

- Tracker and router tests built from the issue's sequence: inject, Enter, the line stays as a transcript row at the tracked row while the turn starts, an unrelated input event bumps `input_revision`, the turn ends and the row changes. The attempt is `Confirmed`, no `OwnLine` or `Drafting` hold remains, and a later report is delivered.
- A real draft typed into the composer still holds reports, before and after a turn end; a report line genuinely left unsent is still detected and resubmitted or held as today.
- Each new regression test must fail without the fix; say which ones in the handoff.
- Gates: `runner-terminal` and `runner-daemon` tests, workspace Clippy with warnings denied, `cargo fmt --all --check`. Drive macOS and Windows CI green.

## Live checks (qa)

Use the macOS development app from this worktree, with an inexpensive model at low effort, tiny prompts and a fresh folder under `/tmp` for each chat and mission. Take the QA lock as AGENTS.md describes; the #1224 trio's qa shares it. Operate only chats and missions you created.

- **L1, untouched linked chat:** start a Claude Code chat, start a small test mission whose member runs `runner ask` after a short task, and link the mission to that chat from the app. The chat receives the `Linked` line, its turn ends, and without touching the pane, the question report and later the settled report arrive in the chat. `runnerd.log` shows `outcome=Confirmed` for the linked line and no lasting hold. Reason: the reported failure.
- **L2, touched pane:** repeat L1, but click into the chat's pane and press an arrow key between the `Linked` line and the question. The reports still arrive. Reason: the input-revision bump the issue suspects.
- **L3, real draft holds:** in a linked chat, type a half-finished draft without sending it, then trigger a report. The report is held while the draft stays and is delivered once the draft is cleared. Reason: the hold must still protect the person's text.

The verdict gives pass or fail per check, with screenshot evidence and the relevant log lines, and the PR description records it.

## Handoffs

The coder implements and commits, then hands the candidate to `reviewer`. When the review is clean, the coder hands the reviewed commit to `qa` with L1–L3. Any failure, and any change after review, goes back through `reviewer` before qa reruns the affected checks.

The PR uses `Fixes #1225` and mentions #1203. It carries `risk-medium` and `needs-live-test`; `core-review` is added automatically if the paths match.

## Authorization

The claude trio may commit, push this branch and open one PR against `main`. It does not merge.
