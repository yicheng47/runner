# #1183 background-work settle — macOS live QA

2026-11-08, 02:19–02:27 UTC. Scope: the approved mission's L1 and L2 only, on reviewed candidate `177ae4acf30f1b38bb74c918d406261a25ff637f`, branch `fix/1183-background-settle`. The worktree was clean before the build. No implementation code or expectations changed during QA. Automated tests and macOS/Windows CI belong to the coder's separate validation; QA did not rerun the automated suite. No native Windows check was selected by this brief.

## Environment and method

macOS 26.6.2 (25G83), arm64; Runner 0.19.5 development build; Claude Code 2.1.319, Sonnet 5.5, low effort. The new role used automatic permissions; existing mission launch policy selected bypass permissions for mission members. No permission setting, authentication, extension, or global runtime setting was changed to obtain a pass. Existing user extensions remained enabled.

Acquired `script/qa-lock acquire --timeout 540`, then built and launched with `env -u NO_COLOR make run`. Copied `Runner` and `runner-agent-cli` into `/private/tmp/runner-1183-qa.syuzuc_q/Runner 1183 QA.app`, verified byte identity, and ran the copy with mission identity variables removed. The app SHA-256 was `81256dd0acc86f9a8d2df32e9eaeb4db3274473ee79839a4bef7bfa7ede6b4ba`; the CLI and installed development daemon SHA-256 was `4da8a0172348ee343041ce7f9682c4557604eebdb65b7f9ec2c8f053c000e4d6`. `status --json` confirmed the development socket under `com.wycstudios.runner-dev`. Native computer use bound to bundle ID `com.wycstudios.runner.qa1183`; opening and dismissing New tab verified keyboard routing before test interaction.

Each mission and linked chat used its own canonical fresh folder beneath `/private/tmp/runner-1183-qa.syuzuc_q`. Used the absolute development CLI with `RUNNER_SESSION_CLI` empty and implementation-mission identity variables removed. Created an empty mission, linked its owned chat, and added one Claude Code role. Captured mission state and event-feed snapshots every two seconds in bounded scripts, plus native screenshots and the owned chats' transcript report entries. Snapshot timestamps bound phase transitions to approximately two seconds; hook/event and transcript timestamps provide the turn/report timing below. No terminal parsing or process-tree inspection was used to determine background state.

## Results

All times below are UTC on 2026-11-08. Evidence root: `/private/tmp/runner-1183-qa.syuzuc_q/evidence/`.

| Check | Result | Observations and evidence |
| --- | --- | --- |
| L1 / macOS / Claude Code | Passed | The member ran Bash `sleep 90` with `run_in_background=true`, replied `L1-BACKGROUND-STARTED`, and ended its turn. Hook observations at 02:23:25.687358 and 02:23:28.213804 were Ready with `background: true` (the second turn handled the join notification). At 02:24:09.201818 (+40.99 s) and 02:24:47.932814 (+79.72 s), mission phase remained Working while the member remained Ready/background true. The linked chat had no settled report. Task completion appeared natively and started a hook-sourced Working turn at 02:24:54.802214; the member replied `L1-WOKEN-DONE`, became Ready without background at 02:24:56.140704, and received exactly one linked-chat settled report at 02:25:26.237, 30.10 s later. Evidence: `l1-checkpoints.json`, `l1-samples.ndjson`, `l1-around40.txt`, `l1-around80.txt`, `l1-background-terminal.png`, `l1-chat-at80.png`, `l1-woken.png`, `l1-settled-chat.png`, `l1-report-messages.json`. |
| L2 / macOS / Claude Code | Passed | The member replied `L2-ONE-LINE-DONE` without background work. After handling its join notification, its final Ready hook was at 02:24:39.308778. The mission settled and its linked chat received exactly one settled report at 02:25:09.398, 30.09 s later. Evidence: `l2-samples.ndjson`, `l2-one-line.png`, `l2-settled-chat.png`, `l2-report-messages.json`. |

NO REMAINING MUST-FIX QA ISSUES. Both selected live checks passed. This is a feature QA pass, not a full regression pass.

The native task-completion notification and hook-sourced Working → Ready observations establish the woken-turn behavior required by L1; this run did not capture raw hook payloads separately. After L1's successful settle, at 02:25:50.358 an unread join-message reminder triggered another turn; it ended at 02:25:52.218858. This was a later inbox stimulus from the test fixture, not continued background work. Exactly one settled report existed in each linked chat through cleanup. The role's tiny prompt discouraged tools, so L1 acknowledged the join notice without consuming its inbox; L2 consumed its notice. No failure was hidden by retrying either check.

## Fixture ledger and cleanup

| Object | ID | Final state |
| --- | --- | --- |
| Temporary role | `qa1183` / `01M6WFDZWC275YVF7VESHCQR8C` | Retained with evidence and conversation metadata. |
| L1 mission | `01M6WFF9NM6BTRMY117MQKXDX2` | Stopped, archived, zero live sessions. |
| L1 member | `01M6WFF9PP7GVZGAPXRAMSYWZY` | Stopped. |
| L1 linked chat | `01M6WFDZX1S3QCZC608GBM0THZ` | Stopped and archived. |
| L2 mission | `01M6WFHF67CA84P6XMD5MA6QNW` | Stopped, archived, zero live sessions. |
| L2 member | `01M6WFHF78V37K2X5Z7N4WN16N` | Stopped. |
| L2 linked chat | `01M6WFH2HJPAC5QYPN3T3MBRPC` | Stopped and archived. |

Stopped and archived only the owned test fixtures through the development CLI, preserving history. Removed only the four owned fresh-folder trust entries from the selected managed Claude account. The first backup captured the default account; discovering that Runner selected the managed account meant its projects snapshot was captured after the first chat launch, before the other three launches. This limits the first launch's before/after configuration evidence. All roots were newly created; the managed account's unrelated project entries were unchanged from its captured snapshot through cleanup (`trust-cleanup.json`).

Quit the owned wrapper, stopped the candidate development daemon, verified neither remained running, unregistered the temporary bundle from LaunchServices, and released the QA lock. Installed Runner was left running. Retained the wrapper, scratch folders, role, transcript history, and evidence; removed no branches or worktrees. Detailed identities and cleanup results are in `identity.json`, `ledger.json` (one level above the evidence root), and `cleanup.json`.
