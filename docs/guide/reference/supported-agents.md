# Supported agents

Runner launches separately installed agent CLIs and keeps their native terminal interfaces. Enable agents, inspect detected executables, and choose executable overrides in **Settings → Agents**.

Runner supports agents in two tiers. **First-tier** agents are Claude Code, Codex, Antigravity CLI, Grok Build and pi: Runner is developed and tested against them. **Community-supported** agents are GitHub Copilot CLI, Cursor, OpenCode v2 and TRAE CLI: they run chats and missions, but Runner relies on the people who use them to find problems, so [open an issue](https://github.com/yicheng47/runner/issues/new) when something breaks. Within each tier, features still vary by agent; the sections below say what each one supports.

## Agent settings

**Settings → Agents** groups agents into Installed and Not installed lists. Every row opens that agent’s page; use the **Agents** breadcrumb to return, or click **Open setup** in the header to reopen the first-run setup page in this window. Installed rows show the version, executable, default badge, enable switch and an Update button when available; Not installed rows for npm-packaged agents (Claude Code, Codex, Copilot CLI, pi) show **Install** when `npm` is on `PATH`. Tab and Shift-Tab move between rows and controls; Enter or Space opens a focused row.

The agent’s page has its enable switch and **Set as default** action, detection and update status, **Executable** field with Browse and Reset, and read-only **Defaults** for model and effort. Leave Executable on Auto to use discovery. A chosen path applies to new sessions using the agent’s default command; roles with a custom command keep it. A missing agent’s page includes its install instructions and Browse.

### Launch options

Every agent page has a **Launch** card and a read-only **Command** preview, including a **Custom flags** field shared by chats and missions. Choose **Chat** or **Mission** to see the command Runner composes, with placeholders for the model, effort and first message. **Copy** copies the displayed command, briefly shows a checkmark and **Copied** before returning to **Copy**, and stays disabled when no command preview is available. Claude Code also shows the active account’s `CLAUDE_CONFIG_DIR` when the launch sets it; native-account launches do not add it.

| Option | Agents | Default | Applies to |
| --- | --- | --- | --- |
| Bypass approvals and sandbox | Codex | Off | Chats |
| Bypass permissions | Claude Code | Off | Chats, every account |
| Bypass permissions | Antigravity CLI | Off | Chats |
| Remote plugin catalog | Codex | On | Chats, resumes, missions and forks |
| Status hooks | Claude Code, pi, Copilot CLI, OpenCode v2; Grok on macOS/Linux | On | Chats |
| Custom flags | All agents | Empty | Chats, roles, missions, resumes and forks |

Bypass lets a chat run without asking for permission; Codex also runs outside its sandbox. Enable it only for trusted folders. The Bypass switch applies to every Codex, Claude Code and Antigravity CLI chat, including role-backed chats. Chat starts, resumes and forks strip stored role permission flags; with Bypass off, the agent’s own configuration governs approvals. Missions always use their broad-access permissions and status hooks (where supported), regardless of the chat switches.

Turn **Remote plugin catalog** off to skip Codex’s catalog download on each launch. Catalog plugins then do not load. Runner adds `-c features.remote_plugin=false` for that process and leaves Codex’s config file untouched.

With **Status hooks** off, Claude Code (2.1.295+) and pi (1.1.0+) report status, waits, failures and unread completion badges through OSC 7501 terminal status; older builds without OSC 7501 and other agents use terminal activity. Activity-only agents lose detailed wait and outcome reports. Conversation identity hooks stay enabled so Resume still opens the same conversation. Codex (0.164.0+) and Antigravity CLI use OSC 7501 terminal status natively without status hooks (Codex keeps its `SessionStart` and `UserPromptSubmit` identity hooks plus transcript abort/error signals; Antigravity’s chat-title callback is separate), Cursor has only an identity hook, and TRAE has no hooks, so none of the four has this switch.

**Custom flags** adds extra CLI arguments to every session launch of that agent, across plain chats, role chats, missions, resumes and forks, from the next session start. Runner’s own helper commands (such as version and help checks, model discovery, usage reads and headless fork steps) do not receive custom flags. Press Enter or leave the field to save, or press Escape to restore the saved value; clearing the field removes the flags.

Runner splits **Custom flags** into shell words on every platform without expanding `~` or environment variables: spaces separate flags, pasted newlines count as spaces, and single quotes, double quotes or backslashes keep values with spaces together. On save, Runner checks three things in order and shows the first problem only: unclosed quotes or a trailing backslash are refused without saving; flags Runner already manages for that agent (including permission flags and Codex’s reserved `-c`/`--config` keys, while allowing `-c` with other keys and repeated `--add-dir`) are refused without saving; and flag names not listed by `<command> --help` are saved with an amber warning (re-checked when opening the Launch card, and skipped if help output cannot be read). Custom flags sit right after any prepended resume subcommand and before role arguments, so role arguments still win if a CLI takes the last value of a flag. Changes apply when a session next starts.

## Install and sign in

Use each provider's setup instructions, then run its CLI in a normal terminal to finish authentication before starting it in Runner.

| Agent | Runtime name for the CLI | Setup and account notes |
| --- | --- | --- |
| Codex | `codex` | [Codex CLI setup](https://developers.openai.com/codex/cli); sign in through Codex |
| Claude Code | `claude-code` | [Claude Code setup](https://code.claude.com/docs/en/setup); sign in through Claude Code |
| Antigravity CLI | `antigravity` | [Antigravity CLI reference](https://antigravity.google/docs/cli/reference); Google account; updates itself at startup |
| pi | `pi` | [pi setup](https://github.com/earendil-works/pi); configure your model provider |
| GitHub Copilot CLI | `copilot` | [Copilot CLI setup](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli); Copilot subscription |
| Cursor Agent CLI | `cursor` | [Cursor CLI setup](https://cursor.com/docs/cli/installation); install the CLI separately from the editor |
| Grok Build | `grok` | [Grok Build setup](https://docs.x.ai/build/overview); existing grok.com sign-in; updates itself |
| OpenCode v2 | `opencode` | [OpenCode v2 docs](https://opencode.ai/v2/docs); configure an available provider and model |
| TRAE CLI | `trae` | Opt-in runtime; enable it in Settings → Agents after installing and configuring its CLI |

All listed runtimes except TRAE are enabled by default when detected. The runtime name is Runner's identifier; it need not be the executable name. For example, Runner's Claude Code runtime is `claude-code`, while its executable is `claude`.

## Capabilities

“Yes” describes an implemented integration. Platform notes below distinguish implementation from native validation. “Catalog” means Runner can display and edit skill documents but has no global skill switch for that runtime.

| Capability | Codex | Claude Code | Antigravity CLI | pi | Copilot CLI | Cursor | Grok |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Chats, missions and saved-conversation resume | Yes | Yes | Yes | Yes | Yes | Yes | macOS checked; relaunch gap |
| Native conversation switches update the resume key | Yes | Yes | Yes | Yes | No | macOS/Linux integration | macOS/Linux integration |
| Windows | Yes | Yes | Unverified | Unverified | Yes | Unverified | Unverified |
| Fork a chat | Yes | Yes | No | Yes | No | No | Yes |
| Working/idle from agent hooks | OSC 7501 (0.164.0+) | Yes | OSC 7501 (1.5.8+) | Yes | Yes | Terminal activity | macOS/Linux integration |
| OSC 7501 program status | 0.164.0+ | 2.1.295+ | 1.5.8+ | 1.1.0+ | Not claimed | Not claimed | Not claimed |
| Needs-input reporting | Approvals, questions and auth via OSC 7501 (0.164.0+) | Approvals and questions | Approvals, questions and auth via OSC 7501 (1.5.8+) | Extension reports only | Approvals and questions | No | Approvals on macOS/Linux |
| Models discovered from the CLI | Yes | Yes | Yes | Yes | No | Yes | Yes |
| Update through Settings → Agents | Yes | Yes | No | Yes | Yes | No | No |
| Usage indicator | Yes | Yes | 1.5.8+ (macOS checked) | No | No | No | No |
| Skills | Catalog and switch | Catalog and switch | Catalog | Catalog | Catalog and switch | Catalog | Catalog |
| Runner skill installed | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| Mission access | Bypass | Bypass | Bypass | Trusted workspace | Bypass | Bypass | Bypass |
| Terminal rendering covered by fixtures | Yes | Yes | Yes | No | No | No | Yes |

**Terminal program status** uses [OSC 7501](https://www.superlogical.com/rex/docs/build/program-status). Antigravity CLI 1.5.8+, Claude Code 2.1.295+ and pi 1.1.0+ detect support through a probe; Codex 0.164.0+ reports unconditionally. Starting these agents by hand in a terminal tab can show Working, Idle and Needs you through the existing sidebar and header indicators. Blocked messages appear in the status detail when the header has room. Codex and Antigravity CLI chats and missions use OSC 7501 as their primary session status, including completed-turn responses, failures, and approval, question and authentication waits; sessions without OSC 7501 fall back to terminal activity. In hooked agent chats, a live hook bridge keeps status authority so stream reports cannot replace a newer hook state, and passive terminal-tab status never creates an unread response on its own. When **Status hooks** is turned off on the Launch card for Claude Code or pi chats, the identity hook remains and OSC 7501 becomes authoritative for Working, waits, failures and unread completion badges on `done`; agents that never report keep terminal-activity fallback.

**Codex status and input attention** reads Codex’s native OSC 7501 reports through the shared program-status parser for both chats and missions, without installing Codex status hooks. In Codex CLI 0.164.0+ (verified through 0.170.0), `working` covers turns, tool calls and compaction, `blocked` carries `kind=permission` (**Approval needed**), `kind=question` (**Answer needed**) or `kind=auth` (generic **Needs you**) and holds crew delivery while open, `done` records completed-turn unread badges when unfocused, `error` reports **Response failed**, and `idle` clears the turn. Runner keeps only Codex’s `SessionStart` and `UserPromptSubmit` identity hooks (for conversation keys, transcript paths and mission turn-start confirmation) and reads Codex’s transcript for **Interrupted** (`turn_aborted` with reason `interrupted`) and API-error (`task_complete` with `error`) outcomes. Customized Codex sessions without usable native reports fall back to terminal activity and have no claimed dialog-delivery protection.

**Antigravity CLI** groups effort-suffixed models from `agy models` into a single base model entry (such as `gemini-4-hydrogen` or `claude-opus-5-5`) paired with the separate **Effort** selector for that model's supported levels (`low`, `medium`, `high`, `xhigh`, and `max` where supported). Because Runner only passes `--effort` together with `--model`, **Effort** is disabled when there is no explicit or inherited launch model (a blank **Model** override in a chat or Crew slot can still inherit the role's **Model**), and selecting a model without effort levels or entering a legacy effort-suffixed model ID (such as `gemini-4-hydrogen-xhigh` or `gpt-oss-120b-medium`) disables **Effort** and omits conflicting `--effort` flags.

**OpenCode v2** supports chats, roles, crews, exact session identities, dynamic models and model variants, hook status, skills catalogs, MCP settings and the Runner skill. It has no chat fork, usage indicator, global skill toggle or update control. Backend/native-data checks cover replies, coordination and stop/resume; complete GUI and Windows checks remain unverified. Mission access uses its automatic permission mode.

**TRAE CLI** remains opt-in. It has a skills catalog, MCP configuration and mission permission support. Do not assume feature or platform parity with the primary runtimes.

## Platform and resume notes

- Runner ships for macOS (Apple Silicon and Intel) and Windows x64. References to Linux above describe a runtime adapter's behavior; the Linux beta's install and validation scope is in [Install Runner](../getting-started/install.md).
- Windows agents run natively, without WSL. Claude Code and pi's bash tool need Git for Windows. npm-installed agents need Node.js; Copilot's npm installation requires Node.js 22. PowerShell 7 is optional.
- Antigravity CLI and pi have Windows implementation paths but no completed native Windows smoke check. Cursor, Grok and OpenCode v2 also have Windows validation gaps.
- Grok chats, missions and stop/resume were checked on macOS; full app-relaunch persistence remains unverified. Switching an already loaded Grok dashboard conversation updates Runner's resume key on the next prompt, not an idle dashboard switch.
- A resume key identifies the runtime's conversation. The runtime must still have that conversation history. Working/idle and needs-input indicators depend on the runtime's reporting and do not certify completion of a task.

See [Sessions](../concepts/sessions.md) for lifecycle behavior and [Troubleshooting](../guides/troubleshooting.md) for detection and resume problems.

## Minimum versions and agent updates

Codex needs **0.164.0 or newer** for OSC 7501 `done`, `error` and `kind` reporting alongside Runner's identity hooks. The launcher marks only an older Codex’s agent row **Update needed**; role rows have no update labels or actions. The notice explains that starting Codex or a role using it updates Codex first and leaves the pane waiting; the chat starts when the update succeeds. A refused resume or fork offers the same recovery. An unknown version is allowed to try launching.

OpenCode v2 needs **2.0.0 or newer** for Runner's `--standalone` server, model discovery and session history. When an older `opencode` (`0.15.x` or `1.x`) is found on `PATH`, Runner keeps **OpenCode v2** under **Not installed** in **Settings → Agents**, explains which version was found, and omits it from the launcher's installed agents. Explicit overrides below 2.0.0 are rejected, and refused launches or resumes offer **Open Settings** instead of an update task because OpenCode v2 has no Runner-managed update command.

On macOS and Linux, Antigravity CLI usage checks require **1.5.8 or newer** so Runner can verify authentication without triggering interactive login before reading `/usage`. Older or unrecognized builds remain usable for chats and missions, while keeping Antigravity CLI usage unavailable without running sign-in checks. On Windows, Runner reads Antigravity's saved sign-in directly and queries quota endpoints over HTTPS without spawning `agy`.

When a supported agent has a newer version, only its agent row shows the installed and available versions with **Update**; its roles keep their normal rows. You can still start chats on the supported installed version. A dot on the sidebar's Settings gear opens **Settings → Agents** and covers enabled agents with an update command, including those without a usage meter.

Manual installs and updates run as [background tasks](../guides/chats-and-windows.md#background-tasks). When `npm` is available on `PATH`, Claude Code, Codex, GitHub Copilot CLI and pi can be installed via `npm install -g` from Setup or **Settings → Agents**; runtimes without an npm package keep manual Copy and guide links. Starting the same task again joins the existing task. Running chats keep their current executable version until they restart. These controls install or update agent CLIs; Runner's own updates remain in **Settings → Updates**. Automatic agent updates are separate work.
