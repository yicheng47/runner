# 766 — Crew messages held at an idle Codex slot's empty composer

Fix [#766](https://github.com/yicheng47/runner/issues/766). Jason requested this mission on 2026-10-03, ahead of #791. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-766-idle-codex-delivery`, on the existing branch `fix/766-idle-codex-delivery`, created from `origin/main` at `6bbba43d`. The root checkout stays on main. Do not create another branch or worktree, and do not share a Cargo target directory. Other worktrees under `.worktrees/` belong to other work; treat them as another machine's checkout.

## What the code shows (read 2026-10-03, not yet proven live)

1. **A held message waits with no timer.** `reserve_delivery` (`runner-backend/src/session/manager/mod.rs`, about line 953) returns `PendingInput` for an observed `Drafting`, or for no observation plus `local_input_pending`. The router (`router/mod.rs`, about lines 719 and 1077) queues the message with `pending_input_blocked` and schedules no retry. Only a `SessionDeliveryEvent` retries it, in practice `InputCleared` on a `Drafting` → `Idle` transition. A false `Drafting` therefore holds until the person types, which matches the issue's timeline.
2. **The composer detector can read an empty composer as a draft after Enter.** `InputTracker` (`runner-terminal/src/input_state.rs`) knows the composer only by screen row and prompt prefix. After a submit, `observe_composer` returns `Drafting` when the tracked row still reads exactly the submitted text, which is meant for an Enter that did not submit. Codex prints a submitted prompt into its transcript with the same `› ` marker as the composer (`runner-terminal/fixtures/codex-title-working.snapshot.txt` shows `› hi` above the composer). If that transcript line lands on the tracked row, or `relocate_row` (the nearest row that starts with the prefix) picks it, the empty composer reads as a draft. Once output stops, nothing re-evaluates it.
3. **A composer the detector cannot find stays `Drafting`** with `composer_visible = false`, and `reserve_delivery` ignores visibility.
4. **`local_input_pending` counts only with no observation.** The terminal reports an initial observation when it attaches (`runner-terminal/src/terminal.rs`, about line 528), so this path is probably inert for a session the app has attached. Verify this rather than assume it.
5. **Not in the issue:** a delivery stuck `in_flight`, or a ticket mismatch (`mod.rs`, about line 947), holds every later message to that slot as `InFlight`.

What caused the 2026-09-30 occurrence is not established. Runner logs no reason for a hold. Replaying the one real Codex recording in the repo (`codex-title-working.ndjson`, Codex 0.154.0) through the tracker was inconclusive: that version's animated composer background kept the tracker from finding the echo at all, so it stayed `Idle`. Current Codex must be recorded.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions; `docs/tests/full-smoke-test.md` (QA's procedure); issue #766.
- `runner-terminal/src/input_state.rs`, `terminal.rs` (`report_input_state` call sites, `observe_input`, `reset_input_state`), `fixtures.rs` (`RUNNER_RECORD_INPUT_FIXTURE` records output bytes and classified input) and `tests/input_state_replay.rs`, which replays every `fixtures/input-*.ndjson` against its `.expected.txt`.
- `runner-backend/src/session/manager/mod.rs` (`reserve_delivery`, `report_input_state`, `finish_delivery`), `session/manager/output.rs` (`classify_local_input`, `update_local_input_state`), `router/mod.rs` (the outbox, `pending_input_blocked`, `blocked_transition`, the retry paths), `router/handlers.rs` (`message_nudge`) and `event_bus/mod.rs` (inbox read watermarks).
- `runner-app/src/surfaces/mission_workspace/` for the blocked-inbox pill (#336).

## Phases

1. **Baseline, before any code change (QA).** The coder hands QA the baseline first and makes no edits until QA reports; it may read the code meanwhile. QA builds the untouched branch and starts the dev app with `RUNNER_RECORD_INPUT_FIXTURE=<evidence-dir>/rec make run`. QA starts a bounded test mission whose crew has a Codex slot, then follows the issue: type a short prompt straight into the Codex slot's terminal, let the turn finish, and post `runner-dev msg post --to <handle>`. A nudge that has not landed within 15 seconds counts as held. Run it on a fresh slot whose screen is not yet full, and again after a long reply has filled the screen. Also run it with a prompt typed while Codex is still working, which queues it. Run the same steps once on a Claude Code slot as a control. For each attempt, record held or delivered, whether the blocked-inbox pill showed, the slot's status, and the recording file. Post the matrix and evidence paths to the coder. If nothing holds after these attempts, say so plainly. Do not keep trying variations without asking Jason through Runner.
2. **Diagnose (coder).** Replay QA's recordings through `InputTracker`, through a scratch fixture or test, and find the transition that left the slot `Drafting`, or show that the hold came from somewhere else. Name the cause, with evidence, in a Runner message to Jason before changing behavior.
3. **Fix (coder).**
   - **Log every hold and release** to `runner.log` with its reason: `Drafting`, and whether the composer was visible; `local_input_pending`; `HumanInteraction`; `InFlight`; `RecentlyTyping`. Log only when the reason changes, not on every retry. Log each release with its trigger. This ships even if nothing else does.
   - **Fix the confirmed cause, and only that.** For the transcript copy in item 2 above: a turn that starts after a local Enter proves the submit landed, and a transcript line is not the composer. Use whichever signal the recording supports, and prefer the smallest change. A real unsent draft must still hold a delivery. Do not add a timeout that could type into a real draft, and do not restructure session status, which is #791's work.
   - **Do not deliver a stale nudge.** When a held `[inbox]` nudge is released, skip it if the recipient's read watermark already covers that message, because the slot ran `runner msg read`.
   - If the baseline did not reproduce the hold, ship the logging and the stale-nudge check, and leave the detector alone unless one of QA's real recordings shows the faulty transition. Otherwise report to Jason before changing it.
4. **Tests.** Add replay fixtures cut from QA's recordings as `runner-terminal/fixtures/input-codex-*.ndjson` with expected transitions. Trim each to the frames that matter and sanitise it: scratch paths only, and no account identifiers, usage figures or unrelated conversation text. Also add router or session tests for hold reasons, the release trigger and the stale-nudge skip. Keep existing behavior tests passing, and change one only with a stated reason. These fixtures become the first scenarios of #791's recorded corpus, so name them by runtime and scenario.
5. **Verification (QA).** On the reviewed branch, rerun every baseline row, the Claude control and two draft checks. In the first, type text into the Codex slot without submitting it, then post a message: it must stay held, with the pill. In the second, clear that draft: the message must then be delivered once, and no nudge may arrive for a message already read. Write `docs/tests/766-idle-codex-delivery.md` with dated baseline and fix matrices, CLI versions, candidate SHAs and evidence locations, and nothing private. Native Windows is pending for Jason.

## Live-test authorization

Jason authorized QA to reproduce and verify live. QA may build this worktree and run its development app with `make run`, using development data only. QA may create one test crew, plus a test role if no suitable Codex or Claude Code role exists, both named with a `qa766-` prefix. It may run bounded test missions on them with the existing sign-ins, inexpensive models, low effort and scratch directories under `/private/tmp`. QA drives the app with computer use and drives `runner-dev` by its absolute path. If a development app is already running when QA starts, ask Jason through Runner instead of quitting it. Do not touch pre-existing chats, missions, crews or roles. Do not touch the installed Runner app, authentication, global agent configuration or permission settings, and do not open Runner's SQLite database. Afterwards, stop and archive QA's test missions, delete the `qa766-` crew and role, quit the dev app QA started, and keep the evidence directory. The coder and reviewer do not run the app. No other agents, crews or subagents.

## Review, verification and authorization

The reviewer waits for an explicit Runner handoff. It then reviews the full branch diff against #766 and QA's evidence, posting must-fix findings first with file:line pointers. Its focus: no real draft is ever typed over; no message is lost or delivered twice; the log stays quiet when nothing changes; Claude Code and the other runtimes behave as before. Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`, then run QA's phase 5.

Run these checks and record each exact command with its exit code:
- `cargo test --locked -p runner-terminal --profile ci`
- `cargo test --locked -p runner-backend --profile ci`
- workspace Clippy: `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- the macOS updater Clippy, with `--features updater`
- `cargo fmt --all --check`
- `git diff --check`

Gate imports and helpers used only by `cfg(unix)` tests with `#[cfg(unix)]`; Windows CI keeps failing on them.

After clean review and QA, Jason authorizes the following:
- Squash all work on this branch, this brief included, into one commit on top of current `origin/main`, with a subject that names the fix.
- Push `fix/766-idle-codex-delivery` and open a PR against main. Use `Fixes #766` only if QA reproduced the hold and the fix cleared it live. Otherwise use `Refs #766` and say what is left.
- If main has moved, rebase; never merge main into the branch.
- Amend review and CI fixes into the same commit and push with `git push --force-with-lease`.
- Drive CI green on macOS and Windows.
- Do not merge, delete the branch or worktree, or cut a nightly or release.

Final Runner handoff: the PR URL, the root cause with evidence, what changed, tests and exit codes, QA's matrices, the CI result, the reviewer's verdict, and what Jason should still check (native Windows, and a real crew mission that he steers by typing into a Codex slot). Then stand by.
