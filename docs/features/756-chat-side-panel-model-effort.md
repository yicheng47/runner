# 756 — Show model and effort in direct chat side panel meta info

> Tracking issue: [#756](https://github.com/yicheng47/runner/issues/756)
> Priority: P2. Platforms: macOS and Windows.

## Motivation

When viewing a direct chat (either role-backed or runtime-direct), the right-hand side panel (`render_chat_side_panel` in `crates/runner-app/src/surfaces/panes.rs`) displays a metadata card with `cmd`, `cwd`, and `session_key`.

However, it does not show the model or thinking effort that the chat is running with. For role-backed chats, model and effort may come from the role template or an override; for direct runtime chats, they are chosen in the Start Chat modal and stored on the session row (`agent_model`, `agent_effort`). Showing model and effort in the side panel, in a setup block like the role page's, makes the active execution settings visible without waiting for the rest of the side panel redesign in #731.

## Design

`design/specs/756-chat-side-panel-model-effort.pen`: the role chat with the panel open (`Zyp5L`), and the role chat and runtime chat panels side by side (`wdEC4`).

## Scope

- **Backend**: Include `agent_model` and `agent_effort` on `DirectSessionEntry` (resolving from session row `agent_model` / `agent_effort` or role defaults).
- **Identity**: the card leads with a 40 px identity. A role chat shows the role's `RoleAvatar` seeded with its handle, the display name and `@handle` in mono. A runtime chat shows the provider mark on a raised tile, as the Start Chat modal's `runtime_mark` draws it, and the runtime's name. The uppercase runtime text badge goes.
- **Setup**: under the identity, laid out like the role page's setup rows (label over value). A role chat shows **Runtime** (the provider mark and name), then **Model** and **Effort** side by side. A runtime chat shows only Model and Effort, since its identity already names the runtime.
- An unset model or effort reads `default` in the faint UI font, as the role page does through `role_setting_label`; set values are mono.
- `cmd`, `cwd` and `session_key` with its copy button stay under the divider with the same layout. Command and cwd come from the chat entry so they describe what the chat runs; only legacy rows without a cwd fall back to the role's working directory. The system prompt section is unchanged.
- Support both role-backed and runtime-only direct chats.

## Non-goals

- The rest of #731: the markdown system prompt clamped with Show all, and an Open role link. This change takes #731's identity and setup rows.
- Editing model or effort from the side panel.

## Implementation phases

1. **Backend**: Extend `DirectSessionRow` / `DirectSessionEntry` in `crates/runner-backend` to expose `agent_model` and `agent_effort`.
2. **Frontend**: Update `render_chat_side_panel` in `crates/runner-app/src/surfaces/panes.rs` to render the identity and setup block above the metadata rows.

## Verification

- `runner-app` and `runner-backend` tests pass; workspace clippy is clean.
- Manual pass on macOS and Windows:
  - Role chat with model and effort configured shows its avatar, the runtime, and both values.
  - Direct runtime chat with model and effort configured shows the provider mark and both values.
  - Chat with default/unspecified model and effort shows `default` for each.
  - The panel dragged to its narrowest width (200 px) keeps the identity and setup readable.
