# #783 cancellation-status validation — 2026-10-02

Independent QA on reviewed candidate `9d0cccc1` passed the exercised macOS #783 cancellation and recovery flows for Claude Code, Antigravity and Copilot, plus the Codex control. Pi cancelled and recovered; its Response failed label matches its native error and Jason accepted that behavior. Native Windows and a Copilot approval remaining open after Esc are still unverified live. Earlier baseline failures, blocked runs and Jason's manual findings remain recorded below; the final section contains the new matrix and evidence.

## Candidate, scope and authorization

Worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/fix-783-cancel-status`, branch `fix/783-cancel-status`. Both runs have HEAD `db5a5041f754473613d0ee460e81b13c1ac82434`. The baseline working tree was clean. The fixed candidate was uncommitted: `copilot/copilot_status.rs`, `session/pty_runtime.rs`, `session/manager/output.rs` and `session/manager/tests/input.rs` under `crates/runner-backend/src/`. Its saved diff SHA-256 is `9a8532d15a3c24740d7e1e6737ad2c0f2f899086b7b475649621a9f3ad1f0feb`. QA changed only this report; implementation remained frozen.

Reviewer reported `NO REMAINING MUST-FIX ISSUES` at 07:30:26 UTC. Coder requested phase 4 through Runner at 07:31:32 UTC. The [mission brief](../impls/briefs/783-cancel-status.md) authorizes bounded development chats using existing sign-ins, inexpensive models, low effort and canonical `/private/tmp` paths. No authentication, global configuration, permission-mode changes, additional agents or SQLite access occurred. This is the feature-specific cancellation matrix, not a complete lifecycle smoke test.

At 07:10:21 UTC, Jason accepted the partial baseline and instructed QA to move the outstanding Antigravity, Copilot, Codex and pi checks to phase 4, explicitly labelled fix-only. He also requested the Finder `open` workaround for native control and early or streamed cancellation for Antigravity. Native Windows remains pending for Jason.

## Environment and method

macOS 26.6.2, arm64; Runner app `0.12.6 (dev)`, CLI `0.12.6`. Versions were captured for both runs and unchanged:

| Runtime | CLI version | Baseline override |
| --- | --- | --- |
| Claude Code | 2.1.287 | `sonnet`, low effort |
| Antigravity CLI | 1.2.14 | `gemini-3.8-flash-low`; effort encoded in model |
| Copilot CLI | 1.0.90 | No baseline chat created |
| Codex CLI | 0.159.3 | No baseline chat created |
| pi | 0.99.2 | No baseline chat created |

Each run built this worktree with `RUNNER_RECORD_INPUT_FIXTURE=<evidence>/rec make run`. CLI and app compilation succeeded. Computer use required a temporary `.app` wrapper with byte-identical copies of the built binaries; hashes are retained. Development status was independently verified through `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/bin/runner status --json`, reporting the development data namespace and `mcp.sock`. Installed Runner and pre-existing chats/missions were not operated.

Baseline native actions, screenshots, running/cancel/settled JSON, hook feeds, hook payloads and native transcript copies are retained. The recorder captures raw terminal output and classified input, **not outbound key bytes**. Claude pushed `CSI > 5 u`; Antigravity pushed `CSI > 1 u`. Native Esc records as `navigate`. Encodings `ESC[27u` and `ESC[99;5u` are inference from mode flags and `mappings.rs`, accepted by coder, rather than captured writes. No fixed-runtime kitty evidence was obtained because native access failed before chat creation.

## Baseline matrix — untouched build

Evidence: `/private/tmp/runner-783-baseline-xe_mzrmt/evidence`, including `baseline-partial.md`, `candidate.json`, `versions.json`, `ledger.json` and `rec.<session-id>.ndjson`.

| Check | Result | Observed evidence or limitation |
| --- | --- | --- |
| Claude ordinary ACK | Passed | Correct native ACK; `claude-first.json` reports Ready/Completed. |
| Claude tool/waiter Esc | Blocked | Harness rejected standalone foreground `sleep 60`. Background sleep and Monitor returned asynchronously. A Python attempt was rejected tool use and raced an earlier task completion; excluded from confirmed reproduction. Hooks/transcripts retained. |
| Claude streamed-response Esc | Failed | Terminal displayed Interrupted; Runner stayed Working with no outcome and no interactions. |
| Claude recovery | Blocked | Native controller lost window access before recovery. |
| Antigravity ordinary ACK | Passed | Correct native ACK; `agy-first.json` reports Ready/Completed. |
| Antigravity tool Esc | Blocked | Three bounded waits backgrounded after about 10 seconds. Esc did not show confirmed cancellation; background tasks later completed. Working while work continued is not counted as reproduction. |
| Antigravity fresh Ctrl+C | Blocked | Fresh wait and running state captured; window access failed before Ctrl+C was sent. Task completed normally. |
| Copilot approved-tool two-Esc | Blocked | Not run; moved to fix-only phase 4 by Jason. |
| Codex cancellation control | Blocked | Not run; moved to fix-only phase 4 by Jason. |
| pi cancellation control | Blocked | Not run; moved to fix-only phase 4 by Jason. |
| Native Windows | Skipped | Pending Jason's machine. |
| Cleanup | Passed | Created chats stopped/archived; test app and collector stopped. |

Confirmed Claude reproduction: after an ordinary ACK and completion of earlier bounded background tasks, send `Write a 1000-word essay about how rain forms, beginning QA783-STREAM. Do not use tools. Keep writing until the essay is complete.` Press one native Esc during visible streaming. The terminal stops mid-word and displays `Interrupted · What should Claude do instead?`. Expected Runner Ready/Interrupted; actual hook observation Working, outcome null, interactions empty at 06:56:22 UTC, 06:56:50 and 07:05:49. The last hook is UserPromptSubmit; no later Stop arrives. This isolates the symptom from tool-owner correlation. See `claude-stream-cancel.png`, `claude-stream-{running,cancel,settled,pre-relaunch}.json` and matching hook/transcript files. The baseline failure remains Failed after cleanup.

The proposed input-path cause is consistent with recorded kitty flags and the legacy-only matcher at baseline `crates/runner-backend/src/session/pty_runtime.rs:1295`; outbound bytes were not recorded. Historical Copilot evidence was inspected by coder separately: [#777](777-runtime-adapter-smoke.md) and its original native transcript contain correlated `permission.completed` approval followed by `user_abort`, without execution completion for the cancelled wait. This run did not independently reproduce that Copilot baseline.

## Fixed-build matrix — reviewed dirty candidate

Evidence: `/private/tmp/runner-783-fixed-l0f5w5ne/evidence`, including `candidate.diff`, `candidate.json`, `bundle-hashes.json`, `dev-wrapper-status.json`, `native-blocker.json`, `versions.json` and `qa-checks.json`.

| Required check | Result | Coverage and limit |
| --- | --- | --- |
| Claude streamed Esc and recovery | Blocked | No fixed-build native input; regression passes automatically. |
| Claude bounded foreground waiter | Blocked | Existing harness limitation plus native-control blocker. |
| Antigravity early Esc and recovery — fix-only | Blocked | No fixed-build chat created. |
| Antigravity fresh Ctrl+C and recovery — fix-only | Blocked | No fixed-build chat created. |
| Copilot approved tool, two-Esc and recovery — fix-only | Blocked | No native permission/completion, execution or abort evidence captured. |
| Copilot Esc while approval remains open, correlated resolution and recovery — fix-only | Blocked | Automated owner-correlation regression passes; live pending-approval boundary unverified. |
| Codex cancel and recovery control — fix-only | Blocked | No fixed-build chat created. |
| pi cancel and recovery control — fix-only | Blocked | No fixed-build chat created; #784 Response failed behavior remains out of scope. |
| Native Windows | Skipped | Pending Jason; CI is not native UI evidence. |
| Build and development endpoint | Passed | Both dev builds finished; wrapper app returned development status. |
| Targeted QA automated regressions | Passed | Three tests below, each exit 0. |
| Cleanup | Passed | Empty fixed-run ledger, collector stopped, own app stopped, endpoint exit 3. |

Native control failed before the harmless New chat action or any fixed-build test chat. Finder workaround attempted exactly with `open '/private/tmp/runner-783-fixed-l0f5w5ne/Runner 783 Fixed.app' --env RUNNER_RECORD_INPUT_FIXTURE=/private/tmp/runner-783-fixed-l0f5w5ne/evidence/rec` (exit 0). Computer-use binding by exact app path and unique bundle ID both returned `cgWindowNotFound` (-10005), although inventory reported that app running and its development CLI responded. During baseline, reconnecting, controller reset and one scoped relaunch had also failed. No alternative UI input mechanism was used and no CLI-only result counts as a native pass.

## Automated checks

Coder-supplied final check metadata: `/private/tmp/runner-783-checks.Mb09mY/checks.json`; reviewed diff: `candidate-reviewed-fix.diff` there. QA inspected metadata and final test summaries. These are coder-run checks, separate from QA's independent targeted executions:

| Command | Exit | Evidence |
| --- | --- | --- |
| `cargo test --locked -p runner-terminal --profile ci` | 0 | `1.log`; test groups 82, 11, 1 and 9 passed; one ignored |
| `cargo test --locked -p runner-backend --profile ci` | 0 | `8.log`; 1060 passed |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 | `9.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | 0 | `10.log` |
| `cargo fmt --all --check` | 0 | `11.log` |
| `git diff --check` | 0 | `12.log` |

An earlier coder Clippy run exited 101 before correction; final candidate reruns above passed. QA independently ran the following against the reviewed working tree; each selected one regression and exited 0, with logs and commands in fixed-run `qa-checks.json`:

```sh
cargo test --locked -p runner-backend --profile ci approved_wait_cancellation_without_tool_completion_closes_its_approval
cargo test --locked -p runner-backend --profile ci permission_completion_resolves_only_the_correlated_approval
cargo test --locked -p runner-backend --profile ci claude_streamed_kitty_escape_without_stop_allows_a_recovery_turn
```

These tests prove the simulated event/input paths, including correlated permission completion and recovery, but cannot establish actual runtime emission, kitty negotiation or native interruption on the fixed app. No PR or CI run existed during this QA handoff.

## Cleanup and remaining work

Baseline created only Claude chat `01M3XP29672JRJASMRH8EAKDQD` and Antigravity chat `01M3XPB6YEV8A0TJNSAG14JK7P`; both show stopped state and archive timestamps in cleanup JSON. All bounded tasks ended. The fixed-run ledger is empty: no chats, missions, crews or roles were created. Both runs stopped their hook-payload collectors and own development apps; scoped process checks found none remaining, and development status returned exit 3. Native quit was unavailable, so only the identified test-app processes were terminated. Scratch directories, temporary wrappers, native conversation history and evidence were retained.

At the QA handoff, no observed fixed-build must-fix product finding was established because live actions were blocked. All required fixed-build live cancellation/recovery rows and Copilot's pending-approval boundary were unresolved. Jason's subsequent manual findings and results are recorded below; they do not change the historical QA matrix. Native Windows, plus Shift+Enter and Alt chords in pi, remain Jason's checks. Raw evidence stays local because screenshots and native transcripts may include account identifiers; this document contains no such content.

After this blocked-QA handoff, Jason explicitly authorized publication with `Refs #783` at 08:07:13 UTC on 2026-10-02 and chose to run the live checks himself: Claude Esc mid-reply and during a tool waiter; Antigravity Esc and Ctrl+C; Copilot two-Esc after approving a tool; Esc while a Copilot approval remains pending, which must stay represented until answered; and Codex/pi cancellation controls. Recovery remains part of each runtime's validation. The PR can switch to `Fixes #783` after his confirmation; this authorization does not change the Blocked results above or authorize a merge.

## Jason's manual follow-up on the published candidate

Jason reported that Claude Esc mid-reply reaches Idle, but Esc during the `sleep 30` tool attempt shows Claude's Interrupted prompt while Runner shows Status unavailable. Read-only inspection of his development session on 2026-10-02 confirmed hook-owned `activity: unavailable`, `outcome: interrupted`, no interactions and no work detail. The transcript has a Bash tool-use record followed at 08:09:55 UTC by its correlated `tool_result` with `toolDenialKind: user-rejected`; no later `turn_duration`, Stop or tool-completion hook was present. This is a Failed tool-cancellation status check on initial PR candidate `431c710e`, not a pending approval or lost hook source. It also establishes tool rejection/cancellation rather than proof that the sleep process had started.

The follow-up fix settles a correlated rejected tool to Ready/Interrupted immediately when no other interaction remains. The previous watcher waited for `turn_duration`, which this live cancellation never emitted. Other unresolved interactions remain Unavailable/Interrupted. The existing transcript-resolution test now expects Ready at the result rather than only after `turn_duration`; that expectation changed specifically because of this live evidence. A sanitized Bash regression covers the missing-duration case, another unresolved question and recovery. Reviewer reported `NO REMAINING MUST-FIX ISSUES` for this two-file follow-up at 08:21:58 UTC, independently passing all 31 Claude status tests and the diff whitespace check.

On 2026-10-02, Jason confirmed: "I self-tested the claude tool calling one, that one is fixed," and authorized proceeding with the PR update. This is his manual live confirmation of the reviewed follow-up, rather than a QA-controlled rerun; no new recorder or native transcript evidence was supplied for the retry.

| Manual check | Result | Source and limit |
| --- | --- | --- |
| Claude streamed-reply Esc | Passed | Jason reported Idle on the initial published candidate. |
| Claude tool cancellation | Passed | Jason confirmed the reviewed follow-up fixes his tool-calling cancellation. |
| Claude recovery | Pending | No manual confirmation yet. |
| Antigravity Esc/Ctrl+C, Copilot approved-tool/pending-approval checks and recovery | Pending | No manual confirmation yet. |
| Codex/pi cancellation and recovery controls, Windows and pi keyboard checks | Pending | No manual confirmation yet. |

Coder reran the full backend suite (1061 passed), workspace Clippy with and without updater, formatting and diff whitespace checks; all exited 0. Commands match the automated-check table above; follow-up logs are `13.log` through `17.log` in `/private/tmp/runner-783-checks.Mb09mY`. `cargo build --locked --workspace` also exited 0 and rebuilt the development binaries without launching the app. The original published candidate's macOS and Windows CI passed; those results do not cover the follow-up. CI for the updated candidate is tracked on [PR #788](https://github.com/yicheng47/runner/pull/788). Keep `Refs #783` until the remaining affected-runtime live checks are confirmed.


## Independent QA rerun — 2026-10-02, 08:38–09:04 UTC

Coder requested this rerun through Runner at 08:38:20 UTC after Jason explicitly requested continued live QA. Candidate HEAD was `9d0cccc1d7d388e7f2d404604a0778ea55d28835` on `fix/783-cancel-status`, with a clean working tree before testing. Reviewer had cleared the Claude follow-up at 08:21:58 UTC. HEAD remained unchanged through cleanup; QA edited only this report afterward, without committing. This run exercises the reviewed implementation independently of Jason's earlier manual confirmations.

Evidence root: `/private/tmp/runner-783-rerun-tee2rgr7/evidence`. `candidate.json`, `build.json`, `bundle-hashes.json`, `ledger.json`, `status-matrix.json`, `recorder-summary.json`, the runtime screenshots and boundary JSON/hook/transcript files retain the evidence. Raw account-bearing evidence remains local. QA independently ran `cargo build -p runner-app -p runner-cli` (exit 0), then opened a temporary, byte-identical development `.app` wrapper with the Finder `open` workaround. Native control succeeded. The absolute development CLI verified the endpoint and captured session states; no database was opened.

The initial wrapper inherited `NO_COLOR=1`, explaining the monochrome Claude terminal Jason noticed. Before any test prompt, QA stopped its empty Claude chat, quit its own wrapper and relaunched with `env -u NO_COLOR open ... --env RUNNER_RECORD_INPUT_FIXTURE=.../rec`. Scoped process environment evidence and native screenshots confirm colors returned. Authentication, global environment and agent configuration were unchanged. The existing empty chat resumed with a new native key before its first prompt. Because the recorder uses `create_new`, its old filename could not be reopened after relaunch; QA repeated the two Claude cases in a fresh sixth scratch chat to retain complete classified-input evidence. Both original and repeated results are preserved.

The machine and runtime versions match the earlier environment table. Models were Claude `sonnet`/low, Antigravity `gemini-3.8-flash-low`, Copilot `gpt-5-mini`/low, Codex `gpt-6.1-sol`/low and pi `deepseek/deepseek-flash`/low. Copilot stayed in Manual Approval; QA selected its one-command Yes option once, without persistent approval. Six direct chats used their own scratch directories under `/private/tmp/runner-783-rerun-tee2rgr7`; no missions, roles or crews were created.

### Native validation matrix

| Check | Result | Observed behavior and evidence |
| --- | --- | --- |
| Ordinary ACK on all five runtimes | Passed | Native replies and completed states captured. Codex's first Return entered a newline during fast input; a separate Return submitted the same prompt, then `codex-first-settled.json` confirmed completion. |
| Claude streamed Esc and recovery | Passed | One Esc during visible streaming produced the native Interrupted prompt and Runner Idle · Interrupted. Running, cancel and settled JSON show Working → Ready/Interrupted with no interactions; recovery produced the requested marker and Ready/Completed. Both original `claude-stream-*` and repeated `claude-recorded-stream-*` evidence retained. |
| Claude Bash tool rejection/cancellation without turn duration, then recovery | Passed | During the displayed Python wait, one Esc produced Interrupted and Runner Ready/Interrupted immediately, with no remaining interaction. Correlated rejected tool results arrived without a later turn-duration or Stop record before recovery. Original `claude-tool-*` and repeated `claude-recorded-tool-*` evidence retained; exact recovery replies completed normally. This proves the rejected-tool status path, not that the Python process started or was killed. |
| Antigravity fresh early Esc and recovery | Passed | Esc was recorded 8.259 seconds after submission, before the 10-second async threshold, while the native tool showed Running command. Terminal Interrupted and Runner Ready/Interrupted agreed; recovery completed. See `agy-esc-{running,cancel,recovery}.json` and native screenshots. No separate pre-recovery settled snapshot was saved for this attempt. |
| Antigravity fresh early Ctrl+C and recovery | Passed | A second bounded wait received one Ctrl+C 9.146 seconds after submission, before async handoff. Terminal Interrupted and CLI cancel/settled Ready/Interrupted agreed; exact recovery completed. `agy-ctrlc-recovery.json` captured work in progress; `agy-ctrlc-recovery-settled.json` is the completed result. |
| Copilot approved tool, two-Esc and recovery | Passed | Real Manual Approval dialog, one-command Yes, matching approval completion, Working tool state, then two Esc. Native Operation aborted/cancelled by user and Runner Ready/Interrupted agreed, with no stale approval; recovery completed. `copilot-approved-*` includes all boundaries and transcript correlation below. |
| Copilot real pending approval through its matching cancellation, then recovery | Passed | The visible dialog and `copilot-2` interaction existed before Esc. Copilot resolved that request itself with a correlated `permission.completed` cancellation, then emitted abort. Runner cleared the interaction and showed Ready/Interrupted; recovery completed. See `copilot-pending-*`. |
| Copilot approval still open after Esc, before completion evidence | Blocked | This runtime immediately cancels the real dialog on Esc. There was no live interval with an open runtime approval after Esc to observe. The passing owner-correlation regression covers delayed completion; this run does not claim a native pass for that interval. |
| Codex streamed cancellation and recovery control | Passed | One Esc during visible streaming produced Conversation interrupted, native `turn_aborted`, and Runner Ready/Interrupted; recovery completed. `codex-stream-*` and `codex-recovery.*` retained. The recovery marker included an extra final period; status recovery passed, without claiming exact output fidelity. |
| Codex initial tool-wait cancellation attempt | Skipped | Excluded: `sleep 30` returned asynchronously and the agent completed the turn before Esc. Native pre-Esc screenshot was already Idle. `codex-excluded-completed-before-escape.json` records the race; it is not cancellation evidence. |
| pi cancellation labelled Interrupted | Failed | Native Command aborted/This operation was aborted settled to `stopReason: error`, and Runner Ready/Failed displayed Response failed. This is the #784 control behavior, explicitly out of scope. Jason accepted that label during this run because it matches pi's error. The failed Interrupted expectation is retained; acceptance and successful recovery do not rewrite it. |
| pi recovery | Passed | Exact recovery marker and Ready/Completed captured in `pi-recovery.*`; the earlier failed outcome cleared on the new turn. |
| pi Shift+Enter and Alt+B input | Passed | An unsent draft visibly became two lines with Shift+Enter; Alt+B moved the cursor to the second line's start. One Ctrl+C cleared the draft. No agent turn was submitted and Ready/Completed remained unchanged. `pi-shift-enter.png`, `pi-alt-b.png`, `pi-keyboard-cleared.*` retained. Other Alt chords were not tested. |
| Native Windows | Skipped | No Windows machine controlled by QA; still pending Jason. Builds or CI do not count as native UI evidence. |
| Cleanup | Passed | All six own chats stopped and archived, conversation keys/history retained, own app quit natively and collector exited 0. No captured own process remained; development status exited 3. |

### Runtime records and input evidence

The original Claude tool record is `toolu_01J7zStpWTJjxSatF8ALsAmR`, followed at 08:47:08.512 UTC by an error `tool_result` with `toolDenialKind: user-rejected`. The recorded repeat uses `toolu_012AW3ZBDoHrVrKQLrFgxjfk`, followed at 09:02:22.711 UTC by the same rejection type. Neither cancellation snapshot contains a later `turn_duration` for that tool turn; earlier completed-turn durations are unrelated. Both remain Ready/Interrupted through the saved settled boundary, then recover normally. See original transcript lines 51–53 and repeated lines 44–46. Foreground-process termination and the historical backgrounded Monitor waiter are not independently established by these rejection records.

Copilot's approved wait uses tool owner `call_g5zCqiZcxn5Z9w8Y1hfjMU2M` and request `a7f54a4a-11b9-425d-8a37-aa8a873cb6fe`. `copilot-approved-settled-transcript.jsonl` lines 52, 57, 60 and 62 show execution start at 08:51:39.390 UTC, permission request, correlated approval completion at 08:51:50.236 UTC and `user_abort` at 08:52:01.024 UTC. No execution-completion record for that cancelled owner appears, including in the final transcript. Its interaction had already closed on the approved completion, and it did not reappear after abort.

The separate pending dialog uses owner `call_jcaK3ryybjT3pu8FXKBx39aK` and request `7ff575bf-8b9c-413a-a150-0c7fe2d172dd`. `copilot-pending-settled-transcript.jsonl` lines 126, 131, 134 and 136 show request at 08:52:34.651 UTC, matching `permission.completed` with kind `cancelled`/Session aborted at 08:52:41.177 UTC, then `user_initiated` abort five milliseconds later. This supports closure on actual completion evidence; it cannot demonstrate a dialog that stays open after Esc without emitting completion.

Codex's native rollout contains `turn_aborted` at 08:54:44.564 UTC for the streamed control. Pi's final transcript lines 9–10 contain the aborted tool result and assistant `stopReason: error`/This operation was aborted. Pi's status adapter maps error to Failed and aborted to Interrupted at `crates/runner-backend/src/runtimes/pi/pi_status.rs:270`; this observed error record explains the header. During the focused chat, `failed_since` was null while outcome remained Failed. The sidebar rolls up the unseen-failure flag (`crates/runner-app/src/ui/agent_status.rs:729`), which viewing clears (`crates/runner-backend/src/session/manager/mod.rs:1336`); the header instead reads the retained outcome (`crates/runner-app/src/ui/agent_status.rs:72`). This explains Jason's sidebar question and is not an observed stale-status bug.

| Runtime | Recorded kitty negotiation | Native functional input |
| --- | --- | --- |
| Claude, fresh recorded chat | `CSI > 5 u` | Two Esc events classified navigate, with both recovery submissions retained. |
| Antigravity | `CSI > 1 u` | Esc classified navigate; Ctrl+C classified cancel, on separate fresh waits. |
| Copilot | `CSI = 1 ; 1 u` | Approved-wait two Esc events 35 ms apart; separate pending-dialog Esc. |
| Codex | `CSI > 7 u` | Confirmed streamed-turn Esc; earlier idle Esc excluded. |
| pi | `CSI > 7 u` | Tool Esc, multiline draft, Alt navigation and draft-clear Ctrl+C. |

Recorder output is base64-encoded terminal data, decoded for `recorder-summary.json`. It records classified native input, not outbound PTY key bytes. Expected Esc `ESC[27u` and Ctrl+C `ESC[99;5u` remain inference from flags and the mapping implementation, not captured writes. No global configuration was changed to force a runtime event sequence.

### Cleanup and conclusion for this rerun

`cleanup.json` records stopped state and archive timestamps for original Claude `01M3XWD52463XBJ8GQNX2RTSM3`, Antigravity `01M3XWPQN3617MGVQAKHMJZ6Q0`, Copilot `01M3XWR08TE3PTWQ9RNVE3P2HW`, Codex `01M3XWS703PD5BBRR0M9Q8RGW7`, pi `01M3XWY5M8C60W128CPB9P2B41` and recorded Claude `01M3XXG4ZSD8B9N5ACT5363A1G`. Only those objects were stopped/archived. The native cleanup screenshot shows their rows gone; the own wrapper quit with Cmd+Q. `cleanup-app.json` records endpoint exit 3 and no remaining PID from the captured own process tree. The payload collector exited 0. Scratch paths, wrappers, hook evidence and native conversation history remain available. Installed Runner and pre-existing chats/missions were not operated.

No remaining must-fix QA issue was observed in #783's exercised macOS flows. This is a scoped result: the open-after-Esc Copilot interval and native Windows remain unresolved live, actual foreground-process termination was not proven for Claude's rejected tool, and pi's Failed control remains recorded with Jason's acceptance. The baseline failure and earlier blocked matrices remain unchanged. QA leaves PR wording, publication and any issue closure to coder/Jason; no merge is authorized by this report.

Reviewer reported `NO REMAINING MUST-FIX ISSUES` for this report at 09:09:55 UTC. At 09:10:06 UTC, Jason authorized amending the report into the PR and switching its body to `Fixes #783`, because Claude, Antigravity and Copilot all passed live. He explicitly retained the limits: native Windows pending, Copilot's still-open-after-Esc interval unreachable in this run because the runtime immediately emits cancellation, pi's Failed outcome tracked by #784, and outbound bytes inferred rather than captured. This supersedes the earlier `Refs #783` publication wording and does not authorize a merge.
