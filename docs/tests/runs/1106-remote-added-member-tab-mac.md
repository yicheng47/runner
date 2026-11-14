# #1106 remote added-member tabs — macOS live QA

Candidate: `08986e0e4070dcc48b3d437348ff71b1cc045d7b`, branch `fix/1106-remote-added-member-tab`, worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1106-remote-added-member-tab`. Working tree was clean at start. Candidate passed reviewer before QA handoff. Automated results belong to the coder and are separate from these live results.

Environment: macOS 26.6.2 (25G83), Apple Silicon. QA lock acquired under mission `01M7AVDDW148M4VNR9R554ZCSC`, handle `qa`. Authorized two-instance fixture uses byte-identical candidate builds on one Mac through the real hosted relay, not physical two-machine or Windows/Linux coverage.

Scratch and evidence: `/private/tmp/qa1106-7rx890y1`; host HOME `/private/tmp/q11h-m27i8x9d`; viewer HOME `/private/tmp/q11c-qao5320w`. Both app namespaces are private `com.wycstudios.runner-dev`; existing development and installed app namespaces are untouched.

Selected checks cover: additions while away and adjacent rail/composer/pane surfaces; live push additions; closed-tab/order/selection/drawer/rail retention across navigation, same-host reconnect and stop/resume; stopped-mission roster reopening. No full regression was requested.

## Fixture

Chrome automation initially unavailable; supported profile-window recovery succeeded without installing or changing extensions. Local enrollment service `http://localhost:5173` was unavailable; used configured production relay `https://runner.yeelinks.com.cn`. Chrome showed no hosts at baseline (`evidence/hub-baseline.png`). Disposable enrollments only.

Build: `make run` with `NO_COLOR` and implementation mission variables removed, private host HOME, and existing Cargo/Rustup homes. Build log: `evidence/make-run.log`.

## Results

All four selected checks Passed on 2026-11-14 (Asia/Shanghai). NO REMAINING MUST-FIX QA ISSUES. No selected check is Blocked, Failed or Skipped. This is a feature pass, not a full regression pass.

| Check | Result | Observed behavior | Screenshot evidence under the scratch root |
| --- | --- | --- | --- |
| 1 — Added while viewer away; adjacent surfaces | Passed | Opened the mission with `a` and `b`, left for a native-created remote shell tab, ran host `mission add-role … qa1106 --handle extra-writer`, returned. Tabs were `a`, `b`, `extra-writer`; rail card and joined notice appeared; composer `@` roster included the new member. Mission feed, terminal tabs and rail remained usable. | `evidence/check1-away.jpg`, `evidence/check1-return.jpg`, `evidence/check1-recipients.jpg` |
| 2 — Added while shown | Passed | Ran host `add-role --handle live-writer` with the mission shown. New tab, rail card and joined notice appeared live without navigating away. | `evidence/check2-live.jpg` |
| 3 — Closed tab and presentation across navigation, reconnect, stop/resume (REMOTE-MIS-03) | Passed | Selected `b`, closed `a`, opened the terminal drawer and Info rail. Left for the shell tab. Turned host Allow remote access off, observed viewer Reconnecting while away, restored it to On, observed the same host connected through encrypted relay, then returned. `a` stayed closed; order `b`, `extra-writer`, `live-writer`, active `b`, drawer and Info rail held. Host stop/resume also preserved them. | `evidence/check3-layout-before.jpg`, `evidence/check3-layout-info-before.jpg`, `evidence/check3-disconnected-away.jpg`, `evidence/check3-after-reconnect.jpg`, `evidence/check3-stopped.jpg`, `evidence/check3-after-resume.jpg` |
| 4 — Stopped mission reopened | Passed | Stopped the mission on host, left to the shell tab, returned and selected Sessions rail. All four members were listed Stopped, including both additions. The deliberately closed `a` tab stayed closed; active `b` and drawer held. | `evidence/check4-reopened-stopped.jpg` |

Structured evidence: `evidence/overview-after-away.json`, `evidence/mission-after-resume.json`, `evidence/overview-stopped-reopened.json`. Native `b` terminal showed `QA1106 READY`; test roles used `gpt-6-luna`, low effort, tiny no-tool tasks. Private development mission settings started in bypass mode; QA did not change that mode. No agent activity beyond fixture prompts is counted as a behavior check.

## Fixture ledger and limitations

Host computer `114768352353714176`; viewer computer `114768504602755072`; role `01M7AWZSE01W7QGK20QZPP2FAX`; crew `01M7AWZXJB5NNWQ0VHKYPBWVAA`. Main test mission `01M7AX257G01VZN51N0Y39A86W`. Initial fixture mission `01M7AX0C9GQ41MM42F9JFFVMNB` used unavailable `gpt-5.1-codex-mini`; its visible unsupported-model result was preserved in `evidence/initial-member-terminal.jpg`, then the fixture was stopped/archived and replaced before the selected checks. Runtime availability was read from the existing Codex model registry. No runtime startup pass is claimed for that first attempt.

Wrapper bundle IDs: `com.wycstudios.runner.qa1106-host` and `com.wycstudios.runner.qa1106-client`. Wrappers live under the scratch root as `Runner 1106 host.app` and `Runner 1106 client.app`. Both executables and both `runner-agent-cli` sidecars were byte-compared with this worktree’s build before launch. Hashes are in `ledger.json`. CLI status verified `/private/tmp/q11h-m27i8x9d/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`; viewer uses the equivalent private viewer HOME endpoint. Both report `0.19.8 (dev)`.

This run covers one Mac, same candidate at both ends, through the real hosted relay. No physical two-machine, Windows or Linux pass is claimed. Member removal while away is covered by the coder’s automated test; it was not added to the selected live matrix. Screenshot clicks used Retina screenshot coordinates; native accessibility exposes only the window/menu shell.

## Cleanup

Both fixture missions were stopped and archived; `evidence/mission-final-archived.json` and `evidence/first-fixture-final.json` show zero live sessions and non-empty archive timestamps. The mission-owned drawer shell was removed automatically with mission archive; the standalone run-created shell was stopped (shell records disappear on stop, so a following archive correctly returned session not found). `evidence/sessions-final.json` is empty. Agent conversation history and private fixture data remain preserved in the scratch homes; the role and crew were retained.

Restored host Allow remote access to its original On value after the reconnect check (`evidence/host-remote-before-reconnect.json`, `evidence/host-paused.json`, `evidence/host-remote-restored.json`, and corresponding native screenshots). Revoked the disposable viewer pairing on host, forgot host on viewer, and logged out both private instances. Chrome returned to the exact empty-host baseline (`evidence/hub-baseline.png`, `evidence/hub-final.png`). Closed the run-owned Hub tab. The supported recovery command opened a blank Chrome profile window; it was left untouched after fixture teardown.

Quit both wrappers natively, stopped both private daemons, unregistered both wrappers from LaunchServices, and verified no private process survived (`evidence/final-processes.txt`). Released `script/qa-lock`. The existing installed Runner and normal development daemon were untouched. Backed up the shared Codex config before agent launch; it remained byte-identical at cleanup (`evidence/codex-config-cleanup.diff` is empty), so no trust removal or global restoration was needed. No implementation code, frozen test expectation, commit or push was changed by QA.

PR creation follows this handoff; coder must copy the four Passed results into its PR description. All selected macOS live checks are complete, so `needs-live-test` is unnecessary once that result is recorded on the matching reviewed candidate.

