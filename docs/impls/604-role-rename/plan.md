# Runner → role rename — mission plan

Missions for [#604](https://github.com/yicheng47/runner/issues/604) ([spec](../../features/604-rename-runner-to-role.md), [README](README.md), [log](impl_log.md)). Both land through the one open PR, [#618](https://github.com/yicheng47/runner/pull/618).

| Mission | Scope | Crew | State |
| --- | --- | --- | --- |
| 1 | Spec phases 1–3: migration 0023, backend rename with SQL confined to `repo/`, MCP cutover, write-new/read-old for persisted values. | codex peer, [brief](../archive/gpui-rewrite/briefs/604-m1-role-rename-backend.md) | Reviewed clean 2026-09-16 (mission `01M2MJ8WE488AJNN0NAHHF3QZ8`, three rounds); committed as `2e417ba` on `feat/604-m1-backend`; dev-database migration verified; in-app smoke pending |
| 2 | Spec phases 4–5: the UI copy table and the two modal fixes, `surfaces/runners/` → `surfaces/roles/`, vision, AGENTS.md, both READMEs. | codex peer, brief to write | Not started |
| follow-up | `runner_status` → `session_status` with read compatibility across the CLI, `crews.signal_types`, the app and mission-watch. | inline | After both missions |
