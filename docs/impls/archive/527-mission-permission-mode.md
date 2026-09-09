# 527 — Mission permission mode, bypass by default

Tracking issue: [#527](https://github.com/yicheng47/runner/issues/527). Spec: [527](../../features/archive/527-mission-permission-mode.md). Feature, P1. Shipped 2026-09-09 in [#532](https://github.com/yicheng47/runner/pull/532) (mission `01M22Y3CWV3PBJFBNRBSW260ZF`, claude crew). Baseline `main` at `cbf4ea4` (2026-09-09), Runner 0.8.4. Design: `design/runner.pen` frame `Settings — Missions` (`ez53n`). Phases 1 and 2 of the spec; phase 3 (the prompt hook) is not in scope.

## What ships

One app setting, **Mission permissions** (`Bypass` default / `Auto` / `Runner default`), on a new Settings → Missions pane that also takes over Default crew from General. Every mission slot spawns with the setting's flags, at mission start and on resume. The mode a mission started with is recorded in its `mission_start` event and shown in the mission metadata panel. Direct chats, forks, and the runner form are untouched. No migration.

## Where the code is

- `crates/runner-backend/src/router/runtime.rs:242` `PermissionMode` (`Default`/`AcceptEdits`/`Auto`/`Bypass`), `:258` `permission_mode_args`, `:366` `apply_permission_mode` (strip then append). codex Bypass today is `--ask-for-approval never --sandbox workspace-write`.
- `crates/runner-backend/src/session/manager/spawn.rs:563` `register_mission_session` — `resolve_runtime_override` (`manager/mod.rs:1328`) then `resolve_runner_executable`; the runner's `args` become `SpawnSpec.args` in `base_spawn_spec` (`:410`) before `apply_runtime_args` (`:479`) appends the resume plan and trailing args. `:1985` is the same runner rebuild inside `resume_with_fresh_fallback`, mission rows and direct rows alike (`snap.mission_id` tells them apart).
- `crates/runner-backend/src/ops/mission.rs:173` `start` writes the `mission_start` signal with payload `{title, cwd}` (`:270`); `:564` `mission_start_impl_with_size` is the only caller and has `state: &AppCore`. The MCP tool (`mcp/tools/mission.rs:430`) and the Start Mission modal (`runner-app/src/surfaces/start_mission.rs:329`) both go through it.
- `crates/runner-app/src/app_settings.rs:167` `AppSettings` (`#[serde(default, rename_all = "camelCase")]`); `FileLinkEditor` at `:110` is the enum-setting pattern (`key`/`label`, kebab-case serde). `app_store.rs:471` `update_settings` is the one mutation chokepoint; the Windows `initialize_mcp_defaults` call inside it is the precedent for a side effect on change. `AppStore::new` at `:245`.
- `crates/runner-app/src/surfaces/settings_page.rs:28` `SettingsPane`, `:44` `from_route`, `:59` `key`, `:76` `label`, `:92` `icon`, `:109` `APP_PANES`; `:1484` `render_general_settings` with the Default crew row at `:1560`; `:554` refreshes crews only for `General`; `:647` saves `default_crew_id`. Nav search test at `:1928` asserts 11 panes. `rocket.svg` is already registered in `assets.rs:156`.
- `crates/runner-app/src/surfaces/mission_workspace.rs:578` derives `self.goal` from the `mission_goal` event payload; `:5911` is the Crew section of the metadata panel.

## Fix shape

1. **Enum.** `router::runtime::MissionPermissionMode { Bypass, Auto, RunnerDefault }`, serde kebab-case (`bypass`/`auto`/`runner-default`), `Default = Bypass`, `key()`/`label()` like `FileLinkEditor`. `mission_permission_mode_args(runtime, mode) -> Option<Vec<String>>`: `RunnerDefault` → `None`; `Auto` → `permission_mode_args(runtime, Auto)`; `Bypass` → `permission_mode_args(runtime, Bypass)` except codex, which gets `--ask-for-approval never --sandbox danger-full-access`. `apply_mission_permission_mode(runtime, args, mode)`: `None` returns `args` unchanged, `Some` strips via `strip_permission_flags` and appends. The runner-level `permission_mode_args` keeps `workspace-write`; the runner form and `infer_permission_mode` are not touched.
2. **Live value.** `SessionManager` holds `mission_permission_mode: RwLock<MissionPermissionMode>` (default Bypass) with `set_mission_permission_mode` / `mission_permission_mode`. It lives on the manager, the spawn-time consumer, so `AppCore`'s three constructors and the spawn/resume signatures do not change and the MCP `mission_start` sees the same value as the modal.
3. **Apply per slot.** In `register_mission_session`, after `resolve_runtime_override` and before `resolve_runner_executable`, `runner.args = apply_mission_permission_mode(&runner.runtime, &runner.args, mode)`. Same line in the mission branch of `resume_with_fresh_fallback` (`snap.mission_id.is_some()`), after the `resolve_runtime_override` rebuild. Direct rows, `spawn_direct`, `spawn_runtime_direct`, and `spawn_fork` (direct-only, `spawn.rs:1468`) are untouched. A slot runtime override still starts from `default_permission_mode()` inside `resolve_runtime_override` and is then converged like any other row.
4. **Record.** `start` takes the mode and writes `"permission_mode": mode` into the `mission_start` payload beside `title` and `cwd`; `mission_start_impl_with_size` reads it from `state.sessions`. The event log is the no-schema home for "what this mission started with"; nothing on `missions` or `sessions` changes.
5. **App side.** `AppSettings.mission_permission_mode: MissionPermissionMode` (field default Bypass, so pre-feature settings files decode to Bypass). `AppStore::new` pushes it into `core.sessions`; `update_settings` pushes again whenever the closure changed it. `SettingsPane::Missions`: route/key `missions`, label `Missions`, icon `rocket.svg`, inserted in `APP_PANES` right after `General` (the canvas nav still shows a Chat item that impl 0034 removed from code; ignore it). Pane per frame `ez53n`: `PaneHeader("Missions", …)`, one `SettingsCard` with the Default crew row moved verbatim from General (subtitle kept; `:554` crew refresh and `:647` save keyed to the new pane) and a `Mission permissions` row with `settings_select` over the three options, subtitle *"Applied to every slot when a mission starts. Bypass never prompts — nobody is watching a mission slot to answer. Direct chats keep their runner's own mode."* General loses only that row. Nav search test count 11 → 12.
6. **Metadata panel.** A `Permissions` `meta_section` after Crew showing `bypass` / `auto` / `runner default`, read from the first `mission_start` signal's `payload.permission_mode` the way `self.goal` reads `mission_goal`. No key (pre-feature missions) → no section.

## Rules of the road

- No migration under `crates/runner-backend/migrations`, no column on `missions`, `slots`, or `sessions`, no per-mission or per-slot value, no Start Mission control.
- `permission_mode_args`, `strip_permission_flags`, `apply_permission_mode`, `ops::runner::create`/`update`, and the runner form keep today's behaviour and tests. Only mission slots read the new setting.
- Keep UI copy and labels exactly as the frame and the spec give them; do not redesign the pane.
- Do not launch the Runner app (`make run`); Jason smoke-tests. Verify with `cargo test -p runner-backend -p runner-app`, `make clippy`, `make fmt`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit on `feat/527-mission-permission-mode`, push, open the PR, drive CI green (`gh pr checks <n> --watch`; the required check is `Rust / macOS`). Do not merge: Jason merges after his own check.

## Tests

- `router::runtime`: each mode's argv for claude-code, codex, trae, shell (shell/unknown → unchanged); a codex row carrying `--ask-for-approval on-request --sandbox workspace-write` converging to `never` + `danger-full-access`; a claude row with `--permission-mode plan` plus `--model opus` keeping `--model opus` under Bypass and keeping `plan` under RunnerDefault.
- `session::manager::tests`: a mission spawn through the fake runtime with the manager set to each mode asserts the spec's argv; a resume after `set_mission_permission_mode` changed carries the new flags; a direct spawn is byte-identical across modes.
- `ops::mission`: the `mission_start` payload carries `permission_mode`.
- `app_settings`: a settings JSON without the key decodes to Bypass; round-trip of each value.
- `settings_page`: the nav count, `from_route("missions")`, and a `VisualTestContext` render of the Missions pane at two rem sizes (follow the Skills pane tests in `settings/skills.rs:1263`).
- `mission_workspace`: the Permissions section renders from a `mission_start` payload and is absent without the key.

## Jason's smoke test (after landing)

1. Fresh settings key: Settings → Missions shows Default crew and Mission permissions at Bypass; General has no Default crew. Start a Peer Coding Crew mission on the runner repo; the coder pushes to a scratch branch and runs `gh pr view`. No prompt in either slot, codex reaches the network, the metadata panel says `Permissions: bypass`.
2. Switch to Auto, start another mission: the claude slot stops on the push; the first mission's panel still says bypass.
3. Runner default with a runner row carrying `--permission-mode plan`: the slot spawns with exactly that.
4. Quit and relaunch with auto-resume on: resumed slots carry the flags for the current setting.

## Non-goals

Phase 3's `permission_prompt` hook, a per-mission or per-slot mode, a schema change, any change to the runner form's permission dropdown or the runner-level codex Bypass mapping, grid-scraping for prompts, and worktree isolation (403).
