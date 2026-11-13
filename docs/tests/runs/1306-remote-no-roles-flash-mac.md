# #1306 remote empty-feed live QA — macOS

## Rerun — Passed (2026-11-13, 21:14–21:23 Asia/Shanghai)

Reran on 2026-11-13, 21:14–21:23 Asia/Shanghai, after the operator restored native control and authorized the same-Mac two-host fixture documented at origin/main `979772e5`. Tested implementation `eb52969234d1a77ef201d94300ad40ddc3ff3f52` / PR #1307 head `114ed43972ee3bd4cbf676cb6321b3032b1ef6d3` in the specified worktree. `git diff eb529692..114ed439 -- crates/` was empty, the source tree stayed clean, and no rebuild was needed. Both wrappers used the earlier candidate app and CLI copies; the app SHA-256 was `b42a663d7ae8db3b9952c844a558d865f21f53989600465bba8c96ee240ae962`, matching `target/debug/Runner`. macOS 26.6.2 (25G83), arm64; Runner/CLI 0.19.8. No automated suite was repeated.

The initial blocked attempt below remains historical evidence. Its prerequisites were resolved for this rerun; it was not an observed product failure.

### Selected matrix

| Check | Result | Observation and evidence |
| --- | --- | --- |
| Remote running zero-role mission with existing events: first open and switch back | Passed | Existing goal and ordinary posted `QA1306 HISTORY marker` displayed. No temporary No events yet or feed No roles yet/Add role appeared in observed frames. Three empty/populated switch-back pairs and a 17-frame transition sequence show the expected selections and content. `remote-populated-00.png`, `switch-populated-{0,1,2}.png`, `populated-transition-*.png`, `switch-sequence.json`, `populated-transition.json`, `transition-comparison.json`. |
| Remote running zero-role mission with zero events: loaded empty state and overview-update stability | Passed | Feed displays No events yet, rail displays No roles yet, and neither feed no-role card nor Add role appears. Three unrelated owned-mission renames were received in the sidebar while this mission stayed open. All 309 captured feed regions were pixel-identical over 18.2 seconds, without jump/flash; the region was `(500,190)`–`(2250,1580)`, excluding the deliberately changing sidebar. `remote-first-open.png`, `switch-empty-{0,1,2}.png`, `empty-transition-*.png`, `remote-empty-stable-start.png`, `remote-empty-stable-end.png`, `stable-sequence.json`, `stability-comparison.json`. |
| Local zero-role mission, MIS-EMPTY-03 empty-state portion only | Passed | Truly empty history shows No events yet; the centered feed No roles yet and green Add role remain positioned without overlap, with the rail notice/Add role and disabled composer. `local-empty-zero-events.png`; the ordinary CLI-created mission with lifecycle/goal events also preserved the no-role panel (`local-empty.png`). |

No Failed or Blocked selected check remains. NO REMAINING MUST-FIX QA ISSUES for this selected macOS matrix. This is not a full smoke/regression pass. Both ends ran the same candidate on one Mac through the actual hosted relay, not two physical computers or Windows/Linux. Native AX exposed only the GPUI window/menu shell, so screenshots drove app interactions. Browser approval and baseline evidence include complete Chrome AX. Natural relay initial loads settled between sampled frames: transition captures show the prior selection then the loaded new selection, without a sampled false empty state; no artificial network delay was injected and no claim is made about unsampled sub-frame behavior. Deterministic initial-loading ordering remains covered by the reviewed unit/UI regression tests.

### Fixture and isolation

Evidence root: `/private/tmp/runner-qa-1306/rerun/`. Wrapper bundles: `Runner QA1306 Host.app` (`com.wycstudios.runner.qa1306-host`) and `Runner QA1306 Client.app` (`com.wycstudios.runner.qa1306-client`). Launched byte-identical candidate binaries with child-only private homes `/private/tmp/r1306h` and `/private/tmp/r1306c`, stripping inherited `CLAUDE*`, `CODEX*`, `RUNNER_*`, `ANTHROPIC_*`, and `NO_COLOR`. Private CLI status confirmed each private `runnerd.sock`; see `host-status.json` and `client-status.json`. Did not use the user's ordinary runner-dev socket or installed Runner.

Chrome transport reconnected using the documented profile-window recovery. Recorded its signed-in `@yeelinks` account baseline before enrollment: no computers and no phones (`chrome-baseline-ax.txt`, `chrome-baseline.png`). Compared each native code/fingerprint with Chrome before approving only `QA1306 Host` (host ID `114714862503854080`, fingerprint `f0c6 7a41 9335 c437 8a98`) and `QA1306 Client` (host ID `114715038966611968`, fingerprint `e033 66f4 f380 6ebb 2da1`). Paired only Client → Host with a disposable run-owned PIN. Host's incoming Client pairing fingerprint was `b798 a005 9f59 bb61 1812`. See enrollment/approval screenshots and `fixture-ledger.json`.

Owned Host missions: `01M7AGW1MVR8DD8B4GH9Q7HH86` (ordinary lifecycle/goal events, renamed three times to generate overview updates), `01M7AGW9NT27BTXT21GWP3EBT0` (populated history, message event `01M7AGWJF30M2YZ0F17X2QNN0R`), and `01M7AGX1FWZAXF32G3N74CTPHW` (true zero-event fixture). Mission start emits lifecycle and goal events even without a supplied goal; for the last fixture, preserved its owned `events.ndjson` as `events.initial.ndjson` and created an empty log before reopening locally and remotely. The private CLI confirmed `events: []` (`zero-events.json`). No database access, implementation edits, global settings edits, roles, crews, agent chats or agent-account usage were needed. `remote-overview.json` and the before-cleanup mission lists capture independent structured state.

### Teardown

Stopped and archived all three owned Host missions; Client had no missions or chats. `mission-cleanup.json` records every result. Revoked the sole Client pairing on Host, forgot Host on Client, and confirmed Host devices `[]`. Both private `remote logout` commands confirmed removal from the account. Reloaded Chrome All hosts: exact computer-list baseline restored, `[]` / No hosts yet, with no phones (`chrome-final-ax.txt`, `chrome-final.png`). Closed only the account tab and recovery blank tab created for this run. Quit both wrappers through native Command-Q, stopped both private daemons with their private CLIs, unregistered both bundles, verified no private wrapper/daemon process survived, then released qa-lock and confirmed free. Kept scratch evidence and private fixture history. Repository remained clean.

---

## Initial blocked attempt (2026-11-13, 20:20–20:24 Asia/Shanghai)

### Candidate and scope

Attempted on 2026-11-13 at 20:20–20:24 Asia/Shanghai against reviewed, unpushed candidate `eb52969234d1a77ef201d94300ad40ddc3ff3f52`, branch `fix/1306-remote-no-roles-flash`, worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1306-remote-no-roles-flash`. The worktree was clean before the build; the only QA edit is this record. macOS 26.6.2 (25G83), arm64; Runner and development CLI 0.19.8.

This run selects the three checks in the clean review and coder handoff: populated remote zero-role feed initial load/switching, empty remote zero-role feed initial load and continuous stability across overview updates, and the local empty-state portion of MIS-EMPTY-03. Agent startup, message delivery, lifecycle and unrelated regression cases are outside scope.

### Results

| Check | Result | Evidence and limitation |
| --- | --- | --- |
| Remote zero-role mission with existing events: open/switch back without temporary No events yet or feed No roles yet, and display history | Blocked | Native computer use failed before returning any app state or screenshot. The development client's listed second test computer was also offline and unpaired; no paired development host was established. |
| Remote zero-role mission with zero events: show No events yet only after loading, keep the rail notice, omit the feed Add role/no-role panel, and remain stable across overview updates | Blocked | Same native-control and second-development-instance prerequisites. No continuous-view/frame-sequence observation was possible. |
| Local zero-role mission: preserve positioned No roles yet, Add role and No events yet | Blocked | Created a bounded zero-role development mission, but native control failed before opening or observing it. CLI fixture creation is setup evidence, not a UI pass. |

No live check passed or failed; all three required checks remain unresolved. Keep `needs-live-test` on any eventual PR. This attempt does not provide a clean QA verdict. No screenshot evidence exists because the native-control service failed before producing an accessibility tree or screenshot.

### Setup and evidence

Acquired `script/qa-lock acquire --timeout 540` as mission `01M7ACMGQ0K2YM7M7GE35Z565K`, handle `qa`, after the coder released its mistaken acquisition. No other development app process was running. Ran `env -u NO_COLOR make run` in the candidate worktree using its own `target/`; the build succeeded and launched `target/debug/Runner`. Automated tests were already reported by the coder and reviewer; QA did not rerun them.

Stopped only that owned bare process, created `/private/tmp/runner-qa-1306/Runner Screenshot Dev.app` with `script/screenshot-app bundle`, verified copied app and CLI bytes with `cmp`, and launched using `script/screenshot-app launch`, which strips inherited agent/mission variables. Bundle ID: `com.wycstudios.runner.screenshot-dev`. App SHA-256: `b42a663d7ae8db3b9952c844a558d865f21f53989600465bba8c96ee240ae962` for both source and wrapper copy. The absolute development CLI reported `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`, confirming the separate development namespace.

Initialized `@oai/sky` through `node_repl`. Both `sky.get_app_state({app: 'com.wycstudios.runner.screenshot-dev'})` and a subsequent `sky.list_apps()` returned `Sky Computer Use native pipe startup failed`. No alternative UI-control technology was used.

Development `remote computers --json` listed this client as `Mac-Studio-Test`; its paired online computers were `Mac-Studio` and `MacBook-Pro`, while `Macbook-Pro-Test` was offline and unpaired. QA did not operate the installed Runner or its missions as a remote fixture. The coder was asked whether an authorized second development fixture was available.

Local evidence root: `/private/tmp/runner-qa-1306/`. Preserved `app.log`, `evidence/binary-sha256.txt`, `evidence/dev-status-before.json`, `evidence/dev-status.json`, `evidence/computers-before.json`, `evidence/local-fixture.json`, and the candidate wrapper. These local artifacts are setup evidence only.

### Objects and cleanup

Created only mission `01M7ADRNV323ZPZPFE6351XAJB`, title `QA1306 local zero roles`, in `/private/tmp/runner-qa-1306/local`, with no crew or agents. Its CLI start result showed `status: running`; it was stopped and archived, and the archive response confirmed `status: completed` with `archived_at: 2026-11-13T12:23:13.905431Z`.

Quit the owned wrapper, unregistered it from LaunchServices, stopped the development daemon started by this launch (PID 87412), and confirmed both processes exited. Released the shared QA lock and confirmed `qa lock is free`. Left evidence files in place. No implementation, global configuration, pre-existing mission/session, installed-app data, branch, commit, push or PR was changed by QA.
