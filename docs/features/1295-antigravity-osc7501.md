# Use OSC 7501 instead of status hooks for Antigravity CLI

Tracking issue: [#1295](https://github.com/yicheng47/runner/issues/1295).

## Motivation

`agy` (`1.5.8` verified) natively supports both OSC 7501 terminal program status and `.agents/hooks.json` lifecycle hooks. Today Runner installs 4 status hooks (`PreInvocation`, `PostToolUse`, `PostInvocation`, `Stop`) via `--add-dir antigravity-hooks` by default on macOS and in all missions. Because `SessionModel` locks out OSC 7501 once hooks arm (`hook_status_armed`), Runner drops `agy`'s richer native status stream while hitting several hook accuracy and overhead issues:

1. **Missing interactive waits**: `agy` has no hook for tool/plan/subagent approvals, `ask_user_question`, authentication/re-auth prompts, or workspace trust. Runner stays falsely `Working` while `agy` is actively emitting OSC 7501 `state=blocked` (`kind=permission|question|auth` with decoded `msg`).
2. **`Stop` edge cases**: `AgyParser::hook` ignores `Stop` when `fullyIdle` is false/absent (background commands or subagents keep Runner stuck `Working`) and ignores `terminationReason` (so `USER_CANCELED` with empty `error` reads as `Completed` instead of canceled/idle).
3. **False `Esc` cancellation**: Raw `Esc` bytes during `Working` inject `LocalCancel` even when `agy` only dismisses UI state or ignores `Esc`.
4. **Hook & `--add-dir` overhead**: `agy` runs hooks synchronously on every turn and tool call, and `--add-dir antigravity-hooks` adds Runner's internal hooks folder as an extra workspace root, requiring the `#787` `PreInvocation` `ephemeralMessage` cwd workaround.

Unlike Codex, Claude Code, pi, Cursor, OpenCode, Copilot, and Grok, Antigravity does **not** use hooks for conversation identity or `/clear` rekeying: `KeyCapture::LogTail` (`agy_capture.rs`) continuously tails `--log-file` for `Streaming conversation <uuid>`. Removing Antigravity status hooks eliminates all lifecycle/status hook shims, `.agents/hooks.json` generation, and `--add-dir antigravity-hooks` at once (leaving `#1292`'s `--settings` chat-title callback separate), aligns with [P0.8](../arch/runtime-integration.md#p08-honest-status-and-capability-boundaries), and works uniformly across macOS, Linux, and Windows.

## Scope

### In scope

1. **Retire Antigravity status hooks and Runner-injected `--add-dir` workspaces**:
   - Return `status_hooks() -> None` and `mission_dir_args() -> Vec::new()` for `Antigravity`, removing `antigravity_status_args` (`--add-dir antigravity-hooks`), `agy_status.rs` hook parsing/installation, `WORKSPACE_CONTEXT_ENV` (`#787` ephemeral context), and the `--add-dir <mission_dir>` mission-bus workspace (missions already run with `--dangerously-skip-permissions` and coordinate through the `runner` CLI, whereas adding Runner's internal bus folder causes `agy` to sort it ahead of later-sorting project `cwd` paths as `Command Working Directory`).
   - Remove `Runtime::Antigravity` from `RuntimeLaunchOption::StatusHooks` (`crates/runner-core/src/protocol/runtime.rs`), Settings launch option copy (`crates/runner-app/src/surfaces/settings/agents/launch.rs`), command previews, and hook-watcher tests.
   - Clean up any previously generated `antigravity-hooks` directory safely on daemon startup/housekeeping without touching user config or `--log-file` capture.
2. **Keep conversation identity via `LogTail`**:
   - Retain `--log-file` and `agy_capture.rs` (`KeyCapture::LogTail`) unchanged for initial conversation capture and `/clear`, `/new`, `/fork`, and `/resume` rekeying.
3. **Promote OSC 7501 (`ProgramStatus`) for managed Antigravity sessions**:
   - **`agy 1.5.8` behavior already emitted on `\x1b]7501;?\x07` + `PROBE_REPLY` (`\x1b]7501;?\x1b\\`)**:
     - `state=working` (`AgentWorking`, `BattleWorking`) -> `Working`
     - `state=blocked` with `kind=permission|question|auth` + base64 `msg` -> `Approval` / `Answer` / `Needs you` with decoded detail
     - `state=done` (turn completed reply) -> `Idle` + `TurnOutcome::Completed` (clears to plain `state=idle` on next keypress)
     - `state=error` (`SettingsError` / `LastStepError`) -> `Idle` + `TurnOutcome::Failed`
     - `state=idle` (interrupted turn `TurnOutcome == 1`, post-keypress dismissal, or `ExitProgramStatus`) -> `Idle`
   - **Direct-chat completion / unread badges (`events.rs`, `state/mod.rs`)**: Keep `#861`'s no-notification contract intact for bare shell tabs (`Runtime::is_shell()`, plus existing `RT-STATUS-14` shell-launched agent tabs), while allowing managed Antigravity sessions (`program_status` active in an agent session without hooks) to arm completion on `Working` and trigger `record_session_completion` on `State::Done` (`TurnOutcome::Completed`). Interrupted (`idle`) and `error` (`Failed`) must not create completion badges.
   - **Startup & mission delivery readiness (`startup.rs`, `status.rs`)**:
     - Update `StartupDelivery` so a runtime using OSC 7501 for status waits deterministically on `ProgramStatus` (holding on `Working`/`Blocked` and for initial `-i` first-turn completion when `pending_turn` is set, releasing on `Idle`/`Done`/`Error`), keeping the `STARTUP_OUTPUT_GRACE` output-silence timer as fallback for older `agy` binaries that never probe OSC 7501.
     - Increment `session.turn_started` (and preserve mission delivery confirmation) when a hookless OSC 7501 agent transitions into `State::Working`.
     - Preserve mission delivery `NeedsYou` holds on `State::Blocked` and wake readiness when resolved to `Working`/`Idle`/`Done`.
4. **No hard CLI version gate (`min_version: None`)**:
   - Leave `min_version: None` for Antigravity (matching Claude Code and pi; `Antigravity` also has `update_args: &[]` / `npm_package: None` and self-updates at startup). Older `agy` versions without OSC 7501 degrade softly to Runner's terminal-activity baseline.
5. **Documentation & regression suite**:
   - Update `docs/guide/reference/supported-agents.md`: remove Antigravity from the `Status hooks` option row/copy, update Antigravity CLI capabilities for `OSC 7501 program status` (`1.5.8+`) and `Needs-input reporting` (`Approvals, questions and auth via OSC 7501`), and update hook-removal / fallback notes.
   - Update `docs/arch/runtime-integration.md` (P0.8 / hooks-off notes) and `docs/tests/regression/runtimes.md` for Antigravity's OSC 7501 contract (e.g., `Idle` on interrupt without hook `Interrupted` outcome, `blocked` approval/question/auth waits, and removing obsolete `#787` hooks-dir assumptions where hooks are gone).

### Out of scope

- Changing Codex, Claude Code, pi, Copilot, Cursor, Grok, or OpenCode status hooks or precedence (`RT-STATUS-15`).
- Changing `#861` behavior for hand-started agents inside plain shell tabs (`RT-STATUS-14` still produces no unread completion alerts).
- Adding an `update_args` self-updater or `RuntimeVersionGate` UI changes for Antigravity.

## Implementation Phases

Single PR (`feat/1295-antigravity-osc7501`), labeled `core-review` (touches `session/state/`, `session/manager/`, `runtimes/`) and `needs-live-test`:

1. **Daemon runtime & state (`runner-daemon`, `runner-core`)**:
   - Remove Antigravity `StatusHooks` implementation, `--add-dir antigravity-hooks` args, and `StatusHooks` launch option metadata for `Runtime::Antigravity`.
   - Extend `SessionModel` / `StartupDelivery` / `CoreSessionEvents::status` / `turn_started` tracking so managed hookless OSC 7501 runtimes (Antigravity) get completion notifications on `Done`, deterministic `StartupDelivery`, and mission `turn_started` confirmation without regressing shell-tab `#861` or hooked-runtime precedence.
   - Clean up legacy `antigravity-hooks` artifacts during startup cleanup if present.
2. **Settings UI & copy (`runner-app`, i18n if needed)**:
   - Remove the Antigravity `Status hooks` switch/caption (`launch.rs`, previews, tests).
3. **Docs & tests**:
   - Update `docs/guide/reference/supported-agents.md`, `docs/arch/runtime-integration.md`, and `docs/tests/regression/runtimes.md`.
   - Replace/update `agy_status` unit tests with daemon state/startup/manager/golden tests covering OSC 7501 transitions (`working`, `blocked` `permission`/`question`/`auth`, `done`, `error`, `idle`), completion notification gating (Antigravity agent vs shell tab), and mission startup/delivery.

## Verification

### Automated

- `cargo test -p runner-terminal`
- `cargo test -p runner-daemon`
- `cargo test -p runner-core`
- `cargo test -p runner-app`
- `make verify` (plus CI-profile check/nextest)

### Live QA (macOS development app)

Scoped per `AGENTS.md` Test Scope:

- **Lifecycle & rekeying (`RT-LIFE-01`, `RT-LIFE-04`, `RT-LIFE-05`, `RT-LIFE-08`)**: Start an `agy` (`1.5.8+`) direct chat; verify `Working` during tool calls, `Done` -> `Idle` + unread completion indicator on finished turn, clean `Idle` on `Esc` interruption (no false `Response failed` and no spurious `LocalCancel` while dismissed UI leaves turn working), and `/clear` + new turn still rekeys via `LogTail`.
- **Interactive `blocked` waits**: Trigger real `agy` tool/plan confirmation (`permission`), `ask_user_question` (`question`), and auth/trust (`auth`/`permission`) prompts if safely reproducible; confirm Runner shows **Approval needed** / **Answer needed** / **Needs you** with `msg`, and clears cleanly back to **Working** / **Idle** upon resolution.
- **Mission & workspace (`RT-START-01`, `RT-START-04` replacement check)**: Start a bounded single-slot Antigravity mission; verify no Runner `--add-dir` (`antigravity-hooks` or mission-bus directory) is passed in argv, prompt executes in requested cwd, startup waits cleanly, and `NeedsYou` / `Idle` delivery gates behave properly.
- **Non-regression (`RT-STATUS-14`, `RT-STATUS-15`)**: Spot check a shell tab + Claude Code chat to verify shell-tab OSC 7501 still creates no unread badge and hooked runtimes retain hook precedence.
