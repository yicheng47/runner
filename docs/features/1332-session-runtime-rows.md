# Runtime, model and effort on mission session rows

Tracking issue: [#1332](https://github.com/yicheng47/runner/issues/1332). Status: designed, signed off by Jason on 2026-11-21. Design: `runner/desktop/specs/1332-mission-session-runtime.pen`, board `qz5R1`, chosen card `RJJ6r` (option A1).

## Motivation

The Sessions rail in a mission's side panel shows each slot's avatar, handle, status and `session_key`, but not what the slot is running. A crew slot can override the runtime, model and effort (`RosterSlot.runtime_override`, `model_override`, `effort_override`), and `runner mission add-role` can add a role on another runtime, such as a codex `reviewer` on `gpt-6-astra` at `xhigh` inside a claude crew. Today the only way to tell what `@reviewer` runs is to open its terminal or read the crew's configuration.

## Behavior

### The card

Each session card in the rail (`render_roles_rail` in `crates/runner-app/src/surfaces/mission_workspace/rail.rs`) changes as follows, top to bottom:

1. **Header row.** The pixel avatar (`RoleAvatar`, unchanged at 25 px) sits on the left. Beside it, the handle and the status are stacked: the handle in its seeded colour above, and the status indicator (icon and label, as today) below. The Stop/Restart/Resume controls and the remove button stay on the right, vertically centred on the header row. The status leaves its own line below the header.
2. **Runtime line.** This is a new line across the card's full inner width: the runtime's provider mark at 12 px (`runtime_mark` / `ChatIcon::for_runtime`, the same mark the sidebar and the chat Info panel use), then the text `Runtime · model · effort` in the meta size and muted colour, such as `Codex · gpt-6-astra · Extra high`. The line is a single row that truncates with an ellipsis at its end. A tooltip on it gives the full text.
3. **Divider and `session_key`.** These are unchanged, and they stay hidden on remote windows as today.

The card grows from 94 px to 108 px. In two-up mode, when an open file widens the rail, the card keeps the same structure, and the runtime line truncates sooner.

### Values

- **Runtime** is the runtime's display name (Claude Code, Codex, pi, Antigravity, OpenCode, Grok, Copilot), taken from the same runtime UI table the sidebar uses.
- **Model** is the session's effective model, shown as the raw id (`opus`, `gpt-6-astra`), as the chat Info panel shows it.
- **Effort** is the session's effective effort, shown through the existing localized label (`effort_label`: Low, Medium, High, Extra high, Max, …).
- The effective values come from what the session was launched with: its launch definition's `model` and `effort`. That launch definition already folds the slot override, the role's defaults and the runtime's defaults together, as `DirectSessionEntry.agent_model` and `agent_effort` do for chats.
- A value that is not known is left out along with its separator, and is never shown as a placeholder or `default`. A pi slot with no effort reads `pi · deepseek-chat`, and a slot whose model was never set reads just `Codex`.
- A shell slot has no runtime line, just as it has no status today.

### Protocol

`SessionRow` (`crates/runner-core/src/protocol/session.rs`) gains `agent_model: Option<String>` and `agent_effort: Option<String>`, both `#[serde(default)]`. `runnerd` fills them from the session's launch definition where it builds `MissionSessionRow` (`crates/runner-daemon/src/repo/session.rs`). Because this adds protocol fields, the PR is not `frontend-only`.

Remote windows receive `SessionRow` through the companion snapshot. A remote window connected to a host that sends the new fields shows the runtime line. With an older host, the fields are absent, and the card shows the runtime name alone, which the row already carries.

## Guide pages

- `docs/guide/concepts/missions.md` and `docs/guide/zh-CN/concepts/missions.md`: one sentence on what a Sessions card shows, including the runtime line.

## Live checks

These are required because the change alters what a person sees in the mission rail.

1. **Mixed-runtime mission.** Start a small claude crew mission in a fresh `/tmp` folder, then `runner mission add-role` a codex role with a model and effort override. Each card shows the header with handle over status, and a runtime line with the right mark and values. The added role's line shows its override, not the crew's defaults.
2. **Unknown values omitted.** A slot without an effort (for example pi, or a role left on defaults) shows no trailing separator or placeholder.
3. **Status still live.** Status changes (Working → Idle, Stopped after Stop) update beside the avatar, and the controls stay aligned on the header row.
4. **Narrow and two-up.** At the rail's minimum width, a long handle and a long runtime line truncate with an ellipsis without overlapping the controls. With a file open (two cards per row), the cards keep the same structure.
5. **Remote window.** On the [same-Mac two-host relay fixture](../tests/full-smoke-test.md#same-mac-two-host-relay-fixture-macos), the remote window's Sessions cards show the runtime line, and `session_key` stays hidden.

## Non-goals

- Changing a slot's runtime, model or effort from the card.
- Showing the runtime anywhere else, such as the member tabs or the feed.
- Replacing the pixel avatar with the provider mark (option C, rejected).
