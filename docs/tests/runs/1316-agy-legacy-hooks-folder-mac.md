# #1316 — Legacy Antigravity hooks folder — macOS QA

## 2026-11-14, 10:26–10:36 Asia/Shanghai

**Passed — NO REMAINING MUST-FIX QA ISSUES in the required macOS matrix.** Both checks assigned by the operator passed on reviewed candidate `5cdae7182c5ede9f9240b5c35e578ed3d95aaf3e`. This is a scoped recovery/new-chat run, not a full regression pass. Windows and Linux native checks were not run.

### Candidate and environment

PR #1316, branch `fix/agy-legacy-hooks-folder`, worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-agy-legacy-hooks-folder`; clean before and throughout the build and checks. Reviewer and coder both handed off the exact SHA above. The pre-#1296 fixture was built from clean detached `c43f8fe61501c26b2a7af5dd97385e43b298f332` in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/qa-1316-pre-1296`. QA changed no implementation or expectations and made no commit or push. The only subsequent worktree addition is this record.

macOS 26.6.2 (25G83), arm64; Runner app/CLI/daemon 0.19.8 development; Antigravity CLI 1.5.8, Gemini 3.8 Flash, low effort. Native interaction and screenshots used `@oai/sky` through `node_repl`. Only the development namespace, `/Users/jason/Library/Application Support/com.wycstudios.runner-dev`, was operated. The absolute development CLI independently confirmed its `runnerd.sock` and outside-mission identity. No SQLite access or private PTY input was used.

Evidence root `E = /private/tmp/runner-1316-qa.2kwic091/evidence`. Screenshot and raw-log artifacts remain local because native headers include account information. The scratch root also retains prompt files, `commands.ndjson`, configuration backups and `ledger.json`. All evidence filenames below are relative to E.

### Build and fixture

QA acquired `script/qa-lock acquire --timeout 540` before building or launching. In both named worktrees, the launch command was `env -u NO_COLOR -u RUNNER_CREW_ID -u RUNNER_MISSION_ID -u RUNNER_HANDLE -u RUNNER_EVENT_LOG -u RUNNER_SESSION_CLI make run`. The candidate also had an earlier successful `cargo build -p runner-cli -p runner-app`. Each worktree used its own target directory. Build output is retained in `pre-make-run.log`, `candidate-prebuild.log` and `candidate-make-run.log`.

After each bare development launch, QA stopped only its own bare app and copied byte-identical app and CLI binaries into distinct temporary `.app` wrappers under the scratch root: `Runner 1316 Pre.app` (`com.wycstudios.runner.qa1316.pre`) and `Runner 1316 Candidate.app` (`com.wycstudios.runner.qa1316.candidate`). `pre-hashes.json` and `candidate-hashes.json` record source/copy comparisons. Candidate app SHA-256 is `5e9f351f2a135ddfc85cbb67e2a1baa84ae9542332e2ed010e698d726d6a62ae`; CLI SHA-256 is `61cae071b8ca0eeecc5fb1ac5b6b33a09fdac8afbbbc13f868d379ef67428929`. The installed development CLI and daemon both matched that CLI hash (`candidate-installed-sidecars.json`).

The old QA chat `01M7BY751KDYB05ZE7VXPR5APF` was created with `chat start --runtime antigravity --model gemini-3.8-flash --effort low --cwd /private/tmp/qa-1316-old --json`. Its first shell command succeeded and taught `QA1316-OLD`. Because that command explicitly selected the project cwd, one additional tiny prompt requested the command tool's default directory: native `pwd` returned the development `antigravity-hooks` folder. This establishes the affected default-directory state, rather than relying on an ACK. Process PID 6078 carried `--add-dir <dev data>/antigravity-hooks`, and `.agents/hooks.json` existed. Evidence: `pre-pwd-completed.png`, `pre-default-completed.png`, `pre-argv.txt`, `pre-hook-exists.txt`, `pre-old-done.json`.

QA stopped the development daemon, quit the old wrapper, preserved the folder contents outside Git, then performed the operator-authorized `rm -r` on that exact development hooks folder. `hooks-deleted.txt` proves it was absent before candidate startup. Two pre-existing marker files were backed up and restored after testing. The explicit daemon stop left the QA chat stopped; its native Resume button was clicked once on the candidate. No automatic-resume pass is claimed.

Native-control setup briefly relaunched the old wrapper after a post-Quit state read; the binding was then retired, the wrapper quit and daemon stopped again, and process absence verified before candidate startup. Candidate binding also opened a duplicate copy of the QA wrapper, producing a transient `window belongs to another connection` toast (`recovery-app-start.png`); only that duplicate was stopped before the selected checks. These setup incidents are retained in `setup-deviation.txt`; neither is attributed to the folder fix or counted as a live-check result.

### Selected checks and results

| Check | Reason | Result | Observation and evidence |
| --- | --- | --- | --- |
| Recovery of a pre-#1296 conversation after the hooks folder was already deleted | Exercises the bug and the fix's absent-folder recreation path | Passed | Candidate startup recreated the folder empty (`recreated-folder.json`). Native Resume retained conversation key `3bb2dbc4-3a52-404e-801a-c78ed940f47d`; argv used that `--conversation` and no hooks grant (`recovery-argv.txt`, `recovery-resumed-state.json`). The next turn recalled `QA1316-OLD` and ran `pwd && ls` using the default directory. Expanded native output shows exit code 0, the recreated hooks-folder path and no listing output, with no missing-directory error. The turn ACKed and completed. `recovery-completed.png`, `recovery-tool-expanded.png`, `recovery-final.json`. |
| Fresh Antigravity direct-chat cwd, the task-selected part of RT-START-04 | Ensures keeping the legacy folder does not grant it to new chats | Passed | New chat `01M7BYF3F3511YSNZBF5P6BQQ5` in canonical `/private/tmp/qa-1316-new`, same model/effort. PID 31468 carried neither `--add-dir` nor `antigravity-hooks` (`new-argv.txt`). Native `pwd` returned `/private/tmp/qa-1316-new` and exited 0; the bounded turn ACKed and completed. `new-completed.png`, `new-tool-expanded.png`, `new-final.json`. This does not claim the suite's separate automatic-append or explicit user-workspace variants. |
| Windows and Linux native checks | Platform gap disclosed by the macOS-only task | Skipped | Not run; no other-platform native pass claimed. |

The operator selected only the two macOS checks above. Hook-file removal, user-file preservation and symlink boundaries remain covered by the coder/reviewer's automated evidence: all 64 Antigravity daemon tests passed, including startup recovery, plus the reported daemon Clippy and formatting checks. QA did not rerun that suite.

### Cleanup and disposition

Both QA chats were stopped and archived, with archive timestamps and original conversation keys retained (`old-cleanup-verified.json`, `new-cleanup-verified.json`). The candidate wrapper quit, the development daemon stopped as the task required, both wrapper registrations were retired, and process inventory confirmed no normal QA app/development-daemon process remained. The two pre-existing legacy-folder files, `pr780-smoke-agy.txt` and `sm772-agy.txt`, were restored byte for byte (`cleanup-processes-and-files.json`). Evidence, test conversations and scratch directories remain.

Antigravity settings were backed up before launching. Cleanup removed only this run's two trusted-workspace entries; the resulting file is byte-identical to its original backup (`config-cleanup-summary.json`, `config-cleanup-diff.txt`). No development setting, authentication or permission mode was changed. The pre-existing pi chat was never individually operated, input into or archived; daemon replacement stopped it, and it remains resumable and unarchived (`pre-session-list.json`, `cleanup-sessions.json`).

`script/qa-lock release` succeeded and status reported free. The PR description records both passing checks and the native-platform limit; `needs-live-test` can be removed because every required check in this handoff passed. Folding this record into the commit, pushing, final-head review and CI remain with coder/reviewer.
