# Fork a chat — implementation plan

Implementation plan for [feature 60 — Fork a chat into a split pane or a new tab](../features/60-fork-chat-to-pane-or-tab.md) ([#398](https://github.com/yicheng47/runner/issues/398)). The spec says what; this file says how, against the GPUI code as surveyed on 2026-08-28. One `codex peer` mission on a task branch off `main`; the brief is derived from §Backend and §App when the mission starts and lives at `docs/impls/archive/gpui-rewrite/briefs/fork-chat.md`.

## Status (2026-08-28)

Planned, not scheduled. Nothing has landed.

## What changed since the spec

- **codex has native fork.** `codex fork <uuid>` is a first-class subcommand in codex 0.150.1, same shape as `codex resume <uuid>`. The spec's phase 3 ("codex investigation") is closed by the CLI itself: codex is in v1 on the native tier next to claude-code.
- **trae is out.** trae shares codex's `resume` shape but has no `fork` subcommand, and Runner is not adding features for trae beyond what it already has. Fork is disabled for trae sessions, and stays so.
- **Two visible icons, no menu rows.** Neither of the spec's entry points survives the GPUI surfaces as written: the sidebar has a per-tab menu (`surfaces/sidebar.rs:3353` — Pin / Rename tab / Archive), not a per-chat one, and the per-pane kebab (`surfaces/panes.rs:1893`) only renders in a grouped tab (`panes.rs:1318`), so a single-pane chat — the common case — would have had no fork affordance at all. Decided 2026-08-28: a `git-fork` icon in two places — (a) on the sidebar tab row, revealed on hover beside the existing ⋯ (`sidebar.rs:2510`), for rows whose tab holds exactly one forkable chat; (b) in the chat header's trailing group before the split button (`panes.rs:254`), acting on the focused pane, which is how grouped tabs fork a specific chat. Both call the same `fork_chat`.
- **One destination: a new tab.** A fork creates a new node (tab) beside the source, always. The spec's split-pane destination is dropped — the layout system caps tabs at three panes (`pane_layout.rs:397`) and the choice added a popover for little gain; anyone who wants the fork beside the source drags or splits after. Decided 2026-08-28.
- **Nothing to draft.** The native fork carries the whole history; there is no handoff prompt, so the spec's "arrives as a draft" line (an Orca carry-over) has no counterpart here. The forked agent comes up at an empty prompt with full context.

## Decisions that bind

1. **Native only.** claude-code `--resume <key> --fork-session`; codex `fork <key>`. No transcript capture, no fallback, no "Copy context".
2. **Runtime-gated by a capability bit, not by a runtime list in the UI.** `RuntimeDefinition` gains `native_fork: bool` (`codex` true, `claude-code` true, `trae` false). Everything above the definition asks the bit: the manager refuses to fork a runtime without it, `DirectSessionEntry` carries `native_fork` (the bit) and `forkable` (the bit plus key presence), and the app reads those two fields — it never matches on runtime names. This is the pattern for every future runtime-specific feature: a feature may be open to some runtimes only, the definition says which, and surfaces disable rather than hide. No feature is forced onto a runtime that lacks the mechanism.
3. **Fork never touches the source.** The source row, its key, its PTY and its conversation file are read only. A fork of a running source is allowed — both CLIs read the source transcript and write a new one.
4. **The fork is an ordinary direct chat afterwards.** New ULID, own row, own key, resumable and archivable like any chat. It copies `project_id`, `runner_id`, `cwd`, `agent_runtime`, `agent_command`, `agent_model`, `agent_effort` verbatim from the source row so it respawns the same engine the source used, and takes the title the app passes (`"<source label> (fork)"`).
5. **No first turn on a fork.** The plan is `resuming: true`, which already suppresses the system prompt and first-turn delivery in `trailing_runtime_args` (`router/runtime.rs:580`) and enables the `resume_failed` heuristic (`session/manager/output.rs:185`) so a fork whose source transcript is gone crashes fast, clears its own key, and warns — the fork row's key, never the source's.
6. **Two icons on existing chrome, one existing placement path.** Both icons are new controls on shipped surfaces, so they were drawn in `design/runner.pen` first (frame `F0ComJ`, §5); the placement reuses the ⌘N new-tab tail. No new surface, no menu, no popover, no side-panel button.

## Surveyed code

- `router/runtime.rs:653` `resume_plan(runtime, prior_key) -> ResumePlan { args, prepend, assigned_key, resuming }` — the fork plan is a sibling.
- `session/manager/spawn.rs:848` `spawn_direct_inner` — fresh direct chat: row insert, claude launch gate, `runtime.spawn`, codex capture context, handle install. `spawn.rs:1153` `resume` — runner resolution for runner-backed vs runtime-only rows (`resolve_runtime_override`, `resolve_runtime_only_resume_runner`). The fork spawn borrows the resolution from `resume` and the spawn body from `spawn_direct_inner`.
- `session/manager/spawn.rs:180` `apply_runtime_args(spec, runner, plan, first_turn, mission_bus_dir)` — composes argv from the plan; unchanged.
- `session/codex_capture.rs:79` `CaptureRequest` / `spawn_capture` — pid-owned rollout detection for a fresh codex spawn; `open_rollout_paths_for_pid` at `:462`.
- `ops/session.rs:238` `DirectSessionEntry` (`resumable` = key present at `:328`); `:750` `session_start_direct_impl`; `:575` `session_rename`.
- `surfaces/sidebar.rs:2438` `render_tab_row(node, layout, members, …)` — the trailing slot (`sidebar_row_trailing_slot`) holds the attention indicator and the ⋯ `IconButton` with `.reveal_on_group_hover("sidebar-row-actions")`; `members` are the tab's `DirectSessionEntry`s, `layout.root.leaves().len()` the pane count.
- `surfaces/panes.rs:173` `render_active_tab` — the header: `title_actions` (tab ⋯ menu, Stop/Resume), `trailing_actions` = split toggle (`:243`) + side-panel toggle (`:256`); `focused_shell` / `layout.focused_session_id()` identify the focused pane.
- `surfaces/start_chat.rs:1030` — the new-tab placement tail after a spawn: `refresh_sessions` → `reload_tabs` → `activate_session` → `sync_active_project_from_active_tab` → `set_route(Chat)` → `ensure_active_tab_attached` → `begin_chat_transition(Starting)`.
- `ui/button.rs` `Button` (variants, `.icon`, `.disabled`, `.tooltip`); `assets.rs` inline SVG icon table (no fork glyph yet).

## Backend

### 0. Verify claude-code key assignment (first task of the mission)

Runner captures a claude-code key by assigning it: fresh spawns pass `--session-id <uuid>` and write that uuid to the row. Whether `--resume <src> --fork-session --session-id <new>` honours `<new>` is unverified. Check headless, in a scratch directory, before writing code:

```sh
A=$(uuidgen | tr A-F a-f); B=$(uuidgen | tr A-F a-f)
claude -p --session-id "$A" --output-format json "Remember the word pineapple. Reply OK."
claude -p --resume "$A" --fork-session --session-id "$B" --output-format json "What was the word?"
```

Read `session_id` from the second JSON envelope and check the answer recalls the word.

- **Honoured** (`session_id == B`): the claude plan carries `--session-id <new>` and `assigned_key: Some(new)`; capture is free, exactly like a fresh spawn. Record the verified claude-code version in the impl log.
- **Not honoured**: the plan leaves `assigned_key: None` and the fork row is captured post-spawn. Claude writes `~/.claude/projects/<encoded cwd>/<uuid>.jsonl` (`router/runtime.rs:757` `claude_code_conversation_exists` already knows the encoding); generalise `codex_capture`'s pid-owned-file scan over a `sessions_root` + file-name pattern so the same watcher serves claude-code, with the source key excluded from candidates. This branch is bigger — if it is the one that applies, land it as its own commit and say so in the handoff.

### 1. Capability + plan (`router/runtime.rs`)

- `RuntimeDefinition { name, display_name, command, native_fork: bool }` — codex `true`, claude-code `true`, trae `false`. `pub fn supports_native_fork(runtime: &str) -> bool` reads the definition (unknown runtime → `false`).
- `pub fn fork_plan(runtime: &str, source_key: &str) -> Option<ResumePlan>`:
  - `claude-code` → `args: ["--resume", source_key, "--fork-session"]` plus `["--session-id", new]` per §0; `prepend: false`; `assigned_key` per §0; `resuming: true`.
  - `codex` → `args: ["fork", source_key]`; `prepend: true`; `assigned_key: None`; `resuming: true`.
  - anything else, or a `source_key` that is not a UUID → `None`.
- `RuntimeCatalogEntry` (`ops/runtime.rs:25`) gains `native_fork: bool` copied from the definition, so Settings → Agents or the runner form can read it later without another plumb. Fix the app-side test constructor at `surfaces/settings/agents.rs`.

### 2. Fork spawn (`session/manager/spawn.rs`)

`pub fn spawn_fork(self: &Arc<Self>, source_session_id: &str, title: Option<String>, cols, rows, app_data_dir, pool, events) -> Result<SpawnedSession>`.

1. Snapshot the source row. Refuse with a plain error when: the row is missing; `mission_id` is set (direct chats only); `archived_at` is set; `agent_session_key` is NULL; `!supports_native_fork(runtime)` where runtime is the effective one (row's `agent_runtime`, else the runner's). Do not refuse on `status == Running`.
2. Resolve the runner the way `resume` does (`spawn.rs:1215`): runner-backed rows load the runner and re-apply the recorded override; runtime-only rows go through `resolve_runtime_only_resume_runner`. Then `resolve_runner_executable`.
3. `plan = fork_plan(runtime, source_key)` (`None` here is a bug — §1 already gated it; return an error, do not panic).
4. `cwd` = source row `cwd`, else the runner's `working_dir`; same "must be a real directory" check as `spawn_direct_inner`.
5. New row: `SessionRowDb::new_running(ulid)` with `project_id`, `runner_id`, `cwd`, `agent_runtime`, `agent_command`, `agent_model`, `agent_effort` copied verbatim from the source row (copy, do not recompute — the fork must respawn the source's engine even if the runner template changed since), `title` from the argument, `agent_session_key = plan.assigned_key`, `last_cols/last_rows` from the initial size. Insert before spawning, as `spawn_direct_inner` does.
6. Spawn body as `spawn_direct_inner` from the row insert onward: `enter_claude_launch_gate`, the two `session_row_exists` checks, `seed_codex_project_trust`, `runtime.spawn`, `update_runtime_metadata`, handle install with `emit_activity: true`. `first_turn: None`; no prompt marker (`codex_capture_prompt_marker` with `None` first turn). Env: `RUNNER_HANDLE` as for direct chats. Extract the shared tail into a helper only if it falls out naturally; a second copy of thirty lines is acceptable over a risky refactor of the fresh-spawn path.
7. Codex capture: build the `CodexCaptureContext` as `spawn_direct_inner` does when `assigned_key` is `None`, plus a new `exclude_key: Option<String>` carrying the source key. The fork process may hold the source rollout open while it copies it, so `open_rollout_paths_for_pid` can return the source; the watcher must skip any rollout whose uuid equals `exclude_key` or the fork row would inherit the source's key and later "resume" the wrong conversation. Fresh spawns pass `None`.
8. The source row is never written. No `session/updated` for the source.

### 3. Op + entry (`ops/session.rs`)

- `pub fn session_fork(state, source_session_id, title: Option<String>, cols, rows) -> Result<SpawnedSession>` → `sessions.spawn_fork(...)`, then `events.emit("session/updated", {session_id: <new>})`. Logged like `session_resume` (`session_fork: source=… new=… cols rows`).
- `DirectSessionEntry` gains `native_fork: bool` (`supports_native_fork(effective runtime)`) and `forkable: bool` (`native_fork && key present && mission_id NULL && archived_at NULL`), both computed in `direct_entry_from_repo` from the row before the key is stripped. Both `session_get` and `session_list_recent_direct` carry them (the list already computes `resumable` the same way).
- No MCP tool, no CLI verb in v1.

### 4. Docs the mission updates

- `docs/arch/arch.md` §5.5 (the paragraph on resume and `agent_session_key`): one paragraph on fork — the plan, the copied columns, capture exclusion, and that trae is excluded by `native_fork`.
- Nothing else; the landing commit moves spec 60 to `docs/features/archive/` and updates this file.

## App

### 5. Design (`design/runner.pen`, done 2026-08-28)

Spec frame `F0ComJ` — "Spec — Fork chat (60) · sidebar row icon + chat header icon", chat-states band right of `FvOH8`. Left, `sidebar_mock`: a project group with four tab rows — a hovered single-chat row showing `git-fork` 12 px beside ⋯ 14 px in the trailing slot; the result row `@coder (fork)` active; a hovered grouped row (`columns-2`) showing ⋯ only; a terminal row showing nothing. Right, two header mocks copied from the shipped header design (`fp5s9/WMmAo`): `header_enabled` with `forkIcon` (15 px, `#9A9BA5`) inserted before `splitIcon`, and `header_disabled` (`#5A5C66` at 50 %). Below them, `confirm_fork` (`ygQBi`): an instance of `cmp/ConfirmDialog` (`tK5Xb`) with a `git-fork` header icon, title "Fork chat?", the body text from §8 and an accent "Fork" button. Captions carry the tooltips. The brief cites these ids.

### 6. Sidebar row icon (`surfaces/sidebar.rs`)

In `render_tab_row`'s non-shortcut trailing slot, before the ⋯ button: an `IconButton` `sidebar-tab-fork-<node id>` with `git-fork.svg` (new inline lucide SVG const in `assets.rs`), `IconButtonSize::Xs`, `.reveal_on_group_hover("sidebar-row-actions")`, `.stop_click_propagation(true)`, tooltip "Fork chat", rendered only when `sidebar_fork_target(&layout, &members) -> Option<&DirectSessionEntry>` returns `Some`: the tab has exactly one pane, its member is not a shell, and `member.forkable`. A hover affordance has no disabled state — when the rule fails the icon is simply absent (grouped tabs fork from the header, where the focused pane is unambiguous). Press → `fork_chat(session_id)` on the shell entity, the same way the ⋯ button reaches `open_tab_menu`. The inline-rename row and the shortcut-pill mode are unchanged.

### 7. Header icon (`surfaces/panes.rs`)

In `render_active_tab`, `trailing_actions` becomes fork → split → panel. The fork control is `IconButton::new("fork-chat", "git-fork.svg")` sized like the split toggle, built from `header_fork_state(focused: Option<&DirectSessionEntry>) -> HeaderForkState { Enabled, Disabled(&'static str), Hidden }` (pure, next to `side_panel_open`, unit-tested):

- `Hidden` when the focused pane is empty or a shell, or the tab is `focused_secondary`.
- `Disabled("Forking needs claude-code or codex")` when `!entry.native_fork`.
- `Disabled("No session key captured yet")` when `!entry.forkable`.
- `Enabled` otherwise, tooltip "Fork chat into a new tab"; disabled without a caption while a `Starting` transition exists for a fork of this session.

The disabled tooltip is the caption text. Press → `fork_chat(focused session id)`.

### 8. Fork action (`surfaces/chat.rs`)

`pub(crate) fn fork_chat(&mut self, session_id: &str, window, cx)`:

1. `entry = self.session_entry(session_id, cx).cloned()`; bail unless `entry.forkable` and not a shell.
2. Open a confirm dialog first — `fork_confirm: Option<ForkConfirm { session_id, pending }>` on the shell, rendered with the existing `ConfirmDialog` (`ui/overlay.rs:345`, the same helper as `render_terminal_close_confirm` at `panes.rs:856`): title "Fork chat?", body `Start a new chat from <label> with its full conversation history. The fork opens in a new tab; the original chat is not changed.`, confirm "Fork", pending "Forking…", non-destructive (accent confirm button). Cancel / Esc / backdrop clear it and nothing else happens. The steps below run from the confirm handler.
3. `title = Some(format!("{} (fork)", session_label(&entry)))` (`surfaces/sidebar.rs:3925`).
4. `spawned = ops::session::session_fork(core, session_id, title, Some(INITIAL_COLS), Some(INITIAL_ROWS))?`.
5. The ⌘N tail from `start_chat.rs:1030`: `refresh_sessions` → `reload_tabs` → `activate_session(&spawned.id)` → `sync_active_project_from_active_tab` → `set_route(Chat)` → `ensure_active_tab_attached` → `begin_chat_transition(&spawned.id, Starting)` → `mark_active_tab_viewed` → `sync_active_chat_detail`.
6. Errors go to `chat_error` (the banner under the header), as `split_pane` does; the dialog closes either way.

The source pane is untouched: no focus change, no rename, no transition. The new tab is a fresh single-pane node created by `ensure_active_sessions` on reload, exactly like a ⌘N chat.

### 9. Not in v1

Side-panel button, split-pane destination, pane-kebab or sidebar-menu rows, command-palette entry, keyboard shortcut, MCP `session_fork`, forking from the mission workspace, cross-runtime forks, cwd picker.

## Tests

- `router/runtime.rs`: `fork_plan` for claude-code (args order, `resuming`, key per §0), codex (`prepend`, `assigned_key: None`), trae → `None`, non-UUID source → `None`; `supports_native_fork` for the three runtimes and an unknown name.
- `session/manager/tests.rs` on the `FakeRuntime` harness: fork of a claude-code direct row records argv `… --resume <src> --fork-session …` and a new row with the copied columns and a different id; fork of a codex row records `fork <src>` ahead of the runner's args and spawns the capture context with `exclude_key == src`; the source row is byte-identical before and after (`SessionRowDb` equality); refusals for a mission row, an archived row, a NULL-key row, a trae row; a running source still forks.
- `session/codex_capture.rs`: a candidate whose uuid equals `exclude_key` is skipped even when it is the only pid-owned rollout.
- `ops/session.rs`: `native_fork` / `forkable` for a keyed claude/codex direct row (true/true), trae (false/false), NULL key (true/false), mission and archived rows (false `forkable`).
- `surfaces/chat.rs` or `panes.rs`: the confirm state — ⑂ sets `fork_confirm`, cancel clears it without a spawn, confirm calls `session_fork` once. `surfaces/panes.rs`: `header_fork_state` for claude/codex with key (Enabled), trae (Disabled, runtime caption), NULL key (Disabled, key caption), shell / empty (Hidden). `surfaces/sidebar.rs`: `sidebar_fork_target` is `Some` only for a one-pane tab whose chat is forkable — `None` for grouped tabs, shells, NULL-key and trae chats.
- Existing assertions extended, never weakened.

## Verification (Jason, before the PR)

1. claude-code chat with a few turns → hover its sidebar row → ⑂ → "Fork chat?" dialog; Cancel does nothing; ⑂ again → Fork → a new tab titled `<label> (fork)` opens and is active; asking the fork about the earlier turns gets the right answer; the source tab is unchanged and its chat keeps answering.
2. Same chat → header ⑂: same result. In a two-pane tab, the header ⑂ forks the focused pane and the sidebar row shows no ⑂.
3. codex chat → either icon: the new tab's side panel shows its own `session_key` within ~10 s, different from the source's; stop and resume the fork → it continues the forked conversation, not the source's.
4. trae chat → header ⑂ disabled with the runtime tooltip, sidebar row without ⑂. A codex chat in its first seconds → header ⑂ disabled with the key tooltip, then enabled once the key lands.
5. Shell pane → no header ⑂, no sidebar ⑂. Archive the source → fork keeps working; archive the fork → source untouched.

## Landing

Task branch off `main` → working-tree review by the crew's reviewer → Jason smoke-tests §Verification → PR → `Rust / macOS` → merge → `docs(fork-chat)` landing commit that records the outcome here (including which §0 branch applied), moves spec 60 to `docs/features/archive/`, and adds the timeline row in `docs/impls/gpui-rewrite/README.md`. Standing GPUI rules from that record apply; crews do not launch the app.
