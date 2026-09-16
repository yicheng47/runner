# Rename the runner entity to role — program record

Implementation record for [feature 604](../../features/604-rename-runner-to-role.md) ([#604](https://github.com/yicheng47/runner/issues/604)). The spec says *what*; this directory says *how, in what order, and what has landed*. Same shape as the [hook-status](../347-hook-status/README.md) record: this file is the condensed state and the decisions that bind, [plan.md](plan.md) is the mission plan, [impl_log.md](impl_log.md) is the dated log. Briefs live in [`docs/impls/archive/gpui-rewrite/briefs/`](../archive/gpui-rewrite/briefs/).

## Status (2026-09-16)

Both missions are implemented and reviewed clean by the codex peer crew and by Claude, on `feat/604-m1-backend` as [PR #618](https://github.com/yicheng47/runner/pull/618) against `main`. Mission 1 renamed the data, backend and MCP layers; mission 2 renamed the words users read, moved `surfaces/runners/` to `surfaces/roles/`, and aligned AGENTS.md, both READMEs, the vision doc and the arch doc. Jason's smoke test on the migrated dev database added the role glyph and a layout fix on the role detail page. Migration 0023 has run on the macOS dev database with every row intact; the Windows dev-database migration is outstanding.

## Decisions that bind

1. **The product stays Runner; only the stored entity becomes role.** Spec decisions 1–4: `role` not `worker`, `slots` stays, the CLI keeps `runner signal` / `runner msg` / `runner status`.
2. **The database renames too.** Migration 0023 renames `runners` → `roles` and the `runner_id` columns on `slots` and `sessions` → `role_id`, with `ALTER TABLE … RENAME` only and `legacy_alter_table` off so SQLite rewrites the foreign keys. Jason considered a code-only rename to avoid the downgrade cliff and reversed it the same hour: an older Runner cannot open a migrated database, and that is accepted.
3. **Every SQL statement naming the entity lives behind `repo/`.** Callers in `ops/`, `session/` and `mcp/` speak only in role terms. Gate: a grep for `roles` / `role_id` inside SQL strings hits only `repo/` and the migration.
4. **Persisted values write the new spelling and read the old.** `role-default` with a `runner-default` alias, start-chat mode `role` accepting `runner`, saved routes `/roles` accepting `/runners`. Nothing is frozen under the old noun for compatibility's sake.
5. **`roster.json` carries only handle and lead**, so the brief's `runner_handle` exception was moot; the two serialize-only in-process payloads use `role_handle`.
6. **`runner_status` stays for now.** It is emitted by the CLI's `runner status busy|idle`, stored in every crew's `signal_types` list and read by the app and by mission-watch. Renaming it to `session_status` with read compatibility is the follow-up after both stages, not part of either.
7. **The MCP rename is a cutover.** `role_create/get/get_by_handle/list/update/delete`, `role_id` on slot inputs, role forms in every response field, no aliases. The installed app keeps the old tool names until a release carries this.

## Open

- Merge #618, archive the spec and this record, then the release that carries the MCP cutover.
- On JASONPC the first `.\make.cmd run` migrates the Windows dev database, which doubles as the Windows check.
- Follow-ups outside this feature: `runner_status` → `session_status` with read compatibility, the router's first-turn prompt still saying "lead runner", and the design canvas labels.
- The leftover branch `feat/604-role-rename` (brief commit only) can be deleted.
