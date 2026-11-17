# #1369 mid-turn delivery — macOS QA

## Verdict and scope

Final reviewed candidate `59371cf1fb6bbfc9ce1a64287b661fb8a5824c8e`: **macOS feature checks passed; overall Incomplete because required Windows coverage is Blocked**. Automatic mid-turn delivery passed for pi, Antigravity, and OpenCode on this enabled candidate. Earlier Codex/Claude results carry forward on unchanged paths, as the final clean review requested. Keep `needs-live-test` until Jason completes MIS-MSG-03 on Windows. Copilot remains not probed, Grok/Cursor remain nonqualifying and disabled. The initial and final runs are preserved separately below.

Initial candidate: **Incomplete overall**. Codex macOS delivery, draft protection, long-turn warning silence, and Claude Code regression passed. Qualification probes passed for pi, Antigravity, and OpenCode; their flags must be enabled in a reviewed follow-up and their automatic delivery rerun there. Cursor and Grok failed the current-turn qualification (accepted into a next-turn queue); keep their flags false. Copilot was Blocked by policy/authentication. Windows was Blocked because no Windows host was available, as Jason confirmed; `needs-live-test` must remain until the Windows three-notice check passes.

Authorization: approved codex trio mission `01M7M1GPTV1B6Z4GGH1ZS0VZK3`, 2026-11-17. Jason explicitly authorized existing Antigravity sign-in and pi's normal DeepSeek configuration without global configuration/auth changes. Only development data and run-owned fixtures were operated. This is the feature's selected live scope, not a full regression.

## Candidate and environment

- Repository/worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1369-mid-turn-delivery`, branch `feat/1369-mid-turn-delivery`.
- Reviewed candidate: `428f4cb5c7537eac0dff6997cfc541bb6c1175d8`; base `c2d252dc9f5f5cf604d65f9fdc178b4c2053f138`. `origin/main` had advanced to `7821e339624a54e790e8275d400f65a8afcb3bbc` by execution; tests describe the frozen candidate, not that newer main.
- Initial tree clean; empty `git diff HEAD --binary` SHA-256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
- macOS 26.6.2, build 25G83, arm64. Runner 0.20.0 (dev).
- Codex 0.169.0 (`gpt-6-luna`, low); Claude Code 2.1.326 (`haiku`, UI resolves Haiku 5.5, low); pi 1.5.2 (`deepseek/deepseek-flash`, low); Antigravity 1.5.9 (`gemini-3.8-flash`, low); Copilot 1.0.112; OpenCode 2.0.28 (`opencode/big-pickle`, default variant); Grok 1.0.77; Cursor agent 2026.10.01-e373342 (`auto`). Grok rejected the requested older model and used its existing Grok 4.7/high fallback; no global model settings were changed.
- Built using `make run` with `NO_COLOR` and mission identity variables removed. Copied app and sidecar byte-for-byte into `/private/tmp/1369-qa-ai72qii0/Runner 1369 QA.app`, bundle ID `com.wycstudios.runner.qa1369`, then stopped the bare app and launched that wrapper.
- App SHA-256: `d1ab7c1006040f657ccf9042e2693b757482b8c553e98f0955cfd74487feffe7`.
- Sidecar/daemon SHA-256: `9386590f276861eb452341fa91d0554e74cd48817c13c9ad0d10034e3b021c60`; the installed development `runnerd` was byte-identical to this worktree's `target/debug/runner-agent-cli`.
- Absolute development CLI reported `com.wycstudios.runner-dev/runnerd.sock`, version 0.20.0, and outside-mission mode. Cleared `RUNNER_CREW_ID`, `RUNNER_MISSION_ID`, `RUNNER_HANDLE`, `RUNNER_EVENT_LOG`, `RUNNER_SESSION_CLI` for product checks. Installed Runner CLI was used only for this implementation mission's coordination.

## Method and selected plan

Followed `docs/tests/full-smoke-test.md` candidate identity, native-control, fixture, evidence, and cleanup rules plus the feature spec's six live checks. Acquired `script/qa-lock` before build/launch. Verified harmless native input by opening and dismissing New tab. Used CUA native clicks/typing/Return for terminal input, the candidate development CLI for mission/message creation and structured evidence, and no private PTY writes or SQLite access. Each runtime used a run-owned single-slot crew/mission in a canonical scratch subfolder. Native busy-Enter probes posted a durable addressed event first and manually typed its notice into the busy composer while the initial flag was false. Those are qualification evidence, not automatic-delivery evidence.

The initial Codex model `gpt-5.4-mini` was retired; its startup prompt was captured and the session stopped/archived without accepting a model/config change. The replacement used a test-role override of `gpt-6-luna`. Initial startup holds are retained in the evidence: the first Codex message was held as Starting until the initial goal completed. The scored mid-turn checks used a subsequent real Working turn. Native typing and Return were separate actions to avoid treating an immediate paste-burst newline as a successful submit. The first Cursor command detached and replied early; it was repeated with an explicit foreground wait. OpenCode's startup response was delayed; its scored probe was repeated during its actual shell tool.

## Result matrix

Evidence root for every filename below: `/private/tmp/1369-qa-ai72qii0/evidence/`.

| Check | Platform/runtime | Result | Observation/evidence |
|---|---|---|---|
| 1 / MIS-MSG-01 | macOS / Codex | Passed | Automatic notice appeared in history and was read during the second Working turn. A subsequent notice also delivered while the first remained in history. `codex-real-mid.png`, `codex-draft-released.png`, `1369-QA-Codex-luna-feed-final.json`. |
| 2 / MIS-MSG-03 | Windows / Codex | Blocked | No Windows host/control surface. Three attempts, paste-burst retry count, and no-stacking remain unexecuted; macOS and CI do not replace this row. |
| 3 / MIS-MSG-01 draft | macOS / Codex | Passed | Typed `QA1369-PERSON-DRAFT` while Working, posted addressed mail, observed intact draft and Inbox waiting. Ctrl+U cleared only that draft; automatic notice then delivered and was read before final. `codex-draft-held.png`, `codex-draft-released.png`. |
| 4 | macOS / Codex | Passed | A 165-second tool window explicitly deferred reading. At more than 120 seconds after posting, still Working with unread mail and `recent_warnings: []`; covering read only followed sleep completion. `codex-long-over120.png`, `codex-long-over120.json`, final feed. The live run did not manufacture an Unsent leftover; fake-injector tests separately cover that clock transition. |
| 5 | macOS / pi | Passed qualification | Busy Enter showed Steering; original 60-second tool completed uninterrupted, then inbox read and ACK preceded ORIGINAL-EXIT final. `pi-busy-enter.png`, `pi-probe-result.png`, final feed. Enable then rerun automatic delivery. |
| 5 | macOS / Antigravity | Passed qualification | Busy Enter queued input; covering read occurred while original 60-second command was preserved/running, before final ORIGINAL-EXIT. `agy-busy-enter.png`, `agy-probe-result.png`, final feed. Enable then rerun automatic delivery. |
| 5 | macOS / OpenCode | Passed qualification | Busy Enter during actual sleep45 accepted notice; original shell completed without error/cancellation and covering read/ACK preceded ORIGINAL-EXIT final. `opencode-tool-busy-enter.png`, `opencode-probe-result.png`, final feed. Enable then rerun automatic delivery. |
| 5 | macOS / Copilot | Blocked, not probed | Failed to load models: access denied by Copilot policy; authorization error says credentials expired/invalid. `copilot-start.png`. No login/config change attempted; keep false. |
| 5 | macOS / Grok | Failed qualification | Enter accepted into Queued 1 without interrupting the command, but original ORIGINAL-EXIT final and Worked-for boundary preceded queued notice processing/new turn. `grok-busy-enter.png`, `grok-original-boundary.png`, final feed. Keep false. |
| 5 | macOS / Cursor | Failed qualification | Enter left composer and preserved foreground sleep90, but ORIGINAL-EXIT final preceded the notice/covering read in the transcript. `cursor-busy-enter.png`, `cursor-probe-result.png`, final feed. Forwarder did not expose a reliable idle boundary between the turns; visible transcript is decisive here. Keep false. |
| 6 | macOS / Claude Code | Passed | Automatic notice read at 06:29:42 before idle at 06:29:45, original sleep job preserved and later completed exit 0. Harness rejected the first foreground sleep, so the agent used a bounded background job; this did not prevent inbox delivery. `claude-regression-result.png`, final feed. |

## Durable events and covering reads

All timestamps UTC on 2026-11-17. Each `inbox_read` payload's `up_to` equals the message ID in its row.

| Variant | Message event | Covering read event / time |
|---|---|---|
| Codex automatic mid-turn | `01M7M309JNF0SA6NZ5752V8DZA` | `01M7M30CE7K0A0E580YN5MJY3H` / 06:27:15.015 |
| Codex history follow-up | `01M7M30NB1M13DQ8ZDRSX3KJ9A` | `01M7M30R1SH6GFB5JHT4Y91RM2` / 06:27:26.905 |
| Codex draft release | `01M7M30XPW6845J611CK2X0M1G` | `01M7M31AYZZ1C9SWD968KBCEYP` / 06:27:46.271 |
| Codex long unread | `01M7M345XJYWGJMKTB3YZKVZPA` / posted 06:29:19.410 | `01M7M398EKPD8TXB6CR6W1X3CF` / 06:32:05.843 |
| pi qualification | `01M7M335645PEFVN6A002ME91G` | `01M7M34WXG6SJ3RERN4MDPV3GG` / 06:29:42.960 |
| Antigravity qualification | `01M7M35YM9AFH2SPCT65SBJRAQ` | `01M7M36HVM4GEMF6XNYNNWVVKE` / 06:30:37.172 |
| OpenCode actual shell probe | `01M7M3B7RH9T0M7KCX26A500VV` | `01M7M3CE9810ZBGBKYW38663Q5` / 06:33:50.120 |
| Claude regression | `01M7M34S539ZVMX1ZGHJQ6A6W4` | `01M7M34WJEQZKE8D9NEJ92JXA6` / 06:29:42.606 |
| Cursor next-turn queue | `01M7M323BZPSCG2TWKYFAFM6R6` | `01M7M357W838E48C5B0B5B251V` / 06:29:54.184, after visible original final |
| Grok next-turn queue | `01M7M36ZKEMYJWN2YARXNR41Q8` | `01M7M39QCVZYFP18BV4Z6RXT4S` / 06:32:21.147, after visible original final |

## Cleanup and retained evidence

Stopped and archived all nine run-owned missions, verified zero live sessions and non-empty archive timestamps, and captured `archived-test-missions.png`. IDs and full CLI responses are retained in `/private/tmp/1369-qa-ai72qii0/ledger.json` and `cli-history.ndjson`; the `*-cleanup.json` evidence files prove archive state. Temporary roles/crews and conversation history are retained, not deleted. Native Quit confirmation stopped the development wrapper and its daemon; process inventory found no run scratch processes or normal development daemon afterward. The unrelated daemon under another private HOME was left alone. Unregistered the wrapper from LaunchServices and released the QA lock.

Removed only run-owned Codex and Claude trust/project entries. Codex projects matched the pre-run snapshot afterward. pi settings/auth hashes remained unchanged. Claude's unrelated existing project usage/statistics bookkeeping changed during resumed development sessions/concurrent account use; those values were preserved, not rolled back, and no unrelated trust flag changed. Private before-config snapshots/diffs remain outside the repository and are not committed. No original files, roles, crews, branches, worktrees, authentication, or permission settings were deleted/changed by QA.

Automated checks are the coder's separately reported evidence (daemon/core tests, clippy, formatting and diff checks); QA did not rerun or substitute them for live execution. A reviewed enabled candidate still needs automatic pi/Antigravity/OpenCode probes; append that run here with its own hashes and evidence.

## Enabled-candidate automatic delivery rerun — 2026-11-17

Candidate `59371cf1fb6bbfc9ce1a64287b661fb8a5824c8e`, same branch/worktree and base as above. Independent clean review: Runner event `01M7M47J0A7SKJSQ1NYNMMK19T`; handoff `01M7M48PH2XR8G1XA4VREJGTZC`. Only pi/Antigravity/OpenCode flags and corresponding docs changed from the previously tested logic. Tracked source clean; empty tracked diff hash unchanged; the untracked QA record was retained for this append. No source changes or rebuilds occurred during this test.

Built with the same cleared child environment and `make run`; froze byte-identical copies in `/private/tmp/1369-qa-4zkzm26g/Runner 1369 Enabled.app`, bundle ID `com.wycstudios.runner.qa1369enabled`. App SHA-256 `241187010bf270857c115073cea48511267221579435e6fb9de407ff0489423f`; sidecar and actual development daemon SHA-256 `762ccc1d4f85237faa454929eb6b72de7d1b8ef0c9d5453d68efc105daaba895`. Development CLI confirmed 0.20.0/dev and the normal `runner-dev/runnerd.sock`, outside-mission mode. OS unchanged. pi 1.5.2/DeepSeek Flash low and OpenCode 2.0.28/Big Pickle unchanged. Antigravity independently updated to 1.5.10 since the qualification pass; its enabled automatic path was tested on that version, Gemini 3.8 Flash low.

Reacquired the shared QA lock before build/launch. Reverified native input with New tab. Used fresh canonical per-runtime folders under `/private/tmp/1369-qa-4zkzm26g`, reusing only run-owned roles/crews, and started three fresh missions. Each first completed a READY setup, then received a native bounded tool prompt. OpenCode initially remained idle without answering its bootstrap; a native setup-only prompt obtained READY before the scored tool turn. That setup interaction did not contain an inbox notice or count as automatic-startup validation. For all three scored tests, the addressed message was posted only after Working, and Runner alone typed/submitted the notice; QA supplied no manual notice or submit retry.

Evidence root for this rerun: `/private/tmp/1369-qa-4zkzm26g/evidence/`.

| Runtime / fresh mission | Result | Durable message / covering read | Native and structured proof |
|---|---|---|---|
| pi / `01M7M4C1E969FB4XNVNE8QRH3C` | Passed automatic delivery | Message `01M7M4DAKZXDXM5D58BVV9J4GG`, 06:51:47.711 UTC; read `01M7M4ECFSDQH3PX9F4HD7D7TP`, 06:52:22.393 | `pi-auto-busy.png` shows automatic Steering while shell runs. `pi-auto-result.png` shows original 45-second command/output, covering read and ACK before ORIGINAL-EXIT. `pi-feed.json` ends idle at 06:52:23.417. |
| Antigravity / `01M7M4C1GG38S26QTCZ7S7WS2T` | Passed automatic delivery | Message `01M7M4EPMW2RZNP29SKQTKBNZ0`, 06:52:32.796 UTC; read `01M7M4F0J1NPVXKEVJCQ38KN4K`, 06:52:42.945 | `agy-auto-busy.png` shows automatic queued notice during tool. `agy-auto-result.png` shows read/ACK before ORIGINAL-EXIT; `agy-auto-expanded.png` proves original command exited code 0 with AGY-AUTO-EXIT output. `antigravity-feed.json` corroborates covering read while Working. |
| OpenCode / `01M7M4C1K47SPZV7G1NWXFHDKY` | Passed automatic delivery | Message `01M7M4J2P22FQQB82FSGNAP3Y8`, 06:54:23.426 UTC; read `01M7M4K1CY53M9RBH451SRFCVG`, 06:54:54.878 | `opencode-auto-busy.png` shows automatic notice during sleep45. `opencode-auto-result.png` shows original shell completed without error, covering read and ACK before ORIGINAL-EXIT. `opencode-feed.json` corroborates delivery/read and subsequent idle. |

Every covering read's `up_to` equals its message ID. No original command was cancelled, restarted, or corrupted. No notice stacked in the composer. All three enabled-runtime checks passed. No new feature failure was found. Retained initial qualification exclusions and Windows Blocked unchanged; the final verdict is **Incomplete overall**, not a full cross-platform pass.

Cleanup: stopped/archived these three missions and verified zero live sessions plus archive timestamps (`pi-cleanup.json`, `antigravity-cleanup.json`, `opencode-cleanup.json`). Kept roles/crews/history/evidence. Native quit stopped the wrapper and development daemon; process inventory found no owned app/daemon/scratch processes (`process-cleanup.txt`). Unregistered the wrapper and released QA lock. pi global settings/auth SHA-256 canaries remained unchanged. This rerun created no Codex or Claude test trust entries and made no global configuration/authentication/permission changes. Existing development sessions resumed as authorized by the procedure and were stopped only with wrapper/daemon shutdown; their records were not archived or edited. No commit/push by QA.
