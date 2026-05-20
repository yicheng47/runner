# 21 — Detect and resume existing agent sessions for the cwd

> Tracking issue: [#176](https://github.com/yicheng47/runner/issues/176)

## Motivation

A lot of Runner's target users already use `claude-code` and `codex` directly in their terminal *before* they install Runner. When they then start a direct chat in Runner against the same working directory, they get a fresh agent with none of the conversation history they just built up. Today the workaround is to keep using the agent CLI outside Runner — exactly the split-attention problem Runner is trying to solve.

Both agent runtimes already store per-project session history on disk and expose a resume verb (`claude --resume <uuid>` / `codex resume <uuid>`). Runner just needs to surface those sessions in the Start Chat modal and pass the right flag at spawn.

## Scope

### In scope (v1)

- **Detection.** When the user picks a runner + cwd in the Start Chat modal, scan the runtime-specific on-disk session store for sessions whose recorded cwd matches. Surface them in a "Recent sessions for this directory" panel below the rest of the modal fields.
- **Picker UX.** Each session row shows: relative timestamp ("3h ago", "yesterday"), one-line preview from the first user message, and the session id (dimmed). Sorted newest first; cap at the 10 most recent. Default selection is "Start fresh"; clicking a row pre-selects it for resume.
- **Resume.** On submit, if a session was selected, pass the runtime's resume flag to the spawned child:
  - claude-code: `claude --resume <uuid>` (the bundle's existing `agent_session_key` capture flow already proves this works in Runner; we just hand it the uuid instead of letting the agent pick).
  - codex: `codex resume <uuid>` (codex uses positional uuid; `--last` is also supported but the picker makes the explicit form natural).
- **Runtime coverage.** Both `claude-code` and `codex` in v1. `shell` runner type has no session concept and skips the panel entirely.

### Out of scope (v1)

- **Mission spawn.** Detection only surfaces in Start Chat. Missions stay fresh-start — multi-slot resume is a different shape (per-slot picker × N) that doesn't fit the modal cleanly. Revisit when there's user demand.
- **Importing the transcript into Runner's event feed.** The resumed agent renders its own history into the PTY (xterm.js paints it like any other output); we don't replay it into Runner's NDJSON log. This means Runner's scrollback ring only sees post-resume bytes, but the user sees the full transcript in the terminal as the agent restores it.
- **Cross-runtime resume.** No "resume a codex session in claude-code" — they're not on-the-wire compatible.
- **Edit / delete from Runner.** Sessions remain owned by the agent CLI. If the user wants to delete a session, they do it from the agent's own surface.

## Implementation phases

### Phase 1 — disk layout adapters

Add `session_store::SessionEntry { id, cwd, started_at, first_user_msg }` and a trait / function pair per runtime that lists entries for a given cwd:

- **claude-code:** sessions live at `~/.claude/projects/<encoded-cwd>/<uuid>.jsonl`. The encoded-cwd convention is the absolute path with `/` replaced by `-` (e.g. `/Users/jason/foo` → `-Users-jason-foo`). Each JSONL's first non-system entry is the first user message — read just the head of the file (cap at a few KB).
- **codex:** sessions live at `~/.codex/sessions/*.jsonl` (flat, not partitioned by cwd). Each session's JSONL records the cwd in its first event; we have to read all session heads and filter. Cache the index in memory across modal opens so we don't re-stat the whole dir on each render.

Both adapters return `Vec<SessionEntry>` sorted by `started_at` desc, capped at N=10.

### Phase 2 — `session_list_resumable` Tauri command

```ts
session_list_resumable({ runner_id, cwd }) → Promise<SessionEntry[]>
```

Looks up the runner template's runtime, dispatches to the right adapter, returns the entries. Errors (missing home dir, unreadable session file) degrade silently to an empty list — never surface a "couldn't list sessions" toast; the user just sees "no recent sessions."

### Phase 3 — frontend: Start Chat modal panel

- Below the existing fields (runner picker, working dir, title), add a `<RecentSessionsPanel>` that calls `session_list_resumable` whenever runner + cwd both have values.
- Default state: "Start fresh" radio is selected. Each session row is a radio option below it. Loading state shows a small spinner; empty state shows "No prior sessions for this directory."
- On submit: if a session radio is selected, pass `resume_session_id: <id>` to the existing `session_spawn_direct` command. Otherwise the field is absent and spawn behaves exactly as today.

### Phase 4 — backend: thread the resume id through spawn

- `session_spawn_direct` grows an optional `resume_session_id: Option<String>`.
- The runtime adapter (`router::runtime::*`) extends its arg-building to prepend the resume flag when set. claude-code: `--resume <uuid>`. codex: positional `resume <uuid>`.
- Persist the chosen uuid on the new `sessions` row as `agent_session_key` (the column already exists for codex auto-capture; reuse it). This lets the existing cross-restart reattach path resume the same agent session if the user restarts the app mid-chat.

### Phase 5 — empty / edge cases

- **Session file disappears between scan and spawn.** Catch the spawn error; show a small toast ("That session is no longer available — starting fresh") and fall back to fresh spawn.
- **User changes runner mid-modal** (claude-code → codex). Clear the selected session id — different runtime, can't carry over.
- **User changes cwd mid-modal.** Re-fetch.
- **Sessions exist for a cwd no longer on disk** (project moved). Still list them; user can resume if they want — the agent will fail if it needs files that don't exist, but that's an agent concern.

## Verification

- **Unit (Rust):**
  - claude-code adapter: feed a temp `~/.claude/projects/` tree, assert entries returned + ordering.
  - codex adapter: feed a temp `~/.codex/sessions/` with mixed-cwd sessions, assert filtering.
  - `session_list_resumable` errors degrade to empty list, not Err.
- **Integration:**
  - Spawn a fresh claude-code direct chat; `agent_session_key` populates; close it; reopen Start Chat for the same runner + cwd; the just-closed session appears in the panel; selecting it + submit produces a `claude --resume <uuid>` argv.
  - Same with codex.
- **Manual smoke:**
  1. With `claude` in your shell, run a few turns in a temp project dir; exit.
  2. In Runner, pick the claude-code runner, set cwd to that dir, open Start Chat. The session should appear in the panel with the right preview + timestamp.
  3. Select + start. The xterm pane should paint the prior conversation as the agent restores it.
  4. Repeat for codex (`codex` outside Runner, then resume from Runner).

## Notes

- The `agent_session_key` column already exists on `sessions` (added for codex auto-capture in #137). This feature reuses it as the explicit-resume slot too — no schema change required.
- Detection is a UX nicety, not a hard guarantee. If a user resumes a session that the agent has since rotated / invalidated, the agent CLI itself will surface the error; Runner doesn't need to validate session ids before spawn.
