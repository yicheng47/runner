# Roles

A role is a saved agent configuration that you can reuse in chats and crews. It describes how an agent should work; starting it creates a separate session.

## What a role stores

| Setting | Purpose |
| --- | --- |
| Name and handle | A readable name and a unique identifier such as `coder` |
| Avatar | A saved choice of handle initial or pixel art, with an optional AI-generated image |
| Agent runtime | The CLI to run, such as Codex or Claude Code |
| Model and effort | Optional choices supported by that runtime; otherwise its defaults apply |
| Speed | Codex's inherited, standard or fast service tier |
| System prompt | The role's responsibilities and working style |
| Working directory | A default folder when the launch does not supply one |
| Command, arguments and environment | How the agent process is launched |

Create and edit roles from **Roles**. A role's system prompt should describe its job, such as “review the diff and report defects without editing files.” Put a particular task in a chat prompt or mission goal so the role remains reusable.

## Role avatars

Choose **Initials** or **Pixel art** below the avatar in the new-role or edit-role form. The choice previews immediately and is saved with the role. Initials is the default: the first character of the handle, uppercased, on a colored rounded tile. A new role shows `?` until you enter a handle. Pixel art uses the same deterministic mirrored pattern as earlier Runner versions.

Under **Theme color**, choose one of the eight named colors or **Custom**, then edit the `#RRGGBB` field. Color changes preview immediately and are saved with the role; **Automatic** (or an empty hex field) uses the existing handle-derived color. Invalid hex colors cannot be saved. The color applies to initials, pixel art, image fallbacks and role identity indicators across chats, crews and missions. Existing AI bitmaps are not recolored automatically.

In the new-role or edit-role form, choose **Generate with AI** to create an illustrated avatar from the role's name, handle and system prompt. This runs your locally installed, signed-in **Codex CLI**, including for roles that use another runtime. Generation requires Codex image-generation access and may consume your Codex usage; Runner does not ask for a separate image API key.

**Generate with AI** and **Regenerate** first open an editable image-prompt dialog. Its default includes the role name, handle, responsibilities and selected theme color. Use **Reset to default**, **Cancel**, or **Generate**; only confirming **Generate** calls Codex. A custom brief is kept for regeneration in the current form, not stored as a global prompt or written into the role's system prompt. Empty or overlong briefs cannot be submitted. The default is refreshed from current role details unless you have customized it.

The generated image is a preview until you **Create role** or **Save**. **Regenerate** produces a new preview. Choosing **Initials** or **Pixel art** switches back from an AI image to that base style; it removes the saved image only after you save the role. Cancelling or leaving the form does not replace the saved image. Generation failures keep the previous avatar and display an error so you can retry. Saving is disabled while a generation is running. If you change the name, handle, system prompt, image brief or theme color while it runs, Runner discards that result; generate again for the updated role.

If Codex reports a terminal failure, Runner stops that generation request and keeps the previous avatar. A genuine timeout includes the last CLI stage and an error category, rather than hiding a provider/authentication failure behind a generic timeout. Runner does not automatically repeat the image request or switch your Codex login/provider.

Saved avatars are reused in the role list, role chat, crew slots and mission views. A crew slot keeps its role's image even if its in-crew handle differs. Missing or unreadable images fall back to the saved base style.

## Roles in chats and crews

In **Start a chat → Role**, choosing a role copies its settings into the launch. Changes in that dialog apply to that chat; they do not rewrite the saved role. **Direct** starts an agent without needing a role.

A [crew slot](crews.md) references a role and gives it an in-crew handle. The same role can fill several slots, each with a different handle. A slot can override the runtime, model, effort and Codex speed. Changing the runtime resets engine-specific settings to the new runtime's defaults before its overrides are applied.

Editing a role does not type instructions into an already-running terminal. Use a message for an immediate instruction, and review the launch settings when starting new work.

See [Build a crew](../guides/build-a-crew.md) for a complete coder/reviewer example and [CLI reference](../reference/cli.md#runner-role) for role commands.
