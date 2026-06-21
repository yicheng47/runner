# 25 — Direct chat runtime picker

> Tracking issue: [#195](https://github.com/yicheng47/runner/issues/195)

## Motivation

Today the Start Chat modal requires picking a **runner** — a
pre-configured template that bundles runtime, command, system prompt,
model, and effort. That makes sense for crews where each runner carries
a tuned persona, but it's overhead for the most common direct-chat
use case: "I want to talk to claude-code in this directory."

The user who hasn't set up runners yet (or who just wants a throwaway
chat) has to navigate Runners → create one → come back to Start Chat.
The one-off case is penalized by the reusable-template design.

The fix: let the Start Chat modal also accept a **runtime + working
directory** directly, without requiring a runner template. The runner
picker stays for users who want personas; a runtime picker sits
alongside it as the faster path. This makes direct chats the "open a
terminal" gesture — pick a runtime, pick a directory, go.

## Scope

### In scope (v1)

- **Runtime picker in StartChatModal.** Two modes, toggled by a
  segmented control or tab pair at the top of the modal:
  1. **Runner** (existing) — dropdown of configured runner
     templates. Same flow as today.
  2. **Runtime** (new) — dropdown of available runtimes
     (`claude-code`, `codex`, etc.), plus the working directory
     picker. No system prompt, no model/effort override — bare
     defaults. Title auto-derives from the runtime name.
- **Backend: `session_start_runtime`.** New Tauri command that
  spawns a direct PTY from a runtime name + optional cwd, without
  requiring a runner row. Internally creates an ephemeral
  in-memory runner config (runtime + command + defaults) and feeds
  it to the existing `spawn_direct` path. The session row stores
  `runner_id = NULL` (or a sentinel) since there's no persisted
  runner template.
- **Schema: nullable `runner_id` on sessions.** Today `runner_id`
  is NOT NULL. Sessions spawned via the runtime picker have no
  runner template to reference. Migration makes `runner_id`
  nullable; the sidebar and chat page handle NULL gracefully
  (show the runtime name instead of the runner handle).
- **Runtime registry.** A hardcoded list of known runtimes with
  their default command and display name:
  ```rust
  [
    ("claude-code", "claude", "Claude Code"),
    ("codex",       "codex",  "Codex"),
  ]
  ```
  Exposed via a `runtime_list` Tauri command so the frontend can
  populate the dropdown. Extensible later to user-defined runtimes.
- **Chat row display.** Sessions without a `runner_id` show the
  runtime display name in the sidebar row and chat header instead
  of `@<handle>`. A small runtime icon/badge distinguishes them
  from runner-based chats.

### Out of scope (deferred)

- **Model / effort / system prompt overrides in runtime mode.**
  v1 uses bare defaults. Adding optional overrides (collapsible
  "Advanced" section) is a follow-up — don't let the runtime
  picker become a runner-creation form.
- **User-defined runtimes.** The registry is hardcoded in v1.
  A settings page for custom runtimes (name + command + args) is
  a follow-up.
- **Auto-detect installed runtimes.** Checking `which claude` /
  `which codex` at startup to filter the dropdown — nice but not
  v1. Show all known runtimes; if the command isn't installed, the
  spawn fails with a clear error.

### Key decisions

1. **Two-mode modal, not a merged form.** Merging runner + runtime
   into one picker creates an awkward hybrid where the user has to
   understand when fields apply. Two distinct modes (tab/segment)
   keep each path simple: runner mode is unchanged; runtime mode
   is two fields. The modal remembers the last-used mode.
2. **Nullable `runner_id`, not a synthetic "default runner" row.**
   The alternative (auto-create a runner row from the runtime
   selection) pollutes the Runners list with throwaway templates
   the user didn't ask for. Nullable FK is cleaner: the session
   stands on its own, and the sidebar just shows the runtime name.
3. **Hardcoded runtime registry, not plugin discovery.** Runtime
   adapters are already hardcoded in `router/runtime.rs`. A
   hardcoded list in a `runtime_list` command mirrors that and
   avoids a premature extension point. When user-defined runtimes
   land, both the router and the registry evolve together.

## Implementation phases

### Phase 1 — backend

- Migration `0009_nullable_runner_id.sql`: make `sessions.runner_id`
  nullable (SQLite: create new table, copy data, rename).
- New `runtime_list` Tauri command returning the known runtimes.
- New `session_start_runtime(runtime: String, cwd: Option<String>,
  cols, rows)` command: validates runtime against the registry,
  constructs an ephemeral `Runner` struct with defaults, calls
  `spawn_direct` with `runner_id = NULL` on the session row.
- Update `session_list_recent_direct` and related queries to handle
  NULL `runner_id` (LEFT JOIN runners, coalesce display name from
  the runtime registry).

### Phase 2 — frontend

- Add segmented control (Runner | Runtime) to `StartChatModal`.
- Runtime mode: runtime dropdown (from `runtime_list`) + cwd picker.
  Title auto-derives from runtime display name.
- Runner mode: unchanged.
- Wire `api.session.startRuntime(runtime, cwd)` to the new command.
- Update `SessionRow` and chat header to render runtime name when
  `runner_id` is null.

### Phase 3 — verification

- Start a chat via Runtime mode with `claude-code` + a cwd → PTY
  spawns, chat works end-to-end.
- Start a chat via Runner mode → unchanged behavior.
- Session row in sidebar shows runtime name for runtime-spawned
  chats, runner handle for runner-spawned chats.
- Chat header shows runtime display name.
- `runner_id = NULL` sessions don't crash any existing query or UI
  surface.
- `pnpm exec tsc --noEmit` clean; `cargo fmt + clippy + test` clean.

## Verification

- [ ] `sessions.runner_id` is nullable; existing sessions unaffected.
- [ ] `runtime_list` returns known runtimes.
- [ ] `session_start_runtime` spawns a PTY without a runner template.
- [ ] StartChatModal has two modes (Runner / Runtime) with the
      correct fields per mode.
- [ ] Runtime-spawned chats show runtime name in sidebar + header.
- [ ] Modal remembers last-used mode across open/close cycles.
- [ ] Invalid runtime name returns a clear error.
- [ ] `tsc --noEmit` clean; `cargo fmt + clippy + test` clean.
