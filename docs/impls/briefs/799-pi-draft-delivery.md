# 799 — pi inbox delivery submits an unsent draft

Fix [#799](https://github.com/yicheng47/runner/issues/799). Jason requested this mission on 2026-10-04 as the next 0.12 bug, after #787. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-799-pi-draft-delivery`, on the existing branch `fix/799-pi-draft-delivery`, created from `origin/main` at `11026afd`. The root checkout stays on main. Do not create another branch or worktree, and do not share a Cargo target directory. Other worktrees under `.worktrees/` belong to other work; treat them as another machine's checkout.

## What is known

1. **Reproduced live and by replay.** QA on #791 PR 3 typed a draft, and once a single `x`, into an idle pi mission slot's composer without Return. A crew message then went out about two seconds later with the draft prepended and submitted. The recording replays to one observation, `Idle composing=false visible=true`, in both the pre-#796 and current detectors. Evidence is listed in the issue; the recording `01M4183Z8DEN41HPCAGW14ZBWS.generation-1.ndjson` and the triage harness under `/private/tmp/qa791p3/` still exist on this machine.
2. **Why the screen detector misses it.** pi's draft starts at column zero with an empty prompt prefix. `looks_like_prompt_prefix` (`runner-terminal/src/input_state.rs`, about line 589) rejects an empty prefix, so the tracker stays Probing, which is reported as Idle. The delivery gate (`runner-backend/src/session/state/mod.rs`, about lines 285–305) holds only for an observed Drafting, or for recent input within two seconds.
3. **pi can say what is in its editor.** pi 1.0.2's extension API (`@earendil-works/pi-coding-agent`, `docs/extensions.md` "UI and modes", and the `.d.ts` types) offers `ctx.ui.getEditorText()` and `ctx.ui.onTerminalInput(handler)`. Runner already loads its own pi status extension (`runtimes/pi/pi_status.rs`), which writes events Runner reads. QA's run used pi 0.99.2; this machine now has pi 1.0.2. Whether those APIs exist in 0.99.2 is unknown.

## Preferred direction, to confirm in phase 1

Have Runner's pi extension report the editor's empty or non-empty state when it changes, as a native draft signal, and let the session model treat that as authoritative for pi, the way hook status outranks byte heuristics. This is preferred over a screen-detector change: an empty prefix cannot be told apart from ordinary output by bytes alone. Do not make Probing plus the raw pending-input latch hold delivery. That latch clears only on Return or Ctrl+C, so a draft deleted with Backspace would hold every later message, which is the #766 failure.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions; `docs/tests/full-smoke-test.md` (QA's procedure); issue #799.
- `runtimes/pi/pi_status.rs` (the extension source, its events and how Runner parses them), `runtimes/pi/mod.rs`, `session/state/` (`SessionModel`, input observations and the delivery gate) and the replay corpus under `session/fixtures/` (README, `scenarios/pi/draft-delivery.ndjson` and its expectation).
- `runner-terminal/src/input_state.rs` (`match_prompt_prefix`, `looks_like_prompt_prefix`), only to understand what the screen detector reports for pi.
- `docs/tests/archive/791-session-state-reducer.md` (PR 3 Phase C and the attribution section) and `docs/arch/runtime-integration.md`.

## Phases

1. **Probe (coder, no repository changes).** In a disposable pi session under `/private/tmp`, with a throwaway extension loaded the way Runner loads its own, confirm that `onTerminalInput` and `getEditorText` see typing, Backspace to empty, paste, multi-line input, a submit, and an external `setEditorText` or programmatic clear. Record when each fires and what it reports. Find the lowest pi version that has these APIs from pi's changelog, and decide what Runner does on an older pi: fall back to today's behavior, never hold forever. Post the findings and the proposed design to Jason through Runner before changing code.
2. **Fix (coder).** Implement the confirmed design. The extension reports a draft change only on transitions, not on every key. The adapter turns it into an input observation the reducer already understands, or a minimal new event if none fits. A pi draft holds a crew message; clearing it releases the held message exactly once without submitting the user's text. Other runtimes and pi's existing status, outcome and conversation-key behavior stay unchanged.
3. **Tests (coder).** Extend the existing compact replay corpus: update or add pi scenarios for a draft, a single-character draft, a draft cleared with Backspace, and a submit. Use the existing compact golden format and keep `expectations/` within its current size budget; do not add a new golden format or pretty-printed JSON. Add a unit test for the extension event parser. Keep every other golden byte-identical, and list any changed rows in the PR.
4. **Verification (QA).** The baseline is already reproduced and recorded under #791, so QA does not repeat it. On the reviewed branch, in the development app with a single-lead pi test mission: type a full draft and wait, then post an addressed message; it must stay held. Clear the draft with Backspace; the message must be delivered once, and the draft text must not be submitted. Repeat with a single `x`. Submit a typed prompt normally, then post a message; it must be delivered after the turn as before. Run one Claude Code or Codex control with a draft, which must behave as before. Write `docs/tests/799-pi-draft-delivery.md` with the matrix, pi and Runner versions, candidate SHAs and evidence locations, and nothing private. Native Windows is pending for Jason.

## Live-test authorization

The coder may run bounded pi probes in phase 1 with the existing sign-in, inexpensive models or no model turn at all, and disposable directories under `/private/tmp`. Load the throwaway extension only for those probe processes, never through pi's global settings. QA may build this worktree and run its development app with `make run`, using development data only, and drive it with computer use and with `runner-dev` by its absolute path. QA may create one test crew and the roles it needs, named with a `qa799-` prefix, and run bounded test missions on them. If a development app is already running when QA starts, ask Jason through Runner instead of quitting it. Do not touch pre-existing chats, missions, crews, roles or pi sessions, the installed Runner app, authentication, pi's global settings and extensions, or permission settings, and do not open Runner's SQLite database. Afterwards, stop and archive QA's test missions, delete the `qa799-` crew and roles, quit the dev app QA started, and keep the evidence directory. The reviewer does not run the app or pi. pi probe processes are test subjects and must not delegate work. No other agents, crews or subagents.

## Review, verification and authorization

The reviewer waits for an explicit Runner handoff. It then reviews the full branch diff against #799 and the phase 1 findings, posting must-fix findings first with file:line pointers. Its focus: a real pi draft is never typed over; a held message is delivered exactly once after the draft clears; an older pi without the APIs is never stuck holding; other runtimes behave as before. Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`, then run QA's phase 4.

Run these checks and record each exact command with its exit code:
- `cargo test --locked -p runner-backend --profile ci`
- `cargo test --locked -p runner-terminal --profile ci`
- workspace Clippy: `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- the macOS updater Clippy, with `--features updater`
- `cargo fmt --all --check`
- `git diff --check`

Gate imports and helpers used only by `cfg(unix)` tests with `#[cfg(unix)]`; Windows CI keeps failing on them.

After clean review and QA, Jason authorizes the following:
- Squash all work on this branch, this brief included, into one commit on top of current `origin/main`, with a subject that names the fix.
- Push `fix/799-pi-draft-delivery` and open a PR against main. Use `Fixes #799` only if QA verified the hold and release live. Otherwise use `Refs #799` and say what is left.
- If main has moved, rebase; never merge main into the branch.
- Amend review and CI fixes into the same commit and push with `git push --force-with-lease`.
- Drive CI green on macOS and Windows.
- Do not merge, delete the branch or worktree, or cut a nightly or release.

Final Runner handoff: the PR URL, the design and the phase 1 evidence, what changed, tests and exit codes, QA's matrix, the CI result, the reviewer's verdict, and what Jason should still check (native Windows). Then stand by.
