# Roles

A role is a saved agent configuration that you can reuse in chats and crews. It describes how an agent should work; starting it creates a separate session.

## What a role stores

| Setting | Purpose |
| --- | --- |
| Name and handle | A readable name and a unique identifier such as `coder` |
| Agent runtime | The CLI to run, such as Codex or Claude Code |
| Model and effort | Optional choices supported by that runtime; otherwise its defaults apply |
| Speed | Codex's inherited, standard or fast service tier |
| System prompt | The role's responsibilities and working style |
| Working directory | A default folder when the launch does not supply one |
| Command and environment | How the agent process is launched |

Create and edit roles from **Roles**. A role's system prompt should describe its job, such as “review the diff and report defects without editing files.” Put a particular task in a chat prompt or mission goal so the role remains reusable.

The list starts with 10 roles per page. Use **10 / page** in the footer to choose 10, 25 or 50; Runner remembers this separately from the Crews list. Changing the size keeps your search and returns to page 1. Longer pages scroll while the column headings and footer stay visible.

## Roles in chats and crews

Search for a saved role in the chat launcher and click it or press Enter to start with its saved runtime, model, effort, speed and instructions. Agent rows start without a role and use the agent’s own configuration. A role’s **Chat** action starts it directly and updates the same per-computer recent list. Change saved role settings on its role page; chats start unnamed and can be renamed from the pane menu.

A [crew slot](crews.md) references a role and gives it an in-crew handle. The same role can fill several slots, each with a different handle. A slot can override the runtime, model, effort and Codex speed. Changing the runtime resets engine-specific settings to the new runtime's defaults before its overrides are applied.

Editing a role does not type instructions into an already-running terminal. Use a message for an immediate instruction, and review the launch settings when starting new work.

See [Build a crew](../guides/build-a-crew.md) for a complete coder/reviewer example and [CLI reference](../reference/cli.md#runner-role) for role commands.
