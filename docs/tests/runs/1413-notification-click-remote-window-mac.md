# #1413 Local notification clicks while remote · macOS QA

## 2026-11-24 · Attempt 1

**Blocked before native testing.** The required checks remain unresolved. Retain `needs-live-test`.

The reviewed candidate was `86e7370bc7e9ab956620a37eba74dc5d3d56f704` on `fix/1413-notification-click-remote-window`. HEAD matched the handoff and the worktree was clean at preflight. macOS 26.6.2 (25G83), arm64. `cargo build --locked -p runner-cli -p runner-app` compiled the candidate app and CLI 0.20.3 in the worktree’s own target directory. The build log, binary hashes, preflight status, and lock result are under `/private/tmp/r1413.OkuflG/evidence/`.

`script/qa-lock acquire --timeout 540` timed out while iOS #87 mission `01M86ECC4DRJS5S32D8A98TAFX`, handle `qa`, held the shared screen lock. That mission was awaiting its existing “Release QA lock” decision. No candidate app or daemon was launched, no relay hosts were enrolled, and no live screenshot was available. No application settings, test chats, or missions were changed or created.

| Check | Result | Reason |
| --- | --- | --- |
| 1. Only window is remote | Blocked | The same-Mac relay fixture requires the screen lock. A native notification click must open a new local window and preserve the original remote tab and terminal control. |
| 2. A local window exists | Blocked | The same-Mac relay fixture requires the screen lock. A native click must focus the existing local window and preserve the remote window. |
| 3. All windows are local | Blocked | The local native click regression requires the screen lock. |
| 4. Usage alert while remote | Blocked | Prepared a scratch Antigravity usage provider that reports 90% without contacting an account. The native check requires the screen lock and has yet to run. |
| Windows and Linux notification clicks | Skipped | The brief selects macOS for live validation. |

## 2026-11-24 · Attempt 2

**Blocked.** A second `script/qa-lock acquire --timeout 540` timed out with the same holder. The result is in `qa-lock-attempt-2.txt` in the evidence directory. The candidate and matrix above still apply. No native setup, fixture enrollments, or notification clicks ran during the retry. QA reported the blocker through Runner. Resume the required checks after iOS #87 releases the shared lock.

## 2026-11-25 · Attempt 3 · PR #1473 head `717285ac8eb18a769fb8950398d4de9edc02f89c`

Acquired the shared lock after iOS #87 released it. Verified a clean worktree and that the QA record was the only difference from reviewed `86e7370b`. Ran `make run` with a private HOME and the mission environment cleared, then stopped the bare app and copied the built app and CLI into disposable wrappers. Both copies matched the build artifacts before signing. Both private CLIs reported Runner 0.20.3 (dev) and separate `runnerd.sock` endpoints. All evidence is under `/private/tmp/r1413.OkuflG/evidence/`.

macOS rejected the initial wrapper because its linker signature did not match the bundle identifier. Ad-hoc signing the disposable Client with its own bundle identity satisfied the identity check. macOS still could not validate the bundle staged under `/private/tmp`. Moving the signed Client bundle to the worktree’s `target/qa1413/` resolved registration and Jason granted notification permission for this bundle. Every file-backed Mach-O section, including `__TEXT,__text`, matched the candidate executable. The CLI remained byte-identical and app data stayed in the private HOME. Evidence includes `macos-wrapper-notification-log.txt`, `macos-wrapper-signed-auth-noted.txt`, `macos-worktree-bundle-auth.txt`, `signed-wrapper-code-verification.txt` and `client-permission-after-approval.png`.

One real pi 1.6.0 chat with DeepSeek Flash and low effort completed unviewed in the local-only case. The daemon reported a completed unread turn and macOS recorded delivery to Notification Center. Focus delayed the banner. The native Computer Use tool timed out when targeting Notification Center, so the notification click remains pending. Evidence includes `chat-3-after-submit.json`, `local-only-prompt.png` and `macos-local-notification-actions.txt`.

The two isolated instances enrolled in the hosted relay through Chrome; each approval fingerprint matched its own CLI, and only Client → Host was paired. The Client CLI retrieved the Host's single owned terminal tab; the Host shell displayed the `QA1413-HOST-READY` marker. No remote click verdict yet. The baseline computer list was empty. The two enrollments, one chat and one shell are owned by this QA run.

### Interruption and teardown

The apps and their private daemons exited during the pause. Private daemon logs record SIGHUP exits at 08:41:55 and 08:41:58 local time. At 09:22 the process inventory showed both apps and daemons gone. The pause ended without an observed notification click, and checks 1–4 remain Blocked. The software-routing checks have no native verdict. Retain `needs-live-test`.

Restarted only the two private daemons with their original data directories and sockets. Daemon startup resumed the two owned sessions. Archived the pi chat and verified its archive timestamp. Stopped the owned zsh shell and verified the Host session list was empty. Revoked the sole disposable Client pairing on Host, forgot Host on Client, and verified both private CLIs reported that the relay removed their enrollments. Both private remote states were Off. Chrome opened the sign-in page when asked to refresh, so the browser could not independently reconfirm the zero-host baseline. Closed the QA-created Chrome tab.

Stopped both private daemons and verified no fixture app, daemon or restarted PTY survived. LaunchServices contained no registrations for the fixture paths after cleanup. Evidence and the signed Client bundle under `target/qa1413/` were preserved. The screen lock was released.

## 2026-11-25 · Attempt 4 · PR #1473 head `717285ac8eb18a769fb8950398d4de9edc02f89c`

**Check 3 passed. Checks 1, 2 and 4 are blocked on Runner Hub sign-in.** QA reacquired the screen lock and reused the verified signed candidate app with a fresh private HOME. The only dirty file was this record. The absolute CLI confirmed version 0.20.3 and its private socket. Chrome opened the Runner Hub sign-in form in the profile used for the previous checkout, so no relay enrollment was created. Jason’s action is pending.

Pi 1.6.0 stalled before showing its composer. A bounded retry with pi’s startup network checks disabled in the private fixture also stalled. Archived that unused chat and used Codex 0.171.0 with GPT-6-Luna low and the existing sign-in for the local regression. Evidence is under `/private/tmp/r1413r.xPcMRd/evidence/`.

| Check | Result | Evidence and observation |
| --- | --- | --- |
| 1. Only window is remote | Blocked | The previous relay fixture was removed after interruption. A new hosted relay enrollment requires the signed-in Chrome session. |
| 2. Existing local window with remote window active | Blocked | The same relay prerequisite is pending. |
| 3. All windows are local | Passed | Submitted a bounded prompt, went to Settings → General and hid the sole client window before completion. The daemon reported an unread completed turn and macOS posted its Finished card. Matched the displayed title to the same chat’s daemon title and the card’s delivery to the disposable bundle ID. Jason authorized real mouse control. Clicked the owned card in Notification Center and macOS recorded its foreground contents response for that exact chat. The existing window came forward on the chat showing `QA1413-LOCAL-CODEX`, and its unread status cleared. See `local-codex-window-before-click.png`, `local-codex-notification.png`, `local-codex-window-after-click.png`, `local-codex-os-click.txt` and `local-codex-state-{before,after}-click.json`. |
| 4. Usage alert while remote | Blocked | The account-free usage provider is ready. Running with a remote window requires the same hosted relay enrollment. |
| Windows and Linux notification clicks | Skipped | Outside the selected macOS matrix. |

Restored the private pi flag, archived both owned chats, and verified the active session list was empty. Quit the private client and stopped its daemon. Unregistered the signed wrapper and verified no fixture app or daemon was live. Removed the private Codex trust entry and temporary sign-in link. Released the screen lock. The QA-created Chrome sign-in tab is available for Jason to complete the relay sign-in. Retain `needs-live-test`.

### Final crew verdict

Jason deferred the relay checks and will run them on his own computer. Check 3 remains Passed on `717285ac`. Checks 1, 2 and 4 were not run and are handed to Jason. The prior interruption and sign-in blocker are retained above. The crew’s live QA is complete with those checks outstanding. Keep `needs-live-test` until the required remote checks pass.

The QA-created sign-in tab was closed. No fixture apps, daemons, PTYs or relay enrollments remain, and the screen lock is free.

## 2026-11-25 · Attempt 5 · PR #1473 head `8c4415b84724ade3dbf005a4678a0c93f08338a4`

**Passed. NO REMAINING MUST-FIX QA ISSUES.** Jason restored Chrome sign-in and resumed crew QA. Checks 1, 2 and 4 passed. Attempt 4 supplies the local-only check 3 pass. The clean retest head differs from `717285ac` only in the brief and this run record. QA reused its intact development binaries, verified the exact prior artifact hashes, and launched the signed Client and byte-identical Host with separate private HOME directories and daemon sockets. The screen lock was held for native interaction through cleanup.

Both disposable computers were enrolled through Chrome with their approval fingerprints matched to their own private CLIs. Only Client → Host was paired. The client picker showed an encrypted connection through the hosted relay. Both instances ran on one Mac. The two completion cases used Codex 0.171.0, GPT-6-Luna and low effort with bounded prompts in fresh scratch directories. Native Computer Use drove the candidate apps. Real mouse clicks targeted the owned macOS notification cards, identified by their title and matching bundle-and-request logs. Screenshot and structured evidence paths below are relative to `/private/tmp/r14q.fO6iBa/evidence/`.

| Check | Result | Observation and evidence |
| --- | --- | --- |
| 1. Only window is remote | Passed | With sole client window `228213` on Host’s zsh tab, the local chat completed unread. Clicking its Finished card opened new local window `228289` on `QA1413-REMOTE-ONE` and cleared unread status. Original window `228213` retained its selected remote tab and wrote `QA1413-CHECK1-REMOTE-CONTROL` into the Host scratch directory through that terminal. See `check1-remote-before-notification.png`, `check1-notification-card.png`, `check1-local-after-click.png`, `check1-remote-terminal-control.png`, `check1-windows-{before,after}.txt`, and `check1-os-click.txt`. |
| 2. Existing local window with remote window active | Passed | Focus was on remote window `228213` and the local window was displaying a different chat before completion. A real click brought existing local window `228289` forward on `QA1413-REMOTE-TWO` and cleared unread status. There were two windows before and after. The original zsh tab retained its transcript and wrote `QA1413-CHECK2-REMOTE-CONTROL` on Host. See `check2-remote-active-before.png`, `check2-notification-card.png`, `check2-existing-local-after-click.png`, `check2-remote-terminal-control.png`, `check2-windows-{before,after}.txt`, and `check2-os-click.txt`. |
| 3. All windows are local | Passed | Verified in Attempt 4 on source-identical `717285ac`. |
| 4. Usage alert while remote | Passed | A scratch Antigravity provider returned synthetic weekly usage of 10%, then 90% after remote window `228213` had focus. It contacted no account. Clicking the native 90% card brought existing local window `228289` from Settings → General to Settings → Usage, with the synthetic 90% shown. Original window `228213` kept the encrypted relay and its terminal wrote `QA1413-CHECK4-REMOTE-CONTROL` on Host. See `check4-local-left-usage.png`, `check4-notification-card.png`, `check4-local-usage-after-click.png`, `check4-remote-terminal-control.png`, `check4-remote-relay-after-click.png`, `check4-os-click.txt`, and `usage-fixture-invocations.txt`. |
| Windows and Linux notification clicks | Skipped | Native validation covered the same-Mac hosted-relay fixture. Cross-platform CI is separate. |

Restored the private Antigravity executable to Auto, archived the two owned chats, and closed the Host shell. Verified empty active session lists, revoked the sole Client pairing on Host, forgot Host on Client, and removed both disposable enrollments. Chrome returned to its exact empty computer-list baseline. Closed the QA Hub and browser recovery tabs, quit both app wrappers, stopped both private daemons, unregistered the wrappers, and removed the owned private Codex trust entries and temporary sign-in link. Verified no QA process remained and released the lock. Evidence and candidate binaries were preserved.
