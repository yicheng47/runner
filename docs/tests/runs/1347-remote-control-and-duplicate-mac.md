# #1347 remote control and duplicate windows — macOS QA

## Attempt 1 — 2026-11-16

**Verdict: Failed.** Host Take back survives passive updates, but adding a host split pane lets the client automatically take the existing host-owned terminals back. Required duplicate-overlay and secondary-resize coverage remains incomplete. This is a feature QA run, not a full regression pass; keep `needs-live-test`.

### Candidate and environment

- Reviewed candidate: `436effe6910e61aba27851ad4e570c32ea87d91d`, branch `fix/1347-remote-control-and-duplicate`; clean worktree at build and unchanged candidate through cleanup.
- Worktree: `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1347-remote-control-and-duplicate` (the mission's `/Users/jason/repos/yicheng47/runner/…` path resolves to this checkout).
- macOS 26.6.2 (25G83), Apple Silicon; Runner 0.19.9 development build.
- Built with `cargo build -p runner-cli -p runner-app`; successful in 35.27 seconds. Automated suites were the coder's gate and were not rerun by QA.
- App SHA-256: `14432b1c384e797f458d12189b368ddc4054a1a2a22afa58b06002e6dff6eb74`; CLI SHA-256: `2abee14d2bb5929f459f82caae7b004ba3108b27c443c245690ac92a988c687c`.
- Followed the authorized same-Mac two-host relay fixture: byte-identical `Runner` and `runner-agent-cli` copies in two private wrappers, through the real relay and Chrome's existing signed-in account. This is not physical two-machine or Windows/Linux coverage. Used shell terminals only; no additional coding agents, agent credentials, global agent settings or normal development data were used.
- Private wrappers: `/private/tmp/r1347-75dx_ooe/Runner 1347 Host.app` and `Runner 1347 Client.app`; bundle IDs `com.wycstudios.runner.qa1347-host` and `com.wycstudios.runner.qa1347-client`.
- Private homes: `/private/tmp/h1347-_t5o2abq` and `/private/tmp/c1347-yowtym2q`. Each CLI's `status --json` independently confirmed its private `com.wycstudios.runner-dev/runnerd.sock`. Wrapper launches cleared inherited `RUNNER_*` and `NO_COLOR` variables.
- Held `script/qa-lock acquire --timeout 540` throughout fixture setup, native testing and cleanup; released it after both private apps and daemons exited.
- Evidence is retained locally at `/private/tmp/r1347-75dx_ooe/evidence/`; setup captures can contain account/enrollment details and are not committed.

### Selected checks and results

These variants exercise the changed auto-claim, host-control protection, recovery and remote-window ownership paths from `REMOTE-TERM-09` and the review handoff. Unchanged runtime lifecycle, IME, missions and unrelated settings were outside scope.

| Check | Result | Observation and evidence |
| --- | --- | --- |
| Free visible remote terminal accepts typing without Take control | Passed | Selecting the paired host opened its running shell without a no-controller banner. Typed immediately, without a terminal or Take control click; the host executed the marker command and created `auto-input.txt` containing `auto-ok`. `04-client-remote-open.png`, `05-auto-input.png`, `06-host-controlled.png`. |
| Host Take back, passive updates and blocked client input | Passed | Host Take back produced the client banner “QA1347 Host took this terminal back” and Take control. A client command did not create `forbidden-client-input.txt`. Twelve host output lines arrived without removing the banner or auto-stealing control. `07-client-taken-back.png`, `08b-taken-back-output.png`; the complete twelve-line output also appears in `09-manual-recovered.png`. |
| Explicit client Take control | Passed | Clicked Take control once; the banner cleared and immediate typing created `manual-input.txt` containing `manual-ok`. `09-manual-recovered.png`. |
| Host adds a split pane after Take back | **Failed** | Existing Desktop-owned panes became client-controlled automatically when a new shell split was created on the host. Confirmed with a single client window and again with another duplicate window open. No client Take control click occurred. `17-host-layout-picker.png`, `18-host-split-empty.png`, `24-single-client-takenback.png`, `26-single-client-host-split-down.png`, `33-host-split-control-repro.png`, `34-existing-pane-stolen-input.png`. |
| Split host tab after client disconnect and reconnect | Passed | Quit the private client, stopped its private daemon, relaunched the same frozen wrapper and selected the host. All three running split panes attached without a no-controller banner or Take control click. Typed immediately into the initially focused pane, then selected each other pane normally; all three marker files were created and `stty size` returned. `28-split-recovery.png`, `29-split-recovery-input.png`; `split-one.txt`, `split-two.txt`, `split-three.txt` in the scratch root. |
| Two remote windows: immediate input after switching focus | Passed, limited | Opened another client window using Shift-Command-N, selected the same host, and cycled windows using Command-grave. Commands submitted without a second terminal click reached the host and appended `B`, `A`, `B2`, `A2` in order to `focus-input.txt`. Different window sizes distinguish the captured windows. This proves the exercised input path only; it does not prove secondary input suppression or duplicate rendering. `11-window-b-primary.png`, `12-focus-window-a.png`, `13-focus-both-directions.png`, `14-two-window-arrangement.png`, `15-other-window-primary.png`, `31-second-remote-window.png`, `32-after-cycle.png`. |
| Secondary blank surface and DuplicateSubjectOverlay; Focus other window / Stay here | Blocked | Native automation exposes the current front window; raising the other window promotes it and clears its secondary state. No reliable inactive-window screenshot was obtained. Do not infer this check passed from successful primary-window typing. Required native confirmation remains. |
| Resizing secondary window does not resize host terminal | Blocked | Could not reliably target and capture an inactive client window without promoting it. Primary-window arrangement and sizing are not evidence of secondary resize isolation. Required confirmation remains. |
| Windows/Linux variants | Blocked | No native Windows/Linux environment was exercised by this macOS QA slot. |

### Must-fix finding

**P1: creating a host split steals the tab back after host Take back.** Reproduction: open a host shell from the client; on the host click Take back; verify the client shows the taken-back banner; on the host use Split Right, or use Split Down in an existing split tab. The host immediately shows “Controlled from QA1347 Client” covering the tab, and the existing client panes lose the taken-back banner. Expected: automatic claims must preserve Desktop control of existing panes, with explicit client Take control required to seize them. Actual: typing into the original client pane after the split, without Take control, created `stolen-input.txt` containing `existing-pane-stolen`. This establishes an actual takeover, not just a misleading group overlay. Baseline attribution was not established; the implementation cause is left to the coder.

Reported immediately to coder and reviewer through Runner, followed by the single-window reproduction and filesystem confirmation. A reviewed fix must rerun host Take back plus split creation, existing-pane input protection, and affected split recovery. Complete the blocked duplicate-overlay/secondary-resize checks on the reviewed candidate as well. The failure remains failed despite successful separate recovery coverage.

### Cleanup

All four owned shells were stopped/closed, and the host CLI's final `session list --json` returned `[]`. Their IDs were `01M7H7M84BTTH463PBKRD1B1NW`, `01M7H7THCSZX4R94H9C0BCTH6Z`, `01M7H7X7Y19PVFGK1K88ATHF89` and `01M7H80SPYG12RPE54Q9DJFS2W`. The first archive attempt was refused because shell terminals close rather than archive; cleanup then used supported session stop and native pane closure. No test missions or roles were created. Scratch homes and evidence were retained without deleting history files.

Revoked only the disposable Client pairing on Host, forgot Host on Client, and ran each private CLI's `remote logout`. Chrome's computer list returned to its exact empty baseline (`00-relay-baseline.png`, `99-relay-cleanup.png`). Closed the run-created Chrome tab, quit both wrappers natively, stopped both private daemons, unregistered both wrappers with `lsregister -u`, verified no private QA processes remained, and released the QA lock. Cleanup evidence: `cleanup-objects.json`, `cleanup-shells.json`, `cleanup-final-sessions.json`, `cleanup-processes.txt`.

No implementation changes, commits, pushes, PR edits or merges were made by QA. No PR existed during this attempt; coder must carry these results and unresolved checks into the eventual description and retain `needs-live-test`.

## Attempt 2 — 2026-11-16

**Verdict: NO REMAINING MUST-FIX QA ISSUES in the completed checks.** The Attempt 1 takeover reproduction now passes for both split directions, and free versus host-held reconnect behavior passes. Required inactive-window duplicate rendering/buttons and secondary resize isolation remain **Blocked**, as does native Windows/Linux coverage. This is not a full feature or cross-platform pass; retain `needs-live-test` and disclose these gaps in the PR. Attempt 1 above is preserved verbatim and remains the failed historical attempt.

### Candidate and environment

- Reviewed candidate: `e5a170c2c245edcd976f80bd1952c7d3a676d7eb`, clean branch `fix/1347-remote-control-and-duplicate` in the same worktree. Coder handed off the candidate after clean reviewer sign-off; QA made no implementation changes.
- Same macOS 26.6.2 (25G83), Apple Silicon, Runner 0.19.9 development environment and authorized real-relay, same-Mac fixture as Attempt 1. No physical second machine or Windows/Linux environment was used.
- `cargo build -p runner-cli -p runner-app` succeeded in 15.54 seconds. No automated suite was rerun by QA. Frozen wrapper copies were byte-identical to the built candidate; coder was notified after the copies were ready.
- App SHA-256: `454f4fcd9cf1b99558baf9a79ac924638a8e7f6791b4146548e20f90232ae0fb`; CLI SHA-256: `2abee14d2bb5929f459f82caae7b004ba3108b27c443c245690ac92a988c687c`.
- Wrappers: `/private/tmp/r1347b-sc7gpt7t/Runner 1347B Host.app` and `Runner 1347B Client.app`; bundle IDs `com.wycstudios.runner.qa1347b-host` and `com.wycstudios.runner.qa1347b-client`.
- Private homes: `/private/tmp/h1347b-ksa7lezu` and `/private/tmp/c1347b-1uk0r5c4`; both absolute CLIs independently confirmed private development sockets. Launches cleared inherited `RUNNER_*` and `NO_COLOR`. Only three owned shell terminals were created; no coding agents or missions were launched.
- Held the shared QA lock through setup, testing and teardown, then released it. Evidence and retained private fixture data: `/private/tmp/r1347b-sc7gpt7t/`; screenshots and structured captures are in its `evidence/` directory. Enrollment/account captures remain local and are not committed.

### Selected checks and results

The clean-review handoff selected the failed takeover variants, affected split recovery, and the remaining duplicate-window checks. Reconnect was induced by temporarily turning **Allow remote access** off and back on in the disposable host's native Settings, leaving the client window and tab open. This tests transport recovery without an explicit client tab reopen. The host's `remote.json` and UI settings were backed up first; the remote settings matched the backup after restoration (`settings-restoration.txt`).

| Check | Result | Observation and evidence |
| --- | --- | --- |
| Initial free opening and immediate input | Passed | Opened the host shell through the client computer picker and typed without clicking Take control or the terminal. `initial-free.txt` contains `initial-free-ok`. No no-controller banner appeared. `05-initial-free-open.png`, `06-initial-free-input.png`, `07-host-controlled.png`. |
| Host Take back, then host Split Right | Passed | Clicked host Take back, then Split Right. Both client panes kept the taken-back banner and Take control; attempted input into the original client pane did not create `forbidden-right.txt`. Host remained usable locally. `08-host-split-right.png`, `09-client-after-split-right.png`. |
| Host Split Down while the tab remains host-held | Passed | Added Split Down in the original host pane. All three client panes retained the taken-back banner; attempted input into the original pane did not create `forbidden-down.txt`. This reruns the exact previously failing behavior. `10-host-split-down.png`, `11-client-after-split-down.png`. |
| Reconnect with host-held split panes | Passed | Paused and restored host remote access with all three panes held by Desktop. Client showed Reconnecting, then recovered with all three taken-back banners intact and no control theft. Input attempt did not create `forbidden-held-reconnect.txt`. `15-host-paused.png`, `16-client-disconnected-host-held.png`, `17-host-restored.png`, `18-held-after-reconnect.png`. |
| Explicit client Take control after the host-held checks | Passed | Clicked Take control once, then typed without another terminal click. `manual.txt` contains `manual-ok`; client input was restored. `19-manual-control.png`. |
| Reconnect with free split panes | Passed | Paused host remote access after client control, then restored it. Waited for the visible Reconnecting state to settle; all three panes became usable without a control click or no-controller prompt. Immediate input in the focused pane and normal pane selection for the others created `reconnect-one.txt`, `reconnect-two.txt` and `reconnect-three.txt`, with the corresponding one-line markers; `stty size` returned in each pane. `20-client-free-disconnected.png`, `21-host-setting-restored.png`, `22-free-reconnected.png` (still reconnecting), `23-free-ready.png`, `24-free-reconnect-input.png`. |
| Two remote windows, two-way focus transfer and immediate typing | Passed | Opened a second client window with Shift-Command-N and selected the same host tab. Typed in the new window, cycled to the original with Command-grave, typed without a terminal click, then cycled back and typed again. `focus.txt` contains exactly `B1`, `A1`, `B2`, once each and in order. Screenshots show the original window's previously focused lower-left pane receiving `A1`, while the new window uses its first pane. `25-new-window.png`, `26-window-b-remote.png`, `27-focus-a-before-input.png`, `28-focus-a-input.png`, `29-focus-b-input.png`. This proves the exercised primary input transfers, not inactive-window suppression. |
| Inactive secondary blank surface, DuplicateSubjectOverlay and its buttons | Blocked | Retried with the two real client windows above. The native control interface exposes the current front window; cycling/raising the other window promotes it, so these captures show primary state rather than a verifiable inactive secondary overlay. No overlay/button pass is claimed. Human or other supported inactive-window capture remains required. |
| Secondary window resize isolation | Blocked | The same inactive-window targeting limitation prevented a verified resize while that window stayed secondary. No host-size isolation pass is inferred from primary-window captures. |
| Windows/Linux live variants | Blocked | No native Windows/Linux environment available to this macOS QA slot; builds or CI cannot substitute for native coverage. |

No failed behavior was observed in Attempt 2. The earlier takeover finding is resolved for the tested Split Right and Split Down paths; Attempt 1 is retained unchanged. The two inactive-window checks and cross-platform live variants remain unresolved requirements, disclosed rather than counted as passes.

### Cleanup and handoff

Stopped/closed the three owned shell sessions with each exact ledger ID: `01M7HA86P15BDSSJ4ZQZP3HFM3`, `01M7HA9CPFYM0EKH6BSERK48MY`, `01M7HA9QQJZAJN04Y80J3GPMWB`. The host's final `session list --json` returned `[]`; shell terminals close rather than archive. No missions, roles or agent trust entries were created.

Restored **Allow remote access** to On before teardown; parsed `remote.json` matched its backed-up baseline exactly. Revoked only the disposable Client pairing, forgot Host on Client, and logged both private enrollments out. Chrome returned to the exact empty host-list baseline (`00-baseline.png`, `99-relay-cleanup.png`); closed the run-created tab. Quit both wrappers natively, stopped both private daemons, unregistered both wrappers, verified no private processes remained, and released the QA lock. Evidence: `cleanup-objects.json`, `cleanup-processes.txt`, `settings-restoration.txt`. Private homes, shell history and screenshots remain retained under `/private/tmp/`.

QA appended only this Attempt 2 record. No implementation changes, commits, pushes, PR edits or merges were made. No PR existed during testing; coder owns including these results and keeping `needs-live-test` on the eventual PR until every required live check passes.
