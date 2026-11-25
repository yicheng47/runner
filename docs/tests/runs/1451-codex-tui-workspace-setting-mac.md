# #1451 Codex TUI workspace — macOS live QA

## 2026-11-24 — Passed

CTW-01, CTW-02 and CTW-03 passed on macOS against reviewed candidate `fff005aba461c4d2b74380ce6a91456ed819b429`. The selected scope was both Settings command previews, one live Codex chat and the Custom flags refusal.

### Candidate and method

Repository `yicheng47/runner`, branch `feat/1451-codex-tui-workspace-setting`, base `4a95822886d610d290e5e45d6b047e8e76a49417`. The worktree was clean at build time, with empty tracked diff SHA-256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`. Coder handoff `01M85RXXMYH2BEBT9AYX7HQ1AW` authorized the [feature checks](https://github.com/yicheng47/runner/blob/v0.20.4/docs/features/1451-codex-tui-workspace-setting.md#live-checks-macos-development-app) in the development app under the shared QA lock.

Environment was macOS 26.6.2 (25G83) arm64, Rust and Cargo 1.97.1, Runner 0.20.3 (dev), development CLI 0.20.3 and Codex 0.171.0. The fresh Codex chat used the existing sign-in, GPT-6-Luna, low effort, standard speed and the existing chat permission mode. Native macOS computer use controlled Settings and submitted the bounded Markdown-link prompt. Opening and dismissing New tab verified native control first.

`make run` built the candidate in the worktree's own `target/`. The app and CLI copies in `Runner QA1451 Dev.app` matched the built binaries byte for byte. The running app was the wrapper's `Contents/MacOS/Runner`, SHA-256 `7ec9d2695712e96ff182f79655e8fb705ab3c804153f5cd998c2d4a223b6ecb6`. The CLI SHA-256 was `cff571f8823a3a37b9101c35dc11c8f98c4a07b586a94c2f16fa4ebdcf17e095`. The absolute development CLI reported `com.wycstudios.runner-dev/runnerd.sock` and `outside a mission`. The launch omitted the coordinating mission variables, `RUNNER_SESSION_CLI` and `NO_COLOR`.

Original screenshots and structured evidence are in `/private/tmp/runner-1451-qa.UAUZ0TI8/evidence/` on the QA Mac. `plan.txt`, `preflight.txt`, `make-run.log`, `binary-hashes.txt`, `running-identity.txt`, `dev-status.json` and `00-native-new-tab.jpg` establish setup and identity. Screenshot and JSON names below are relative to that directory.

### Results

| ID | Result | Observations and evidence |
| --- | --- | --- |
| CTW-01 | Passed | TUI workspace started Off with the Chats and missions badge and flag hint. Chat and Mission each showed `-c features.tui_workspace=false` with the green “TUI workspace off” note. Turning On removed the command line and note in both previews. Closing and reopening Settings preserved On. Restoring Off restored both previews. Screenshots `ctw01-default-off-{chat,mission}.jpg`, `ctw01-on-{chat,mission}.jpg`, `ctw01-reopened-on-chat.jpg`, `ctw01-restored-off-{chat,mission}.jpg`. |
| CTW-02 | Passed | The fresh chat in `codex-chat/` ran Codex 0.171.0. macOS `KERN_PROCARGS2` showed exactly one adjacent `-c`, `features.tui_workspace=false` pair in PID 9123's argv. The native terminal response displayed `CTW-02 QA file (qa-link-1451.md)` with the relative file path visible and the Codex sidebar closed. Screenshots `ctw02-codex-ready.jpg`, `ctw02-native-draft.jpg`, `ctw02-link-visible.jpg`, `ctw02-link-click.jpg`. Process and session evidence `ctw02-process-cwd.txt`, `ctw02-live-argv.json`, `ctw02-session-after-link.json`. Config evidence is below. |
| CTW-03 | Passed | Submitting `-c features.tui_workspace=true` produced the red refusal “Runner already sets -c features.tui_workspace.” The Chat command remained identical to the original Off preview. Reopening showed the original empty field with its placeholder. Screenshots `ctw03-reserved-flag.jpg`, `ctw03-reopened-empty.jpg`. The subsequent live argv contained the switch-generated `false` override only. |

Command-click on the displayed bare relative filename left both viewers closed. Runner Files panel link routing is tracked in [#1452](https://github.com/yicheng47/runner/issues/1452). The required rendering check covered visibility of the path and absence of Codex's sidebar.

`~/.codex/config.toml` had the same SHA-256 before and after the Settings checks. Launching the scratch chat added the standing-authorized project trust entry for its canonical folder. A parsed comparison verified every other value matched, and removing that one entry restored the original file byte for byte. See `config-before-chat-hashes.txt`, `ctw02-config-launch.diff` and `cleanup-config.txt`.

### Object ledger and cleanup

The run created one chat, `01M85S6DGNXAMH0AHZHVZJBVQC`, in `/private/tmp/runner-1451-qa.UAUZ0TI8/codex-chat`. The development CLI verified it stopped and archived, and native search returned no matches for its unique path. See `cleanup-chat-final.json` and `cleanup-chat-absent.jpg`. The fixture was moved to `evidence/ctw02-fixture.md`, and the empty chat directory was removed. The archived session and its conversation history are retained.

The development launch automatically replaced the previous disconnected development daemon and resumed saved development sessions. Cleanup quit the owned wrapper, stopped the candidate development daemon and unregistered the wrapper. Installed Runner and the separate pre-existing private-home daemon remained running. Process checks confirmed the owned bare app, wrapper, development daemon, Codex process and lock holder exited. The shared QA lock was released and verified free. See `cleanup-daemon-stop.json`, `cleanup-runner-processes-final.txt` and `cleanup-lock-status.txt`. The wrapper, screenshots and private raw evidence remain in the scratch root for review.
