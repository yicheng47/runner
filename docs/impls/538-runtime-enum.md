# 538 — Replace bare runtime name strings with a `Runtime` enum

Tracking issue: [#538](https://github.com/yicheng47/runner/issues/538). Chore, P2. Baseline `main` at `23626c5` (2026-09-09). Consolidation-shaped like [478 pass 1](./archive/478-consolidation-pass-1.md): one mission, one PR, one commit per numbered section, no behavior change beyond the one named below. Scheduled by the M6 audit ([m6-remainder](./gpui-rewrite/m6-remainder.md), "one mechanical commit at or after cutover"); the cutover landed 2026-08-23.

## Why

Runtime identity is a bare string everywhere. On the baseline, outside test files: `"codex"` 338 times, `"claude-code"` 202, `"shell"` 122, `"trae"` 82, across 42 non-test files; `router/runtime.rs` alone has 142 literal sites, `ops/skills.rs` 68, `surfaces/panes.rs` 32. Dispatch is `match runtime { "codex" => …, _ => … }` and `entry.agent_runtime == "shell"` at each site, so a wildcard arm silently absorbs `trae` and `shell`, a typo compiles, and adding or retiring a runtime means grepping every arm. `RuntimeDefinition` in `router/runtime.rs` is already the registry, but it is keyed by `name: &'static str`, so every consumer goes back through a string.

## What ships

Three commits on one feature branch:

1. A migration that normalizes runtime names already in user databases, so the strict type in commit 2 never meets a value it cannot decode.
2. A `Runtime` enum in `runner-backend` — `ClaudeCode`, `Codex`, `Trae`, `Shell` — with the wire names unchanged (`claude-code`, `codex`, `trae`, `shell`). Model fields, ops inputs, the router adapter, spawn, skills, runtime status and defaults, the MCP tool args, and the app surfaces dispatch on the enum. Strings survive only at the boundaries listed in section 2. The one intentional behavior change: runner create/update and the direct-session MCP tool reject an unknown runtime name at the JSON boundary instead of persisting it.
3. The one arch line that promised unknown rows stay readable, rewritten to describe the migration.

Tests, `make clippy`, and `make fmt` stay green after each commit.

## Where it touches

### 1. Migration `0021_runtime_names.sql`

Facts to build on: the Qoder runtime was dropped in `add02aa` (2026-08-28) with no data migration, so colleague databases may still hold `runners.runtime = 'qoder'` and `sessions.agent_runtime = 'qoder'` rows; `docs/arch/arch.md` §3.2 currently says those rows "stay readable but cannot spawn". Runner create (`ops/runner.rs::create`) never validated `runtime`, so any string could have been persisted; a test on the baseline even inserts `"Runtime-Needle"`. Both databases on the author's machine hold only the four known names in `runners.runtime`, `slots.runtime_override`, and `sessions.agent_runtime` (checked 2026-09-10).

The migration, registered in `db.rs` next to `0020`:

- `runners.runtime` not in the four known names → `'shell'`. Every wildcard arm on the baseline already treats an unknown name exactly like `shell`: `resolve_runner_executable` returns the runner untouched when `runtime_definition` finds nothing, and every `model_effort_args` / `permission_mode_args` / `system_prompt_args` helper returns empty. So the row keeps spawning its `command` verbatim, as it did; the runner keeps its handle, persona, and args, and the user can pick a real agent runtime in the edit form. The only visible difference is the sidebar showing the shell icon instead of the raw name.
- `slots.runtime_override` not in the three agent names → `NULL` (fall back to the runner's own runtime). `shell` was never a valid override — `validate_runtime_override` rejects it — so it is normalized too.
- `sessions.agent_runtime` non-NULL and not in the four known names → `'shell'`.
- `sessions.runtime` is the PTY discriminator (`native-pty`), not the agent kind. Do not touch it. Same for `runtime_socket` / `runtime_session` / `runtime_window` / `runtime_pane`.

Test: seed a connection at migration 20 with one row per affected column carrying `'qoder'`, run the migrations, assert the rewritten values. Follow the existing migration-test pattern in `db.rs`.

### 2. The `Runtime` enum and every consumer

**The type.** Define it in `crates/runner-backend/src/model.rs` beside `SessionStatus`, shaped like `MissionPermissionMode` in `router/runtime.rs`:

- `#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]` with `#[serde(rename_all = "kebab-case")]`, which yields exactly the four wire names. Assert them in a unit test (`serde_json::to_string(&Runtime::ClaudeCode) == "\"claude-code\""`, and back).
- `pub const ALL: [Self; 4]`, `pub fn key(self) -> &'static str`, `pub fn parse(value: &str) -> Option<Self>`, and `impl Display` that writes `key()` so `format!` sites keep working.
- No `Default`. There is no default runtime; the form's default of `codex` stays where it is.

**The registry.** `router::runtime::RuntimeDefinition.name` becomes `Runtime`. `runtime_definition(Runtime) -> Option<RuntimeDefinition>` stays `Option` and returns `None` for `Shell`: the three-entry catalog is the list of *agents* the UI can select for a runner or slot, skills have catalogs for, and executable discovery tracks; `shell` is a runtime a session can be (direct terminals, the drawer) but not an agent. Every place that gates on `runtime_definition(..).is_none()` today (slot override validation, `runtime_set_override`, `runtime_clear_override`, skills' `catalog_at`, the resume path) keeps its meaning without a rewrite. `runtime_display_name` and `supports_native_fork` take `Runtime`.

**The rule for every other site.** A struct field, function parameter, or local that names an agent runtime becomes `Runtime` (or `Option<Runtime>` where it was `Option<String>`). Strings remain only where the value is persisted or parsed as text:

- SQLite columns — through `serde_rusqlite` (`from_row` / `to_params_named`) exactly as `SessionStatus` already crosses, no hand-written mapping. Where a repo function binds the value with `rusqlite::params!` (e.g. `repo::slot::set_runtime_override`), bind `runtime.key()`. The `COALESCE(s.runtime_override, r.runtime)` queries in `repo/crew.rs` and `repo/session.rs` are unchanged SQL; their decoded field becomes `Runtime`.
- MCP JSON — `CreateRunnerInput.runtime`, `UpdateRunnerInput.runtime`, `UpdateSlotInput.runtime_override`, `CreateSlotInput.runtime_override`, `StartDirectSessionArgs.runtime` become the enum; serde does the rejection. The `Runner`, `Slot`, `CrewMemberPreview`, `SessionRow`, `DirectSessionEntry` outputs serialize to the same strings as before.
- Settings JSON — `enabled_agents`, `disabled_agents`, `initialized_mcp_clients` in `app_settings.rs` and the `runtime_overrides` map in `db.rs` stay `String` collections; lookups pass `runtime.key()`. `is_agent_enabled` may take `Runtime`.
- `SelectOption` values in the app — the select widget is string-keyed. Build options from `runtime.key()`; on change, `Runtime::parse(value)`, and treat `None` as no-op (the options come from the catalog, so it cannot happen).

**Dispatch.** `match` on a `Runtime` never has a `_` arm: list the variants each arm covers, so adding or retiring a runtime is a compile error at every site. Equality checks (`runtime == Runtime::Shell`) are fine where a match would be noise. `matches!(runtime, Runtime::Codex | Runtime::Trae)` is fine.

**Modules, in order of literal count, all in scope:** `router/runtime.rs` (every `runtime: &str` parameter — `model_effort_args`, `claude_settings_args`, `permission_mode_args`, `strip_permission_flags`, `apply_permission_mode`, `mission_permission_mode_args`, `apply_mission_permission_mode`, `infer_permission_mode`, `system_prompt_args`, `first_turn_argv`, `trailing_runtime_args`, `mission_bus_sandbox_args`, `resume_plan`, `fork_plan`), `ops/skills.rs` and `skills.rs` (`SkillCatalog.runtime`, the three `runtime: &str` entry points, `skill_catalog`, `read_entry`), `session/manager/spawn.rs` (`resolve_runtime_only_resume_runner`, `enter_claude_launch_gate`, `seed_codex_project_trust`, `apply_runtime_args`, `codex_capture_prompt_marker`, `runtime_direct_runner`), `session/codex_capture.rs`, `runtime_status.rs` (`RuntimeExecutableStatus.name`, `effective_runtime_command`, `validate_override`, `runtime_not_found_error`), `runtime_defaults.rs`, `ops/runtime.rs` (`RuntimeDefinition.name`, `RuntimeCatalogEntry.name`, `runtime_set_override`, `runtime_clear_override`), `ops/runner.rs`, `ops/slot.rs` (`validate_runtime_override` collapses to the agent check), `ops/session.rs` (`SessionRow.runtime`, `DirectSessionEntry.agent_runtime`, the `effective_runtime(..) == Some("shell")` gates), `ops/node.rs`, `ops/crew.rs` (`CrewMemberPreview.runtime`), `ops/mcp.rs`, `mcp/tools/session.rs`, `repo/{runner,slot,session,crew,node}.rs`, `db.rs` seeds. In the app: `surfaces/runners.rs` (`permission_modes`, `permission_mode_description`, `RuntimeLayerResolution`, `resolve_slot_runtime_layers`, `runtime_entry` and friends), `surfaces/settings/agents.rs`, `surfaces/settings/skills.rs`, `surfaces/panes.rs` (`pane_identity_icon`, `pane_identity_shows_status`, `starting_overlay_label`, `pane_action_items_for`, `pane_close_behavior`, every `agent_runtime == "shell"`), `surfaces/sidebar.rs` and `sidebar_logic.rs`, `surfaces/start_chat.rs`, `surfaces/crews.rs`, `surfaces/chat.rs`, `surfaces/command_palette.rs`, `surfaces/mission_composer.rs`, `surfaces/mission_workspace.rs`, `surfaces/settings_page.rs`, `surfaces/settings/archived.rs`, `app_settings.rs`, `app_store/mcp_defaults.rs`, `ui/select.rs`. `cli/`, `runner-core`, and `runner-terminal` have no agent-runtime sites; leave them alone.

**Not this runtime.** `repo::session::Session.runtime` and `session::runtime::RuntimeSession.runtime` are the PTY discriminator (`"native-pty"`). `runtime_status`'s shell discovery, `runtime_shell_env`, and `direct_chat_path` are about the login shell, not `Runtime::Shell`. Strings named `runtime` in those places stay strings.

**Tests.** Test files (`tests.rs`, `tests/`, `#[cfg(test)]` blocks) are updated to use the enum. Tests that exercised made-up names — `"Runtime-Needle"` in `ops/runner.rs`, `"aider-future"` in `ops/slot.rs`, `"qoder"` / `"unknown"` in `skills.rs` and `ops/skills.rs` — are rewritten to what still holds: the slot-override test asserts `Shell` is rejected as an override, the skills tests assert `Trae` and `Shell` have no catalog, and a new test asserts that `serde_json::from_value::<CreateRunnerInput>` with `"runtime": "aider-future"` fails. The `runner_get`/`runner_list` MCP JSON shape gets one assertion that `runtime` still serializes as `"codex"`.

**Optional, only if it falls out.** `ops::runtime::RuntimeDefinition` is a `String` copy of `router::runtime::RuntimeDefinition` for the app; once both carry `Runtime` the copy may be pointless. Collapse it only if the five app callers of `runtime_list()` get simpler, not as a separate refactor.

### 3. Docs

`docs/arch/arch.md` §3.2: replace the sentence "`qoder` rows from before v0.6.7 stay readable but cannot spawn" with what is true after commit 1 (migration `0021` rewrites unknown runtime names to `shell`), and name the `Runtime` enum as the identity type. No other doc edits.

## Rules of the road

- No behavior change beyond the JSON-boundary rejection. If a site's rewrite makes you want to change logic, note it in the handoff and leave the logic alone.
- Mission authorization: commits on the task feature branch are authorized, one per numbered section, so the reviewer can read them separately. Push, PR, and merge are **not** authorized — the human lands the branch.
- Do not launch the Runner app (`make run`) — the human smoke-tests. Verify with `cargo test --workspace`, `make clippy`, `make fmt`.
- Follow existing patterns; no new modules, traits, or helpers beyond the enum and its `key` / `parse` / `Display`. No `From<&str>` that panics, no lossy `unwrap_or(Runtime::Shell)` shims — a parse that can fail returns `Option` or an error.
- Do not move code between files; do not split `router/runtime.rs` or the surfaces. File splits are their own passes.
- The `rusqlite`-level `FromSql` / `ToSql` impls are not needed; serde_rusqlite covers the rows. Do not add them.

## Verification

Per commit: `cargo test --workspace`, `make clippy`, `make fmt`, all green. After commit 2, `grep -rn --include='*.rs' '"codex"\|"claude-code"\|"trae"\|"shell"' crates cli` outside test code should return only: the four `key()` strings in `model.rs`, `RUNTIME_DEFINITIONS`' `command` values (`"codex"`, `"claude"`, `"traecli"`), the `'codex'` inside `db.rs`'s seed `INSERT` SQL, the settings-JSON keys in `app_store/mcp_defaults.rs` and `app_settings.rs`, and the migration. The handoff lists every remaining literal with its reason, the migration test's before/after rows, and every test that was rewritten rather than mechanically updated. Reviewer checks the diff for accidental behavior change first (a `_` arm that used to cover `trae` or `shell` and now names only one of them is the thing to look for), then the remaining-literal list.

## Non-goals

Adding a variant: feature [539](../features/539-pi-runtime.md) adds `pi` and is sequenced after this lands precisely so the compiler, not grep, finds every arm it must cover. An `Error` enum (the audit's sibling item), file splits, renaming `runtime_override` columns, changing the MCP tool schemas beyond what the enum's `JsonSchema` derive produces, adding a runtime, a `Default` runtime, README or product-doc edits, and anything in `docs/impls/archive/` or `docs/features/archive/`.
