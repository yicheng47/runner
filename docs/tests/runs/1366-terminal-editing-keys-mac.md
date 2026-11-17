# #1366 Phase 1 — macOS terminal editing keys

## Candidate and environment

Run: 2026-11-17, QA slot of the authorized hydrogen trio. Repository `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1366-terminal-editing-keys`, branch `feat/1366-terminal-editing-keys`, reviewed candidate `8609c1e3a0b60ac1a8e993fb413238abde785f8f`, base `352a19fc959663d9f9718067c82164f55cea61e0`. Initial working tree was clean; empty diff SHA-256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.

macOS 26.6.2 (25G83), arm64; Runner 0.20.0 development build; Rust/Cargo 1.97.1; zsh 5.9; Python 3.14.7; Claude Code 2.1.326, Haiku 5.5 with low effort and existing sign-in. Codex CLI 0.169.0 was available but was not part of the selected live checks.

Built the reviewed candidate using `env -u NO_COLOR -u RUNNER_CREW_ID -u RUNNER_MISSION_ID -u RUNNER_HANDLE -u RUNNER_EVENT_LOG -u RUNNER_SESSION_CLI make run`. The temporary wrapper `/private/tmp/1366-keys-8iuoqpku/Runner 1366 QA.app` has bundle ID `com.wycstudios.runner.qa1366` and byte-identical app/CLI copies from this worktree. App SHA-256: `3c7d9447189631fa046285cff9fa937b336a2fa071391fdb1249d1875325348e`; CLI SHA-256: `43464647ed2335f49f3729386d42d2f823269f394837237eafdfbf888db348a5`. The development CLI's public `status --json` confirmed the development namespace and outside-mission mode. The app startup log confirmed `com.wycstudios.runner-dev`; wrapper PID 37544 and development daemon PID 34369 were recorded. The installed app and the unrelated isolated development daemon were preserved.

## Scope and method

Followed the candidate identity, native control, isolation, evidence and cleanup sections of `docs/tests/full-smoke-test.md`, with the six feature checks below and the Pinyin subset of TERM-IME-01. The human's 2026-11-17 scope decision explicitly excludes mission, drawer, alternate-screen and remote variants; those are outside scope, not blockers. Windows/Linux are outside this macOS-only change's selected live scope. Antigravity was skipped as instructed.

Acquired `script/qa-lock` before building/launching. Used native computer use exclusively for app input, first verifying control by opening/dismissing New tab. Used the absolute development CLI with crew identity variables removed for the owned Claude chat; no private PTY writes or database access. Large evidence and private backups remain outside the repository under `/private/tmp/1366-keys-8iuoqpku`; screenshots are in its `evidence/` subdirectory. The pre-execution plan is `plan.md`, candidate metadata is `identity.json`, and owned objects are in `ledger.json`.

## Result matrix

Final verdict: **Incomplete — four checks Passed, two Blocked**. The human instructed QA to leave the held-key and Pinyin checks Blocked and clean up at 2026-11-17 03:29 UTC. Keep `needs-live-test` until those two required checks pass. No product regression has been established.

| Check | Result | Observation and evidence |
| --- | --- | --- |
| Zsh line editing | Passed | Started with unsent `echo hello world`; ⌘← then X yielded `Xecho hello world`; ⌘→ then Y yielded `Xecho hello worldY`; ⌘⌫ cleared it and the fresh command ran. A scratch receiver entered through the terminal recorded exact bytes `010515` from the three native chords. Screenshots `01-zsh-start.png`–`04-native-chord-bytes.png`; `received-keys.hex`. |
| Claude Code prompt editing | Passed | Same three edits in the owned Haiku prompt; cursor moved to start/end, then line cleared. No turn was submitted. Screenshots `13-claude-start.png`–`15-claude-clear.png`; public state in `claude-session.json`. |
| Held k and e | Blocked | Native automation `pressKey` with duration 1100 ms produced a single character for each; this does not establish physical repeat behavior. No accent picker was visible. Screenshots `05-held-k.png`, `06-held-e.png`. Physical input requested through Runner. |
| One Pinyin composition | Blocked | ABC and Pinyin were already enabled, but the automation Ctrl+Space attempt left literal `nihao` in the receiver with no observed preedit/candidates. Cancelled that receiver without claiming an IME pass or regression. Screenshot `07-ime-attempt.png`. Physical input requested through Runner. |
| ⌘C/V/K/T | Passed | Double-clicked owned `CLIP1366` output; ⌘C copied exact bytes `CLIP1366`, ⌘V inserted it once, ⌘K opened command palette, ⌘T created a new shell. Closed only that new shell with `exit`. Screenshots `08-copy-fixture.png`–`11-command-t-terminal.png`; `clipboard-copied.txt`. |
| User override wins | Passed | Backed up development settings first. Through Settings → Keyboard shortcuts, bound Toggle sidebar to ⌘←. In the zsh draft `override1366`, ⌘← collapsed sidebar and subsequent X appended (`override1366X`), proving no ^A cursor movement. Toggled sidebar back and reset only the changed binding. Screenshots `16-override-binding.png`–`18-override-restored.png`; `override-active.json` and `settings-restored.json` confirm override `{}` and sidebar collapsed `false` match baseline. |

## Findings and pending work

Physical-key repeat and one actual Pinyin composition require human assistance because the native automation attempts did not exercise those paths. These remain Blocked, not Passed or attributed regressions. The owned three-line receiver prepared for human assistance is shown in `20-physical-check-ready.png`; the human chose to leave both checks Blocked, and QA cancelled the receiver during cleanup.

The owned Claude chat initially displayed Attaching terminal; navigating away to the owned shell and back rendered the prompt, after which its selected checks passed. This observation is retained without attributing it to this change.

## Cleanup

Completed after the human's cleanup instruction. Cancelled the owned physical receiver and exited the owned zsh shell PID 41835 natively. The shortcut-created extra shell had already exited natively. Archived only the owned Claude chat `01M7KRDD9B518M5DZQQW24AJY9` through the absolute development CLI. Quit wrapper PID 37544 natively, stopped development daemon PID 34369 that this run started, and unregistered the temporary wrapper with `lsregister -u` while retaining its files and evidence. Process inventory confirmed none of the candidate app/daemon/shell PIDs or scratch-root processes survived. The installed Runner and the unrelated `/private/tmp/rfr-home` daemon were not stopped.

Restored and verified the keymap `{}`, sidebar visibility (`sidebarCollapsed: false`) and original clipboard text. Removed only this run's Claude trust-project entry. Comparing Claude's project map with the private baseline also found an unrelated `/private/tmp/beacon` entry changed during the automatic resumption of saved development chats; that unrelated entry was preserved. The projects-only cleanup evidence is `claude-project-cleanup.json`; credentials and account identifiers are excluded. Other evidence includes `settings-restored.json`, `processes-before.txt`, `processes-after.txt` and `ledger.json`.

Released the QA lock; `script/qa-lock status` confirmed it is free. Final HEAD remained `8609c1e3a0b60ac1a8e993fb413238abde785f8f`, with no product-source diff; this QA record was the only untracked file. No commit, push, PR, branch or worktree operation was performed by QA.
