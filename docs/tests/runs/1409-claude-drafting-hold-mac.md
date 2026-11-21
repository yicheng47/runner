# #1409 Claude Code drafting holds on macOS

## 2026-11-21, candidate 39ff870c

**Verdict: Failed.** The same recheck diagnostic appeared twice during a continuous Drafting hold with a pane resize between the two samples. Draft protection, clear-to-release, blank-composer report delivery and stale-report dropping worked.

- Reviewed candidate `39ff870c83a8dfc512ad711a56de981718b5a75c` on `fix/1409-claude-drafting-hold`. The worktree and tracked diff were clean at build time. Approved neon trio mission `01M7YNG398CZ09GSVQG8E2GZKR`, handoff `01M7YQ5XGP3P893C0WNBFJDCXW`, PR #1412.
- macOS 26.6.2 (25G83), arm64. Runner 0.20.2 (dev), CLI 0.20.2, Claude Code 2.1.330 with Haiku 5.5 and low effort. The chat inherited the existing bypass mode. The development app listed one signed-in Claude account. It hit its weekly limit during the seed turn, then returned to an empty composer after Escape. The submitted reports reached Claude's native transcript and Runner confirmed them. Claude's model responses and a dynamically generated post-success dim suggestion were unavailable.
- Build and execution were approximately 09:33–09:42 UTC. Held `script/qa-lock`, cleared inherited mission variables and `NO_COLOR`, and ran `make run`. Copied byte-identical app and sidecar binaries into a temporary `Runner 1409 QA.app` wrapper. Verified the development socket at `com.wycstudios.runner-dev/runnerd.sock` and the runnerd binary against the sidecar. App SHA-256 `c777df7ecab2e78065043286f617005fbf677c404591784a311d268bd3065628`. Sidecar and daemon SHA-256 `30963e95967ed03d19d89ed254df19078b1e6b4ee8164d9c9f891206b1833a88`.
- Evidence root `/private/tmp/runner-1409.pDDYCA/evidence/`. Native input and screenshots came from the exact development wrapper, including a harmless New tab check. The development CLI created and linked three bounded missions in separate scratch directories, recorded their phases, and archived the fixtures. Each empty mission settled after 30 seconds. The third added one Codex worker with `gpt-6-luna`, low effort and a reply-only task to move the mission back to Working.

| Check | Result | Evidence |
| --- | --- | --- |
| 1. Blank-composer Settled delivery | Passed for Runner delivery | `check1-after-dismissal.png` shows the blank composer after the rate-limited turn. The daemon queued Settled at 09:38:03.098496Z and confirmed at 09:38:04.690735Z, with no Drafting hold. `check1-settled-delivered.png` shows the submitted report and the subsequent weekly-limit response. The optional post-turn dim suggestion did not appear. |
| 2. Genuine draft, redraw, recheck and clear | Failed on diagnostic deduplication | The visible unsent marker survived a real-cursor sidebar drag that changed the PTY from 76×34 to 69×34 and stayed intact across the 30-second recheck. `check2-draft-entered.png`, `check2-after-resize.png`, and `check2-across-heartbeats.png` show the intact draft and no Settled report. The 09:38:50.203925Z and 09:39:20.475008Z diagnostics have identical payloads despite no input or release between them. The following stationary heartbeat added no identical line. After the full clear with Ctrl+E then Ctrl+U, delivery released at 09:40:11.317500Z and confirmed at 09:40:12.913897Z. `check2-cleared-full.png` shows the delivered report and no submitted draft. |
| 3. Stale Settled drop on return to Working | Passed | The daemon held Settled behind a real draft at 09:40:47.608534Z. Adding the worker moved the mission from Settled to Working while the draft remained. After clearing, the daemon logged release at 09:41:07.074601Z and dropped the State report at 09:41:07.074721Z with `reason=state changed (Working)`. `drop-status-working.json`, `check3-working-held.png`, and `check3-cleared-no-stale-report.png` show the transition and no stale report entered in the transcript. |
| Ten-minute screen-observed cap and uncapped native pi draft | Skipped, unit-only in this run | The mission scoped live QA to the three macOS Claude checks. The daemon unit test `report_draft_hold_shares_the_observed_cap_while_native_drafts_stay_uncapped` covers those boundaries. |

Chat `01M7YQDK3ST0YW5K54G928E9G7`. Check 1 mission `01M7YQFPX0M2BM5ZKBQH3B5KYB`. Check 2 mission `01M7YQH4X1G81YCWPYG4Q0N0XX`. Check 3 mission `01M7YQMQHZZ88ZTXFDSRQ8FV4B`, worker `01M7YQNY2B65537VQQPCX2X7K9`.

```text
09:38:50.203925Z input tracker recheck kept drafting session=01M7YQDK3ST0YW5K54G928E9G7 composing=false visible=true submitted=false content_pending=false tracked_row=Some(28) cursor=Some((28, 28)) cursor_prefix_col=false cursor_prefix_match=true cursor_empty=false
09:38:50.203954Z report delivery held session=01M7YQDK3ST0YW5K54G928E9G7 reason=Drafting
09:39:20.475008Z input tracker recheck kept drafting session=01M7YQDK3ST0YW5K54G928E9G7 composing=false visible=true submitted=false content_pending=false tracked_row=Some(28) cursor=Some((28, 28)) cursor_prefix_col=false cursor_prefix_match=true cursor_empty=false
09:40:11.317500Z report delivery released session=01M7YQDK3ST0YW5K54G928E9G7
09:40:12.913897Z report attempt outcome=Confirmed session=01M7YQDK3ST0YW5K54G928E9G7 mission=01M7YQH4X1G81YCWPYG4Q0N0XX
09:41:07.074601Z report delivery released session=01M7YQDK3ST0YW5K54G928E9G7
09:41:07.074721Z report dropped session=01M7YQDK3ST0YW5K54G928E9G7 mission=01M7YQMQHZZ88ZTXFDSRQ8FV4B kind=State reason=state changed (Working)
```

Reproduce the diagnostic failure by typing a draft in the linked chat, letting Settled hold, then narrowing the terminal once without typing. Compare the initial hold diagnostic with the first heartbeat after resize. Both rows should only log when the diagnostic fields change. The complete filtered daemon log is in `runnerd-filtered.log`, and the duplicate pair is in `check2-recheck.log`.

Stopped and archived the QA chat and all three missions. Verified zero fixture sessions remained live and the original session and mission inventories matched. Closed the QA tab, quit and unregistered the wrapper, and stopped the development daemon after confirming zero pre-existing live sessions. Restored the original sidebar width and removed only the run-owned Claude and Codex trust records. Codex config matched its original bytes and no unrelated Claude project values differed. Released the QA lock. Scratch directories, evidence, archived history, and the test role remain available.

## 2026-11-21, candidate 337467d, Check 2 rerun

**Verdict: Passed.** The reviewed fix produced one diagnostic across the same resize and the 30-second recheck. The draft stayed intact until cleared, then the Settled report reached the native transcript. Checks 1 and 3 carry forward from the original run. Claude's account limit still prevented model responses, and the optional post-turn dim suggestion was unavailable. The cap and native pi exception have unit coverage for this run.

- Reviewed candidate `337467d78162c47f464a33a1c8ebe0a5e36346c4`, handoff `01M7YR1FKKMWH89QMBG1663TCE`. The tracked diff was empty at build and during the retest. This QA record was the only untracked file.
- Ran the macOS retest from 09:56 to 10:00 UTC using `make run` and a byte-identical `Runner 1409 Retry.app` wrapper. The OS, app and Claude versions and Haiku low effort were unchanged. The dev socket and copied binaries were verified. App SHA-256 `d1bfbca96da869fdec3fd41b469f327d9cc58a84150f97ef1443616bf6b04a91`. Sidecar and daemon SHA-256 `076e894e6b429fead3bc557af27e469e4b15ca37c1ab826f60fa9abdbd59ec58`.
- Evidence root `/private/tmp/runner-1409-retry.zl0oXg/evidence/`. Claude chat `01M7YRMVXQMB4T2BRX79VTJFZG`, mission `01M7YRP9XD9XD1WD1XR930RFD6`. Used a fresh native Claude chat and one linked empty mission. Dismissed the weekly-limit dialog after the seed turn, linked the mission, and typed the same unsent marker as the first run.

| Check | Result | Evidence |
| --- | --- | --- |
| 2. Genuine draft, redraw, recheck and clear | Passed | `draft-entered.png`, `after-resize.png`, and `across-heartbeat.png` show the unchanged unsent draft and no Settled report during the hold. `runnerd-filtered.log` records the PTY resize from 76×34 to 69×34 and exactly one recheck diagnostic between hold and release, a 48.44-second interval. Clearing with Ctrl+E then Ctrl+U released delivery. The daemon confirmed 1.60 seconds later, and `settled-delivered.png` shows the submitted report and empty composer. |

```text
09:59:07.749606Z mission 01M7YRP9XD9XD1WD1XR930RFD6 is Settled; reporting State to chat 01M7YRMVXQMB4T2BRX79VTJFZG
09:59:07.749807Z input tracker recheck kept drafting session=01M7YRMVXQMB4T2BRX79VTJFZG composing=false visible=true submitted=false content_pending=false tracked_row=Some(28) cursor=Some((28, 28)) cursor_prefix_col=false cursor_prefix_match=true cursor_empty=false
09:59:07.749836Z report delivery held session=01M7YRMVXQMB4T2BRX79VTJFZG reason=Drafting
09:59:21.660319Z resize request: session=01M7YRMVXQMB4T2BRX79VTJFZG 70x34 -> 69x34 size_changed=true controller=Desktop -> Desktop applied=true
09:59:56.189891Z report delivery released session=01M7YRMVXQMB4T2BRX79VTJFZG
09:59:57.785191Z report attempt outcome=Confirmed session=01M7YRMVXQMB4T2BRX79VTJFZG mission=01M7YRP9XD9XD1WD1XR930RFD6
```

Stopped and archived the run-owned chat and mission and verified zero live fixture sessions and matching pre-existing inventories. Closed the QA tab, quit and unregistered the wrapper, stopped the development daemon after confirming zero pre-existing live sessions, restored the sidebar width, and removed the run-owned Claude trust entry. Unrelated Claude projects matched the baseline. Released the QA lock and retained the local evidence.
