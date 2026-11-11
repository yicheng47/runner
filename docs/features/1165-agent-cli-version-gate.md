# 1165 — Agent CLI version gate, update notice and background updates

Tracking issue: [#1165](https://github.com/yicheng47/runner/issues/1165). Related: [#803](https://github.com/yicheng47/runner/issues/803) (automatic background updates) builds on the background tasks this spec adds.

Design: `runner/desktop/specs/1165-agent-cli-version-gate.pen` in `yeelinks_design`, signed off on 2026-11-11 (`8e9db8f`), with Jason’s same-day agent-only launcher labels/copy revision saved at `d0068fd`. That revision supersedes B’s former role-row badges; the launch gate and recovery still cover roles. Frames:

- Row 1, an agent below its minimum version: `pdilP` B · gate at the launcher, `P5ZLz` the fallback card for a rejected launch.
- Row 2, a newer version exists: `jkXP9` G · tag in the launcher and a dot on the Settings gear.
- Row 3, running an update: `LmHc5` today's blocking dialog, `fpEQV` the Settings → Agents row, `B7aVW` the background task pill, `gedmM` the task list, `rh80T` a task that may need input, `g9cXt` the update dialog opened by Answer, `wkpG8` a failed update and a stop that broke the agent, `K7b0i2` the pill's states and priority.

The frames win on layout and copy; this spec wins on behavior.

## Motivation

Runner launches Codex with config overrides that older Codex versions reject. Runner passes its status hooks as `type="mcp_tool"` (`crates/runner-daemon/src/runtimes/codex/mod.rs`), which Codex parses from 0.148.0 (openai/codex#37363) and runs in sessions from 0.149.0 (openai/codex#39296). On an older Codex the chat exits at once with ``Error loading config.toml: unknown variant `mcp_tool`, expected one of `command`, `prompt`, `agent` in `hooks` ``, and the pane shows only the generic stopped card, whose Resume relaunches into the same error. A user hit this in the field on 2026-11-04.

Nobody is told that Codex is out of date: Runner passes `-c check_for_update_on_startup=false` (#475, #498) so Codex's own notice doesn't interrupt a chat's startup, and Runner's own check (#533, `runtime_status/versions.rs`) only shows in Settings → Agents.

When someone does update, the update dialog (`crates/runner-app/src/surfaces/agent_update.rs`) covers the whole window and cannot close until npm or brew exits, because stopping an installer halfway can break the install. runnerd already owns that process, so the block protects nothing the window needs.

## Scope

### 1. Minimum version

- `RuntimeCatalog` (`crates/runner-core/src/protocol/runtime.rs`) gains `min_version: Option<&'static str>`. Codex declares `0.149.0` (confirmed by the [portable CLI probes](../tests/1165-codex-compatibility.md)); every other runtime declares none.
- The runtime status the app loads carries each runtime's minimum and whether the probed installed version is below it. An unknown installed version (probe not answered, timed out or unparsable) is never below the minimum.

### 2. Gate at the launch (B)

- runnerd refuses to compose a launch for a runtime below its minimum: new chats, resumes, forks and mission slots, from the app, the CLI or a remote window. The refusal is a typed error carrying the runtime, the installed version and the minimum, not a spawn that fails later.
- The launcher (`pdilP`) marks only that agent’s row **Update needed**. Role rows never show update labels or actions, including below the minimum. The notice under the list explains the underlying agent requirement: "Codex 0.147.0 is too old for Runner. Codex needs 0.149.0 or newer. Starting Codex or any role using it first updates Codex to 0.167.0."
- Starting that agent or a role using it starts that runtime's update as a background task (section 5) and opens the pane with the existing stopped-chat card (`cmp/StoppedChatCard`) reading "Starts when Codex finishes updating". When the task succeeds, the pane launches as it would have; when it fails, the card shows the failure with Update Codex.
- A pane whose launch was refused elsewhere (a resume, a mission slot, a CLI start) shows the same card with **Update Codex**, which starts the task; the pane launches when it succeeds.

### 3. Fallback for a rejected launch

- When an agent exits with a non-zero code within 10 seconds of starting and before its first turn, runnerd keeps the last lines of its output and asks the runtime adapter to classify them. Codex's classifier matches `Error loading config.toml: …`; other runtimes have none yet.
- A classified exit is recorded with the session's stop, and the stopped card (`P5ZLz`) shows "Codex didn't accept Runner's launch flags", the cause in plain words, and **Update Codex** when the runtime has an update command. An unclassified exit keeps today's card.
- This covers a flag that breaks in a future version before the minimum is raised.

### 4. An update exists (G)

- In the launcher (`jkXP9`), an agent with a newer version shows `<installed> → <latest>` and an **Update** action in its row. Roles that run on it show nothing, so one release is one tag. The chat still starts on the installed version; Update starts the background task and does not block it.
- The sidebar footer's Settings gear carries a dot while any enabled agent has an update, and opens Settings → Agents. It replaces the dot on the usage popover's gear (#533), so agents without a usage meter are covered.
- Covers every runtime with an update command. No toast and no dialog for an available update.

### 5. Background tasks replace the blocking dialog

An update is a background task owned by runnerd. The window stays usable, and nothing opens unless the person asks.

- **Starting.** Update in Settings → Agents (`fpEQV`), in G's launcher tag, or from a gated pane starts the runtime's update task. A runtime has at most one: starting it again while it runs joins the running one. The Settings row shows "Updating to 0.167.0…" while it runs.
- **Pill** (`B7aVW`, states in `K7b0i2`). At the bottom-right of each window, shown only while there is a task: an icon, the most urgent task's label, a progress bar while running, a count of the other tasks, and ×. Priority: may need input › failed › running › finished. Clicking the pill opens the task list above it. × hides the pill while the tasks keep running; it comes back only when a task may need input or fails, never for a success.
- **Task list** (`gedmM`). Each task shows the agent, `<from> → <to>`, and its latest output line as the step. A running task has a progress bar, an output button and × to stop it.
- **May need input** (`rh80T`). runnerd moves today's guess (`waiting_for_input`: quiet for 2 seconds with the cursor mid-line) from the dialog to the task. The pill and the row turn amber and say "may", because it is a guess. Nothing opens by itself, so chat typing never lands in a password prompt. **Answer** opens the update dialog.
- **Update dialog** (`g9cXt`). The existing dialog, attached to the task's terminal and focused, opened by Answer or a task's output button. Hide and the close button always work; closing it leaves the task running.
- **Stop.** × on a running task interrupts the installer and closes the row at once. runnerd then probes the agent's version: if it still starts, nothing remains; if it doesn't, a row stays, "pi is broken after a stop", with **Reinstall**, which reruns the update command (`wkpG8`).
- **Failure** (`wkpG8`). A failed task stays with its exit code, a plain reason, **Retry** and **Show output**, until retried or dismissed.
- **Success.** The row shows the new version, and "Running Codex chats keep 0.166.0 until they restart" when sessions are running. When every task has finished successfully, the pill shows the result for 5 seconds, then the pill and the finished tasks clear themselves.

### Storage and protocol

- No database change. The task list lives in runnerd's memory: tasks do not survive a daemon restart, and an update's process is already tied to runnerd.
- The protocol gains: start a runtime's update (replacing `runtime_update_prepare` / `runtime_update_run`, with runnerd choosing the initial terminal size and the dialog resizing on attach), list tasks, stop a task, dismiss a task, and a task-changed event. The dialog attaches to a task's terminal through the existing unlisted-terminal attach by session id.
- Runtime status gains the minimum version and the below-minimum flag. Session stop records gain the classified startup error.
- The CLI gets no new commands. A CLI start refused by the gate prints the typed error.

## Out of scope

- Automatic updates and the "Update agents automatically" setting: [#803](https://github.com/yicheng47/runner/issues/803), which reuses these tasks and treats a wait for input as a failure because no one is watching.
- Dropping launch flags an old version doesn't support (the issue's option C).
- Showing another computer's update tasks in a remote window. A gated launch on a remote computer shows its card there; the pill and list show the local daemon's tasks only.
- Runner's own app updates (#1204), which keep their sidebar icon and popover.

## Guide

- `docs/guide/reference/supported-agents.md`: minimum versions, the update notice in the launcher, and updates running as background tasks.
- `docs/guide/guides/troubleshooting.md`: "Codex is too old for Runner" and "Codex didn't accept Runner's launch flags".
- `docs/guide/guides/chats-and-windows.md`: the background task pill and list.
- The `zh-CN` guide pages that mirror them.

## Verification

### Automated

- Version comparison against a minimum, including unknown and prerelease versions.
- The launch gate refuses each launch path for a below-minimum runtime and allows an unknown version.
- Codex's startup-error classifier on the field error and on unrelated output.
- The task state machine: start, join, may need input, success, failure, stop with the agent still working, stop that breaks it, and the 5-second clear after all tasks succeed.

### Live checks

The change alters what people see and operate, so it needs live checks, run by the `qa` slot of the codex trio Jason requested on 2026-11-11. Each check uses a scratch executable set as the agent's Executable override: a script answering `--version` with a chosen version and `update` with the behavior under test (succeed after a few seconds, exit 243, prompt with `read -s`, or break `--version` when interrupted). QA never updates the machine's real agent installs.

1. **Gate (B).** Codex reporting 0.147.0: only its agent row shows Update needed; a Codex role has no update label or action. The notice names Codex’s minimum and explains that starting Codex or any role using it updates the agent first. Starting either opens the pane’s "Starts when Codex finishes updating" card and the pill; the chat starts when the update succeeds. Reason: new gate and launch behavior, with Jason’s agent-only row scope.
2. **Gate on resume.** A stopped Codex chat resumed while below the minimum shows the card with Update Codex instead of relaunching. Reason: launch paths beyond the launcher.
3. **Fallback card.** A Codex whose version is unknown but whose start prints the field's `Error loading config.toml` error and exits: the stopped card names the cause and offers Update Codex. Reason: new exit classification.
4. **Update available (G).** Codex reporting an older but supported version: the launcher row shows the versions and Update, roles show nothing, the Settings gear has a dot; Update starts a task and the chat starts immediately. Reason: new notice surfaces.
5. **Background task and pill.** From Settings → Agents, Update shows the row's "Updating to …", no dialog, the window stays usable; the pill appears, opens the list, × hides it, and it clears 5 seconds after success. Reason: the dialog's replacement.
6. **May need input.** A prompting update turns the pill and row amber, opens nothing by itself, and Answer opens the dialog focused on the prompt; Hide closes it and the task keeps waiting. Reason: input handling moved out of the modal.
7. **Failure and stop.** A failing update stays with Retry and its reason; stopping a running task closes its row when the agent still starts, and leaves Reinstall when it doesn't; the hidden pill returns on failure. Reason: new stop and failure behavior.

Review regression checks extend this matrix: in check 1, have two empty panes share one update and verify both launch even when a tab start holds the reservation; in checks 2/5, update from Settings after a refused resume and verify the old gate disappears; in check 3, read the actual bounded cause for both the unknown hook type and another config error; in check 5, finish deferred mission/fork updates after opening later work or switching computers and verify the saved host and input focus remain correct. While a deferred mission/fork update still runs, open a new launcher, visit Settings and Back to app, then type: only the visible launcher receives input. In checks 5/6/7, verify the loader/keyboard/error/success icons, progress only while running, and progress before the other-task count.

Also run a bounded remote refusal → Update → retry check for a chat resume, fork, mission resume and slot restart with a safe scratch host fixture. A restart must stay a restart and host tasks must stay out of the local pill/list. This is directly affected UI coverage; if a safe remote fixture is unavailable, record the prerequisite as blocked rather than changing accounts, trust or installed agents to obtain a pass.

## Minimum-version decision

The 2026-11-11 [portable CLI probes](../tests/1165-codex-compatibility.md) confirmed the parsing/execution boundary on 0.147.0, 0.148.0 and 0.149.0, with 0.163.0 and 0.164.0 as additional execution probes. Runner's enabled, session-scoped MCP server delivers hooks on 0.149.0 and 0.163.0 without the later cached/dormant-server fix. Keep the minimum at **0.149.0**; see the evidence record for the tested cases and limits.

## Implementation evidence

Implemented and checked on macOS arm64 on 2026-11-11. The daemon owns the minimum-version refusal, classified startup stop and in-memory update lifecycle. The app consumes task snapshots for launcher recovery, Settings, the local pill/list and explicitly opened terminal dialogs; remote recovery uses host-scoped task requests and does not enter the local list. A local gated start reserves an empty tab, releases the launch reservation while updating, and fills that tab after success without replacing later work or keyboard focus.

Rebased automated validation on macOS arm64: `cargo test --locked -p runner-core -p runner-daemon --profile ci` passed (core 176; daemon 1,869). Nextest ran 3,839 cases across app/core/daemon/CLI: 3,831 passed in the initial batch, with eight daemon subprocess-deadline failures and six existing manual/authenticated skips. All eight failed cases passed a targeted Nextest rerun with one worker. The app's 1,409 library cases and integration targets, and the CLI's 86 unit cases, 186 command goldens and 24 daemon-process cases, passed within the Nextest run. The initial aggregate result remains failed in its log; every selected case has a passing result after the targeted reruns. An earlier Cargo app run timed out waiting for a filesystem change, and an earlier CLI golden run missed its fixture daemon's 10-second startup deadline; both passed in isolation and in Nextest. These timing failures did not change test assertions or expectations.

The catalog golden intentionally adds only Codex's `min_version: 0.149.0`; the new companion goldens cover task requests and the typed refusal. Tests cover required versus optional launcher notices, agent/role launch intent, mission refusal recovery, explicit dialog dismissal/focus, hidden-pill transitions, remote duplicate joining and retained-terminal isolation, in addition to the spec's version/gate/classifier/task-state cases.

`cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, its `--features updater` pass, `cargo fmt --all -- --check`, `git diff --check` and `python3 script/check-guide-translations.py` passed. Native UI evidence is recorded separately in the #1165 sections of [the macOS run](../tests/runs/2026-11-11-mac.md); these automated results do not claim Windows, Linux or native-app coverage. The portable Codex compatibility evidence above is separate from UI QA.

The reviewed UI delta additionally tests target-validated remote refusal recovery, operation-preserving slot restarts, host-scoped stale-gate removal on a new probe, two deferred panes queued behind a tab reservation, saved mission/fork clients across computer switches, retained later work/focus, visible bounded startup causes and the signed-off pill state presentation. These are in-process app tests; their native equivalents remain selected above.

Rebased onto `origin/main` at `1f5f159ad1b0a5b62fe668ee262620019e913bf2` after #1269/#1270 landed. Conflict resolution kept the new Usage settings route, per-agent Skills/MCP tabs, usage state and protocol compatibility tests alongside the update tasks. The Settings-return regression keeps a suspended mission/fork page and a new visible launcher alive during the update, visits Settings and Back to app, and verifies that only the visible launcher receives typing; launcher focus now uses its canonical cache key and visibility predicate.

Jason’s same-day agent-only row revision was drawn and saved at design commit `d0068fd` before implementation. The launcher applies the agent-row predicate before either required or optional update decorations; role launch gating, deferred intents and recovery are unchanged. The revised agent/role intent test and full `runner-app` targets passed (1,409 library tests plus integration targets, with existing ignored cases). Native checks 1 and 4 passed the revised row/copy expectations on `c9654ac7`; earlier observations retain their original candidate attribution.

Native QA on `86c9aed0` found that Escape did not hide the update dialog when the attached installer terminal had focus (SET-UPDATE-14). A focused-terminal regression reproduced the missed dismissal before the fix. The dialog now captures Escape before terminal input consumes it; the regression and full `runner-app` targets passed (1,410 library tests plus integration targets, with existing ignored cases). QA’s affected-case rerun on `c9654ac7` passed focused-terminal Escape, restored chat typing, same-task reattach and Hide/X; the original failure remains part of the run evidence.

Local native checks found no remaining must-fix issues after the affected retest, but selected coverage remains incomplete. The safe remote-host check/cross-host completion, Windows/Linux native checks and exact busy-tab reservation overlap are unpassed, as [the QA record](../tests/runs/2026-11-11-mac.md) details. Keep PR #1268 draft with `needs-live-test`; the two waiting-tab join/launch observation and automated reservation regression do not establish the exact native overlap.
