# 1253 — Remove custom arguments from roles

Tracking issue: [#1253](https://github.com/yicheng47/runner/issues/1253).

## Motivation

A role's detail page shows `Command: codex --ask-for-approval on-request --sandbox workspace-write`, but its edit form's Args field is empty. Runner stores a role's permission mode as those flags in `role.args`. The detail page joins the raw args onto the command, while the edit form strips the permission flags (`role_visible_args`), so the two disagree and the Command row reads as if the role had custom arguments. The role forms have no Permissions picker: create stores Default, and edit preserves the stored mode internally.

Custom role arguments have no real use: every role in use holds only permission flags. Per-agent launch options belong on the agent's settings page ([#1246](https://github.com/yicheng47/runner/issues/1246)), not on each role.

## Scope

- **Create role form:** no Args field.
- **Edit role form:** the Args field appears only when the role already carries non-permission arguments (`role_visible_args` is non-empty), so an older role's extra arguments stay visible and can be cleared. Clearing them and saving removes the row the next time the form opens. A role without them shows no Args row, and saving it never drops its permission flags.
- **Role detail page:** the Command row shows the command plus only the non-permission arguments, so it matches the edit form. A role without extra arguments shows just the executable.
- **CLI:** `runner role create` and `runner role update` no longer take `--arg`. The CLI reference is regenerated.
- **Storage and launch:** no schema or protocol change. `role.args` keeps holding the permission flags, and existing extra arguments keep applying at launch.
- **Guide:** `docs/guide/concepts/roles.md` and its `zh-CN/` counterpart no longer list arguments among a role's settings; both locales' `reference/cli.md` pages are regenerated.

No new UI is drawn: the change removes a field and reuses the existing Args row conditionally, so there is no Pencil frame.

## Non-goals

- Agent-level launch options (#1246).
- `--env` and role environment.

## Verification

Automated: the roles surface tests in `runner-app` cover the create form without Args, the edit form showing Args only for a role with extra arguments, the dirty check and save keeping permission flags, and the detail Command string; the CLI tests cover `--arg` being rejected.

Live checks in the development app, because the change alters what a person sees on the Roles pages:

1. **Create form:** New role shows no Args field; a role created in the form stores Default permissions.
2. **Detail and edit without extra arguments:** for a CLI-created Codex role with Auto permissions, the detail Command row shows only `codex`, and Edit shows no Args row; editing and saving keeps its permission flags.
3. **Edit with extra arguments:** for a qa-created role seeded with one extra argument, the detail Command row shows `command <arg>`, Edit shows the Args row with that argument, and clearing it and saving removes the argument and the row.
