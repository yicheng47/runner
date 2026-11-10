# 1246 — Agent launch options

Tracking issue: [#1246](https://github.com/yicheng47/runner/issues/1246). It absorbs the status-hooks switch from [#1051](https://github.com/yicheng47/runner/issues/1051), which keeps only the OSC 7501 comparison.

Design: `runner/desktop/specs/1246-agent-launch-options.pen` in `yeelinks_design`, signed off on 2026-11-10. Frames: `VvFIf` Codex · Launch (Chat command), `LoBEK` Codex · Launch (Mission command), `LOEzL` Claude Code · Launch (Chat command), `LJcuZ` pi · Launch (Chat command), and `dkDAX`, the table of switches per runtime. The frames win on layout and copy; this spec wins on behavior.

## Motivation

People keep adding launch flags to Runner's agents by hand, and nothing shows what Runner actually runs.

- **Codex remote plugin catalog.** Every Codex start downloads the full remote plugin catalog from `chatgpt.com`, about 24–30 MB, because startup forces a refresh past the 3-hour disk cache ([openai/codex#47483](https://github.com/openai/codex/issues/47483), still present in `rust-v0.166.0`). Every chat start, resume, mission slot and headless fork pays it. `-c features.remote_plugin=false` skips it; the usage probe already passes `features.plugins=false`.
- **Bypass for chats.** Missions always launch in Bypass, but a chat starts in the runtime's default mode and asks for every approval.
- **Status hooks.** Hooks write into each agent's configuration and need an IPC path to `runnerd`. In a chat they only report status, which recent Codex, Claude Code and pi versions also report through the terminal (OSC 7501). People who don't want hooks have no way to turn them off (#1051).
- **No view of the command.** The role page used to show permission flags as a command (#1253); the full launch command, with model, hooks and mission additions, was never visible anywhere.

## Scope

### Launch section

Each agent's page in Settings → Agents gets a **Launch** card below Defaults (for Claude Code, between Defaults and Accounts). It holds the agent's switches, a divider, and a **Command** row: Chat/Mission tabs, a Copy button, and a read-only command block. A grey caption sits under the card.

Switches per runtime, from the `dkDAX` table:

| Agent | Bypass (chats) | Remote plugin catalog | Status hooks (chats) |
|---|---|---|---|
| Codex | Yes | Yes | Yes |
| Claude Code | Yes | — | Yes |
| pi | — | — | Yes |
| Copilot CLI | — | — | Yes |
| OpenCode | — | — | Yes |
| Grok | — | — | macOS, Linux |
| Antigravity | — | — | macOS |
| Cursor | — | — | No switch |
| TRAE | — | — | No switch |

Every agent page shows the Launch card with its Command row, even with no switches. The scope label beside each switch says where it applies: **Chats** or **Chats and missions**.

The card caption on every agent page is "Missions always start in Bypass. Changes take effect when a session next starts." (Jason, 2026-11-10). It replaces the stale role-arguments caption in the frames. The `dkDAX` table is a spec frame, not a screen; its note about each role's permission picker is likewise stale.

### Switch behavior

- **Bypass approvals and sandbox** (Codex), off by default. On: a chat launches with `--ask-for-approval never --sandbox danger-full-access`, the same flags as Codex's `mission_bypass`.
- **Bypass permissions** (Claude Code), off by default. On: a chat launches with `--permission-mode bypassPermissions`, and the `--settings` payload carries `skipDangerousModePermissionPrompt: true` as it does for a Bypass role today.
- Bypass applies to every Codex and Claude Code chat, plain or role-backed. Direct starts, resumes and forks always strip the role’s stored permission flags, preserving the attended-chat policy from 328545ff; the switch then supplies Bypass when enabled (Jason, 2026-11-10). With the switch off, the runtime’s own configuration governs approvals. Missions are unchanged and always launch in Bypass.
- **Remote plugin catalog** (Codex), on by default. Off: every Codex launch Runner makes adds `-c features.remote_plugin=false`: chats, resumes, mission slots and the headless `codex exec fork`. Runner does not write `~/.codex/config.toml`.
- **Status hooks**, on by default. Off: a chat launches without its status hooks and takes status from the terminal, as the table's last column in `dkDAX` describes. The identity hook stays, so Resume keeps working: for Claude Code and pi the SessionStart rekey after `/clear` and `/resume`, and for Codex the hooks that carry its session id. Missions always install hooks, because crew messages are delivered through them. With hooks off, a chat's startup delivery uses the existing output-settle gate.
- A runtime's own `--disable hooks` / `features.hooks=false` in a role's args still wins, as `inject_codex_hooks` does today.
- Changes apply when a session next starts; running sessions keep their flags.

### Command preview

The Command block shows the launch the daemon would build for a new chat or a new mission slot of that runtime, line by line, with placeholders such as `<model>`, `<effort>`, `<first message>` and `<mission folder>`. Each line carries a grey note: `always`, `from the chat`, `from the role`, `first turn`, or the switch that adds it, in accent color when a switch drives it. Claude Code's preview includes the active account's `CLAUDE_CONFIG_DIR` only when a new launch would set it; the native account does not add that variable. The preview is built by the daemon from the same code that composes real launches, so it cannot drift from what runs. Copy copies the command as shown.

### Storage and protocol

- The switches live in the daemon's `_app_state` table under one key, a JSON map by runtime, beside `runtime_overrides`. No migration.
- The client protocol gets a request to set one runtime's launch option and a request for the command preview; the runtime status the Agents page already loads carries the current options. The CLI is unchanged.
- Spawn composition in `session/manager/spawn_spec.rs` and the runtime adapters reads the options when building a chat's args.

### Guide

Update `docs/guide/reference/supported-agents.md` (launch options per runtime), `docs/guide/concepts/missions.md` (Permissions: missions ignore the chat switches), and `docs/guide/guides/troubleshooting.md` if it covers slow Codex starts or status. Update `docs/arch/runtime-integration.md` for the hooks-off status path.

## Non-goals

- Bypass switches for runtimes other than Codex and Claude Code.
- Changing what a role's stored Codex Bypass means (`workspace-write`); see Open questions.
- The OSC 7501 vs hooks comparison and any shrinking of mission hooks (#1051).
- Editing agents' own configuration files.

## Open questions

- Whether a role's stored Codex Bypass should move from `--sandbox workspace-write` to `danger-full-access`, matching missions and the chat switch.

## Verification

Automated: spawn-spec tests for each switch on and off, per runtime, for chats and missions (mission args unchanged by Bypass and hooks switches; plugin catalog flag on chats, missions and fork); stored role permission modes are stripped with Bypass off, and Bypass applies to role-backed chats too; hooks off keeps the identity hook; option persistence round-trip; the preview matches the composed args for a fixed input.

Live checks in the development app, because the change adds a settings card and alters how chats launch:

1. **Launch card:** the Codex, Claude Code, pi and Cursor pages show the card with the switches the table names, Chat/Mission tabs switch the preview, and Copy copies it.
2. **Codex catalog off:** a new Codex chat's process arguments include `-c features.remote_plugin=false`, and the preview shows the line.
3. **Codex Bypass on:** a new plain Codex chat runs a shell command without an approval prompt; a mission slot's flags are unchanged.
4. **Claude Code Bypass on:** a new plain Claude Code chat shows no permission prompt for an edit.
5. **Status hooks off:** a new Claude Code or Codex chat still shows Working and Idle in the sidebar, and Resume after stopping it reopens the same conversation.
