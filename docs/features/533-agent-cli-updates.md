# Update agent CLIs from Settings → Agents

Tracking issue: [#533](https://github.com/yicheng47/runner/issues/533). Status: planned; Pencil frames pending. Priority P2.

## Motivation

[#475](./archive/475-codex-auto-update.md) (0.8.2) made agent launches quiet. Codex gets `-c check_for_update_on_startup=false` from `trailing_runtime_args` (`crates/runner-backend/src/router/runtime.rs:602`), and Claude Code gets `DISABLE_INSTALLATION_CHECKS=1` and `CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY=1` from `base_spawn_spec` (`crates/runner-backend/src/session/manager/spawn.rs:410`). That was the right call: Codex's startup prompt installed the update and exited, leaving a direct chat on **Chat paused** and, inside a mission, sometimes taking sibling slots down with it. But the prompt was also the only place a Runner user learned that a CLI was stale. #475 recorded the trade explicitly — "CLI updates remain outside Runner" — and this spec is the other half of it.

The gap is uneven across the two CLIs. Codex has no background updater at all: with the startup check off, anyone who runs Codex only through Runner never updates. Claude Code's native installer keeps updating itself in the background (Runner does not set `DISABLE_AUTOUPDATER`), but npm and Homebrew installs sit still, and even a native install that updated underneath a running session stays on the old version until the session relaunches. Nothing in Runner shows which version a session is actually running, so there is no way to tell the two situations apart.

Settings → Agents already has everything the update needs. `runtime_status::status_list` (`crates/runner-backend/src/runtime_status.rs:85`) resolves each agent's effective executable, auto-detected on the login-shell PATH or overridden; the login-shell capture carries the proxy quartet the update download will need; and the app spawns real PTYs. Both CLIs ship their own updater as a subcommand — `claude update` ("Check for updates and install if available") and `codex update` ("Update Codex to the latest version") — and each one picks the right method for its own install: the native version store under `~/.local/share/claude/versions`, `npm install -g`, or `brew upgrade`. Runner never has to know how the CLI was installed. Updating is one more command through the spawn path, shown in a terminal pane so the CLI's own progress, prompts, and failures stay visible.

## Behavior

### Version in the row

Each agent row in Settings → Agents (`crates/runner-app/src/surfaces/settings/agents.rs`) gains the installed version in its caption, beside the path it already shows: `2.1.266 · Detected: /Users/jason/.local/bin/claude`. The version comes from `<effective command> --version`, run off the UI thread with a five-second timeout at three moments: when background discovery completes (`apply_discovery_result`, `runtime_status.rs:359`), on **Refresh**, and when an update pane exits. The parse takes the first semver-looking token in the output — `2.1.266 (Claude Code)` and `codex-cli 0.153.4` both yield one — and a row whose command prints nothing parseable shows no version and no error. Rows in the **Not found** state never probe.

The probe runs with the same environment a session would get (login-shell PATH, proxy quartet, runner env not included), so the version shown is the version a new session would launch. It is a plain child process, not a PTY, and it never appears in the sidebar.

### The Update button

Claude Code and Codex rows gain an **Update** button next to the enabled toggle. TRAE has no update subcommand and gets no button. The runtime definition names the update arguments (`update` for both); a runtime without them has no button, which is the extension point if a fourth runtime arrives.

Pressing Update opens a terminal pane whose PTY child is the update command itself: the effective executable with `update` as its only argument, the agent spawn environment from `base_spawn_spec` minus the runner-specific layers (no runner env, no `--settings`, no mission vars), cwd set to the home directory. It is a shell-kind session so the sidebar, pane header, drawer, and close paths treat it as a terminal, but it is **not resumable**: relaunch never re-runs an update, and the archive path applies as for any ended terminal. Its title is `Update Claude Code` / `Update Codex`.

The pane lands where a fork lands (60): `prepare_new_pane` (`crates/runner-app/src/pane_layout.rs:526`) fills an empty pane in the current tab, else splits, and at the three-pane cap it opens a new tab. Settings stays open; the user switches to the tab to watch, or not.

The CLI does the rest. `claude update` prints its progress and either "already up to date" or the new version; `codex update` spawns npm or brew, both of which print what they are doing. Prompts — Homebrew's, npm's, a sudo password on a global npm prefix, Windows asking to close a program — render in the PTY as they would in any terminal, and the user answers them there. When the process exits, the existing shell-exited overlay (`SessionOverlay::shell_exited`, `crates/runner-app/src/ui/session_overlay.rs:71`) shows the exit code. On `session/exit` for an update session, the Agents pane re-runs the version probe for that runtime, so the row reflects the outcome without a manual Refresh.

Runner does not parse the pane's output. Whether the update succeeded is what the CLI printed plus what `--version` now says.

### Running sessions

Updating a CLI that is currently running is safe on macOS and not on Windows, and the guard follows the platform rather than pretending otherwise:

- **macOS.** Update is always enabled. Unlinking a mapped executable is fine on Unix, and Claude Code's native installer keeps the old version's directory anyway. Running sessions keep the version they started with, so when any session of that runtime is alive the caption says so: "3 running Claude Code sessions keep 2.1.266 until they relaunch." Idle or busy makes no difference; alive is what matters.
- **Windows.** Update is disabled while any session of that runtime is alive — `claude.exe`, or the native binary inside the npm package for Codex, is in use and the installer would fail or half-apply. The caption says "Stop the 3 running Codex sessions first." Counting is by runtime across direct chats, mission slots, drawers, and forks; a running mission with Codex slots blocks a Codex update.

The count is the live process count from the session manager, the same source `session_activity_snapshot` (`crates/runner-backend/src/ops/session.rs:80`) reads, and the caption updates as sessions start and stop while Settings is open.

### Update available

Knowing that an update exists is what the suppressed startup prompt used to provide, so the row provides it instead. When the Agents pane opens, and on Refresh, Runner fetches the `latest` dist-tag for `@anthropic-ai/claude-code` and `@openai/codex` from the npm registry (`https://registry.npmjs.org/<package>/latest`, the `version` field) through the app's existing `reqwest` client (the one the Windows updater builds in `crates/runner-app/src/updater/windows.rs:515`) and the captured proxy environment. Both CLIs publish every release to npm under the same version number they ship natively, so one endpoint covers all install methods; a Homebrew formula that lags npm shows the badge a day early, and `brew upgrade` in the pane says "already installed", which is acceptable.

The result is cached per app run for six hours per runtime. A newer version than the installed one shows an **Update available** badge in the row (warning tone, beside the state badge) and the caption becomes `2.1.266 → 2.1.270 · Detected: …`. No network, a timeout, a non-200, or a malformed body degrades silently to version-only; the row never shows a network error, because the row is about the CLI, not about npm. The check runs only while the Agents pane is open: nothing polls in the background and nothing lights up the Settings icon.

### What stays the same

The #475 suppression is unchanged. Sessions still get `check_for_update_on_startup=false` and `DISABLE_INSTALLATION_CHECKS=1`; Claude Code's native background updater stays enabled. The point of this spec is that the update signal moves out of the session and into Settings, not that it comes back into the session.

## Decisions

1. **A terminal pane, not a captured log.** The update commands are interactive in the ways that matter — brew and npm prompt, sudo prompts, Windows complains about files in use — and their output is long. Capturing it into a row caption would be lossy and would need per-CLI parsing that drifts with every release. A real PTY is the product's primitive; the pane shows exactly what the CLI did, and the row shows the version afterward. The cost is one extra tab or pane the user closes.
2. **The CLI's own `update`, never a package manager Runner chooses.** `claude update` and `codex update` already detect native versus npm versus Homebrew. Runner calling `npm install -g` itself would guess wrong for native and brew installs and would need Node on PATH. The one thing Runner adds is the environment: PATH and proxies from the login shell, which is also what makes the update find the same executable the sessions use.
3. **npm registry for "latest", one endpoint for both CLIs.** Claude Code's native installer reads a manifest from a GCS bucket and Codex publishes GitHub releases, but both also publish to npm at the same version, and one URL shape with one JSON field is less to maintain than two vendor channels. If a vendor stops mirroring to npm, the badge goes quiet and the Update button still works; the check is advisory.
4. **Platform-split guard.** A universal "close all sessions first" rule would make the button useless on a busy macOS day for no safety gain. Windows needs the rule; macOS needs the caption.
5. **No auto-relaunch of stale sessions.** After an update, running sessions keep the old version until the user relaunches them. Relaunching a chat on the user's behalf would interrupt whatever the agent is doing, which is exactly the interruption #475 removed. The caption names the situation; the human picks the moment.
6. **Priority P2.** Codex users drift stale silently today, which is a real loss, but they can still run `codex update` in any terminal, including a Runner terminal pane. Nothing is blocked; the feature makes the right thing visible and one click away.

## Non-goals

- **Background polling** of the npm registry or a dot on the Settings icon. The check runs while the Agents pane is open. If users ask how they would know to open it, that is the signal to add the dot, not a reason to ship it now.
- **Installing a missing CLI.** A **Not found** row stays a not-found row with the same caption it has today. Install flows are vendor-specific and change often.
- **Changing an install method**, such as migrating an npm Claude Code to the native installer. `claude update` prints that advice itself when it applies.
- **TRAE.** No update subcommand is known; the row gains the version probe if `traecli --version` prints one and nothing else.
- **An MCP tool** for `runtime_update_start`. Updating is a human action in Settings; agents do not update their own runtime.
- **Restoring the Claude Code startup nudge** by dropping `DISABLE_INSTALLATION_CHECKS`. The sessions stay quiet.
- **Remote hosts** ([510](./510-remote-ssh-session.md)). The version probe and the update pane run on this machine against the local executable; a remote CLI is updated on the remote.

## Implementation Phases

1. **Pencil frames** in a feature-scoped `design/agent-updates.pen`: the Agents row with the version caption and Update button in the Detected, Override, and Not found states; the Update available badge and the `a → b` caption; the macOS running-sessions caption and the Windows disabled caption; the update pane's title in the pane header and sidebar row. Not drawn with this spec; signed off before phase 3 starts.
2. **Backend.** The version probe in `runtime_status` (`--version` off-thread with timeout, semver-token parse, stored on `RuntimeExecutableStatus` as `installed_version`); `update_args` on the runtime definition; `runtime_update_start(state, runtime)` building the non-resumable shell-kind session from the effective command and the agent spawn environment; a per-runtime live-session count for the guard; `session/exit` carrying enough for the app to recognize an update session. Tests: the parser against both CLIs' real output and against garbage; the spawn spec's argv, env, cwd, and `resumable = false`; the guard count across direct, mission, and fork sessions; the relaunch path refusing an update session.
3. **App.** The row caption and Update button; the guard captions from the live count, split by platform at compile time like the rest of the platform chrome; the pane destination through `prepare_new_pane` with the cap fallback; the version re-probe on `session/exit`. Runner-app tests for the presentation mapping (version, badge, captions, disabled state) alongside the existing `RuntimePresentation` tests.
4. **Update available.** The npm fetch through the shared client and proxy env, six-hour per-run cache, badge and caption, silent degradation. Tests for the version comparison, the cache, and the degrade paths with a stubbed fetch.
5. **Docs.** A line beside #475's decision in `docs/arch/arch.md` recording that the update signal lives in Settings; the README index entry; archive this spec when #533 closes.

## Verification

- `make verify` green on each phase.
- macOS smoke (Jason): open Settings → Agents and confirm both versions match `claude --version` and `codex --version`; press Update on Codex with an npm install and on Claude Code with the native install, watch the pane, confirm the exit overlay and that the row's version changes without pressing Refresh; start a Claude chat and confirm the running-sessions caption counts it; confirm the Update available badge against a deliberately older install and that it clears after the update.
- Windows smoke: with a Codex chat running, confirm Update is disabled with the count caption; stop it and confirm the update runs through the pane and the row updates; confirm nothing changed for a nightly build that never opens the Agents pane.
- Both platforms: confirm a fresh chat after the update reports the new version in `/status` or `--version`, and that the #475 flags are still on the session's argv and env.
