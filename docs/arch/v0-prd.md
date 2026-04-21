# Runners — v0 PRD

> Status: draft, open for feedback. Anything in **[OPEN]** is a decision we haven't taken yet.

## 1. Problem

Coding agents like Claude Code, Codex, and aider are each powerful alone, but there's no good way to run several of them *together* on one machine with different roles, a shared view of what's happening, and a sane way to pull in the human when they disagree or hit a wall.

Today, coordinating two agents means juggling terminal windows, eyeballing logs, and manually relaying messages. It breaks down past one agent, and doesn't scale as people start combining specialists (a coder + a reviewer + a tester + a fixer).

## 2. Goal

A local desktop app where one person can:

1. Assemble a **crew** of CLI coding agents on their own machine.
2. Give each **runner** a role and a brief (system prompt / instructions).
3. Watch every runner's live output in one window.
4. Let runners **coordinate** through a shared event log.
5. Get pulled in by an **orchestrator** only when a decision needs a human.

v0 proves the loop works end-to-end with two runners. v1+ scales it.

## 3. Vocabulary

- **Crew** — a named group of runners working together on a goal.
- **Runner** — an individual CLI agent process (one PTY, one role, one system prompt).
- **Session** — a single run of a runner's process.
- **Event** — a structured NDJSON line runners emit to coordinate; routed by the orchestrator.
- **Orchestrator** — the rule-based router that reads events and decides what happens next (route to another runner, ask the human, etc.).

## 4. Non-goals for v0

- Multi-session per runner, session replay, session history browsing
- LLM-based orchestrator (v0 is rule-based only)
- Remote / cloud / multi-host runners
- Cost tracking, observability dashboards, telemetry
- Runner templates, presets, or marketplace
- Multi-human collaboration (a crew is a crew of runners, not humans)
- Secrets management beyond plain env vars

## 5. User journey (the v0 demo)

The concrete loop v0 must support end-to-end:

1. User creates a crew called *Feature Ship*.
2. User spawns two runners:
   - **Coder** — runtime `claude-code`, working dir `~/src/myproj`, brief "Implement feature X. When done, emit `review_requested`."
   - **Reviewer** — runtime `claude-code`, same working dir, brief "Wait for `review_requested`. Read the diff. Emit `approved` or `changes_requested`."
3. User clicks **Start Crew**. Both PTYs spawn. User sees two terminals, one per runner.
4. Coder writes code, runs tests, then calls `runners emit review_requested`. An event appears on the crew timeline.
5. Orchestrator policy routes the event: injects a message into Reviewer's stdin ("There's a review pending, please proceed.").
6. Reviewer looks at the diff, emits `changes_requested` with a payload listing issues.
7. Orchestrator policy for `changes_requested` says `ask_human`. The human-in-the-loop panel pops a card: *"Reviewer requested changes. Accept and forward to Coder, or override?"*
8. User clicks **Accept**. Orchestrator writes a `forward_to_coder` event with the reviewer's notes, which injects into Coder's stdin.
9. Coder fixes the issues, re-emits `review_requested`. Loop continues until `approved`.

If v0 doesn't ship this flow working end-to-end, it hasn't shipped.

## 6. Features

### 6.1 Crew CRUD
- Create, rename, delete crews.
- A crew has: `name`, `goal` (free-text brief describing what the crew is trying to do), a list of runners, an orchestrator policy.
- Persisted in SQLite.

### 6.2 Runner CRUD (scoped to a crew)
- Spawn, edit, remove runners within a crew.
- A runner has:
  - `name` (display, e.g. "Coder")
  - `role` (label, e.g. "implementation")
  - `runtime` — enum: `claude-code | codex | shell`. Adds the right default `command` + `args`.
  - `command` + `args` — concrete binary to spawn. Pre-filled from runtime, editable.
  - `working_dir`
  - `system_prompt` / brief — the runner's role-specific instructions, passed via `--system-prompt` or equivalent depending on runtime.
  - `env` — key/value list, optional.

### 6.3 Live per-runner terminal (first-class)
- One PTY subprocess per running runner, spawned via `portable-pty`.
- Stdout streams to the frontend via a Tauri event (`session:{id}:out`).
- Frontend renders with **xterm.js** to preserve ANSI colors, cursor control, and TUI layouts. A dumb `<pre>` will look broken for claude/codex.
- Scrollback retained per session (cap at ~10k lines; overflow dumped to a per-session log file on disk).
- Status chip per runner: `idle | running | waiting_for_input | blocked_on_human | crashed`. Derived from PTY state + last event.
- Stdin input box so the human can type directly into any runner at any time.
- Terminals stay alive across tab/selection switches — we don't re-create the xterm instance on hide, or we lose the ANSI state machine and scrollback.

### 6.4 Event bus (inter-runner comm)
- Single append-only **NDJSON** file per crew, at `$APPDATA/runners/crews/{crew_id}/events.ndjson`.
- One line per event. Event schema:
  ```json
  {
    "id": "uuid",
    "ts": "2026-04-21T12:34:56Z",
    "from_runner": "coder",
    "type": "review_requested",
    "payload": { "...": "..." },
    "correlation_id": "uuid?"
  }
  ```
- File is watched with the `notify` crate. UI and orchestrator both subscribe.
- File is tailable with `tail -f` for debugging. Deliberate.

#### 6.4.1 How runners emit events — **[OPEN — decision needed]**

Three options, pick one:

| Option | How | Pros | Cons |
|---|---|---|---|
| **(a) MCP tool** | Each runner gets a `runners_emit` MCP tool | Clean, typed, structured | Requires MCP support in every runtime |
| **(b) CLI wrapper** | A `runners emit <json>` binary on PATH; runner calls it from bash/tool use | Works with **any** CLI agent unmodified | Slightly uglier; runner has to know the command |
| **(c) Stdout parsing** | Runner prints `[[runners:event:{json}]]` lines; we parse them out of PTY stream | No runner cooperation at all | Fragile, interferes with TUI output, hard with ANSI |

**Recommendation: (b).** Universal, works for every CLI agent today and tomorrow, no hidden parsing magic. The `runners` CLI is a thin shim that appends to the NDJSON file. (a) is worth revisiting for v1 once MCP is reliably supported by all runtimes.

### 6.5 Orchestrator (rule-based)
- Policy is per-crew, stored as JSON on the crew row.
- Policy is an ordered list of `{ when, do }` rules. First match wins. Schema:
  ```json
  [
    { "when": { "type": "review_requested" },
      "do": { "action": "inject_stdin", "target": "reviewer",
              "template": "A review is pending. Please proceed." } },
    { "when": { "type": "changes_requested" },
      "do": { "action": "ask_human",
              "prompt": "Reviewer requested changes. Accept or override?",
              "choices": ["accept", "override"] } },
    { "when": { "type": "approved" },
      "do": { "action": "notify_human", "message": "PR approved by reviewer." } }
  ]
  ```
- Supported actions in v0:
  - `inject_stdin` — write a message into the target runner's stdin.
  - `ask_human` — show a card in the HITL panel, wait for response, emit a follow-up event with the answer.
  - `notify_human` — fire a toast, don't block.
  - `pause_runner` / `resume_runner` — send SIGSTOP/SIGCONT to the target PTY.
- No expressions, no scripting, no LLM. v0 is a lookup table.

### 6.6 Human-in-the-loop panel
- Right-rail panel showing all pending `ask_human` cards.
- Each card shows: triggering event, orchestrator prompt, choices.
- User clicks a choice → orchestrator writes a response event `{type: "human_response", correlation_id: <triggering event id>, choice: "accept"}` which any downstream rule can match.
- Visible across all views so the user never misses one.

### 6.7 Mission control UI

Single screen per crew. Layout:

```
┌─────────────────────────────────────────────────────────────┐
│ Crew: Feature Ship          ▶ Start │ ⏸ Pause │ ⏹ Stop      │
├──────────┬──────────────────────────────────┬───────────────┤
│ Runners  │  ▌ Coder (running)               │ Pending asks  │
│          │  ┌───────────────────────────┐   │ ┌───────────┐ │
│ ● Coder  │  │ [xterm live output]       │   │ │ Reviewer  │ │
│ ○ Reviewer│ │                           │   │ │ requested │ │
│          │  └───────────────────────────┘   │ │ changes.  │ │
│ + Spawn  │  > _                             │ │ [Accept]  │ │
│          │                                  │ │ [Override]│ │
│          │  Events (this runner)            │ └───────────┘ │
│          │  12:34 emit review_requested     │               │
│          │  12:35 stdin injected            │ Event stream  │
│          │                                  │ (all runners) │
└──────────┴──────────────────────────────────┴───────────────┘
```

- **Left rail**: runner list with status dots. Click to focus.
- **Main area**: focused runner's live terminal + that runner's event log below.
- **Right rail**: HITL panel (top) + global event timeline (bottom).
- **[OPEN]**: interleaved terminal + events, or split? **Recommendation: split.** Terminal output is noisy TUI redraws; events are the semantic layer. Mixing them makes both harder to read. Events below the terminal, timestamp-aligned.
- **[OPEN]**: side-by-side view of two runners' terminals. Nice-to-have for the demo, but deferrable to v0.x.

## 7. Data model

```sql
crews (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  goal TEXT,
  orchestrator_policy TEXT,       -- JSON
  created_at TEXT, updated_at TEXT
);

runners (
  id TEXT PRIMARY KEY,
  crew_id TEXT REFERENCES crews(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  role TEXT NOT NULL,
  runtime TEXT NOT NULL,          -- claude-code | codex | shell
  command TEXT NOT NULL,
  args_json TEXT,                 -- JSON array
  working_dir TEXT,
  system_prompt TEXT,
  env_json TEXT,                  -- JSON object
  created_at TEXT, updated_at TEXT
);

sessions (
  id TEXT PRIMARY KEY,
  runner_id TEXT REFERENCES runners(id) ON DELETE CASCADE,
  status TEXT NOT NULL,           -- running | stopped | crashed
  started_at TEXT, stopped_at TEXT
);
```

Events live in the NDJSON file per crew, not in SQLite. SQLite is for config + session lifecycle only.

## 8. Tech boundaries

- **Backend:** Rust, Tauri 2. PTY via `portable-pty`. File watching via `notify`. Persistence via `rusqlite` (WAL).
- **Frontend:** React 19, TypeScript, Tailwind 4, xterm.js, React Router.
- **Event log:** NDJSON file. No message broker, no WebSockets, no DB row stream.
- **Orchestrator:** Rust module, runs in the Tauri backend, subscribes to the event file via `notify`.

## 9. Open questions

1. **Event emission mechanism** — a, b, or c in §6.4.1. Recommendation: (b).
2. **Terminal + events visual layout** — interleaved or split. Recommendation: split.
3. **Side-by-side runner terminals** in v0, or defer to v0.x.
4. **How does the system prompt actually get passed** to each runtime? `claude-code` takes `--append-system-prompt`; `codex` has its own flag. The runtime enum in §6.2 should own this mapping.
5. **Restart semantics** — if a runner crashes, auto-restart? v0 answer: no, surface the crash and let the human click Restart.
6. **Event ordering guarantees** — single-writer (the `runners` CLI) per file should give us total order via append + fsync. Need to verify `fs::OpenOptions::new().append(true)` on macOS is atomic for small writes (<PIPE_BUF). Probably fine, worth a note.

## 10. Risks

- **PTY flakiness across platforms** — especially Windows. v0 targets macOS only; Linux best-effort; Windows deferred.
- **TUI rendering in xterm.js** — claude/codex use rich TUIs. xterm.js is mature but some escape sequences may still render oddly. Budget time for tuning.
- **Runners that don't know the `runners emit` convention** — they can't coordinate. We need starter briefs / system-prompt snippets that teach them. Ship those as part of each runtime's defaults.

## 11. Done criteria

v0 ships when:
- [ ] A user can create a crew, spawn two runners, start them, and see two live terminals.
- [ ] Runners can emit events via `runners emit` and the UI shows them in real time.
- [ ] A rule-based policy can route an event into another runner's stdin.
- [ ] A rule-based policy can pause the crew and ask the human a question, then resume based on the answer.
- [ ] The Coder + Reviewer demo loop from §5 works end-to-end without the user touching a terminal outside the app.
