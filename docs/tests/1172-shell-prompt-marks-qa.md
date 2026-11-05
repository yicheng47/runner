# #1172 — Shell prompt status live QA

Verdict: **Passed** for all seven selected macOS live checks on `9d2514ff050523e5178bbffb3b8334ca26207a67`, tested 2026-11-05. This is feature-scoped validation, not a full regression pass. Windows/Linux native UI and the full RT-STATUS-14 permission/question matrix were outside this live scope; CI remains separate evidence owned by the coder.

## Candidate and environment

- Repository/worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1172-shell-prompt-marks`.
- Branch: `fix/1172-shell-prompt-marks`; base: `2ae1474b7bbf2fd053846c04c03eab2a42f3794c`.
- Reviewed candidate: `9d2514ff050523e5178bbffb3b8334ca26207a67`. The tree was clean before the build and remained at this SHA with no source changes through cleanup. SHA-256 of the empty uncommitted diff: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
- macOS 26.6.2 (25G83), arm64; rustc 1.97.1; Cargo 1.97.1; zsh 5.9; `/bin/sh` backed by bash 3.2.57(1)-release.
- Real hand-started agent: Claude Code 2.1.318, Haiku 5.5, low effort, existing auto permission mode. One text-only turn; no agent tools requested. Pi was not launched.
- Built with `make run` from the reviewed worktree. Runner and CLI reported 0.19.3 (development build).
- Frozen native-control wrapper: `/private/tmp/runner-qa-1172-sxllzwln/Runner QA 1172 Dev.app`, bundle ID `com.wycstudios.runner.qa1172-dev`. App and CLI were copied from this worktree and hash-matched. The installed development CLI and daemon also matched the copied CLI.
- App SHA-256: `d46e44f5dd1790166a3016839439f75edbd4fc89b412a72fd8b5d86c1dc6272b`.
- CLI/daemon SHA-256: `31d2c3ed12e5eb393ba6ceb62df9ace504c04245313116e9603613142d0afcfd`.
- Endpoint verified through the absolute development CLI: `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`, outside a mission. Process inventory proved the running wrapper executable path.

## Authorization, plan and method

The mission authorized these live checks under AGENTS.md Test Scope. QA waited for the coder's clean reviewed handoff and for the operator to release the development app from other QA runs. No other run's app or sessions were stopped or operated.

The pre-execution plan in the scratch root selected only the five changed zsh lifecycle behaviors, the unchanged silence fallback, and the directly affected program-status handoff. Each check's reason is in the matrix below. Setup, candidate identity, isolation, evidence and cleanup followed `docs/tests/full-smoke-test.md`; this did not expand scope to the full suite.

All terminal input used native computer use. Native input was first verified with a harmless Finder menu action, then in the candidate by opening an owned terminal and visibly changing its directory. No private PTY writes or database access were used. Terminals were opened with Cmd-T, then moved into fresh canonical scratch directories before test commands. The existing cyan/red zsh prompt confirmed that colors were present.

`NO_COLOR`, `RUNNER_CREW_ID`, `RUNNER_MISSION_ID`, `RUNNER_HANDLE`, `RUNNER_EVENT_LOG` and `RUNNER_SESSION_CLI` were omitted from app launches. Development CLI calls used the absolute development CLI path with mission variables removed and `RUNNER_SESSION_CLI` empty. Bounded read-only `session show --json` sampling captured activity and program-status presence at approximately 100 ms intervals alongside native screenshots. These observations are independent of the input path.

The fallback run relaunched the same copied app and daemon with child-only `SHELL=/bin/sh`, creating an actual top-level unintegrated shell, rather than a bash child inside a mark-aware zsh. No persistent shell setting changed. The native window zoom action changed the window dimensions and was reversed for the resize check.

## Results

Evidence filenames below are relative to `/private/tmp/runner-qa-1172-sxllzwln/evidence/`.

| Check | Reason | Result | Observation and evidence |
| --- | --- | --- | --- |
| 1. zsh `ls` returns immediately to Idle | Detects the former two-second completion trail | Passed | Prompt and Idle visible 523 ms after Enter, including capture overhead. `01-ls-immediate-idle.png`; `zsh-five-checks.jsonl`. |
| 2. Typing without Enter stays Idle | Prompt echo must not create Working | Passed | `echo QA1172-TYPING` remained unsubmitted with Idle visible 531 ms after typing; no sampled Busy transition during typing. Cleared with Ctrl-U. `02-typing-idle.png`; `zsh-five-checks.jsonl`. |
| 3. zsh `sleep 10` stays Working, then Idle | Silent execution must follow command lifecycle | Passed | Working at start and mid-run; sampled Busy lasted 9.994 seconds with no intermediate Idle. Prompt returned to Idle. `03-sleep-working-start.png`, `03-sleep-working-mid.png`, `03-sleep-return-idle.png`; `03-sleep-working-state.json`; `zsh-five-checks.jsonl`. |
| 4. Ctrl-C during zsh `sleep 30` | Interrupted command must release Working at the returned prompt | Passed | Working before interruption; returned prompt and Idle visible within 456 ms of Ctrl-C, including capture overhead. `04-before-ctrl-c-working.png`, `04-ctrl-c-immediate-idle.png`; `zsh-five-checks.jsonl`. |
| 5. Resize the idle window | Resize redraw/grace must not change mark-driven state | Passed | Native zoom visibly changed dimensions and was reversed; Idle throughout both actions. `05-idle-window-resized.png`, `05-idle-window-restored.png`; `zsh-five-checks.jsonl`. |
| 6. Top-level unintegrated shell fallback | Shells without integration must keep the old output/silence detector | Passed | `/bin/sh` and bash 3.2 version visible. Output caused Working; sample returned Idle after 2.094 seconds while the subsequent `sleep 10` still ran. A separate silent `sleep 30` returned Idle after 2.005 seconds, visibly before any prompt returned; it was then interrupted. `06-fallback-initial-idle.png`, `06-fallback-output-working.png`, `06-fallback-sleep-start-working.png`, `06-fallback-sleep-running-idle.png`; `fallback-shell.jsonl`, `fallback-silent-sleep.jsonl`; `fallback-owned-session.json` in the scratch root proves `agent_command=/bin/sh`. |
| 7. Scoped RT-STATUS-14 agent-to-shell handoff | Real agent reports must win while open, then release shell activity on exit | Passed | Hand-started Claude showed Working during the tiny turn, emitted `QA1172-AGENT-OK`, and showed Idle while still open. Structured state retained `program_status` for both states. After `/exit`, the shell prompt was Idle and `program_status` absent. A following shell `sleep 3` showed Working for 2.966 sampled seconds, then Idle, with no program status. `07-agent-turn-working.png`, `07-agent-turn-result.png`, `07-shell-prompt-idle-after-agent.png`, `07-shell-sleep-working-after-agent.png`, `07-shell-sleep-return-idle.png`; `07-agent-working-state.json`, `07-agent-open-idle-state.json`, `07-shell-after-agent-state.json`, `agent-handoff.jsonl`. |

No selected product check failed or remained Blocked. The first fallback screenshot after the ten-second command was captured after its prompt returned, so the longer silent command supplied the required visual evidence while running; both captures and the first trace are retained. The initial agent-exit screenshot captured process teardown before the shell prompt, still Working; the next capture and structured state show the returned prompt Idle. These are intermediate observations, not discarded failures.

## Cleanup and retained evidence

Owned terminal sessions were `01M6MSAZQJVTNA81B8J8XX7E5T` (zsh, also used for the agent) and `01M6MSHY67FZQQAH71STHWJ9JX` (top-level sh). Both were stopped through the development CLI and their ephemeral shell rows/tabs were removed. An archive attempt after stopping the first shell reported session not found because stopping had already removed it. No roles, crews or test missions were created.

Claude's trust entry for the run's own `agent` folder was accepted under the documented scratch-folder authorization and removed afterward. The remaining `projects` map equals the captured pre-run map; other current configuration fields were preserved. See `claude-trust-cleanup.json`. Existing authentication, permission mode and agent settings were not changed for the check; conversation history was retained.

The initial `make run` app, copied wrapper launches, both candidate development daemons and every bounded sampler exited. Final `ps` checks found none of the run's app/daemon PIDs (69597, 75687, 79791, 1308, 1775) surviving; only the pre-existing installed Runner and its daemon remained. The wrapper was unregistered from LaunchServices. Its files, scratch fixtures and evidence are retained; no branches, worktrees or files were deleted.

The canonical scratch root holds `candidate.json`, `build-identity.json`, `dev-status.json`, `fallback-status.json`, `ledger.json`, build/launch logs, the copied binaries and screenshots. `evidence/transitions-summary.json` indexes all sampled transitions. Large screenshots and private setup snapshots remain outside the repository; this record contains no credentials or account identifiers.
