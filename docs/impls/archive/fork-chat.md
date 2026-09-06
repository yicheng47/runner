# Fork a chat — implementation plan

Implementation plan for [feature 60 — Fork a chat into a split pane or a new tab](../../features/archive/60-fork-chat-to-pane-or-tab.md) ([#398](https://github.com/yicheng47/runner/issues/398)). The spec says what; this file says how, against the GPUI code as surveyed on 2026-08-28. One `codex peer` mission on a task branch off `main`; the brief is derived from §Backend and §App when the mission starts and lives at `docs/impls/archive/gpui-rewrite/briefs/fork-chat.md`.

## Status

Shipped 2026-09-01 in [#460](https://github.com/yicheng47/runner/pull/460) (`4b82c82`); [#398](https://github.com/yicheng47/runner/issues/398) closed the same day. Archived 2026-09-05.

## Implementation log

- 2026-09-01: the mission author verified on claude-code 2.1.250 that the headless `--resume <source> --fork-session --session-id <new>` form honors `<new>` and carries source history. The reviewer then verified on 2.1.252 that an interactive fork is copy-on-write: the new JSONL does not exist until the first submitted turn.
- 2026-09-01: codex-cli 0.152.0 was also verified copy-on-write: an interactive `codex fork <source>` held no new rollout or file descriptor through 20 seconds idle, and a positional prompt only prefilled the composer. `codex exec fork <source> --json <note>` emitted the new key in its first `thread.started` event, completed the note turn, materialized the rollout immediately, and preserved the source history.
- 2026-09-01: decision 5 was amended to a two-step fork for both runtimes: a headless fork-note turn materializes the conversation and yields its key, then the visible TUI starts through the ordinary resume path. This removes both the unused-fork loss and Codex's 30-second post-spawn capture race without a schema change or capture generalization.
- 2026-09-01: Jason's smoke pass found that Codex exec-fork applies a separate repository gate even after Runner pre-seeds TUI project trust, so phase 1 now carries `--skip-git-repo-check`; the operator already selected the source chat's exact cwd. The same pass found the sidebar hover fork icon overlapping the Starting indicator, and Jason moved that affordance into the tab's ⋯ menu. The header icon remains unchanged.
- 2026-09-01: initial timing probes put completed Claude and Codex materializer turns at roughly 7–15 seconds; `--strict-mcp-config`, minimal reasoning, and empty MCP overrides produced no consistent wall-time win. `spawn_fork` permanently logs the launch-gate and total duration for every real fork, Codex's materialization and resume-spawn durations, and Claude's direct-spawn duration with `materialize_ms=not-applicable`.
- 2026-09-01: V5 removed Codex's self-inflicted completed-turn wait. Two clean-environment probes on codex-cli 0.152.0 found `thread.started` and the keyed rollout at 0.15/0.17 seconds and 0.064/0.064 seconds from process start: the rollout appears at thread creation, effectively alongside `thread.started`, rather than a second later at prompt submission. The coder's mandatory production-shaped probe then killed the exec process group with SIGKILL after the complete lineage-bearing `session_meta` landed and resumed that exact key through the real `codex resume` TUI; the fork answered `pineapple`, a marker present only in the source history. The rollout's creation-time lineage is therefore sufficient before any reply or `turn.completed`. Codex now polls for a newline-terminated `session_meta` whose id is the new key and whose `forked_from_id` is the source key, bounded at five seconds, then kills and reaps the exec group immediately and proceeds to phase 2.
- 2026-09-01: V6 extended early termination to claude-code 2.1.252. The reviewer measured the keyed file at 2.58–3.30 seconds, copied history at 3.43 seconds, and the exact injected note at 3.44 seconds, roughly 9.5 seconds before the reply. The coder then ran three production-shaped repetitions without a model override: the file appeared at 2.08–3.32 seconds and the exact user note at 2.20–3.44 seconds with the materializer still alive; every process group exited within the two-second SIGTERM grace, every resulting JSONL remained fully parseable with the note keyed to the assigned session, Keychain-backed auth and `.claude.json` stayed valid after every termination, and a subsequent authenticated resume recovered the source-only marker `mango`. Claude therefore polls for that exact keyed note with every current JSONL entry complete, uses SIGTERM before SIGKILL escalation, and accepts the visible note with no assistant reply.
- 2026-09-01: V7 removed the phase-1 model overrides from both runtimes. A Codex fork from a Sol source persisted the materializer's Luna override into `thread_settings_applied`, causing the real resumed chat to run on Luna; with no override, `session_meta` and settings both inherit Sol. Codex also writes its injected note only 8.5 seconds after `task_started`, beyond the two-second cutoff, so its fast path remains note-less: title-only provenance plus the standard "Conversation interrupted" banner. V7 retained Claude's exact note because it landed about 0.14 seconds after its file and well before the reply; V8 supersedes that path.
- 2026-09-01: Jason's V8 decision returns Claude to a direct TUI fork: one normal guarded spawn with `--resume <source> --fork-session --session-id <new>`, `resuming: true`, and no positional turn. Claude's headless phase was a second full CLI boot costing about 3.3 seconds, and its injected note was the only product of that wait; speed and note-less UX symmetry with Codex now outrank untouched-fork durability. Jason explicitly accepts that a Claude fork which is never typed into has no conversation file: manual Stop → Resume cleanly degrades to a fresh empty chat, while quitting with it open makes resume-on-launch report the conversation unavailable. This is accepted, do not fix with fork-origin state or recovery machinery; re-fork the source. Once any turn is submitted, the Claude fork is durable. Codex keeps the fast two-step materializer and is immune.

## What changed since the spec

- **codex has native fork.** `codex fork <uuid>` is a first-class subcommand in codex 0.150.1, same shape as `codex resume <uuid>`. The spec's phase 3 ("codex investigation") is closed by the CLI itself: codex is in v1 on the native tier next to claude-code.
- **Native TUI forks are copy-on-write, with runtime-specific launch paths.** Claude starts the visible TUI directly with an assigned fork key and materializes only after the user submits a turn. Codex first runs `exec fork <source> --json --skip-git-repo-check <note>`, reads `thread.started`, waits for complete keyed lineage, kills the exec process group, and opens the TUI as a plain resume. Neither runtime shows an injected note; the `"(fork)"` title is the provenance marker.
- **trae is out.** trae shares codex's `resume` shape but has no `fork` subcommand, and Runner is not adding features for trae beyond what it already has. Fork is disabled for trae sessions, and stays so.
- **One header icon plus one sidebar menu row.** The 2026-08-28 plan put visible `git-fork` icons in the sidebar hover chrome and the chat header. Jason superseded the sidebar half on 2026-09-01 after the hover icon overlapped the Starting/attention indicator: single-chat non-shell tabs now carry a "Fork chat" row in their ⋯ menu, disabled with the capability/key caption when unavailable. The chat-header icon stays before the split button and acts on the focused pane. Both call the same `fork_chat`.
- **One destination: a new tab.** A fork creates a new node (tab) beside the source, always. The spec's split-pane destination is dropped — the layout system caps tabs at three panes (`pane_layout.rs:397`) and the choice added a popover for little gain; anyone who wants the fork beside the source drags or splits after. Decided 2026-08-28.
- **Nothing to draft.** The native fork carries the whole history; there is no handoff prompt, so the spec's "arrives as a draft" line (an Orca carry-over) has no counterpart here. The forked agent comes up at an empty prompt with full context.

## Decisions that bind

1. **Native only.** claude-code `--resume <key> --fork-session --session-id <new>`; Codex `exec fork <key> --json --skip-git-repo-check`. No transcript capture, no fallback, no "Copy context".
2. **Runtime-gated by a capability bit, not by a runtime list in the UI.** `RuntimeDefinition` gains `native_fork: bool` (`codex` true, `claude-code` true, `trae` false). Everything above the definition asks the bit: the manager refuses to fork a runtime without it, `DirectSessionEntry` carries `native_fork` (the bit) and `forkable` (the bit plus key presence), and the app reads those two fields — it never matches on runtime names. This is the pattern for every future runtime-specific feature: a feature may be open to some runtimes only, the definition says which, and surfaces disable rather than hide. No feature is forced onto a runtime that lacks the mechanism.
3. **Fork never touches the source.** The source row, its key, its PTY and its conversation file are read only. A fork of a running source is allowed — both CLIs read the source transcript and write a new one.
4. **The fork is an ordinary direct chat afterwards.** New ULID, own row, own key, resumable and archivable like any chat. It copies `project_id`, `runner_id`, `cwd`, `agent_runtime`, `agent_command`, `agent_model`, `agent_effort` verbatim from the source row so it respawns the same engine the source used, and takes the title the app passes (`"<source label> (fork)"`).
5. **Runtime-owned launch, no visible fork note.** Claude launches its visible PTY directly with `resuming: true`, an assigned new key, and no first turn; this suppresses the runner persona and system prompt. Codex sends a pure provenance note only to its headless materializer, reads `thread.started`, waits for complete keyed lineage, kills the process before the note reaches the transcript, then opens the visible PTY through the existing resume path with `resuming: true`. Provenance on both surfaces is the `"(fork)"` title. Jason accepts Claude's untouched-fork degradation described in the implementation log; do not add recovery machinery, and recover by re-forking the source.
6. **Header icon plus sidebar menu row, one existing placement path.** The header control remains the `git-fork` icon drawn in `design/runner.pen` frame `F0ComJ`; the sidebar affordance is the 2026-09-01 human-ruling menu row described in §6. The placement reuses the ⌘N new-tab tail. No new surface, popover, or side-panel button.

## Surveyed code

- `router/runtime.rs:653` `resume_plan(runtime, prior_key) -> ResumePlan { args, prepend, assigned_key, resuming }` — Claude's direct fork uses this plan shape; Codex's materialized fork returns to the ordinary resume path for its visible PTY.
- `session/manager/spawn.rs:848` `spawn_direct_inner` — fresh direct chat row/env construction. `spawn.rs:1153` `resume` — runner resolution for runner-backed vs runtime-only rows (`resolve_runtime_override`, `resolve_runtime_only_resume_runner`) and the guarded plain-resume spawn. Fork borrows the direct-spawn body for Claude and the resume path for Codex.
- `session/manager/spawn.rs:180` `apply_runtime_args(spec, runner, plan, first_turn, mission_bus_dir)` — composes argv from the plan; unchanged.
- `session/codex_capture.rs:79` `CaptureRequest` / `spawn_capture` remains the pid-owned rollout detector for ordinary fresh Codex spawns; fork does not use or generalize it.
- `ops/session.rs:238` `DirectSessionEntry` (`resumable` = key present at `:328`); `:750` `session_start_direct_impl`; `:575` `session_rename`.
- `surfaces/sidebar.rs:2438` `render_tab_row(node, layout, members, …)` — the trailing slot (`sidebar_row_trailing_slot`) holds the attention indicator and the ⋯ `IconButton`; `open_tab_menu` / `tab_menu_entries` build the per-tab menu from the tab's `DirectSessionEntry` members and pane count.
- `surfaces/panes.rs:173` `render_active_tab` — the header: `title_actions` (tab ⋯ menu, Stop/Resume), `trailing_actions` = split toggle (`:243`) + side-panel toggle (`:256`); `focused_shell` / `layout.focused_session_id()` identify the focused pane.
- `surfaces/start_chat.rs:1030` — the new-tab placement tail after a spawn: `refresh_sessions` → `reload_tabs` → `activate_session` → `sync_active_project_from_active_tab` → `set_route(Chat)` → `ensure_active_tab_attached` → `begin_chat_transition(Starting)`.
- `ui/button.rs` `Button` (variants, `.icon`, `.disabled`, `.tooltip`); `assets.rs` inline SVG icon table (no fork glyph yet).

## Backend

### 0. Verify native fork persistence (completed 2026-09-01)

Runner captures a claude-code key by assigning it: fresh spawns pass `--session-id <uuid>` and write that uuid to the row. The mission author verified the assigned fork id and source-history carry headlessly on claude-code 2.1.250:

```sh
A=$(uuidgen | tr A-F a-f); B=$(uuidgen | tr A-F a-f)
claude -p --session-id "$A" --output-format json "Remember the word pineapple. Reply OK."
claude -p --resume "$A" --fork-session --session-id "$B" --output-format json "What was the word?"
```

The second envelope reported `session_id == B` and recalled the word, so Runner keeps assigning the Claude fork key. Subsequent probes established that both interactive runtimes are copy-on-write and require a submitted turn to materialize the new conversation. V8 deliberately returns Claude to this original direct-TUI honored branch and accepts the untouched-fork edge; Codex retains a headless materialization step.

### 1. Capability + plan (`router/runtime.rs`)

- `RuntimeDefinition { name, display_name, command, native_fork: bool }` — codex `true`, claude-code `true`, trae `false`. `pub fn supports_native_fork(runtime: &str) -> bool` reads the definition (unknown runtime → `false`).
- `pub fn fork_plan(runtime: &str, source_key: &str, source_label: &str) -> Option<ForkPlan>` returns one of two runtime-owned plan shapes so no caller above the definition matches runtime names:
  - `claude-code` → `ForkPlan::Direct(ResumePlan)` with `--resume <source> --fork-session --session-id <new>`, `assigned_key: Some(new)`, `prepend: false`, and `resuming: true`. Normal runner args and model/effort settings remain; there is no `-p`, positional prompt, or persona replay.
  - `codex` → `ForkPlan::Headless` with `exec fork <source> --json --skip-git-repo-check "This chat was forked from '<source label>'."` and the source key used to validate the first `thread.started` lineage. The exec-fork command rejects the TUI-only `--ask-for-approval` and `--sandbox` flags carried by default Codex runners, and the interrupted materialization does not need them. Exec's separate repository gate is skipped because the operator already selected this exact cwd for the source chat; phase 2 still runs `seed_codex_project_trust` through the ordinary resume path before the TUI spawns.
  - anything else, or a `source_key` that is not a UUID → `None`.
- `RuntimeCatalogEntry` (`ops/runtime.rs:25`) gains `native_fork: bool` copied from the definition, so Settings → Agents or the runner form can read it later without another plumb. Fix the app-side test constructor at `surfaces/settings/agents.rs`.

### 2. Fork spawn (`session/manager/spawn.rs`)

`pub fn spawn_fork(self: &Arc<Self>, source_session_id: &str, title: Option<String>, cols, rows, app_data_dir, pool, events) -> Result<SpawnedSession>`.

1. Snapshot the source row. Refuse with a plain error when: the row is missing; `mission_id` is set (direct chats only); `archived_at` is set; `agent_session_key` is NULL; `!supports_native_fork(runtime)` where runtime is the effective one (row's `agent_runtime`, else the runner's). Do not refuse on `status == Running`.
2. Resolve the runner the way `resume` does (`spawn.rs:1215`): runner-backed rows load the runner and re-apply the recorded override; runtime-only rows go through `resolve_runtime_only_resume_runner`. Then `resolve_runner_executable`.
3. Build `plan = fork_plan(runtime, source_key, source_label)` (`None` here is a bug — §1 already gated it; return an error, do not panic). The Claude direct plan carries no first turn; the Codex headless plan owns its materializer note. Neither includes `runner.system_prompt`.
4. `cwd` = source row `cwd`, else the runner's `working_dir`; same "must be a real directory" check as `spawn_direct_inner`.
5. New row: `SessionRowDb::new_running(ulid)` with `project_id`, `runner_id`, `cwd`, `agent_runtime`, `agent_command`, `agent_model`, `agent_effort` copied verbatim from the source row (copy, do not recompute — the fork must respawn the source's engine even if the runner template changed since), normalized `title` from the argument, the assigned Claude key or NULL while Codex reports its key, and `last_cols/last_rows` from the initial size. Insert and emit `session/updated` before the runtime work so the tab exists in the existing spawning state.
6. Branch on the runtime-owned plan. Claude applies its direct `ResumePlan` to the normal spawn spec, enters the ordinary Claude launch gate, spawns one interactive PTY, installs the handle/forwarder, and emits the normal spawned/activity events. Codex enters the no-op gate, then runs its headless plan with the same resolved command, env, PATH, cwd, and terminal size as a normal direct spawn while omitting incompatible TUI-only permission/sandbox args. Do not hold a pool connection during either spawn. Codex drains stdout/stderr concurrently, polls every 25 ms, and keeps the 120-second outer bound: the first non-empty JSONL event must be `thread.started` with a UUID `thread_id`; the rollout root comes from the exact spawn environment's `CODEX_HOME` or the inherited Codex home; and a five-second inner bound requires the keyed rollout's first line to be non-empty, newline-terminated, valid `session_meta` JSON whose `payload.id` equals the new key and whose `payload.forked_from_id` equals the source key. A nonzero exit before that lineage fails; once ready Runner kills and reaps the exec process group without waiting for the note or reply.
7. Codex persists the acquired key, transitions the temporary running row to stopped, then calls the real no-fresh-fallback resume path by id. Its visible PTY is therefore a plain resume with the normal claim guard, strict conversation check, row update, handle install, and `resuming: true`; it does not use Codex's post-spawn capture watcher. Claude is already running from step 6 with its assigned key.
8. A Claude direct-spawn error, Codex materialization/key-persistence error, or Codex phase-2 resume error removes the fork from every node and deletes its row, emits `session/updated`, and returns the error for the app banner. The source row is never written and receives no update event.

### 3. Op + entry (`ops/session.rs`)

- `pub fn session_fork(state, source_session_id, title: Option<String>, cols, rows) -> Result<SpawnedSession>` → `sessions.spawn_fork(...)`, then `events.emit("session/updated", {session_id: <new>})`. Logged like `session_resume` (`session_fork: source=… new=… cols rows`).
- `DirectSessionEntry` gains `native_fork: bool` (`supports_native_fork(effective runtime)`) and `forkable: bool` (`native_fork && key present && mission_id NULL && archived_at NULL`), both computed in `direct_entry_from_repo` from the row before the key is stripped. Both `session_get` and `session_list_recent_direct` carry them (the list already computes `resumable` the same way).
- No MCP tool, no CLI verb in v1.

### 4. Docs the mission updates

- `docs/arch/arch.md` §5.5 (the paragraph on resume and `agent_session_key`): one paragraph on the direct Claude / two-step Codex fork paths, copied columns, failure rollback, and TRAE exclusion through `native_fork`.
- Nothing else; the landing commit moves spec 60 to `docs/features/archive/` and updates this file.

## App

### 5. Design (`design/runner.pen`, done 2026-08-28)

Spec frame `F0ComJ` — "Spec — Fork chat (60) · sidebar row icon + chat header icon", chat-states band right of `FvOH8`. Left, `sidebar_mock`: a project group with four tab rows — a hovered single-chat row showing `git-fork` 12 px beside ⋯ 14 px in the trailing slot; the result row `@coder (fork)` active; a hovered grouped row (`columns-2`) showing ⋯ only; a terminal row showing nothing. Right, two header mocks copied from the shipped header design (`fp5s9/WMmAo`): `header_enabled` with `forkIcon` (15 px, `#9A9BA5`) inserted before `splitIcon`, and `header_disabled` (`#5A5C66` at 50 %). Below them, `confirm_fork` (`ygQBi`): an instance of `cmp/ConfirmDialog` (`tK5Xb`) with a `git-fork` header icon, title "Fork chat?", the body text from §8 and an accent "Fork" button. Captions carry the tooltips. The 2026-09-01 human ruling in §6 supersedes the sidebar-hover portion; `F0ComJ` still draws the old hover icon, and Jason owns that canvas update. The header and dialog portions remain current.

### 6. Sidebar menu row (`surfaces/sidebar.rs`)

Jason's 2026-09-01 smoke-test ruling removes the sidebar hover icon because it occupied the Starting/attention indicator's trailing position. `tab_menu_entries` adds a `git-fork.svg` "Fork chat" row only when `sidebar_fork_menu_target(&layout, &members)` finds a single-pane non-shell chat. The row is enabled when `forkable`; `!native_fork` disables it with "Forking needs claude-code or codex", and a missing key disables it with "No session key captured yet". Grouped and terminal tabs omit it. Activation dispatches `SidebarMenuAction::ForkChat(session_id)` to the shared `fork_chat`; the sidebar trailing slot returns to its pre-feature attention + ⋯ layout.

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
4. Run `ops::session::session_fork(core, session_id, title, Some(INITIAL_COLS), Some(INITIAL_ROWS))` on GPUI's background executor. The backend call is synchronous; Codex materialization can take up to 120 seconds and Claude's direct runtime spawn can also block, so it must never run on the UI thread.
5. The backend emits `session/fork-started { source_session_id, session_id }` immediately after inserting the temporary row. The app closes the dialog, tracks the source/fork pair, and runs the placement portion of the ⌘N tail immediately: `refresh_sessions` → `reload_tabs` → `activate_session(session_id)` → `sync_active_project_from_active_tab` → `set_route(Chat)` → `mark_active_tab_viewed` → `sync_active_chat_detail`. While the pair is tracked, the new pane renders the existing `Starting` state labeled "Forking chat…" without trying to attach a PTY; the source remains interactive, and both source and fork controls reject another fork.
6. On `session/spawned`, clear the materialization tracking, attach the PTY, and start the ordinary `Starting` transition. The background call's success tail repeats placement idempotently for event-order safety. On error, clear tracking, refresh sessions and tabs after backend rollback, reactivate the source, and put the visible error in `chat_error`.

The source pane is untouched: no focus change, no rename, no transition. The new tab is a fresh single-pane node created by `ensure_active_sessions` on reload, exactly like a ⌘N chat.

### 9. Not in v1

Side-panel button, split-pane destination, pane-kebab rows, command-palette entry, keyboard shortcut, MCP `session_fork`, forking from the mission workspace, cross-runtime forks, cwd picker.

## Tests

- `router/runtime.rs`: `fork_plan` for claude-code (`Direct`, exact TUI fork args, assigned key, `resuming: true`, no positional prompt) and Codex (`Headless`, `exec fork`, JSON mode, repository-gate skip, no model override, internal note and source lineage), TRAE → `None`, non-UUID source → `None`; `supports_native_fork` for the three runtimes and an unknown name; capability bits match plan availability.
- `session/manager/tests.rs` on the `FakeRuntime` plus Codex executable materializer fixtures: Claude performs one direct TUI spawn with exact runner/fork/model/effort/settings argv, same cwd/env, assigned key persisted, copied columns, different id, source row byte-identical, and no positional prompt or persona; Codex's phase-1 argv carries its runtime-owned note with no system prompt/model override/TUI-only flags, uses the same cwd/env including custom `CODEX_HOME`, persists the acquired key, then phase 2 is a plain resume with no capture context. Codex terminates a ten-second materializer as soon as its complete keyed-and-sourced `session_meta` appears, while empty, partial, malformed, or wrong-lineage files are not ready; missing rollout, missing first event, nonzero, and timeout errors remain materializer failures; failed materialization removes the repaired tab and row; refusals for missing, mission, archived, NULL-key, TRAE, and shell rows; a running source still forks.
- `ops/session.rs`: `native_fork` / `forkable` for a keyed claude/codex direct row (true/true), trae (false/false), NULL key (true/false), mission and archived rows (false `forkable`).
- `surfaces/chat.rs` or `panes.rs`: the confirm state — ⑂ sets `fork_confirm`, cancel clears it without a spawn, confirm calls `session_fork` once; `session/fork-started` moves the busy state from the dialog to the source/fork pair, and only the fork destination renders as materializing with "Forking chat…" before the ordinary "Starting chat…" phase. `surfaces/panes.rs`: `header_fork_state` for claude/codex with key (Enabled), trae (Disabled, runtime caption), NULL key (Disabled, key caption), shell / empty (Hidden). `surfaces/sidebar.rs`: `sidebar_fork_menu_target` returns enabled, runtime-disabled, and key-disabled single-chat rows, while grouped and shell tabs omit the menu action.
- Existing assertions extended, never weakened.

## Verification (Jason, before the PR)

1. claude-code chat with a few turns → sidebar row ⋯ → Fork chat → "Fork chat?" dialog; Cancel does nothing; open it again → Fork → a new tab titled `<label> (fork)` opens and is active, showing the existing fork/Starting state around the single direct TUI spawn; no fork note appears; asking the fork about the earlier turns gets the right answer; the source tab is unchanged and its chat keeps answering.
2. Same chat → header ⑂: same result. In a two-pane tab, the header ⑂ forks the focused pane and the sidebar ⋯ menu has no Fork chat row.
3. codex chat → header icon or sidebar menu row: after materialization the new tab shows Codex's accepted standard "Conversation interrupted" banner and its side panel shows its own `session_key`, different from the source's; ask about source history, then stop and resume the fork → it continues the forked conversation, not the source's.
4. trae chat → header ⑂ disabled with the runtime tooltip, sidebar ⋯ → Fork chat disabled with the runtime caption. A codex chat in its first seconds → both controls disabled with the key caption, then enabled once the key lands.
5. Shell pane → no header ⑂ and no Fork chat menu row. Archive the source → fork keeps working; archive the fork → source untouched.
6. Codex: fork a chat, type nothing in the new TUI, Stop, then Resume — the forked history returns and the fork key remains distinct from the source. Claude: repeat the untouched fork and verify the accepted degradation is clean rather than a crash — manual Stop → Resume opens a fresh empty chat; if the app is quit with the untouched fork open, resume-on-launch reports the conversation unavailable. Recover by re-forking the source; once anything is typed, Stop → Resume must retain the forked conversation.
7. Start a Codex chat from a non-git cwd, then fork it — phase 1 clears the repository gate, the new tab reaches its own TUI, and the forked history is present.

## Landing

Task branch off `main` → working-tree review by the crew's reviewer → Jason smoke-tests §Verification → PR → `Rust / macOS` → merge → `docs(fork-chat)` landing commit that records the outcome here (including which §0 branch applied), moves spec 60 to `docs/features/archive/`, and adds the timeline row in `docs/impls/gpui-rewrite/README.md`. Standing GPUI rules from that record apply; crews do not launch the app.
