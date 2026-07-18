# 21 — Import native agent sessions into a project

> Tracking issue: [#176](https://github.com/yicheng47/runner/issues/176)

> **Pivot note:** this doc originally specced "detect and resume existing sessions in the Start Chat modal". With Projects landed (feature 40, #292), the feature pivoted to a project-level **import** operation — feature 40 explicitly deferred a "group existing chats by cwd" importer as a follow-up, and this is that follow-up. The old detect-and-resume shape (per-spawn picker in the Start Chat modal) is superseded.

## Motivation

A lot of Runner's target users already use `claude-code` and `codex` directly in their terminal *before* they install Runner. Their real conversation history lives in the agents' native on-disk stores, invisible to Runner. Today the workaround is to keep using the agent CLI outside Runner — exactly the split-attention problem Runner is trying to solve.

A project (feature 40) binds a working directory, which makes it the natural anchor: **Import sessions** on a project scans the native session stores of each runtime for sessions recorded under the project's cwd and turns the selected ones into real Runner chats nested under the project. Imported chats appear in the sidebar with a title and timestamp like any stopped chat, and opening one goes through the existing resume path (`claude --resume <uuid>` / `codex resume <uuid>`), which repaints the prior conversation in the terminal.

Why import beats the original detect-and-resume shape:

- **Durable, browsable rows** — an imported session is a first-class chat (renameable, archivable, pinnable, nested under its project), not a one-shot spawn flag that leaves no trace in the sidebar.
- **One bulk operation** — backfill months of native history in one modal pass, instead of re-picking per spawn in Start Chat.
- **Nearly free on the backend** — resume for runner-less rows already works: `session_resume` → `resume_plan` reads `agent_runtime` + `agent_session_key` off the row (`session/manager/spawn.rs`), and migration 0007 made runtime-only sessions (no runner template) a supported shape. Import is row insertion; every downstream flow already exists.

## Scope

### In scope (v1)

- **Entry point.** "Import sessions…" in the project row's context menu (kebab), opening an import modal. Projects only — no project, no import surface.
- **Scan.** Read both runtimes' native stores, filter to sessions whose recorded cwd matches the project's cwd (canonicalized):
  - **claude-code:** `~/.claude/projects/<encoded-cwd>/<uuid>.jsonl`. Encoded-cwd is the absolute path with path separators/dots replaced by `-`; encode the project cwd and read just that directory.
  - **codex:** `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl` (date-partitioned). First line is a `session_meta` envelope with `payload.id` and `payload.cwd` — the same format `codex_capture.rs` already parses; factor that parsing out and reuse it.
  - Read only file heads (few-KB cap) for the preview; sort newest first by file mtime.
- **Dedup.** Exclude native ids already present in `sessions.agent_session_key`. This is load-bearing, not a nicety: Runner-spawned claude-code/codex chats write to the same native stores, so without this filter import would re-list Runner's own chats as importable and duplicate them. Re-running import is therefore idempotent.
- **Modal UX.** Checkbox multi-select list, all pre-checked. Each row: runtime badge, relative timestamp, one-line first-user-message preview, dim session id. Loading / empty ("No sessions found for this directory") states. Import button shows selected count.
- **Import.** For each selection, insert a `sessions` row — no PTY is spawned:
  - `status = 'stopped'`, `runner_id = NULL`, `mission_id = NULL`
  - `agent_runtime = 'claude-code' | 'codex'`, `agent_command = NULL` (resume falls back to the runtime's default command)
  - `agent_session_key = <native uuid>`, `cwd = <recorded cwd>`, `project_id = <project>`
  - `title` from the first user message (truncated), fallback "Imported <runtime> session"; `started_at` / `stopped_at` from the native file's timestamps.
- **Open / resume.** Nothing new: the imported row is a stopped, resumable chat. Opening it drives the existing `session_resume` flow; `resume_plan` builds `--resume <uuid>` (claude-code) / `resume <uuid>` (codex).

### Out of scope (v1)

- **Replaying transcripts into Runner's event feed / scrollback ring.** The resumed agent repaints its own history into the PTY; Runner's ring only sees post-resume bytes. Same decision as the original spec.
- **Cross-runtime resume.** Not on-the-wire compatible.
- **Mission import.** Chats only; missions stay fresh-start.
- **Auto / continuous import.** No import-on-project-create, no filesystem watcher picking up new native sessions. Manual, user-initiated, repeatable.
- **Mutating the native stores.** Import never moves, edits, or deletes native session files; deleting an imported chat in Runner leaves the native file untouched. Sessions remain owned by the agent CLI.
- **`shell` runners** — no session concept.

## Implementation phases

### Phase 1 — native store adapters

New `session/native_store.rs` with `NativeSessionEntry { runtime, id, cwd, started_at, last_active_at, preview }` and one `list_for_cwd(cwd) -> Vec<NativeSessionEntry>` per runtime. Claude-code adapter reads the encoded-cwd directory; codex adapter walks the date partitions (cap the walk — most recent ~200 files by mtime) and reuses the `session_meta` parsing factored out of `codex_capture.rs`. All errors degrade to an empty list.

### Phase 2 — `project_list_importable` command

```ts
project_list_importable({ project_id }) → Promise<ImportableSession[]>
```

Looks up the project's cwd, runs both adapters, filters out ids already present in `sessions.agent_session_key`, returns merged newest-first. Lives in `commands/project.rs` beside the existing `project_*` commands; exposed as `api.project.listImportable`.

### Phase 3 — `project_import_sessions` command

```ts
project_import_sessions({ project_id, selections: [{ runtime, id }] }) → Promise<DirectSessionEntry[]>
```

Re-reads each selected native file (skip-and-continue if one vanished since the scan), inserts the session rows per the scope table above, emits the existing session-updated fanout so all windows' sidebars pick the new chats up, and returns the created entries.

### Phase 4 — frontend

Project kebab menu entry + `<ImportSessionsModal>` (multi-select list per the UX scope). Post-import, the chats render through the existing project-nested stopped-chat rows — no new row component.

### Phase 5 — edge cases + polish

- Native file gone at import time → skip, toast "N imported, M no longer available".
- Native file gone at resume time → already handled: resume degrades to a fresh spawn with the existing toast.
- cwd canonicalization (trailing slash, symlinks) before matching; claude-code's encoding is lossy (`-` for both `/` and `.`) — encoding the project cwd and reading that one directory sidesteps guessing, and collisions are acceptable noise (the preview makes misfiled rows obvious).
- Project deleted while modal open → import returns not-found; close modal.

## Verification

- **Unit (Rust):**
  - claude-code adapter: temp `~/.claude/projects/` tree → entries, ordering, preview extraction, head-read cap.
  - codex adapter: temp date-partitioned `~/.codex/sessions/` with mixed-cwd rollouts → cwd filtering, walk cap.
  - dedup: a native id already on a `sessions` row is excluded from `project_list_importable`.
  - import idempotency: running `project_import_sessions` twice creates no duplicate rows.
- **Integration:** import a fixture session → row lands with `status='stopped'`, `agent_runtime`, `agent_session_key`, `project_id`; `session_resume` on it produces the right resume argv for each runtime.
- **Manual smoke:**
  1. Run a few `claude` turns in a repo dir outside Runner; exit.
  2. In Runner, create/open a project bound to that dir → kebab → Import sessions. The session appears with preview + timestamp; import it.
  3. The chat appears nested under the project; open it → the terminal repaints the prior conversation.
  4. Repeat for codex. Re-open the import modal → already-imported sessions no longer listed.

## Notes

- The row shape import writes is exactly what `session_start_runtime` (migration 0007) spawns, minus the live PTY — which is why resume, chat header meta, archive, and rename all work with no changes.
- Runner's own claude-code chats double-write into `~/.claude/projects/`; the `agent_session_key` dedup is what keeps import from surfacing them. If a Runner chat predates key capture (NULL key), it can show up as importable — acceptable: importing it just creates a second handle on the same native session.
- Import doesn't validate that the native session is still resumable by the CLI (rotated/invalidated sessions). The agent CLI surfaces its own error at resume time; same stance as the original spec.
