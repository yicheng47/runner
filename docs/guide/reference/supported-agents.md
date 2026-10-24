# Supported agents

Runner launches separately installed agent CLIs and keeps their native terminal interfaces. Enable agents, inspect detected executables, and choose executable overrides in **Settings → Agents**.

## Agent settings

**Settings → Agents** groups agents into Installed and Not installed lists. Every row opens that agent’s page; use the **Agents** breadcrumb to return. Installed rows show the version, executable, default badge, enable switch and an Update button when available. Tab and Shift-Tab move between rows and controls; Enter or Space opens a focused row.

The agent’s page has its enable switch and **Set as default** action, detection and update status, **Executable** field with Browse and Reset, and read-only **Defaults** for model and effort. Leave Executable on Auto to use discovery. A chosen path applies to new sessions using the agent’s default command; roles with a custom command keep it. A missing agent’s page includes its install instructions and Browse.

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
| OpenCode v2 | `opencode` | [OpenCode](https://opencode.ai); configure an available provider and model |
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
| Working/idle from agent hooks | Yes | Yes | macOS only | Yes | Yes | Terminal activity | macOS/Linux integration |
| Needs-input reporting | Generic native Needs you (0.163.0) | Approvals and questions | No | Extension reports only | Approvals and questions | No | Approvals on macOS/Linux |
| Models discovered from the CLI | Yes | Yes | Yes | Yes | No | Yes | Yes |
| Update through Settings → Agents | Yes | Yes | No | Yes | Yes | No | No |
| Usage indicator | Yes | Yes | Yes | No | No | No | No |
| Skills | Catalog and switch | Catalog and switch | Catalog | Catalog | Catalog and switch | Catalog | Catalog |
| Runner skill installed | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| Mission access | Bypass | Bypass | Bypass | Trusted workspace | Bypass | Bypass | Bypass |

**Codex input attention** uses the native OSC 7501 reports verified in Codex CLI 0.163.0. A `blocked` report shows generic **Needs you** in the existing session, sidebar, header and background-notification surfaces. Codex does not identify whether the prompt is an approval or a question. Native `working`, `idle` or `clear` resolves this attention; typing alone does not. Hooks still report work, completion, interruption and conversation identity. This also holds crew delivery while a reported native wait is open, and resolving it preserves an independent draft. Older or customized Codex sessions without usable native reports retain their existing status behavior and have no claimed dialog-delivery protection. Native app validation of this signal remains pending on macOS and Windows.

**OpenCode v2** supports chats, roles, crews, exact session identities, dynamic models and model variants, hook status, skills catalogs, MCP settings and the Runner skill. It has no chat fork, usage indicator, global skill toggle or update control. Backend/native-data checks cover replies, coordination and stop/resume; complete GUI and Windows checks remain unverified. Mission access uses its automatic permission mode.

**TRAE CLI** remains opt-in. It has a skills catalog, MCP configuration and mission permission support. Do not assume feature or platform parity with the primary runtimes.

## Platform and resume notes

- Runner ships for Apple Silicon macOS and Windows x64. References to Linux above describe a runtime adapter's behavior, not a Linux build of Runner.
- Windows agents run natively, without WSL. Claude Code and pi's bash tool need Git for Windows. npm-installed agents need Node.js; Copilot's npm installation requires Node.js 22. PowerShell 7 is optional.
- Antigravity CLI and pi have Windows implementation paths but no completed native Windows smoke check. Cursor, Grok and OpenCode also have Windows validation gaps.
- Grok chats, missions and stop/resume were checked on macOS; full app-relaunch persistence remains unverified. Switching an already loaded Grok dashboard conversation updates Runner's resume key on the next prompt, not an idle dashboard switch.
- A resume key identifies the runtime's conversation. The runtime must still have that conversation history. Working/idle and needs-input indicators depend on the runtime's reporting and do not certify completion of a task.

See [Sessions](../concepts/sessions.md) for lifecycle behavior and [Troubleshooting](../guides/troubleshooting.md) for detection and resume problems.
