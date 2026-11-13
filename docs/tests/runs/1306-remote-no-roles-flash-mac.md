# #1306 remote empty-feed live QA — macOS

## Candidate and scope

Attempted on 2026-11-13 at 20:20–20:24 Asia/Shanghai against reviewed, unpushed candidate `eb52969234d1a77ef201d94300ad40ddc3ff3f52`, branch `fix/1306-remote-no-roles-flash`, worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1306-remote-no-roles-flash`. The worktree was clean before the build; the only QA edit is this record. macOS 26.6.2 (25G83), arm64; Runner and development CLI 0.19.8.

This run selects the three checks in the clean review and coder handoff: populated remote zero-role feed initial load/switching, empty remote zero-role feed initial load and continuous stability across overview updates, and the local empty-state portion of MIS-EMPTY-03. Agent startup, message delivery, lifecycle and unrelated regression cases are outside scope.

## Results

| Check | Result | Evidence and limitation |
| --- | --- | --- |
| Remote zero-role mission with existing events: open/switch back without temporary No events yet or feed No roles yet, and display history | Blocked | Native computer use failed before returning any app state or screenshot. The development client's listed second test computer was also offline and unpaired; no paired development host was established. |
| Remote zero-role mission with zero events: show No events yet only after loading, keep the rail notice, omit the feed Add role/no-role panel, and remain stable across overview updates | Blocked | Same native-control and second-development-instance prerequisites. No continuous-view/frame-sequence observation was possible. |
| Local zero-role mission: preserve positioned No roles yet, Add role and No events yet | Blocked | Created a bounded zero-role development mission, but native control failed before opening or observing it. CLI fixture creation is setup evidence, not a UI pass. |

No live check passed or failed; all three required checks remain unresolved. Keep `needs-live-test` on any eventual PR. This attempt does not provide a clean QA verdict. No screenshot evidence exists because the native-control service failed before producing an accessibility tree or screenshot.

## Setup and evidence

Acquired `script/qa-lock acquire --timeout 540` as mission `01M7ACMGQ0K2YM7M7GE35Z565K`, handle `qa`, after the coder released its mistaken acquisition. No other development app process was running. Ran `env -u NO_COLOR make run` in the candidate worktree using its own `target/`; the build succeeded and launched `target/debug/Runner`. Automated tests were already reported by the coder and reviewer; QA did not rerun them.

Stopped only that owned bare process, created `/private/tmp/runner-qa-1306/Runner Screenshot Dev.app` with `script/screenshot-app bundle`, verified copied app and CLI bytes with `cmp`, and launched using `script/screenshot-app launch`, which strips inherited agent/mission variables. Bundle ID: `com.wycstudios.runner.screenshot-dev`. App SHA-256: `b42a663d7ae8db3b9952c844a558d865f21f53989600465bba8c96ee240ae962` for both source and wrapper copy. The absolute development CLI reported `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`, confirming the separate development namespace.

Initialized `@oai/sky` through `node_repl`. Both `sky.get_app_state({app: 'com.wycstudios.runner.screenshot-dev'})` and a subsequent `sky.list_apps()` returned `Sky Computer Use native pipe startup failed`. No alternative UI-control technology was used.

Development `remote computers --json` listed this client as `Mac-Studio-Test`; its paired online computers were `Mac-Studio` and `MacBook-Pro`, while `Macbook-Pro-Test` was offline and unpaired. QA did not operate the installed Runner or its missions as a remote fixture. The coder was asked whether an authorized second development fixture was available.

Local evidence root: `/private/tmp/runner-qa-1306/`. Preserved `app.log`, `evidence/binary-sha256.txt`, `evidence/dev-status-before.json`, `evidence/dev-status.json`, `evidence/computers-before.json`, `evidence/local-fixture.json`, and the candidate wrapper. These local artifacts are setup evidence only.

## Objects and cleanup

Created only mission `01M7ADRNV323ZPZPFE6351XAJB`, title `QA1306 local zero roles`, in `/private/tmp/runner-qa-1306/local`, with no crew or agents. Its CLI start result showed `status: running`; it was stopped and archived, and the archive response confirmed `status: completed` with `archived_at: 2026-11-13T12:23:13.905431Z`.

Quit the owned wrapper, unregistered it from LaunchServices, stopped the development daemon started by this launch (PID 87412), and confirmed both processes exited. Released the shared QA lock and confirmed `qa lock is free`. Left evidence files in place. No implementation, global configuration, pre-existing mission/session, installed-app data, branch, commit, push or PR was changed by QA.
