# #1385 remote Settings and terminal control — macOS QA

## Attempt 1 — 2026-11-19

**Verdict: Blocked.** Both candidate apps launched with separate development daemons. Native Computer Use failed before Chrome enrollment or any live Settings check. The remote behavior remains unverified.

### Candidate and fixture

- Reviewed candidate `fb3668254cbdf225e1626320e9d0ab923d00fc5e` on `fix/1385-remote-settings-keep-control`. The named worktree was clean at build time. This record is the only QA edit.
- macOS 26.6.2 (25G83), arm64, 00:57–01:02 Asia/Shanghai. Runner and CLI version 0.20.1, development build. `make run` built and launched with a private host HOME and inherited mission variables and `NO_COLOR` removed. Automated suites were the coder's gate.
- Held `script/qa-lock acquire --timeout 540` before building through cleanup. Copied the candidate into `com.wycstudios.runner.qa1385-host` and `com.wycstudios.runner.qa1385-client` wrappers. Both app and CLI copies matched the worktree build with `cmp` and SHA-256. App hash `547f9f192471280cf212ca5807e656492f4cc3af77a576f771fd7fca191d59cc`, CLI hash `4f607370cebcc2a965dc4f61cce556c0a42488a110ae5ad95ad3e3a24a40572b`.
- Evidence is local at `/private/tmp/r1385.TG29a3/`. `make-run.log`, `evidence/artifact-hashes.txt` and the two `evidence/*-status.json` captures verify the build and the distinct private sockets under `/private/tmp/h1385.982PX` and `/private/tmp/c1385.g90zJ`. Both remote statuses were off and both session lists were empty.

### Selected live checks

Each check targets a changed entry or exit path. For each path, the required evidence is the same controlled host tab before, during and after this computer's Settings, plus a keystroke that reaches the host immediately after returning without a terminal click or a new Take control action.

| Case | Platform / variant | Result | Evidence and reason |
| --- | --- | --- | --- |
| REMOTE-WIN-01 / REMOTE-TERM-01 | mac / footer gear → local Settings → Back | Blocked | Chrome baseline and disposable enrollment could not start. No native screenshot or terminal input observation. |
| REMOTE-WIN-01 / REMOTE-TERM-01 | mac / host popup → Remote access settings → Back | Blocked | Same fixture blocker. No native screenshot or terminal input observation. |
| REMOTE-WIN-01 / REMOTE-TERM-01 | mac / Command-comma → Settings → Escape | Blocked | Same fixture blocker. No native screenshot or keyboard dispatch observation. |
| REMOTE-WIN-01 / REMOTE-TERM-01 | win / native Windows | Blocked | No native Windows instance was available in this macOS QA run. |

### Blocker and cleanup

The Chrome integration returned `Browser is not available: chrome` even after confirming the running profile's extension and native host and requesting a fresh profile window. The operator approved native Computer Use in the same signed-in Chrome profile. Native app discovery and state inspection failed with `Sky Computer Use native pipe startup failed`, including after resetting the native connection and when targeting the exact Runner host wrapper. See `evidence/native-control-error.txt`. The error occurred before any page or app interaction. The computer list was never opened, and no enrollment, pairing, test chat, agent process or settings change was made. The profile recovery requested a blank Chrome window. Its existence and closure could not be verified through the unavailable controls.

QA stopped both owned wrapper processes by their verified PIDs because native Quit was unavailable, then stopped both private daemons through the copied CLIs. Both sockets disappeared and no owned processes survived. Both wrappers were unregistered and removed, and the QA lock was released. Logs and private fixture evidence remain outside Git. See `evidence/cleanup.txt`. The installed app and pre-existing development sessions were left alone.

Retry the three macOS checks on the same reviewed source once native control can reach Chrome and both Runner wrappers. Capture the baseline hosted computer list before enrolling and compare it after teardown. No clean-QA verdict is supported by this attempt.
