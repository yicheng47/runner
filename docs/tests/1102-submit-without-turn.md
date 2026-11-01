# #1102 / #1103 live validation

## Verdict

**Passed — NO REMAINING MUST-FIX QA ISSUES in the required targeted live matrix.** All seven selected checks have adequate passing evidence on reviewed code SHA `57beb252789530161b8e21dc902806a760077c28`. The original Codex busy check was masked by a draft hold; reviewer follow-up withdrew its initial acceptance, and the isolated rerun at 14:30–14:31 UTC resolved that gap with an empty composer and `Ready { idle: false }` hold evidence. The other six results are retained. No implementation changes were made by QA. The record correction is the only dirty source-tree file and is left uncommitted for the coder.

This is targeted delivery QA, not a full regression pass. Native checks cover macOS with Claude Code and Codex. Windows and Linux have CI evidence only; interruption, general resume/rekey and unrelated runtime behavior were outside this matrix. Deterministic tests cover timer races, late turn-start, failed input writes, lifecycle cleanup and warning timing; QA did not simulate a two-minute stuck-busy warning in the native app.

## Candidate and environment

Branch: `fix/1102-submit-without-turn`. PR: [#1105](https://github.com/yicheng47/runner/pull/1105). Code commits: `ba2b88c9` (#1102) and `57beb252` (#1103). QA checked the clean initial tree and verified the first run at that code SHA with only this then-untracked record. The first-run GitHub snapshot reports OPEN, that exact code head SHA, `core-review` and `risk-high`, and successful Rust checks on macOS, Windows, Linux x86_64 and Linux aarch64; the core-label check also succeeded. Evidence: `github-final.json`.

OS: macOS 26.6.2 (25G83), arm64. Codex CLI: 0.165.0, model `gpt-6-luna`, effort `low`. Claude Code: 2.1.316, role model alias `haiku`, native UI reports Haiku 5.5, effort `low`. The existing mission policy was `bypass`; no global agent permissions were changed to obtain a pass.

The candidate was built in its own worktree target with `env -u NO_COLOR -u RUNNER_CREW_ID -u RUNNER_MISSION_ID -u RUNNER_HANDLE -u RUNNER_EVENT_LOG make run`. App SHA-256: `ec129ce85cdc34419bad21a331b47b016328cfd2b47ad9df11ecd611caf32152`. CLI SHA-256: `05903a2abb9c35a446f3415f5b5368070008b4f4d58d5b23706119eb9e4cc1fa`. The wrapper copies and installed development sidecar were byte-identical. Evidence: `active-wrapper.json`, `build-hashes.json`, `rebuild.json`.

First run’s final native wrapper: `/private/tmp/runner-1102-qa-xiyiuzd_/Runner QA1102 Active.app`, bundle ID `com.wycstudios.runner.qa1102active`, PID 74421. Native computer use inspected this exact app before local keyboard input. Structured status independently used `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/bin/runner` with implementation-mission environment variables unset. Verified endpoint: `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock` (`active-candidate-status.json`). The installed Runner was used only to coordinate this implementation mission.

Evidence root: `/private/tmp/runner-1102-qa-xiyiuzd_/evidence`. Canonical scratch cwd: `/private/tmp/runner-1102-qa-xiyiuzd_/mission`. The `devcli.py` helper saves command arguments, UTC timestamps, exit codes and raw output in each labeled JSON artifact. `final-live-feed.stdout` retains 81 mission events before cleanup, `router-live.log` retains filtered daemon evidence, and `verified-metrics.json` records assertions over event ordering and durations. Private account snapshots remain outside Git.

## Selected matrix and results

The checks were selected before execution from the mission brief, `MIS-MSG-01` (busy/idle delivery) and the hold/release portion of `MIS-DRAFT-01` (composer protection). Empty local submission and post-submit typing were added specifically for #1102. Mid-turn delivery was added for the human-approved #1103 scope.

| Case | Runtime / platform | Result | Observed behavior and evidence |
| --- | --- | --- | --- |
| QA1102-EMPTY | Claude Code / macOS | Passed | A keyboard Return in an idle empty composer did not strand the member. The addressed notice was read and the member returned idle. A second passive empty Return, without an intervening notice, published `input-timeout` idle after 10.022 seconds. `claude-before-empty.png`, `claude-empty-ack.png`, `claude-passive-empty-key.json`, `claude-passive-recovery-ack.png`, feed. |
| QA1102-EMPTY | Codex / macOS | Passed | Local empty Return published busy at 14:00:17.449184Z, then `input-timeout` idle at 14:00:27.534450Z (10.085 seconds). The queued notice was read at 14:00:29.142822Z and final hook-backed idle followed. `codex-empty-key.json`, `codex-empty-busy.png`, `codex-empty-ack.png`, feed. |
| QA1102-TYPING | Codex / macOS | Passed | Typed `QA1102-TYPING-ALIVE` appeared after the ignored empty Return and was then cleared. Later real local submissions also worked. `codex-typing-after-empty.png`. |
| MIS-MSG-01 | Claude Code busy / macOS | Passed | Notice posted during an active tool turn was confirmed submitted, then read before the original finish marker and before Idle. No intervening Idle occurred. `claude-mid-turn-notice.png`, `claude-mid-turn-read-done.png`, feed and router log. |
| MIS-MSG-01 | Codex busy / macOS | Passed (isolated rerun) | Hook-confirmed Busy lasted 50.829 seconds with an empty composer. The router held the queued notice solely as `Ready { idle: false }` until original Idle, then delivered it for read and returned to Idle. No competing hold or delivery attempt occurred during that hold. `gate-busy-empty-notice-held.png`, `gate-router-live.log`, `gate-feed-final.stdout`, `gate-verified-metrics.json`. The original masked run is retained below as inadequate evidence. |
| MIS-DRAFT-01 (hold/release) | Claude Code / macOS | Passed | Unsent draft stayed intact with the notice waiting for 27.710 seconds. Clearing it without Return released delivery; the exact notice was read and final state was idle. `claude-draft-held.png`, `claude-draft-clear.json`, `claude-draft-release-ack-idle.png`, cursor snapshots and feed. |
| MIS-DRAFT-01 (hold/release) | Codex / macOS | Passed | Unsent draft stayed intact with the notice waiting for 89.818 seconds. Fully clearing it without Return released delivery; the exact notice was read and final state was idle. `codex-draft-held.png`, `codex-draft-clear*.json`, `codex-draft-release-ack-idle.png`, cursor snapshots and feed. |

All selected required native checks are resolved, including the isolated Codex runtime-gating rerun; the other six checks retain their original passing evidence. No case is counted Passed from a CLI command, visible ACK alone or process survival: the required empty-submit/typing actions and UI were observed natively, then exact message IDs, inbox reads and hook state were checked independently. The isolated busy rerun was triggered by an addressed mission request, without typing a local draft.

## Reproduction and timing evidence

For each empty-submit case, select the owned member's idle pane, inspect the empty composer, press keyboard Return, address a unique marker through the development CLI, and verify its exact inbox read plus final idle. For Codex, type and clear a marker after the empty Return to test continued input. Claude now permits mid-turn delivery, so its first addressed notice could start real work before the timeout; the additional passive Return demonstrates the timeout independently.

| Observation | UTC event ordering |
| --- | --- |
| Claude passive empty recovery | Input submit 14:01:22.845626Z → input-timeout Idle 14:01:32.867736Z. |
| Codex empty recovery | Input submit 14:00:17.449184Z → input-timeout Idle 14:00:27.534450Z → inbox read 14:00:29.142822Z. |
| Claude busy delivery | Post 14:02:17.547632Z → submission confirmed 14:02:19.149168Z → read 14:02:51.191153Z → original finish marker 14:02:53.149576Z → Idle 14:02:55.563972Z. |
| Codex busy delivery (original masked run; inadequate for runtime gating) | Hook turn start 14:01:05.443636Z → post 14:01:10.726841Z → original Idle 14:01:19.522471Z → read 14:01:21.530260Z. Real turn duration: 14.079 seconds. |
| Claude draft | Post 14:04:06.572496Z → clear without Return 14:04:34.282Z → read 14:04:36.795517Z. |
| Codex draft | Post 14:01:52.344649Z → fully clear without Return 14:03:22.163Z → read 14:03:24.369235Z. |

Exact notice IDs: Claude empty `01M6BPJ2CB09370SBBFGGADNTA`, Claude passive recovery `01M6BPN1176SZ8ARD26H4M4T7X`, Codex empty `01M6BPJK13FYBYS0RNKWVQCWRG`, Claude busy `01M6BPP32BNQ0XN90S2AE8ERQ7`, Codex busy `01M6BPM1T6KQGEKT35ZJ7NTB6X`, Claude draft `01M6BPSDHCES5X4E28YCJQZZQW`, Codex draft `01M6BPNAERHP6DZ20AEYB26Q8Q`. Draft cursors stayed unchanged during the holds and advanced through the corresponding exact message after clearing. Both final session snapshots report `activity=idle` and completed hook-backed observations.

For busy checks, submit a bounded local multi-tool task, verify hook-backed Working, then post the notice. Claude must read it before the original task finishes. Codex must wait for the original Idle. The Codex fixture's nominal 35-second subprocess was backgrounded by its tool after roughly ten seconds; the agent finished its actual turn after 14.079 seconds. The verdict uses hook lifecycle and feed ordering rather than that subprocess's lifetime. The `codex-real-turn-held.png` screenshot was captured after the original turn ended, so it is not used as evidence of the active hold. The original Codex live hold log used `Drafting`, so that run establishes only the observed wait/idle ordering and is inadequate to establish the runtime capability boundary. The isolated rerun below is the acceptance evidence for that boundary.

For draft checks, type a unique unsent draft, post the notice, preserve it beyond recent typing, verify unchanged cursor and visible waiting, then clear without Return. The first Codex Ctrl+u left a trailing `P` because the caret was before it; the router correctly continued holding (`codex-first-clear-left-P.png`). QA moved to the end and fully cleared it. The measured release starts at that actual empty composer, not the earlier incomplete clearing action.

The Codex empty notice's router attempt outcome was `Unverified`; the verdict relies on the later exact `inbox_read`, native ACK and final idle, and does not claim that attempt was marked Confirmed. Claude's busy attempt was Confirmed before the read and original finish.

## Automated coverage and Codex finding

The reviewer reported clean code review plus 101 passing state/router/startup tests and `git diff --check` on this SHA. The coder owns full-workspace gates. QA independently captured the successful cross-platform PR checks in `github-final.json`; these are automated evidence, separate from native results. QA's record-only `git diff --check` passed.

The configured recovery bound is ten seconds. The live passive-submit measurements were 10.022 seconds for Claude and 10.085 seconds for Codex. Hook-confirmed real turns remained busy beyond that bound, and the Claude mid-turn read occurred without an Idle transition. Early/late turn-start and revision races are deterministic-test coverage rather than claims about this live run.

Codex 0.165.0 continued accepting typed input after the ignored empty Return. The historical issue's local typing without submission does not establish a Codex-side input failure, and this run did not reproduce an input stall. No external issue was filed. Historical cause remains unproven rather than being assigned to Codex.

## Setup history and limits

Preflight found development daemon PID 89803. QA left it untouched until Jason approved stopping the development app/daemon through Runner at 11:30:31Z. Native access initially worked at 11:33:01Z, then returned `cgWindowNotFound`. OS evidence later established screen lock at 11:33:07Z (`screen-lock.json`), and the first matrix remained Blocked at that stage. The earlier draft record is preserved as `draft-record-history.md` outside Git.

Jason reported zero windows for an earlier wrapper and requested a fresh rebuild/relaunch plus a System Events count. Rebuild succeeded with the same hashes, but automatic approval review rejected the proposed read-only `osascript` count because commands starting with `osascript` were forbidden (`window-count-blocked.json`). It was not bypassed. A process log saying `restored count=1` was not treated as an independent window-count result. After Jason confirmed unlock at 13:57:19Z, OS state independently reported unlocked and native access worked; no additional wrapper/app defect was established.

The #1097 QA owned the shared development namespace at unlock. QA coordinated through Runner, waited for its wrapper to close, and received the namespace at 13:58:23Z before launching this candidate. Only owned test sessions were operated. Claude's initial fixture had not completed folder trust; QA restarted the owned member, accepted the authorized disposable-folder trust prompt, and verified startup marker plus hook-backed idle before testing. Both members wrote exactly `QA1102-STARTUP\n` to their startup files. This setup recovery was not counted as lifecycle/resume validation.

Configuration snapshots covered native `~/.claude.json` and Codex before launch. Claude actually used the managed `/Users/jason/.claude-work/.claude.json`; its full baseline was captured before cleanup, not before initial startup. Therefore no complete before-start canary assertion is made for that account file. No global settings were changed to obtain a pass. Exact test-folder trust removal and preservation of all other values were checked at cleanup.

## Owned objects and cleanup

| Object | Identifier |
| --- | --- |
| Test mission | `01M6BE56ERYS6XSBK8QXAAJEZ2` — QA1102 submit and delivery |
| Crew | `01M6BE502S4G3YMYTDB8WXVD2G` |
| Codex role / session | `01M6BE4ZWMBM8MNDW92PKZAKB5` / `01M6BE56ESGGP66WEJZY26AKCK` |
| Claude role / session | `01M6BE4ZZQQ633H3JTXRQX6QEP` / `01M6BE56ET84S2P532QM0GPH2S` |
| Final conversation keys | Codex `01a196e2-a979-7c12-b196-2e767e30203b`; Claude `80b984a8-5aff-48a3-bcfb-60a49cb9988b` |

Cleanup completed at 14:07:54Z. QA stopped and archived only the owned mission; `cleanup-mission-status.json` reports archived at 14:06:00.176811Z, two stopped sessions and zero live sessions. Native Recents removal is captured in `cleanup-mission-archived.png`. The exact owned runtime PIDs 73078 and 75652 and wrapper PID 74421 were verified exited. Four temporary wrappers were unregistered from LaunchServices and retained on disk. The development daemon was left running to preserve unrelated sessions; #1097 QA was notified of namespace release and completed trust cleanup.

Only the exact scratch cwd trust entries were removed from Codex TOML and managed Claude JSON using structured parsers, with assertions preserving other projects and values. Evidence: `configuration-cleanup.json`, `codex-cleanup.diff`, `claude-work-cleanup.diff`, `cleanup-verdict.json`. Native `~/.claude.json` had no owned entry to remove. Scratch evidence, conversations, roles and crew history were retained. No files, branches, worktrees or unrelated sessions were deleted, and the installed Runner app and its sessions were not operated.

## Reviewer adequacy correction — 2026-11-01 14:19 UTC

The original Codex busy run logged `Drafting`, and its held screenshot was captured after the original turn ended. A runtime incorrectly allowing mid-turn input could therefore have passed that run because an independent draft hold blocked delivery. This is a gap in the test evidence, not a demonstrated product failure. The earlier overall acceptance was withdrawn pending one isolated rerun with a visibly empty composer, hook-confirmed Busy, no competing hold, no delivery attempt during Busy, and delivery/read after Idle. The required daemon evidence is `Ready { idle: false }`, or equivalent independent state establishing absence of another hold. Successful evidence from the other six checks is retained. QA coordinated the shared development namespace with #1097 before relaunch; the isolated rerun below resolves this correction.

## Isolated Codex busy rerun — 2026-11-01 14:28–14:32 UTC

**Passed.** #1097 yielded the shared development namespace at 14:27:55Z after stopping and archiving its objects. Preflight also observed raw source process PID 94418; #1097 later confirmed it was not theirs. QA did not stop or operate that app, and it was absent at the next process inspection. QA ran `make run` after the namespace handoff; Jason subsequently said he had closed the app and explicitly requested a restart before the fixture began. QA relaunched the verified wrapper as PID 388. The rerun candidate was HEAD `f89c6d26af524d06afa74b342bc38e55cb82ee9f`, which differs from reviewed code `57beb252` only by the committed QA record; the dirty diff was only this record’s adequacy correction. App and CLI hashes remained exactly the first-run hashes above (`gate-preflight.json`, `gate-wrapper.json`), with the verified development endpoint (`gate-dev-status.json`). No product code changed.

QA created one new bounded single-Codex mission, `01M6BR85TMX10ENZG8CM3160VY` (crew `01M6BR7ZTS8SMJPSPAJWM9FZ6M`, session `01M6BR85TP08P42GYP6BEB3M3C`, runtime PID 1922, conversation key `01a19784-2296-7522-b495-69ce0ea895f0`), reusing its own `qa1102-codex` role with `gpt-6-luna` and low effort. Scratch cwd: `/private/tmp/runner-1102-qa-xiyiuzd_/codex-gate-rerun`. The member wrote exactly `QA1102-GATE-STARTUP\n` and reached hook-backed Idle before the case. Native computer use selected this exact mission and its terminal; `gate-idle-terminal.png` shows the empty composer. No unrelated session was operated. Automatic background watching was unavailable on this host and was disclosed; no undeliverable follower was left running.

An addressed request (`01M6BR9GF08HYKTW9683P1RGPG`) started a 45-second scratch script and explicitly required the agent to wait on the same running tool session, then execute a finish command before ending the turn. This avoids creating a local draft. QA independently captured hook-backed Busy and native Working with an empty composer, then posted the actual test notice `01M6BR9YH69EG1EK3XMQB74MW3` (`QA1102-GATE-NOTICE`). `gate-busy-before-notice.png` and `gate-busy-empty-notice-held.png` show an empty placeholder composer during the original work. `gate-busy-before-notice.json` and `gate-busy-held.json` report `activity=busy`, working observation, source hook, using tools.

| Observation | UTC ordering |
| --- | --- |
| Original hook Busy | 14:30:22.590217Z |
| Test notice posted | 14:30:36.838315Z |
| Router hold | 14:30:36.849059Z, `Ready { idle: false }` |
| Scratch tool end / finish marker | 14:31:10.937324Z / 14:31:12.124965Z |
| Original hook Idle / router release | 14:31:13.419282Z / 14:31:13.419429Z, releasing the same `Ready { idle: false }` hold |
| Exact notice read / confirmed attempt outcome | 14:31:14.918388Z / 14:31:15.009535Z |
| Final hook-backed Idle | 14:31:16.953547Z |

The original turn remained Busy for 50.829 seconds. The notice waited 36.581 seconds until original Idle. The daemon retained only the `Ready { idle: false }` hold from posting until Idle, with no competing hold or attempt in that interval. The held read cursor still acknowledged only the trigger request; the final cursor advanced through the exact test notice. No `input-timeout` occurred and no intermediate Idle broke the original real turn. Native `gate-after-turn-progress.png` shows original turn completion before the later inbox read/ACK, followed by idle. Assertions over the feed, cursor snapshots, marker files and router log are preserved in `gate-verified-metrics.json`; the full structured evidence is `gate-feed-final.stdout`, `gate-router-during.log`, `gate-router-live.log`, `gate-cursor-held.json`, `gate-cursor-final.json` and `gate-session-final.json`. This run establishes the idle-only boundary independently of draft protection.

Cleanup completed at 14:32:36Z: the isolated mission is archived at 14:31:54.022494Z with zero live sessions, runtime PID 1922 and wrapper PID 388 are verified exited, and the wrapper is unregistered from LaunchServices but retained. Native navigation away from the archived mission is captured in `gate-cleanup-archived.png`. The exact rerun cwd trust entry was removed with a structured TOML parser; all other current values were preserved, and the resulting semantic Codex configuration equals the prelaunch snapshot (`gate-config-cleanup.json`, `gate-codex-cleanup.diff`). Evidence/history and the test crew are retained; the development daemon remains running to preserve unrelated sessions. `gate-cleanup-verdict.json` records the checks.
