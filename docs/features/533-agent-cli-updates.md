# Update agent CLIs from Settings → Agents

Tracking issue: [#533](https://github.com/yicheng47/runner/issues/533). Status: planned; Pencil frames drawn 2026-09-23, waiting for sign-off. Priority P2.

Related: [706](./706-agent-usage.md). Its usage popover does not update anything and shows no versions (Jason, 2026-09-23). While any agent has an update available, the popover's Agent settings gear carries an accent dot, and the gear already opens Settings → Agents. The sidebar Settings row gets no dot.

Design: `design/specs/533-agent-cli-updates.pen`: `533 — Settings → Agents with updates` (`n1krgH`), `533 — Agent row update states` (`gFO05`), `533 — Usage popover: update dot on Agent settings` (`x4g8D1`) `533 — Update modal over Settings → Agents` (`efOIW`), `533 — Update modal states` (`IbVDl`), and the components `cmp/AgentUpdateDialog` (`VRlqP`) and `cmp/UsagePopover` (`h3doV`).

## Motivation

[#475](./archive/475-codex-auto-update.md) (0.8.2) made agent launches quiet. Codex gets `-c check_for_update_on_startup=false` from `trailing_runtime_args` (`crates/runner-backend/src/router/runtime.rs:602`), and Claude Code gets `DISABLE_INSTALLATION_CHECKS=1` and `CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY=1` from `base_spawn_spec` (`crates/runner-backend/src/session/manager/spawn.rs:410`). That was the right call: Codex's startup prompt installed the update and exited, leaving a direct chat on **Chat paused** and, inside a mission, sometimes taking sibling slots down with it. But the prompt was also the only place a Runner user learned that a CLI was stale. #475 recorded the trade explicitly — "CLI updates remain outside Runner" — and this spec is the other half of it.

The gap is uneven across the two CLIs. Codex has no background updater at all: with the startup check off, anyone who runs Codex only through Runner never updates. Claude Code's native installer keeps updating itself in the background (Runner does not set `DISABLE_AUTOUPDATER`), but npm and Homebrew installs sit still, and even a native install that updated underneath a running session stays on the old version until the session relaunches. Nothing in Runner shows which version a session is actually running, so there is no way to tell the two situations apart.

Settings → Agents already has everything the update needs. `runtime_status::status_list` (`crates/runner-backend/src/runtime_status.rs:85`) resolves each agent's effective executable, auto-detected on the login-shell PATH or overridden; the login-shell capture carries the proxy quartet the update download will need; and the app spawns real PTYs. Both CLIs ship their own updater as a subcommand — `claude update` ("Check for updates and install if available") and `codex update` ("Update Codex to the latest version") — and each one picks the right method for its own install: the native version store under `~/.local/share/claude/versions`, `npm install -g`, or `brew upgrade`. Runner never has to know how the CLI was installed. Updating is one more command through the spawn path, shown in a terminal inside a modal so the CLI's own progress, prompts, and failures stay visible.

## Behavior

### Version in the row

Each agent row in Settings → Agents (`crates/runner-app/src/surfaces/settings/agents.rs`) gains the installed version in its caption, beside the path it already shows: `2.1.266 · Detected: /Users/jason/.local/bin/claude`. The version comes from `<effective command> --version`, run off the UI thread with a five-second timeout at three moments: when background discovery completes (`apply_discovery_result`, `runtime_status.rs:359`), on **Refresh**, and when an update exits. The parse takes the first semver-looking token in the output — `2.1.266 (Claude Code)` and `codex-cli 0.153.4` both yield one — and a row whose command prints nothing parseable shows no version and no error. Rows in the **Not found** state never probe.

The probe runs with the same environment a session would get (login-shell PATH, proxy quartet, runner env not included), so the version shown is the version a new session would launch. It is a plain child process, not a PTY, and it never appears in the sidebar.

### The Update button

A Claude Code, Codex or Copilot row shows an **Update** button next to the enabled toggle only while the npm check below finds a newer version, and the caption then reads `0.153.4 → 0.155.0 · Detected: …`. There is no separate badge and no version on the button (Jason, 2026-09-23: a badge beside a button said the same thing twice). An up-to-date row, or one whose check failed, has no button; `codex update` in any terminal still works. TRAE has no update subcommand and never gets a button. The runtime definition names the update arguments (`update` for all three) and the npm package; a runtime without them has no button, which is the extension point if a fourth runtime arrives.

Pressing Update opens a modal over Settings (`cmp/AgentUpdateDialog`) with a terminal inside it, the same terminal element the panes render, whose PTY child is the update command itself: the effective executable with `update` as its only argument, the agent spawn environment from `base_spawn_spec` minus the runner-specific layers (no runner env, no `--settings`, no mission vars), cwd set to the home directory. The PTY is not a session: it is an unlisted process in the session manager, with no `sessions` row, that reports its output and exit only to the modal, so it never appears in the sidebar, a tab, the archive, relaunch, the CLI's session lists or the MCP. Its header reads `Updating Codex` with `0.153.4 → 0.155.0 · codex update` under it (Jason, 2026-09-23: opening a pane in the chat surface for this was weird).

While the process runs the modal has no button and cannot be dismissed: Esc and a scrim click do nothing, because stopping npm or brew halfway can leave a broken install. The footer says it is running, or "Waiting for input in the terminal." in the warning tone when the CLI is reading from the PTY. Runner cannot see a read on the PTY, so it shows this after two seconds of silence with the cursor left after a prompt rather than at the start of a line.

The CLI does the rest. `claude update` prints its progress and either "already up to date" or the new version; `codex update` spawns npm or brew, both of which print what they are doing. Prompts — Homebrew's, npm's, a sudo password on a global npm prefix, Windows asking to close a program — render in the PTY as they would in any terminal, and the user answers them there. When the process exits, the Agents pane re-runs the version probe for that runtime and the modal's footer shows the outcome: on exit code 0, "Codex is now 0.155.0." with a **Done** button (plus "Running sessions keep 0.153.4 until they relaunch." when any are alive); on a non-zero exit, "Exited with code 243." in the danger tone with a **Close** button, and the output stays for the user to read. The row reflects the outcome without a manual Refresh.

Runner does not parse the terminal's output. Whether the update succeeded is what the CLI printed plus what `--version` now says.

### Running sessions

Updating a CLI that is currently running is safe on macOS and not on Windows, and the guard follows the platform rather than pretending otherwise:

- **macOS.** Update is always enabled. Unlinking a mapped executable is fine on Unix, and Claude Code's native installer keeps the old version's directory anyway. Running sessions keep the version they started with, so when any session of that runtime is alive the caption says so: "3 running Claude Code sessions keep 2.1.266 until they relaunch." Idle or busy makes no difference; alive is what matters.
- **Windows.** Update is disabled while any session of that runtime is alive — `claude.exe`, or the native binary inside the npm package for Codex, is in use and the installer would fail or half-apply. The caption says "Stop the 3 running Codex sessions first." Counting is by runtime across direct chats, mission slots, drawers, and forks; a running mission with Codex slots blocks a Codex update.

The count is the live process count from the session manager, the same source `session_activity_snapshot` (`crates/runner-backend/src/ops/session.rs:80`) reads, and the caption updates as sessions start and stop while Settings is open.

### Update available

Knowing that an update exists is what the suppressed startup prompt used to provide, so the row provides it instead. When the Agents pane opens, and on Refresh, Runner fetches the `latest` dist-tag for `@anthropic-ai/claude-code`, `@openai/codex` and `@github/copilot` from the npm registry (`https://registry.npmjs.org/<package>/latest`, the `version` field) through the proxy-aware `reqwest` client the usage popover already uses (`usage.rs`) and the captured proxy environment. Claude Code follows its release channel: with `autoUpdatesChannel` set to `stable` in its user settings (`~/.claude/settings.json`, or `$CLAUDE_CONFIG_DIR`), Runner reads the `stable` dist-tag instead of `latest`, since `claude update` on that channel installs `stable` and would otherwise leave a button it never clears. The CLIs publish every release to npm under the same version number they ship natively, so one endpoint covers all install methods; a Homebrew formula that lags npm shows the button a day early, and `brew upgrade` in the pane says "already installed", which is acceptable.

The result is cached per app run for six hours per runtime. A newer version than the installed one shows the Update button and the caption `2.1.266 → 2.1.270 · Detected: …`. No network, a timeout, a non-200, or a malformed body degrades silently to version-only with no button; the row never shows a network error, because the row is about the CLI, not about npm. The check runs when the Agents pane opens and when the usage popover opens, sharing the same cache. When any row would show the button, the popover's Agent settings gear shows an accent dot, and its tooltip reads "Agent settings · Update available". Nothing polls in the background, and the sidebar Settings row never shows a dot.

### What stays the same

The #475 suppression is unchanged. Sessions still get `check_for_update_on_startup=false` and `DISABLE_INSTALLATION_CHECKS=1`; Claude Code's native background updater stays enabled. The point of this spec is that the update signal moves out of the session and into Settings, not that it comes back into the session.

## Decisions

1. **A terminal in a modal, not a captured log and not a pane.** The update commands are interactive in the ways that matter — brew and npm prompt, sudo prompts, Windows complains about files in use — and their output is long. Capturing it into a row caption would be lossy and would need per-CLI parsing that drifts with every release, so the modal hosts a real PTY and shows exactly what the CLI did. A pane in the chat surface was the first design and read as out of place for a Settings action; the modal keeps the update where it was started and needs no session row, sidebar row, or pane placement.
2. **The CLI's own `update`, never a package manager Runner chooses.** `claude update` and `codex update` already detect native versus npm versus Homebrew. Runner calling `npm install -g` itself would guess wrong for native and brew installs and would need Node on PATH. The one thing Runner adds is the environment: PATH and proxies from the login shell, which is also what makes the update find the same executable the sessions use.
3. **npm registry for "latest", one endpoint for both CLIs.** Claude Code's native installer reads a manifest from a GCS bucket and Codex publishes GitHub releases, but both also publish to npm at the same version, and one URL shape with one JSON field is less to maintain than two vendor channels. If a vendor stops mirroring to npm, the button stops appearing and the CLI's own update in a terminal still works.
4. **Platform-split guard.** A universal "close all sessions first" rule would make the button useless on a busy macOS day for no safety gain. Windows needs the rule; macOS needs the caption.
5. **No auto-relaunch of stale sessions.** After an update, running sessions keep the old version until the user relaunches them. Relaunching a chat on the user's behalf would interrupt whatever the agent is doing, which is exactly the interruption #475 removed. The caption names the situation; the human picks the moment.
6. **Priority P2.** Codex users drift stale silently today, which is a real loss, but they can still run `codex update` in any terminal, including a Runner terminal pane. Nothing is blocked; the feature makes the right thing visible and one click away.

## Non-goals

- **Background polling** of the npm registry, or a dot on the sidebar Settings row. The check runs only when the Agents pane or the usage popover opens.
- **Updating from the usage popover.** The popover shows no versions and no Update button; the gear's dot points at Settings → Agents.
- **Installing a missing CLI.** A **Not found** row stays a not-found row with the same caption it has today. Install flows are vendor-specific and change often.
- **Changing an install method**, such as migrating an npm Claude Code to the native installer. `claude update` prints that advice itself when it applies.
- **TRAE.** No update subcommand is known; the row gains the version probe if `traecli --version` prints one and nothing else.
- **An MCP tool** for `runtime_update_start`. Updating is a human action in Settings; agents do not update their own runtime.
- **Restoring the Claude Code startup nudge** by dropping `DISABLE_INSTALLATION_CHECKS`. The sessions stay quiet.
- **Remote hosts** ([510](./archive/510-remote-ssh-session.md)). The version probe and the update modal run on this machine against the local executable; a remote CLI is updated on the remote.

## Implementation Phases

1. **Pencil frames** in `design/specs/533-agent-cli-updates.pen` (listed under Related): the Agents page with version captions and the Update button; the row's up-to-date, update-available, macOS running-sessions and Windows disabled states; the usage popover's gear with and without the dot; the update modal over Settings → Agents and its running, waiting-for-input, succeeded and failed states. Drawn 2026-09-23; signed off before phase 3 starts.
2. **Backend.** The version probe in `runtime_status` (`--version` off-thread with timeout, semver-token parse, stored on `RuntimeExecutableStatus` as `installed_version`); `update_args` on the runtime definition; `runtime_update_spawn_spec(state, runtime)` returning the argv, env and cwd for the modal's PTY from the effective command and the agent spawn environment; a per-runtime live-session count for the guard. Tests: the parser against both CLIs' real output and against garbage; the spawn spec's argv, env and cwd; the guard count across direct, mission, and fork sessions.
3. **App.** The row caption and Update button; the guard captions from the live count, split by platform at compile time like the rest of the platform chrome; the update modal hosting the PTY with its four states and no dismissal while running; the version re-probe when the process exits. Runner-app tests for the presentation mapping (version, button label, captions, disabled state) alongside the existing `RuntimePresentation` tests.
4. **Update available.** The npm fetch through the shared client and proxy env, six-hour per-run cache, the button's visibility and label, silent degradation. Tests for the version comparison, the cache, and the degrade paths with a stubbed fetch.
5. **Docs.** A line beside #475's decision in `docs/arch/arch.md` recording that the update signal lives in Settings; the README index entry; archive this spec when #533 closes.

## Verification

- `make verify` green on each phase.
- macOS smoke (Jason): open Settings → Agents and confirm both versions match `claude --version` and `codex --version`; press Update on Codex with an npm install and on Claude Code with the native install, watch the modal, answer a prompt in it, confirm the result footer and that the row's version changes without pressing Refresh; start a Claude chat and confirm the running-sessions caption counts it; confirm the Update button against a deliberately older install and that it disappears after the update.
- Windows smoke: with a Codex chat running, confirm Update is disabled with the count caption; stop it and confirm the update runs in the modal and the row updates; confirm nothing changed for a nightly build that never opens the Agents pane.
- Both platforms: confirm a fresh chat after the update reports the new version in `/status` or `--version`, and that the #475 flags are still on the session's argv and env.
