# #1285 custom launch flags — macOS live QA

## Attempt 1 — 2026-11-14

**Verdict: Blocked before the first live check.** Native computer use could not find the running candidate's window. No selected live check passed or failed, and this record is not a feature pass. CLF-01 through CLF-07 need another macOS attempt; CLF-08 remains pending Windows. Keep `needs-live-test` on the eventual PR.

Candidate: `7bb216efa569c4b2feceeea4e12bb502f62418a0`, branch `feat/1285-custom-launch-flags`, worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1285-custom-launch-flags`. The worktree was clean before QA. Environment: macOS 26.6.2, arm64; Runner `0.19.8 (dev)`. Evidence root: `/private/tmp/runner-clf1285-vpupsjt_`.

Selected scope: CLF-01 through CLF-07 from `docs/features/1285-custom-launch-flags.md`, plus the clean review's real Agents-breadcrumb save/discard checks and agent-name interpolation in English and Simplified Chinese. These checks cover the changed Settings editing/persistence, command preview/Copy, and launched argv; no unrelated regression matrix was added. Automated validation belongs to the coder and reviewer and was not rerun by QA.

### Setup and evidence

- Took `script/qa-lock acquire --timeout 540` after waiting for mission `01M7BZV9MRAZNSA6BQKWP50QQ6`, handle `qa`, to release it. This run's holder PID was `2658`, owner PID `51805`.
- Confirmed no development app was running before launch. Ran `make run` with inherited `CLAUDE*`, `CODEX*`, `RUNNER_*`, `ANTHROPIC_*`, and `NO_COLOR` variables omitted. Build completed successfully; log: `make-run.log`.
- Stopped only the bare development process started by this test and copied the candidate app/CLI into a temporary `.app` wrapper. Source and wrapper binaries matched byte for byte. App SHA-256: `63dd1c83d2f6040da0146b665606d4758fd45f08c328c239d054dc0b7f11030f`. Both hashes: `evidence/binary-hashes.txt`.
- The absolute development CLI's `status --json` confirmed `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock` and `mode: outside a mission`; saved in `evidence/dev-status.json`. No SQLite database was opened.
- Initial wrapper: `Runner Screenshot Dev.app`, bundle ID `com.wycstudios.runner.screenshot-dev`. `@oai/sky` `get_app_state` returned `Computer Use server error -10005: cgWindowNotFound` by bundle ID and exact path. The app log reported `restored count=1`.
- Bounded recovery: retried the app-name binding, then stopped only this wrapper and used a fresh path `Runner CLF1285 QA.app` and bundle ID `com.wycstudios.runner.clf1285-qa`. `list_apps` confirmed that wrapper was running, but `get_app_state` by its exact path and the Settings shortcut still returned `cgWindowNotFound`. Error transcript: `evidence/native-control-errors.txt`; startup log: `app.log`.
- No screenshot was returned by native control. Screenshot evidence for the selected checks is therefore unavailable; CLI status and a restored-window log are setup evidence only.

### Selected matrix

| ID | Result | Evidence and unresolved work |
| --- | --- | --- |
| CLF-01 | Blocked | Native window unavailable before opening Settings; saving, navigation/restart, preview/Copy, and EN/zh-CN agent-name interpolation remain untested. |
| CLF-02 | Blocked | Escape, blur save, and valid focused draft saved by a real Agents-breadcrumb click remain untested. |
| CLF-03 | Blocked | Quote/reserved-flag/config-key refusal, unchanged saved preview, reopening, and invalid draft discarded through navigation remain untested. |
| CLF-04 | Blocked | Unknown-flag warning and save remain untested. |
| CLF-05 | Blocked | Native Codex chat/resume/fork and their argv, spaced argument, and helper exclusion remain untested. |
| CLF-06 | Blocked | One-slot Codex mission argv and both `--add-dir` arguments remain untested. |
| CLF-07 | Blocked | Native Claude Code startup and spaced custom argument remain untested. |
| CLF-08 | Blocked | Windows platform unavailable in this macOS run; explicitly pending in the mission brief. |

### Cleanup and handoff

No custom flags, language settings, runtime overrides, roles, crews, chats, or missions were changed or created. No agent-account launch occurred, so no test trust entries required removal. Private config backups were captured before launch and left under the private evidence root; they were never used to overwrite global configuration.

Stopped the owned fresh wrapper (PID `14508`), stopped the development daemon via the absolute development CLI, unregistered the wrapper from LaunchServices, and released the QA lock after verifying those processes had exited. The installed Runner and all pre-existing development sessions were left untouched. The wrapper and logs are retained with the evidence. No PR existed at this handoff; the coder must include this blocked matrix in the eventual PR description and retain `needs-live-test`. A new macOS attempt needs working native window access; there is no implementation finding from this attempt.

## Attempt 2 — 2026-11-14, after screen unlock

**Verdict: Passed CLF-01 through CLF-07 on macOS. NO REMAINING MUST-FIX QA ISSUES in the selected macOS checks.** CLF-08 remains Blocked/pending Windows, so this is a macOS feature pass and the PR retains `needs-live-test`. No product failure was observed. Attempt 1 remains recorded as Blocked; the operator explained at 13:36 +08 that the Mac's screen had been locked, then confirmed it was unlocked and approved this rerun.

Candidate: PR [#1317](https://github.com/yicheng47/runner/pull/1317), exact head `42c6ff6f65de18bd57e624198641cbf3c3e12b50`. The worktree was clean before this attempt. Environment: macOS 26.6.2 arm64, Runner `0.19.9 (dev)`, existing 140% app zoom, original language English. Codex `0.169.0` used `gpt-6.1-sol`, low effort; Claude Code `2.1.325` used Haiku 5.5, low effort. Prompts were bounded to one literal response with no tools or file changes, in fresh canonical folders under `/private/tmp/runner-clf1285-r2-nrhn5fhf`.

### Setup and evidence

Took `script/qa-lock acquire --timeout 540` before building; no competing development app was running. Ran `make run` with inherited agent/mission variables and `NO_COLOR` omitted, then stopped only that bare app process and launched a byte-identical copy through `/private/tmp/runner-clf1285-r2-nrhn5fhf/Runner CLF1285 R2.app`, bundle ID `com.wycstudios.runner.clf1285-r2`. The development CLI confirmed `0.19.9 (dev)`, `mode: outside a mission`, and the `com.wycstudios.runner-dev/runnerd.sock` endpoint. Native control worked after unlock. The coder was told when the build finished; no source changes or rebuilds occurred during live checks.

Evidence root: `/private/tmp/runner-clf1285-r2-nrhn5fhf`; screenshots and argv files below are relative to its `evidence/` directory. Original screenshots are retained; smaller copies were used only for on-screen inspection. `make-run.log`, `app.log`, `app-restart.log`, and `environment.json` record build/startup and candidate identity. App SHA-256: `71ef9c704ecd4f32503894fbd1b47df068d83c1351fcc53b22045403d479a6c6`; CLI SHA-256: `f0a7c040dcbb9b221962adc989f7247eabe5e674143fbaebe59ca774c0ca7867`. Source and wrapper copies matched byte for byte (`binary-hashes.txt`).

The Custom flags fields were both empty before testing, recorded in `original-settings.json` and the baseline screenshots. The saved Codex text was exactly `--search -c model_verbosity=low --add-dir "/tmp/runner qa/extra dir"`. Launch proof includes `ps -o args= -p <pid>` plus macOS `KERN_PROCARGS2` argv arrays, which establish that the spaced path is one argument. No process environment was captured. The fork helper was captured by a bounded process observer filtered to this test's source conversation.

### Selected matrix

| ID | Result | Observations and evidence |
| --- | --- | --- |
| CLF-01 | Passed | Enter saved the verbatim text with double quotes. Leaving/reopening the agent page and quitting/relaunching the app retained it. Chat and Mission each showed one Custom flags line immediately after the executable; both Copy results contained it, shell-requoted. English and Simplified Chinese descriptions named Codex with no literal `{agent}`; language was restored to English. Screenshots: `clf01-chat-copy.png`, `clf01-mission-preview.png`, `clf01-reopened.png`, `clf01-chinese-description.png`, `clf01-restored-english.png`. Copy text: `clf01-chat-copy.txt`, `clf01-mission-copy.txt`. |
| CLF-02 | Passed | Escape restored saved text. Editing to `--search -c model_verbosity=high` and clicking the description saved it. Replacing that with the CLF-01 text, keeping focus, scrolling to the top, and clicking the actual Agents breadcrumb without Enter saved the focused draft; reopening showed the CLF-01 text and preview. Screenshots: `clf02-escape.png`, `clf02-blur-save.png`, `clf02-breadcrumb-draft.png`, `clf02-breadcrumb-before-click.png`, `clf02-breadcrumb-navigated.png`, `clf02-breadcrumb-reopened.png`. |
| CLF-03 | Passed | Unclosed quote, `--model gpt-x`, and `-c features.remote_plugin=true` each showed one red refusal and left the saved preview unchanged. Reopening after each restored the CLF-01 text. A fresh invalid `--model gpt-x` draft submitted by the actual Agents breadcrumb without Enter was also discarded on reopen. Screenshots: `clf03-unclosed-quote.png`, `clf03-reserved-model.png`, `clf03-reserved-config-key.png`, their `*-reopened.png` companions, and `clf03-invalid-navigation-reopened.png`. |
| CLF-04 | Passed | `--serch` saved with an amber message naming `codex --help` and `--serch`, normal field border, and an updated Custom flags preview. Reopening retained the value and recomputed the warning. The valid CLF-01 flags were restored and confirmed before launching sessions. Screenshots: `clf04-unknown-warning.png`, `clf04-warning-reopened.png`, `clf04-valid-restored-confirmed.png`. |
| CLF-05 | Passed | Native Codex chat responded `CLF1285_CHAT_OK`; stop/resume retained its conversation/history. The fork opened a distinct conversation with inherited history. New-chat argv had custom flags at indices 1–5; resume and fork argv had them after `resume <conversation-id>`, before Runner flags. `/tmp/runner qa/extra dir` remained one argument. The observed headless helper argv was `codex exec fork <source-id> --json --skip-git-repo-check <fork-note>`, without custom flags. Screenshots: `clf05-chat-ack-final.png`, `clf05-chat-stopped.png`, `clf05-chat-resumed.png`, `clf05-fork-ready.png`. Process proof: `clf05-chat-argv.json`, `clf05-resume-argv.json`, `clf05-fork-live-argv.json`, `clf05-fork-helper-argv.json`. |
| CLF-06 | Passed | One-slot Codex mission responded `CLF1285_MISSION_OK`. Custom flags preceded the role/Bypass permission arguments. Both `--add-dir` arguments were present: the spaced custom path and this mission's log folder. Screenshot: `clf06-mission-codex-ack.png`; proof: `clf06-mission-argv.json`, `clf06-mission-status.json`. |
| CLF-07 | Passed | Claude Code saved `--add-dir "/tmp/runner qa/extra dir"`, preview/Copy showed it immediately after the executable, argv retained the path as one argument before permission/Runner flags, and native Haiku returned `CLF1285_CLAUDE_OK`. No trust dialog had to be accepted. Screenshots: `clf07-claude-saved.png`, `clf07-claude-started.png`, `clf07-claude-ack.png`; proof: `clf07-claude-copy.txt`, `clf07-claude-argv.json`, `clf07-claude-session.json`. |
| CLF-08 | Blocked | Windows npm `.cmd` launch remains pending; this Mac cannot validate that path. Keep `needs-live-test`. |

### Object ledger and cleanup

| Object | ID / path | Final state |
| --- | --- | --- |
| Codex chat | `01M7C97FD6Q4K3KNRX9T7B9YR5`, `codex-chat/` | Stopped and archived. Original conversation `01a1d893-c924-7483-9550-484f5a246f8c`; new PID `76606`, resumed PID `78039`. |
| Codex fork | `01M7C9ATDJS3H4CY1NA5TZ39WG`, same scratch folder | Stopped and archived. Fork conversation `01a1d895-6a8a-75c3-851a-7ccbac021fbe`, PID `78708`; helper PID `78681`. |
| One-slot mission | `01M7C9BNRS9G5W62QM9TG8GCTX`, `codex-mission/` | Stopped and archived; zero live sessions. Member `01M7C9BNRWPHZDC4ZPRGRVMTR4`, PID `79395`. |
| Mission fixtures | Role `clf1285-r2` (`01M7C9BD8JAKZS4AHYG709QW5J`); crew `CLF1285 R2 QA` (`01M7C9BDB8JQY93CSTVW0E2PY3`) | Retained as non-running fixture records with the archived mission; no data deleted. |
| Claude Code chat | `01M7C9DP59GNV5E8J0T5NF1H25`, `claude-chat/` | Stopped and archived; PID `80093`. |

Cleared both tested Custom flags fields through native Settings and reopened each to verify the empty saved field and absent Custom flags preview line (`cleanup-codex-reopened-empty.png`, `cleanup-claude-reopened-empty.png`). English was restored (`cleanup-english-restored.png`); original clipboard contents were restored. Other launch switches, executable overrides, account selection, and pre-existing sessions were not operated.

Removed only this run's two Codex trust entries and its Claude `projects` entry from `~/.claude.json`. Parsed before/after assertions confirmed that cleanup preserved every other live config value; Claude's remaining projects map matched its prelaunch backup. Evidence: `codex-trust-cleanup.diff`, `claude-trust-cleanup.json`. Private backups stayed outside the repo. `cleanup-objects.json` confirms all three chats archived and the mission archived with zero live sessions. Quit the owned wrapper, stopped the development daemon, verified both exited, unregistered the wrapper, and released the QA lock. Logs, wrapper, fixture records, and conversation history were preserved.

PR #1317's description was updated with this matrix and the pending Windows limitation. The QA record is handed to the coder to fold into the branch; QA made no implementation changes, commits, or pushes.
