# #1102 and #1103 — Recover a stuck busy, and deliver to working members on runtimes that queue mid-turn input

Jason approved this mission on 2026-11-01: the `claude pair` crew (coder on Claude Code, reviewer on Codex) with a `qa` role (Codex) added for the live checks. He folded #1103 into it the same day, after launch. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1102-submit-without-turn` on `fix/1102-submit-without-turn`, based on `origin/main` at `b59ddf2a`. The root checkout stays on `main`; this worktree uses its own `target/`.

Issues: [#1102](https://github.com/yicheng47/runner/issues/1102) (P0, a regression from #1085) and [#1103](https://github.com/yicheng47/runner/issues/1103) (P1). Read both in full; #1102 holds the evidence. Related: #1046, #1085, #753, #1075, #1018.

## Problem

`SessionEvent::Input { submitted: true }` on an idle session sets the activity to `Busy` with `StatusSource::InputSubmit` in `SessionModel::apply`. A hook-armed session returns to idle only on a later hook. When the submit starts no turn (an empty composer, a key the agent ignores, a submit that never reaches the agent), nothing ever clears it. Since #1085 the router delivers a notice only on `DeliveryReadiness::Ready { idle: true }`, so the member's inbox is blocked indefinitely, and silently: `Dispatcher` treats `Ready { idle: false }` as no hold, so the wait is neither logged nor warned about.

A person pressing Return in an idle agent's pane is normal use and must never leave the member undeliverable.

## Outcome

1. **Recover.** A submit-busy that sees no turn start within a bounded time reverts to the session's last hook state and publishes idle, whatever produced the submit. The model has no timer today: add the smallest timing mechanism that fits the session manager's existing clock (`session::clock`) and test seams. Decide, and state in the PR, what counts as a turn start for each runtime: hook-armed Claude Code and Codex at least, and check what non-hook-armed sessions already do. Terminal output alone is not a turn start, because Codex redraws its composer on an ignored Enter. Choose the bound so a slow-starting real turn is not flipped to idle, and make a turn-start signal that arrives after the revert put the session back to busy.
2. **Surface.** A busy hold with unread messages is logged when the hold reason changes, as the other holds are, and a stuck busy posts the existing two-minute feed warning, as draft holds do. The warning is for a stuck busy, not a long turn: an ordinary turn that runs past two minutes with messages waiting must not post it. If existing state cannot tell the two apart, ask Jason before choosing.
3. **Investigate.** Check whether Codex's TUI kept accepting input after that submit (the issue's rollout shows local typing from 09:56 to 10:01 and no submission). If it is a Codex-side problem, put the evidence in the final handoff; do not file an issue without Jason.

4. **Deliver mid-turn where the runtime queues input (#1103).** Since #1085 every notice waits for idle, so a Claude Code member in a long turn sees no correction until it ends, although Claude Code queues input typed during a turn and shows it to the agent at its next tool call. Record per runtime, in `crates/runner-core/src/protocol/runtime_metadata.rs`, whether its composer reliably accepts and submits input during a turn. Claude Code qualifies; Codex and every runtime not shown to do so reliably (pi, Copilot, Antigravity, Grok) keep idle-only delivery. For a qualifying runtime, a working member is ready for delivery again; draft holds, recent typing, Needs you, startup and in-flight still block as today, so no notice is typed into a composer the person is using. Confirm the attempt's submit confirmation and unsent detection still hold when the target is already busy. A busy member on a mid-turn runtime is not a hold, so item 2's busy hold and warning apply only to idle-only runtimes.

Out of scope: a Settings override for mid-turn delivery (UI, needs a design first), hook-based mid-turn delivery (#1046's later work), and qualifying further runtimes. Beyond the #1103 runtime-metadata capability, no UI or protocol change is expected. If the work needs one, stop and ask Jason.

## Read first

- `crates/runner-daemon/src/session/state/mod.rs`: `SessionModel::apply` (`Input`, `Delivered`, `InputWriteFailed`), the status publication path, `hook_status_armed`, `provisional_idle`.
- `crates/runner-daemon/src/session/manager/status.rs` and `events.rs`: how model effects reach the router.
- `crates/runner-daemon/src/router/dispatch.rs`: `Dispatcher::advance` and `Dispatcher::hold`; `router/mod.rs`: `DeliveryReadiness`; `session/manager/bookkeeping.rs`: the `Ready { idle }` mapping.
- `crates/runner-core/src/protocol/runtime_metadata.rs`: per-runtime capabilities.
- Tests: `session/state/tests.rs`, `session/manager/tests/{input,status,codex}.rs`, `router/tests/mod.rs`.

## Validation

In-process tests, at the narrowest layer that covers each behavior:

- The issue's case: an idle hook-armed session gets a submitted input and no hook; after the bound it is idle again and the queued notice is delivered.
- A real turn whose turn-start signal arrives within the bound stays busy; one that arrives after the revert puts the session back to busy.
- A long, hook-confirmed turn with unread messages logs the busy hold and posts no warning; a stuck busy posts exactly one warning after two minutes.
- Cover Claude Code and Codex separately wherever their turn-start signals differ.
- #1103: a notice to a working Claude Code member is attempted while it is busy; one to a working Codex member waits for idle; draft, recent-typing, Needs you, startup and in-flight holds still block both.

Gates: `cargo fmt --all --check`, `git diff --check`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and the relevant crate tests while iterating, then `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`. The required macOS and Windows CI checks must be green.

Live checks (qa, on the reviewed SHA): in a development-app test mission with one Codex member and one Claude Code member (inexpensive model, low effort, tiny prompts, a fresh folder under `/tmp`), press Return on each idle member's empty composer from the keyboard (local input, not a router delivery), then message that member. Pass when the message is delivered within the bound and the member shows idle afterwards. Record whether Codex accepts typed input after the empty Return (item 3). For #1103, run the MIS-MSG cases from that issue: a message to a busy Claude Code member is submitted and read before its turn ends; a message to a busy Codex member still waits for idle; a draft in the composer still holds both. Record the run in `docs/tests/1102-submit-without-turn.md` on this branch. Update `MIS-MSG-01` in `docs/tests/regression/missions.md`, whose expected result says no notice is typed mid-turn, to the per-runtime behavior.

The #1097 mission (codex trio, with its own qa) is running at the same time and may be using the development app. `make run` stops the development daemon, which would end its test sessions. Before launching, check whether a development app or daemon you did not start is running; if one is, ask Jason instead of stopping it.

## Roles and handoffs

- **coder** implements and runs the in-process checks, then hands the reviewer a commit SHA with the changed behaviors and the checks that cover them.
- **reviewer** reviews the code only. This is a P0 on the core delivery path; look hardest at whether the revert can fire during a real turn, races with `Delivered` and the `InputWriteFailed` checkpoint, `activity_revision` handling, the published status against `agent_status.observation`, timer cleanup on exit and respawn, warning noise, and for #1103 whether a mid-turn attempt can be typed into a composer the person is using or be misread as confirmed or unsent. Every finding, including nits, is fixed in this PR. A clean review names the live checks and sends `@qa` the SHA.
- **qa** runs the live checks above on that SHA and reports pass or fail with evidence to the coder and reviewer. A code change after a failure goes back through review before qa tests again.

## Authorization and handoff

Jason authorized implementation, validation, commits on this branch, pushing it, and opening one PR against `main` that closes both #1102 and #1103. The `core-review` label is added automatically; never remove it. Add exactly one risk label, chosen from the diff. The #1102 fix and the #1103 feature may be separate commits if each builds and passes on its own. Fold this brief into an implementation commit, with each subject naming the change. Fold review, qa and CI fixes into the commit they correct and push with `git push --force-with-lease`; never plain force, never merge `main` into the branch.

Stop at an open PR with green CI and a qa pass. Report through Runner with the PR URL, final SHA, the changes, the turn-start rule and bound you chose, the runtimes marked for mid-turn delivery, validation results, the qa verdict and the Codex input finding. Do not merge, release, cut a nightly, archive the mission, delete branches or remove worktrees. When waiting for a teammate or Jason, finish your turn and rely on the pushed inbox notification.
