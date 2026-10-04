# #787 — Antigravity working-directory validation

## Result — 2026-10-04

NO REMAINING MUST-FIX QA ISSUES for the authorized macOS matrix. The untouched baseline reproduced the wrong directory in all six required runs. The reviewed fix passed three sole-lead missions, three direct chats and the direct chat with a user-supplied added workspace: each marker landed exactly once in the requested cwd, the process cwd was correct, hooks reported Working then completed, and the expected ACK was visible in the native development app. Native Windows remains Skipped, pending Jason; this is not a Windows live-test pass or a complete cross-runtime smoke test.

## Candidate and environment

Both phases used branch `fix/787-antigravity-mission-cwd`, candidate HEAD `b25bc7b328f090f4c581bcd49e52d9217ecf9a1a`. The baseline working tree was clean. Fix validation used the reviewer's clean working-tree handoff at 04:09:25 UTC, before an implementation commit or PR. SHA-256 of the exact bytes from `git diff -- crates` was `df9ff4619688e8323116884ef54a5277a5c1ce6da64b8c984423df72567ddf04` before and after live validation. The saved diff and environment record identify the 12 reviewed implementation/test files. This report is the only QA repository edit.

Environment: macOS 26.6.2 (25G83), arm64; Runner 0.12.8 development build; Antigravity CLI 1.2.16; `gemini-3.8-flash`, effort `low`, existing sign-in. Missions inherited the development app's existing bypass setting; direct roles used default permissions. QA did not change authentication, global hooks, global configuration or permission settings, and did not open Runner's database.

For each phase, no development app was running at the start, verified with development status exit 3 and process inventory. QA built this worktree with `env -u NO_COLOR -u RUNNER_CREW_ID -u RUNNER_MISSION_ID -u RUNNER_HANDLE -u RUNNER_EVENT_LOG make run`. Both Cargo builds completed successfully. QA deliberately terminated the bare app to run the full-smoke procedure's disposable native `.app` wrapper, so the long-running `make run` command ended with exit 2/SIGTERM; this was not a compiler failure. The wrapper used byte-identical binaries from this worktree and remained separate from the installed app.

The reviewed Runner binary hash was `9b37b1fedf4b838e0f15aed9245a20a0202a31fc3081df8400f13b9d9f3af544`. `build.json` records the CLI sidecar hash and wrapper paths. The final installed development sidecar was verified byte-identical to the reviewed `target/debug/runner-agent-cli` (`sidecar-hash.json`). A wrapper sidecar naming mistake was corrected by relaunching QA's own app before any test objects were created.

All development commands used `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/bin/runner` with inherited mission identity removed. Structured status confirmed the development endpoint `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/mcp.sock` and outside-mission identity before creating test objects. QA crew coordination used the mission's ordinary `runner` command.

## Authorization and method

The [mission brief](../impls/briefs/787-antigravity-mission-cwd.md) authorized bounded Antigravity subjects, one temporary QA crew per phase, the required `qa787-` roles, development data, build and scoped cleanup. The subjects were instructed to execute one terminal command, reply with the unique ACK, and remain idle without delegation, messages, file inspection or settings changes. At 03:49:55 UTC Jason explicitly authorized direct-chat instructions in each temporary role's prompt, delivered by Antigravity's automatic `-i` first turn, with native ACK recorded Blocked if computer use was unavailable. The coder's 04:10:00 UTC handoff explicitly requested phase 5 after clean review.

Each run had a fresh canonical `/private/tmp` cwd and unique marker. The command used a relative filename from `RUNNER_HANDLE`, without supplying a tool workdir: `python3 -c "import os; from pathlib import Path; p=Path(os.environ['RUNNER_HANDLE']+'.txt'); f=p.open('a'); f.write('<unique marker>\n'); f.close()"`. QA independently compared the requested cwd, hooks folder, mission bus folder where applicable, actual file contents, `lsof -a -p <own agy PID> -d cwd -Fn`, process argv, structured session status and native terminal output. A visible ACK alone was not used to establish correct placement or status.

Mission status history was captured with `mission feed <own-id> --all --json --oldest-first`; direct status was sampled with `session show <own-id> --json`. The custom-workspace case also retained a read-only copy of its own hook payload before Runner consumed it. No hook implementation or frozen expectations were changed by QA to collect evidence.

## Untouched baseline

Baseline evidence: `/private/tmp/runner-qa787-baseline-woy0zfl0/evidence`. `baseline.md` and `baseline-matrix.json` retain the full initial handoff; `commands.ndjson` and individual command JSON files retain exact CLI arguments, timestamps, stdout, stderr and exit codes. The runs occurred on 2026-10-04, before the coder's implementation.

| Required row | Marker in requested cwd | Process cwd | Hook Working | Hook completed | Native ACK |
| --- | --- | --- | --- | --- | --- |
| Sole-lead mission 1 | Failed — hooks | Passed | Passed | Passed | Passed |
| Sole-lead mission 2 | Failed — hooks | Passed | Passed | Passed | Passed |
| Sole-lead mission 3 | Failed — hooks | Passed | Passed | Passed | Blocked |
| Direct automatic chat 1 | Failed — hooks | Passed | Passed | Passed | Blocked |
| Direct automatic chat 2 | Failed — hooks | Passed | Passed | Passed | Blocked |
| Direct automatic chat 3 | Failed — hooks | Passed | Passed in targeted repeat | Passed | Blocked |
| Native Windows | Skipped — pending Jason | Skipped | Skipped | Skipped | Skipped |

Expected: the unique line appears exactly once in `<requested-cwd>/<handle>.txt`. Observed: every required baseline marker appeared exactly once in the development `antigravity-hooks/<handle>.txt`, and was absent from its requested cwd, while `lsof` confirmed the requested process cwd. The issue therefore reproduced independently of the coder's probes. The relevant launch surface is `crates/runner-backend/src/runtimes/antigravity/mod.rs`, which registers the hooks folder with `--add-dir`.

Native coordinate actions later failed with `noWindowsAvailable` while the app process and endpoint remained available; screenshots eventually became unavailable. Rebinding, raising/centering, refreshing screenshots and resetting the binding did not restore native control during baseline. No Runner cause was established. The first multiline `typeText` attempt was excluded because it submitted separate queued messages; a fresh typed retry was never submitted. Both were stopped and archived. Native-control gaps remain Blocked in this historical matrix.

Baseline direct chat 3 completed before its first status sample. Its targeted fresh repeat observed hook Working then ready/completed and also wrote into hooks. Both the original placement failure and the repeat remain recorded. Mission 1's separately authorized read-only diagnostic visibly printed the hooks cwd and confirmed its marker existed there (`mission-1-diagnostic.png`).

## Coder's mechanism probes

The coder ran 33 controlled probes on the untouched candidate, three per variant, using stand-in hooks and one command printing/saving `os.getcwd()`. All 33 completed. Under Jason's explicit approval, only the exact disposable cwd trust dialogs were accepted; `/private/tmp/runner-787-probes-ZfQBFZ/trusted-paths.txt` records those paths, which remain trusted as requested. Two initial untrusted setup attempts are excluded from the controlled matrix.

| Variant | Requested cwd | Hook completed |
| --- | ---: | ---: |
| No added directory | 3/3 | 3/3 |
| Hooks only | 0/3 | 3/3 |
| Hooks then bus | 0/3 | 3/3 |
| Bus then hooks | 0/3 | 3/3 |
| Cwd then hooks then bus | 0/3 | 3/3 |
| Hooks then cwd then bus | 0/3 | 3/3 |
| Typed first turn, hooks then bus | 0/3 | 3/3 |
| Native context, hooks only | 3/3 | 3/3 |
| Native context, hooks then bus | 3/3 | 3/3 |
| Native context, typed first turn, hooks then bus | 3/3 | 3/3 |
| Hooks-folder rules naming cwd | 3/3 | 3/3 |

Hook `workspacePaths` contained the requested cwd and every added folder, with varying order between events. The model explicitly supplied the hooks folder as `run_command.Cwd` in the failing variants; that argument matched actual `os.getcwd()`. This establishes default-workspace ambiguity while retaining the process cwd. It does not establish an internal agy sorting algorithm. Reordering arguments, explicitly registering cwd and changing first-turn timing did not resolve it.

The selected documented native `PreInvocation` reply uses `injectSteps` with an `ephemeralMessage` naming the canonical session cwd. It passed 9/9 probes across hooks-only, hooks plus bus and typed-input variants. The per-session environment context avoids changing shared rules between concurrent sessions or global hook configuration. The reply does not grant a tool permission and permits an explicit user-selected alternative directory.

Probe evidence root: `/private/tmp/runner-787-probes-ZfQBFZ`; sanitized source summary `phase1.md`, matrix `matrix.json`, and each `trusted-*/{summary.json,hooks.jsonl,transcript.jsonl,actual-cwd.txt,launch.json}`. Raw local logs/transcripts are retained locally and are not included in this report.

## Reviewed-fix validation

Fix evidence: `/private/tmp/runner-qa787-fix-o30ugdg8/evidence`. The seven complete rows below ran on 2026-10-04 at approximately 04:16–04:20 UTC. `fix-matrix.json` identifies the exact session IDs, markers, cwds, results and supplemental attempts; `ledger.json` tracks all created objects. The implementation diff fingerprint remained unchanged throughout these runs.

| Required row | Evidence prefix | Marker in requested cwd exactly once | Process cwd | Hook Working → completed | Native ACK | User added folder remains a workspace |
| --- | --- | --- | --- | --- | --- | --- |
| Sole-lead mission 1 | `mission-1` | Passed | Passed | Passed | Passed | Skipped — no custom folder |
| Sole-lead mission 2 | `mission-2` | Passed | Passed | Passed | Passed | Skipped — no custom folder |
| Sole-lead mission 3 | `mission-3` | Passed | Passed | Passed | Passed | Skipped — no custom folder |
| Direct chat 1 | `chat-1-verified` | Passed | Passed | Passed | Passed | Skipped — no custom folder |
| Direct chat 2 | `chat-2-verified` | Passed | Passed | Passed | Passed | Skipped — no custom folder |
| Direct chat 3 | `chat-3-verified` | Passed | Passed | Passed | Passed | Skipped — no custom folder |
| Direct chat with own `--add-dir` | `chat-add-dir-verified` | Passed | Passed | Passed | Passed | Passed |
| Native Windows | Not run | Skipped — pending Jason | Skipped | Skipped | Skipped | Skipped |

The first four direct fix runs also passed placement, process cwd, hook completed and native ACK, but their background samplers ended when the launcher exited; observed hook Working was therefore Blocked. An intermediate three-chat attempt likewise lacked continuous Working samples because its launcher rejected an overlength fourth role handle and exited. The rejected role was not created. These incomplete observations remain in the supplemental matrix, and are not promoted to complete passes. QA then repeated only the four affected direct workflows with fresh markers, short handles and a launcher that remained alive through sampling. All four complete repeats observed actual hook Working followed by hook ready/completed. No product check failed in these attempts.

The custom role passed its own `--add-dir /private/tmp/runner-qa787-fix-o30ugdg8/user-added-workspace`. `chat-add-dir-verified-files.json` retains its unchanged argv and correct process cwd. `chat-add-dir-verified-payloads.json` independently records all three workspace paths: requested cwd, the custom folder and Runner's hooks folder. Its `PostToolUse` payload shows `run_command.Cwd` equal to the requested cwd; the marker file contains exactly the expected line. Workspace ordering still varies, while the directory result remains correct.

Native control recovered with the freshly launched reviewed wrapper. QA inspected all three mission terminals, all four original direct chats and all four continuously sampled direct repeats, and retained their visible expected ACKs as `*-terminal.jpg`. These are actual native observations of the exact development build. The earlier baseline native gaps remain in the baseline record; successful fix observations do not rewrite those gaps.

`*-files.json` retains placement, contents, process cwd and argv; `*-session.json` retains final structured status; mission `*-feed.json` and direct `*-status-samples.ndjson` establish hook transitions. Own-session runtime logs, hook feeds, captured payloads and conversation transcripts are retained locally. Raw logs and screenshots may include private startup/account metadata and must not be copied into a PR.

Live cancellation, recovery, original-conversation resume, new-conversation rekey, mission workers and other runtimes are Skipped for this narrow phase-5 matrix. The focused automated tests cover direct/lead/worker launch and chat/mission resume environment shapes, unchanged conversation arguments and user-added directories. This run does not claim broader lifecycle coverage than requested.

## Automated checks

Coder's checks on the reviewed diff, independently inspected by QA in `/private/tmp/runner-787-checks-xbyiL3/results.jsonl` and its logs:

| Exact command | Result | Exit | Evidence |
| --- | --- | ---: | --- |
| `cargo test --locked -p runner-backend --profile ci` | Passed — 1104 tests | 0 | `1.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | Passed | 0 | `2.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | Passed | 0 | `3.log` |
| `cargo fmt --all --check` | Passed | 0 | `4.log` |
| `git diff --check` | Passed | 0 | `5.log` |

`prechecks.json` additionally records `cargo test --locked -p runner-backend --profile ci antigravity` (42 passed, exit 0) and the coder's deliberate regeneration of the existing spawn/startup-file goldens (exit 0). QA did not regenerate or change frozen expectations, and did not rerun already-passed Rust checks without a new implementation change. QA reran `git diff --check` after adding this report.

## Cleanup and limits

Baseline cleanup stopped/archived all its own missions, chats, excluded/unused attempts and status repeat, then deleted its QA crew and roles with not-found verification. Its own misplaced marker files were moved into evidence. Its own wrapper exited 143 and development status returned exit 3. No baseline QA agy subjects remained. Native Recents cleanup observation was Blocked at that time.

Fix cleanup stopped/archived three QA missions and eleven QA chats, including all partial observational repeats. Structured snapshots confirmed archive timestamps and stopped sessions before metadata deletion. QA deleted only its one `qa787-` crew and twelve QA roles, and verified each returned not found. No own agy process remained. All fix markers were retained in scratch cwds; none appeared in hooks or mission bus folders. Own feeds, session/role/crew metadata and conversation transcripts were captured before deletion, whose normal cascade may remove Runner metadata. Scratch evidence and runtime conversation history remain.

QA verified the native Recents surface no longer contained its created objects, then quit only its own development wrapper; exit 143 and development status exit 3 confirmed shutdown. Pre-existing chats, missions, crews, roles, agent conversations and the installed Runner app were not operated. QA left implementation changes and this report uncommitted for the coder.

No required macOS behavior check remains unresolved. Historical baseline ACK gaps and partial sampler observations remain disclosed as Blocked; native Windows remains Skipped pending Jason. The brief's PR publication and CI gates belong to the coder, and this report does not claim they have run or passed.
