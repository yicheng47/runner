# #1201 Starting pill — macOS live QA

**Current verdict: Passed on `7b49383632092947a12eb8d7fe527a2db5821f3a`.** All four selected macOS checks passed in the appended retry. The earlier Failed/Blocked attempt is preserved below; the retry's sampling limits and launch-context recovery are disclosed separately.

## 2026-11-15 — candidate 7cebc50

**Verdict: Failed / Blocked. Required live validation is unresolved; retain `needs-live-test` when the PR is opened.** Resume exposed an early pill dismissal without a visible Claude frame. Fresh Claude and shell checks could not reach their first frame or prompt because the launched processes stalled during startup. No successful TUI handoff was observed.

### Scope and candidate

Jason authorized the `agy trio` mission and bounded development-app checks. The selected matrix is fresh Claude from the launcher and a split pane, stopped-Claude resume, and a new terminal tab. These cover the changed Starting timing and the adjacent resume/shell paths; this is not a full regression run. The existing lifecycle suite's RT-LIFE-07 supplies the stop/resume setup, without adding its unrelated conversation-memory checks.

Tested commit: `7cebc50ff037870369ae39a9d4c6ec612ebfcde6`, tree `76f327e6c8d0325b98fff6943163f91ffbc4be24`, branch `fix/1201-starting-pill`, canonical worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1201-starting-pill`. The working tree was clean before build and after live execution; this record is the only QA edit. The reviewer passed the identical tree before handoff. Automated checks belong to the coder/reviewer evidence and were not rerun by QA.

Environment: macOS 26.6.2 (25G83), arm64; Runner app `0.19.9 (dev)` and CLI `0.19.9`; Claude Code `2.1.325`; `/bin/zsh`. Existing app zoom was 140%. The runtime-only launcher chat used the existing native Claude account and inherited bypass permission mode, with no prompt sent. The split fixture used a newly created role with model `haiku`, effort `low`, and its default auto permission mode, with no prompt sent. Existing runtime settings/extensions were preserved.

Evidence root: `/private/tmp/runner-1201-82go05t4/evidence/`. Native controls and captures used `@oai/sky` through `node_repl`, bound to `com.wycstudios.runner.screenshot-dev`. The wrapper was `/private/tmp/runner-1201-82go05t4/Runner Screenshot Dev.app`. The absolute development CLI independently confirmed `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`; installed Runner was untouched. `dev-status.json` contains that confirmation.

Built with `env -u NO_COLOR -u RUNNER_CREW_ID -u RUNNER_MISSION_ID -u RUNNER_HANDLE -u RUNNER_EVENT_LOG make run`, then stopped that run-owned bare app and daemon. `script/screenshot-app bundle /private/tmp/runner-1201-82go05t4` copied the app and CLI; `script/screenshot-app launch /private/tmp/runner-1201-82go05t4` relaunched with inherited agent variables removed. No source was edited or rebuilt while testing the copied candidate.

| Artifact | SHA-256 |
| --- | --- |
| Worktree app and wrapper app, byte-identical | `28ee8749872329bffc8ac9a318cdb4a7111ccfc197bb2ab920f7eb4b02a9654e` |
| Worktree CLI and installed development CLI/daemon sidecars | `16a57e4a4bda44c39565a8fcd4dba885b9ee9db7b165dfe9037dbb7428476b7b` |

### Results

Paths below are relative to the evidence root. Burst directories retain every native screenshot plus `frames.json`, with capture-start/end timing. They are sampled screenshots, not continuous video: maximum adjacent capture intervals were 624 ms for shell, 675 ms for split, and 602 ms for resume. They cannot rule out shorter flashes. No check is promoted to a no-gap pass on that basis.

| Check | Result | Observation and evidence |
| --- | --- | --- |
| 1201-LAUNCH / mac / Claude launcher | Blocked | Selected Claude Code from the native launcher in the never-trusted canonical folder `launcher/`. `Starting chat…` appeared immediately (`launcher-starting.jpeg`). Claude produced no visible TUI before or after the 10-second safety timeout; the resulting pane contained only a cursor (`launcher-no-tui.jpeg`, `launcher-after-raise.jpeg`). A no-gap handoff could not be tested. |
| 1201-SPLIT / mac / Claude split pane | Blocked | Used Command-D on the owned chat, then selected the QA1201 Claude role in the empty pane, with the fresh canonical `split/` cwd. The pill remained visible throughout the 110 captured frames spanning 9.37 seconds (`split-start/000.jpeg` through `109.jpeg`). Later the pane showed only a cursor, with no Claude frame. First-frame handoff remains unobserved. |
| 1201-RESUME / mac / Claude stopped chat | Failed | Stopped the owned launcher chat, verified `Chat stopped`, then clicked Resume. `Resuming…` appeared in `claude-resume/000.jpeg` and remained through frame `003`; by frame `004` it had disappeared, leaving only a cursor through frame `139`. This occurred about 1.23–1.83 seconds after capture began, well before the 10-second timeout. No visible Claude frame preceded dismissal. `claude-stopped.jpeg`, `resume-state.json`, and `resume-blank.jpeg` supplement the burst. A successful previously painted conversation was unavailable, so that resume variant also remains blocked. |
| 1201-SHELL / mac / new terminal tab | Blocked | Command-T created the owned zsh tab. `Starting terminal…` stayed visible throughout 60 frames spanning 7.58 seconds (`terminal-start/`). No shell prompt appeared after its timeout (`terminal-no-prompt.jpeg`). The zsh child `path_helper` was stalled in a filesystem-open call; prompt-to-pill timing could not be validated. |

### Finding and prerequisite limit

**Early resume dismissal, attribution unresolved:** reproduce by starting Claude in a fresh scratch folder while its startup is stalled before visible output, stopping that owned chat, then clicking Resume. Expected: keep the Resuming pill until visible output, or the final 10-second safety timeout. Actual: the pill disappeared after roughly 1–2 seconds onto a cursor-only pane. Relevant settling boundaries are `crates/runner-app/src/surfaces/chat_lifecycle.rs:86`, `crates/runner-app/src/surfaces/chat/mod.rs:719`, and snapshot restoration at `crates/runner-terminal/src/terminal.rs:776`. QA did not establish which condition fired, inspect raw PTY bytes, or change implementation code. The coder and reviewer received the finding during execution.

The launcher Claude process remained alive without a visible frame for multiple minutes. A one-second process sample showed Bun worker threads blocked in `openat$NOCANCEL` / `__openat_nocancel` (`claude-startup-sample.txt`). The new shell's `path_helper` sample showed `__fts_open` / `open$NOCANCEL` (`path-helper-sample.txt`). This suggests a host/filesystem startup prerequisite, but no baseline build was tested, so neither a pre-existing bug nor a candidate regression is established. No system permission, authentication, global runtime setting, shell startup file, or implementation was changed to obtain a pass. A native system-notice inspection was unavailable through computer use; no bypass was attempted.

Native folder-picker Return/paste confirmation was unreliable during setup; selecting the verified path suggestion by double-click and then clicking Pick a working directory succeeded. This setup recovery occurred before launching Claude and does not alter the result matrix. A continuous recording was not obtained; sampled evidence and its limits are disclosed above.

### Cleanup

The QA lock was acquired before `make run` and held through teardown. Only these objects were created or operated:

| Object | ID / path | Final state |
| --- | --- | --- |
| Launcher Claude chat | `01M7F7G18T7DK0XC9SNZM1JFJ7`, canonical scratch `launcher/` | Stopped and archived; history retained. |
| Split Claude chat | `01M7F7NY2NB71HT7MW7W8J0WAM`, canonical scratch `split/` | Stopped and archived; history retained. |
| New terminal tab | `01M7F7JF9RX1W6NEY0GY9G8VVJ` | Stopped; the ephemeral shell record was removed by stop. Subsequent archive/show returned session not found. |
| QA role | `qa1201-claude`, `01M7F7NK279EXHM6YSE265V6YB` | Retained as a stopped-chat fixture; no live agent. |
| Scratch root and wrapper | `/private/tmp/runner-1201-82go05t4` | Retained with evidence; wrapper unregistered from LaunchServices. |

Before first launch, backed up native `~/.claude.json` and `~/.claude/settings.json` privately under the evidence root. Cleanup removed only the two run-created scratch-folder entries from `.claude.json`'s `projects` map; no unrelated project entry changed. `settings.json` remained byte-identical. `configuration-cleanup.json` records the comparison. No trust dialog was accepted.

`session stop` / `session archive` and the final session JSON files record cleanup. `script/screenshot-app quit`, development `runner daemon stop`, and `lsregister -u` shut down and unregister the owned wrapper. Process inventory confirmed no candidate app, development daemon, or tracked test descendants remained (`cleanup-processes.txt`), then `script/qa-lock release` succeeded. Pre-existing development chats and the installed app were not operated. The required live checks remain unresolved; no clean-QA verdict or PR-label removal is authorized by this run.

## 2026-11-15 — retry on candidate 7b49383

**Verdict: Passed. NO REMAINING MUST-FIX QA ISSUES for the four selected macOS checks.** Fresh Claude launches from the launcher and split pane, stopped-chat resume, and the new terminal prompt all handed off from the pill to visible terminal content without a sampled blank frame. This is feature QA, not a full regression pass. The prior failure remains a historical result; this retry establishes the current candidate's result.

### Candidate and setup

The coder handed off frozen commit `7b49383632092947a12eb8d7fe527a2db5821f3a`, tree `b4b25334117d25b9d0665b1c675c63cc4bd0df09`, after clean review. Branch and canonical worktree were unchanged. Only this existing, untracked QA record was dirty; there were no source edits during build or execution. Automated delta checks were not repeated by QA. The delta's actual output/paint provenance and stop/resume baseline changes require the same four native checks, without widening the matrix.

The shared QA lock was initially held by mission `01M7F5ZFZRREQKHPHMYYH3PG55`; QA reported the holder to the coder and waited in the bounded acquisition. No other mission's app or sessions were operated. After acquiring the lock, verified the prior macOS development app had exited, then ran `env -u NO_COLOR -u RUNNER_CREW_ID -u RUNNER_MISSION_ID -u RUNNER_HANDLE -u RUNNER_EVENT_LOG make run` from this worktree.

Evidence root: `/private/tmp/runner-1201-retry-iljku5zp/evidence/`. App/CLI/namespace verification is in `dev-status.json`; build/wrapper comparisons are in `artifact-hashes.json`. The app remained `0.19.9 (dev)`, CLI `0.19.9`, macOS 26.6.2 arm64, Claude Code `2.1.325`, with the existing app zoom and settings. Both run-owned Claude roles requested `haiku` / `low`; the native TUI displayed Haiku 5.5 with low effort and the existing bypass-permissions state. No permission mode, account, authentication, extension, or global setting was changed.

| Artifact | SHA-256 |
| --- | --- |
| Worktree app and copied wrapper app | `8b74f9de22aee6d52b44a942980f339ea8a3acf82219425458b9359f1d19419d` |
| Worktree/copied CLI and installed development CLI/daemon sidecars | `01772d8279bac30c94071b34ee98d0ef3f07bfa198975128d89552cd79e60c77` |

After `make run`, stopped only that run-owned bare app/daemon and copied the byte-identical artifacts with `script/screenshot-app bundle /private/tmp/runner-1201-retry-iljku5zp`. For this retry, launched the wrapper executable directly with inherited `CLAUDE*`, `CODEX*`, `RUNNER_*`, `ANTHROPIC_*`, and `NO_COLOR` variables removed, as the full-smoke setup procedure supports, instead of using `open` through `script/screenshot-app launch`. Native controls targeted the new wrapper's full path, `/private/tmp/runner-1201-retry-iljku5zp/Runner Screenshot Dev.app`; the absolute development CLI confirmed the standard development socket before any input.

The previous startup stalls did not reproduce in this launch context. A separate read-only `/usr/libexec/path_helper -s` prerequisite probe outside the development app also completed in 3 ms. Since the candidate and wrapper launch method both changed, this does not establish the cause of the earlier filesystem-open stalls or retroactively change those results. No system permission or runtime configuration workaround was applied.

### Retry matrix and evidence

All evidence paths in this section are relative to the retry evidence root. Native screenshot bursts retain every original capture and `frames.json`; contact sheets and `observations.json` support visual review. Timing below is measured from the start of capture immediately after the native action, not exact process-spawn latency. No frame in the four captured transitions lacked both the pill and visible output. The maximum adjacent capture interval was 418 ms for launcher, 401 ms for resume, 116 ms for split, and 78 ms for shell; sub-interval flashes cannot be excluded by sampled screenshots. No continuous video was obtained.

| Check | Result | Native observation and evidence |
| --- | --- | --- |
| 1201-LAUNCH / mac / Claude launcher | Passed | Selected the run-owned Claude role from the native launcher in the fresh canonical `launcher/` folder. `Starting chat…` was visible in frames `000`–`005`; frame `006` showed Claude's TUI header and prompt, with no sampled blank intermediate frame. Last pill capture: 1.451 s; first TUI capture: 1.520 s. `launcher-selection.jpeg`, `launcher-start/`, and `launcher-turn/069.jpeg` show the native selection, handoff, submitted tiny prompt, and visible `ACK1201`. |
| 1201-RESUME / mac / painted Claude chat | Passed | Stopped the owned launcher chat after its completed tiny turn. The stopped terminal contained Claude's `Resume this session with:` text, exercising the retained pre-stop output boundary. Clicked native Resume: frames `000`–`005` showed `Resuming…`; frame `006` showed the actual resumed TUI with the prior ACK and a usable prompt. Last pill capture: 1.780 s; first resumed TUI: 1.846 s. `claude-stopped.jpeg`, `claude-resume/`, `launcher-before-stop.json`, and `launcher-after-resume.json` show the transition and unchanged Runner/native conversation identity. Retained pre-stop text did not dismiss the pill early. |
| 1201-SPLIT / mac / Claude split pane | Passed | Command-D opened an empty pane; selected the run-owned split role using the fresh canonical `split/` folder. Frame `000` showed the Starting pill; frame `001` showed Claude's TUI and prompt. Captures were 0.416 s and 0.530 s respectively, with no sampled blank gap. `split-selection.jpeg`, `split-start/`, and `split-turn/059.jpeg` show the selection, first-frame handoff, submitted tiny prompt, and visible `ACK1201S`. The existing owned left pane remained rendered. |
| 1201-SHELL / mac / new terminal tab | Passed | Command-T created the owned zsh tab. Frames `000`–`010` showed `Starting terminal…`; frame `011` showed the colored shell prompt, retained through frame `099`. Last pill capture: 1.010 s; first prompt: 1.068 s. `terminal-start/` and `shell-contact.jpg` show the prompt handoff without a sampled blank gap or stuck overlay. No command was sent to the shell. |

The launcher and split prompts were respectively `Reply exactly ACK1201. Do not use tools.` and `Reply exactly ACK1201S. Do not use tools.` Native draft captures verified the exact text before submission. Both completed visibly with the requested ACK. The resumed session retained its non-empty native key and the prior turn. No trust dialog or manual startup approval appeared in either fresh folder.

### Retry cleanup and delivery

| Run-owned object | ID | Final state |
| --- | --- | --- |
| Launcher Claude chat | `01M7FASS7BWAA8QPPG0S5PJ87S` | Stopped and archived; conversation retained. |
| Split Claude chat | `01M7FAX19T3MQPGPQPKF3JN5SS` | Stopped and archived; conversation retained. |
| Terminal tab | `01M7FAY5GN74C85G0H973WRBQQ` | Stopped; ephemeral shell removed, subsequent show returned session not found. |
| Launcher fixture role | `qa1201r-launch`, `01M7FAR53G4CPE39MEKBNJ20EE` | Retained with the archived chat; no live agent. |
| Split fixture role | `qa1201r-split`, `01M7FAR53TKD7WN39Q733KGC20` | Retained with the archived chat; no live agent. |

`ledger.json` and per-session before/after cleanup JSON files retain final states. Removed only the two run-owned scratch-folder trust entries from the native `.claude.json` projects map; no unrelated project entry changed, and `.claude/settings.json` remained byte-identical (`configuration-cleanup.json`). No global agent configuration, development setting, or pre-existing session was edited.

Stopped the owned wrapper and development daemon, unregistered only this wrapper from LaunchServices, verified their processes and descendants had exited (`cleanup-processes.txt`), and released the QA lock. Other installed, simulator, and private-test processes were left untouched. The scratch root, binaries, screenshot originals, and conversation history remain available for review. The PR had not been opened at handoff; the coder should include these four passing checks and the evidence record in its description. The required macOS live-test gate may be cleared for this candidate or an identical source tree.
