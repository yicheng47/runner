# Direct Chat Runtime Picker

> Implements [#195](https://github.com/yicheng47/runner/issues/195). Product spec: [docs/features/archive/25-direct-chat-runtime-picker.md](../../features/archive/25-direct-chat-runtime-picker.md).

## Context

Direct chats are currently runner-template chats end to end. `StartChatModal` fetches runners, `session_start_direct` requires a `runner_id`, `sessions.runner_id` is `NOT NULL`, direct-chat listing joins `runners`, and `RunnerChat` is routed through `/runners/:handle/chat/:sessionId` so it can fetch the runner by URL handle for the header and terminal runtime.

Issue #195 adds the second direct-chat path: pick an agent runtime plus cwd, spawn immediately, and do not create a runner template. The main design constraint is that a runtime-only chat has no stable runner handle or runner row, so this cannot be just a modal change.

## Decisions

1. Replace the runner-scoped direct-chat route with `/chats/:sessionId`.
2. Make `sessions.runner_id` nullable while preserving `ON DELETE CASCADE` for rows that still reference a runner.
3. Add explicit agent-runtime metadata on `sessions` instead of reusing `sessions.runtime`.
4. Keep runtime mode intentionally bare: runtime dropdown, cwd picker, Start. No model, effort, permission, args, env, or prompt fields.
5. Runtime-only direct chats are resumable. Resume reconstructs the same ephemeral runner config from the session row instead of looking up `runners`.
6. Add a Chat settings section for the default runtime picker. Do not add default args.

The route change is the biggest tradeoff. Keeping `/runners/:handle/chat/:sessionId` would make runner-template chats look structurally different from runtime-only chats even though both are the same product surface: a direct session. A session-owned route is cleaner; runner context can be loaded from `session_get` when the row still references a runner.

The metadata naming matters. `sessions.runtime` already exists, but it is PTY-runtime reattach metadata (`pty`, legacy `tmux`, etc.), not the agent runtime kind. Add columns such as `agent_runtime` and `agent_command` so `claude-code` / `codex` state does not collide with runtime-layer bookkeeping.

Runtime mode uses the app's normal new-runner defaults in memory. Model and effort stay `NULL` so the CLI picks its own model/reasoning defaults; permission mode uses `default_permission_mode()` and is applied to the ephemeral args with `router::runtime::apply_permission_mode`. The user does not see or edit those fields in runtime mode.

Settings should only choose the default runtime shown in Start Chat's Runtime mode. A global default-args setting would create an invisible runner template without the runner list's affordances, so custom args stay in Runner mode.

## Step 1: Schema and model shape

**Files:** `src-tauri/migrations/0007_direct_runtime_sessions.sql`, `src-tauri/src/db.rs`, `src-tauri/src/model.rs`, `src/lib/types.ts`, `src/lib/api.ts`

- Add migration `0007_direct_runtime_sessions.sql`.
- Rebuild `sessions` in SQLite so `runner_id TEXT REFERENCES runners(id) ON DELETE CASCADE` is nullable.
- Add nullable `agent_runtime TEXT` and `agent_command TEXT` columns.
- Copy existing rows with `agent_runtime = NULL` and `agent_command = NULL`; runner-template rows continue deriving agent metadata from `runners`.
- Register migration version 7 in `db.rs`.
- Change shared frontend/backend types so direct-session surfaces accept `runner_id: string | null`.
- Add `agent_runtime`, `agent_command`, and `runtime_label`/`display_name` fields to direct-session DTOs where the UI needs a label without fetching a runner.

Do not overload `SessionRow` for mission sessions. Mission sessions should still have non-null runner ids because slots always reference runners. Keep `session_list` as an inner join on `runners`.

## Step 2: Runtime registry command

**Files:** `src-tauri/src/router/runtime.rs`, `src-tauri/src/commands/runtime.rs`, `src-tauri/src/commands/mod.rs`, `src-tauri/src/lib.rs`, `src/lib/api.ts`

- Add a small backend registry:

```rust
claude-code -> command claude -> display Claude Code
codex       -> command codex  -> display Codex
```

- Expose it through `runtime_list`.
- Add a `RuntimeDefinition` TS interface and `api.runtime.list()`.
- Use the backend list for Start Chat runtime mode.

The existing frontend `RUNTIME_OPTIONS` can stay for runner create/edit in this pass. Consolidating both forms onto the backend registry is reasonable later, but it is not required to ship #195.

## Step 3: Runtime-only spawn path

**Files:** `src-tauri/src/commands/session.rs`, `src-tauri/src/session/manager.rs`, `src-tauri/src/router/prompt.rs`

- Add `session_start_runtime(runtime: String, cwd: Option<String>, cols: Option<u16>, rows: Option<u16>)`.
- Validate the runtime name through the registry.
- Construct an ephemeral `Runner` value with:
  - `id` as a non-persisted synthetic value used only inside the live manager map.
  - `handle` as the runtime name.
  - `display_name` as the runtime display name.
  - `runtime` and `command` from the registry.
  - args computed from `router::runtime::apply_permission_mode(runtime, &[], default_permission_mode())`.
  - empty env and null working_dir/system_prompt/model/effort.
- Call a direct-spawn path that can insert `runner_id = NULL`, `agent_runtime = runtime.name`, and `agent_command = runtime.command`.

Implementation detail: either extend `spawn_direct` with a `DirectSpawnIdentity` enum or add a focused `spawn_runtime_direct` wrapper that shares the same internal helper. Avoid pretending the ephemeral runner id is persisted; that will create subtle bugs in `runner/activity`, runner deletion, and resume.

Do not emit `runner/activity` for runtime-only chats. There is no runner row to update, and the sidebar/chat list will refresh through session events.

## Step 4: Direct-session queries

**Files:** `src-tauri/src/commands/session.rs`, `src/components/Sidebar.tsx`, `src/pages/RunnerChat.tsx`, `src/components/CommandPalette.tsx`

- Change `session_list_recent_direct` and `session_get` to `LEFT JOIN runners`.
- Return:
  - `runner_id: Option<String>`
  - `handle: Option<String>` or a separate `runner_handle`
  - `agent_runtime: String`
  - `agent_command: String`
  - `display_name` derived from runner display/name for runner rows or registry display for runtime rows.
- For existing runner-template rows, compute agent runtime from `r.runtime`.
- For runtime rows, compute display from registry and fall back to raw `agent_runtime` if a future registry no longer knows the value.

Update default labels:

- Runner-template chat: `@handle · <started>`.
- Runtime-only chat: `Runtime Display · <started>`.

## Step 5: Resume and reattach

**Files:** `src-tauri/src/session/manager.rs`, `src-tauri/src/commands/session.rs`

- In `SessionManager::resume`, read nullable `runner_id`, `agent_runtime`, and `agent_command`.
- If `runner_id` is present, keep the existing runner lookup path.
- If `runner_id` is null, reconstruct an ephemeral runner from the persisted agent runtime metadata and the registry display fallback.
- Reapply the same in-memory new-runner defaults used by `session_start_runtime`: default permission mode, null model/effort/system_prompt, empty env.
- Use the row `cwd` as the effective cwd. Runtime-mode rows have no runner working_dir fallback.
- Keep agent-native resume behavior (`agent_session_key`) unchanged for claude-code and codex.
- Update startup reattach (`collect_running_rows` / `attach_existing`) to handle null `runner_id`; the forwarder should skip runner activity emission for runtime-only rows.
- Change `SessionHandle.runner_id` to `Option<String>` or otherwise tag runtime-only handles so `kill_all_for_runner` ignores them.

This is the main correctness risk. If resume keeps assuming `runner_id` is a string, stopped runtime chats will render as resumable in the UI but fail when clicked.

## Step 6: Start Chat modal

**Files:** `src/components/StartChatModal.tsx`, `src/components/ui/RuntimeSelect.tsx`, `src/lib/settings.ts`

- Add a segmented control at the top: `Runner` and `Runtime`.
- Remember the last selected mode in local storage.
- Runner mode keeps the existing runner picker, title behavior, cwd precedence, and `session_start_direct` call.
- Runtime mode loads `api.runtime.list()`, defaults to the Chat settings runtime when present and valid, otherwise the first runtime, shows runtime dropdown plus working directory, and calls `session_start_runtime`.
- Runtime mode title default should be `Chat with <display name>`, but keep the title field optional and user-editable like runner mode.
- Runtime mode cwd handling should mirror runner mode without a runner fallback: typed cwd wins; otherwise pass `readDefaultWorkingDir() || null`.
- Runtime mode cwd placeholder should show the global default working directory, then `(no working directory)`.
- Runtime mode does not expose model, effort, permission, args, env, or system prompt; those come from the default ephemeral runner config above.
- Start button is enabled when the selected mode has the required selection: runner id for runner mode, runtime name for runtime mode.

Design pass required before implementation: update `design/runners-design.pen` for the segmented modal state and confirm the dropdown/cwd spacing. The Pencil file was not open during this planning pass, so this plan does not claim node-level validation.

## Step 7: Chat settings

**Files:** `src/components/SettingsModal.tsx`, `src/lib/settings.ts`

- Add a `Chat` section in Settings.
- Add a `Default runtime` dropdown backed by `runtime_list`.
- Persist as a local setting, for example `runner.default_chat_runtime`.
- On load, validate the stored runtime against `runtime_list`; if it is missing or stale, fall back to the first runtime and do not block opening the modal.
- Do not add default args, model, effort, permission, env, or prompt settings here.

## Step 8: Chat route and header

**Files:** `src/App.tsx`, `src/pages/RunnerChat.tsx`, `src/components/Sidebar.tsx`, `src/components/CommandPalette.tsx`, `src/pages/Runners.tsx`, `src/pages/RunnerDetail.tsx`

- Add route `/chats/:sessionId` pointing at `RunnerChat`.
- Remove `/runners/:handle/chat/:sessionId`.
- Make `RunnerChat` load `chatMeta` first and treat it as the source of truth for title, status, cwd, runtime, and optional runner handle.
- Fetch runner details only when `chatMeta.runner_id` exists.
- Pass `runnerRuntime={chatMeta.agent_runtime}` to `RunnerTerminal` so runtime-only chats still get the claude-code/codex resize behavior.
- Sidebar and command palette should navigate to `/chats/:sessionId` for all direct-session rows after this change.
- Runner Detail and Runners page should also navigate to `/chats/:sessionId` after spawning a runner-template chat.
- Back button behavior:
  - Runner-template row with runner metadata: back to `/runners/:handle`.
  - Runtime-only row: back to `/runners` or the prior location if present.
- Side panel behavior:
  - Runner-template chat: current runner detail panel.
- Runtime-only chat: compact runtime panel showing display name, command, cwd, and no system prompt section.

## Step 9: Tests

**Rust**

- Migration preserves existing runner sessions and permits `runner_id = NULL`.
- `runtime_list` returns `claude-code` and `codex`.
- `session_start_runtime` rejects unknown runtime names.
- `session_start_runtime` applies the same default permission args a new runner would get for the selected runtime.
- Direct-session listing returns runtime-only rows without joining `runners`.
- `session_get` returns archived runtime-only rows for direct URL reload.
- Runtime-only resume reconstructs an ephemeral runner and does not call `runner::get`.
- Startup reattach handles a running runtime-only row.

**Frontend**

- Typecheck catches nullable runner ids across Sidebar, CommandPalette, and RunnerChat.
- Settings Chat default runtime persists and Start Chat uses it when valid.
- Invalid/stale stored default runtime falls back to the first runtime.
- Add focused component tests only if the existing setup already supports them; otherwise rely on typecheck plus manual app verification.

## Step 10: Manual verification

1. Start Chat → Runner mode → existing runner-template chat still spawns and navigates.
2. Start Chat → Runtime mode → `claude-code` with a cwd spawns without creating a runner row.
3. Settings → Chat → Default runtime changes which runtime is preselected in Start Chat Runtime mode.
4. Start Chat → Runtime mode → `codex` with no cwd uses the app/global fallback behavior.
5. Sidebar CHAT row for runtime-only chat shows runtime display name, not `@handle`.
6. Chat header and meta row show runtime display, status, started time, cwd, and no runner handle.
7. Reload `/chats/:sessionId` for a runtime-only chat and reattach successfully.
8. Stop a runtime-only chat, reload, click Resume, and verify it resumes the same row.
9. Delete a runner that has runner-template chats and confirm existing cascade behavior is unchanged.
10. Invalid runtime through the command returns a clear error.

## CI gates

- `pnpm exec tsc --noEmit`
- `pnpm run lint`
- `cargo fmt --check`
- `cargo test --workspace`

Run a targeted Rust test first while iterating on the session manager, then the full workspace test once the null-runner path compiles.
