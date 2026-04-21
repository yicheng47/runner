# Runners — v0 Architecture

> Companion to `v0-prd.md`. The PRD defines *what* v0 ships; this doc defines *how* it works.

## 1. Overview

Runners is a local desktop app. A user configures a **crew** of CLI coding agents, launches a **mission** to activate it, and watches the crew coordinate in real time. The app is a Tauri 2 binary: Rust backend, React webview, SQLite for config, and a per-mission NDJSON file for live state.

### 1.1 Runtime picture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Tauri process (runners desktop app)                                         │
│                                                                             │
│  ┌──────────────────────┐   ┌──────────────────────┐   ┌─────────────────┐  │
│  │ MissionManager       │   │ SessionManager       │   │ EventBus        │  │
│  │  - mission lifecycle │   │  - PTY spawn/kill    │   │  - tail NDJSON  │  │
│  │  - compose prompts   │   │  - reader threads    │   │  - notify watch │  │
│  │  - roster + brief    │   │  - scrollback rings  │   │  - fact project │  │
│  └────────┬─────────────┘   └────────┬─────────────┘   └────────┬────────┘  │
│           │                          │                          │           │
│           │                          │                          ▼           │
│           │                          │               ┌──────────────────┐   │
│           │                          │               │ Orchestrator     │   │
│           │                          │               │  - policy rules  │   │
│           │                          │               │  - action dispch │   │
│           │                          │               └────────┬─────────┘   │
│           │                          │                        │             │
│           │      inject_stdin / ask_human / pause / ...       │             │
│           └─────────────────────────►│◄───────────────────────┘             │
│                                      ▼                                      │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │ Runner session (one per runner × mission)                            │   │
│  │   ┌──────────┐   PTY   ┌─────────────────────────────────────────┐   │   │
│  │   │  master  │ ◄────►  │  child: claude-code / codex / shell     │   │   │
│  │   └──────────┘         │  env: RUNNERS_CREW_ID,                  │   │   │
│  │                        │       RUNNERS_MISSION_ID,               │   │   │
│  │                        │       RUNNERS_RUNNER_NAME,              │   │   │
│  │                        │       RUNNERS_EVENT_LOG, PATH=…         │   │   │
│  │                        └─────┬───────────────────────────────────┘   │   │
│  └─────────────────────────────┼──────────────────────────────────────┘   │
│                                │ runs `runners emit` / `runners ctx`      │
│                                ▼                                          │
│                  ┌─────────────────────────────┐                          │
│                  │  events.ndjson (per mission)│                          │
│                  └──────────────┬──────────────┘                          │
│                                 │ notify → EventBus → Orchestrator + UI   │
└─────────────────────────────────┼─────────────────────────────────────────┘
                                  ▼
                         ┌───────────────────────┐
                         │ React + xterm.js      │
                         │  terminals, timeline, │
                         │  HITL cards, facts    │
                         └───────────────────────┘
```

### 1.2 The one-paragraph story

The user defines a **crew** (configuration: runners + policy). They click **Start Mission**, which creates a **mission** (runtime container), spawns one PTY-backed **session** per runner, and composes each runner's system prompt with the mission brief, the crew roster, and coordination instructions. Runners run real CLI binaries inside PTYs; they emit **events** via a bundled `runners` CLI that appends to the mission's NDJSON file. The **orchestrator** tails that file, applies a rule-based policy, and dispatches actions (inject stdin, ask human, pause, etc.). Runners share state through a **fact** whiteboard backed by the same event log. The UI is a read-only tail that renders terminals, events, facts, and HITL prompts.

## 2. Concepts

Seven domain objects. Two kinds: **configuration** (persistent, edited by the user) and **runtime** (live, created at mission start, torn down at mission end).

### 2.1 Relationship diagram

```
Configuration (persistent)          Runtime (live or historical)

  Crew ─┬── Runner ────────────────► Session ────► PTY process
        │                              ▲
        │                              │ spawns
        └── Orchestrator Policy        │
              │                        │
              └─ attached to ─► Mission ────► events.ndjson
                                  │             │
                                  │             ├─► Event
                                  │             └─► Fact (via fact_recorded)
                                  │
                                  └─► Shared context (brief + roster + facts)
```

### 2.2 Crew — *a configured team*

The persistent "who's on the team and how they work together" record. A crew has a name, a default mission goal, a list of runners, an orchestrator policy, and an event-type allowlist. It does not run. It is blueprint.

Lifecycle: created by the user, edited freely, deleted when no longer needed. Persisted in SQLite.

### 2.3 Runner — *one configured agent*

An individual CLI agent within a crew: what binary to run, with what args, in what working directory, with what system prompt (the role's brief). Persistent config. A runner doesn't run either; it describes a process that will be spawned when a mission starts.

A runner belongs to exactly one crew. Examples: "Coder (claude-code)", "Reviewer (claude-code)", "Tester (shell)".

### 2.4 Orchestrator Policy — *the crew's decision rules*

A JSON list of `{when, do}` rules attached to the crew. This is where all routing and human-in-the-loop behavior is expressed. Shared across every mission the crew runs (in-memory state like pending asks is per-mission, but the rule set is per-crew).

There is no code here — just a lookup table. No scripting, no LLM. v0 is deliberately dumb.

### 2.5 Mission — *one activation of the crew*

A runtime container. When the user clicks **Start Mission**, a mission row is created, sessions are spawned for every runner, the orchestrator is booted with a fresh in-memory state, and an NDJSON event log is opened. When the mission ends (explicit stop, or all sessions exited), everything in the container shuts down together.

A mission scopes: the event log, the fact whiteboard, pending HITL cards, orchestrator memory. Each mission starts empty on all four.

v0 constraint: a crew can have at most one live mission at a time. A crew can have many historical missions.

### 2.6 Session — *one runner's PTY process*

The live process for one runner inside one mission. One runner × one mission = one session. Owns: a PTY master handle, a reader thread, a writer, a scrollback ring buffer. When the session's child process exits, the session is done; a new one is only created by starting a new mission.

A session is the only object that actually *executes* something — everything else is metadata or a coordination channel.

### 2.7 Event — *a durable coordination message*

One line in a mission's NDJSON file. Structured JSON: `{id, ts, crew_id, mission_id, from, to, type, payload, correlation_id, causation_id}`. Emitted by runners (via the `runners emit` CLI), humans (via the UI), or the orchestrator itself (as a side effect of actions). Consumed by the orchestrator for policy dispatch and by the UI for the timeline.

Events are the system's source of truth for runtime state. The orchestrator's in-memory state is a *projection* of the events; on crash, it's rebuilt by replaying the file.

### 2.8 Fact — *an entry on the shared whiteboard*

A key-value pair any runner can read or write during a mission via the `runners ctx` CLI. Implemented as a specific event type (`fact_recorded`), so facts are just events with a particular shape — no separate store. Last-writer-wins per key. Mission-scoped: each mission starts with an empty whiteboard.

Facts are how runners share state that doesn't fit the event model cleanly: "the PR URL is X," "we're targeting branch Y," "the build is green." Think of events as *things that happened* and facts as *things that are currently true*.

## 3. Mission lifecycle

### 3.1 Start

```
user clicks Start Mission on a crew
  └─► MissionManager.start(crew_id):
        ├─ insert `missions` row (status=running, mission_id = ULID)
        ├─ mkdir $APPDATA/runners/crews/{crew_id}/missions/{mission_id}/
        ├─ touch events.ndjson
        ├─ for each runner in crew:
        │     composed_prompt = compose(runner.system_prompt,
        │                                mission.brief,
        │                                roster(crew),
        │                                coordination_notes(crew.event_types))
        │     SessionManager.spawn(mission_id, runner, composed_prompt)
        ├─ Orchestrator.start(mission_id)  ← fresh in-memory state
        │     open events.ndjson, read history (empty), tail via notify
        └─ emit Tauri event: mission:{id}:started
```

### 3.2 End

```
user clicks End Mission  (or all sessions have exited)
  └─► MissionManager.end(mission_id, status):
        ├─ SessionManager.kill_all_in_mission(mission_id)
        ├─ Orchestrator.stop(mission_id)
        ├─ update `missions` row: status (completed/aborted), stopped_at
        └─ emit Tauri event: mission:{id}:ended
```

### 3.3 v0 constraint

One live mission per crew. Starting a new one while one is live is blocked in the UI. (v1: relax to concurrent missions.)

## 4. PTY runner sessions

### 4.1 Why PTY

Claude Code and Codex are TUIs. They check `isatty()`; if false, they degrade (no colors, no spinner, sometimes outright refuse). Their output is a stream of escape sequences (`\x1b[2K`, alt-screen toggles) that only a terminal emulator can render.

A pseudo-terminal gives the child a real terminal on stdin/stdout/stderr (full TUI mode) and hands us the master end as a byte stream that we forward to **xterm.js** in the webview.

Anything less (plain pipes, stdout-only capture) will look broken.

### 4.2 Spawn

```
portable_pty::openpty(rows, cols)
  ├─ master handle  → kept by SessionManager
  └─ slave handle   → given to child via spawn_command()

Child inherits:
  PATH                = $APPDATA/runners/bin:<original PATH>
  RUNNERS_CREW_ID     = <ulid>
  RUNNERS_MISSION_ID  = <ulid>
  RUNNERS_RUNNER_NAME = coder
  RUNNERS_EVENT_LOG   = $APPDATA/runners/crews/<crew>/missions/<mission>/events.ndjson

Reader thread (blocking):
  loop { read(master) → emit session:{id}:out event, push to scrollback ring }
  on EOF: wait(child) → emit session:{id}:exit { code } → update sessions row
```

System prompt is passed to the runtime via its native flag (`--append-system-prompt` for claude-code; equivalent for each runtime). The runtime enum in the `runners` table owns the flag mapping.

### 4.3 The composed system prompt

MissionManager builds each runner's prompt from four parts:

1. **The user-authored brief** (`runners.system_prompt`).
2. **The mission brief** (`missions.goal_override` or `crews.goal`).
3. **The roster** — crewmates' names, roles, one-line brief summaries.
4. **Coordination notes** — how to use `runners emit`, `runners ctx`, and the crew's allowed event types.

Example for a Reviewer:

```
You are Reviewer, a runner in crew "Feature Ship".
Your role: code review.

== Your brief ==
Wait for review_requested events. Read the diff on the branch recorded
in the `pr_branch` fact. Emit `approved` or `changes_requested`.

== Mission ==
Goal: Implement feature X with tests and a clean PR.

== Your crewmates ==
- Coder (implementation): Writes code. Emits review_requested when ready.

== Coordination ==
Use `runners emit <type> [--payload '{...}']` to signal milestones.
Use `runners ctx get <key>` to read facts, `runners ctx set <key> <value>` to record them.
Event types in this crew: review_requested, changes_requested, approved, blocked.
```

### 4.4 Frontend wiring

- On first view: fetch the session's scrollback ring; write to xterm.js to restore history.
- Subscribe to `session:{id}:out` for live output.
- xterm.js `onData` → `send_input(session_id, bytes)` → `master.writer.write_all(bytes)`.
- Frontend window resize → debounced (~100ms) `master.resize(rows, cols)` → SIGWINCH to child. Non-optional; without it, TUIs mis-render.

### 4.5 Threads, not async

`portable-pty`'s reader is blocking. Spawn an OS thread per session. Writers stay on the Tauri async runtime (writes are short).

### 4.6 Scrollback in Rust

`VecDeque<String>` ring (~10k lines) per session in SessionManager, so scrollback survives tab-switches and app restarts. Overflow lines append to `missions/{mission_id}/sessions/{session_id}.log`. The ring sees raw bytes including alt-screen toggles — acceptable v0 scuff.

### 4.7 Death and kill

Reader thread owns the child handle. On EOF, it calls `wait()`, emits `session:{id}:exit`, updates the sessions row. No auto-restart in v0.

Kill: drop master → SIGHUP via `portable-pty`; escalate to SIGKILL if child lingers. v0 targets macOS; Linux best-effort; Windows deferred.

## 5. Event bus (inter-runner communication)

### 5.1 Transport

```
$APPDATA/runners/crews/{crew_id}/missions/{mission_id}/events.ndjson
```

One line per event. Append-only. **Each mission has its own file** — this scopes log rotation (no rotation policy needed; a new mission = a new file), crash-replay (orchestrator reads only the live mission's file), and deletion (drop the mission directory).

Why a file instead of an in-memory bus:
- **Debuggable** — `tail -f events.ndjson | jq .`.
- **Crash-durable** — whatever's on disk survived the crash.
- **Atomic** — writes < `PIPE_BUF` (4KB on macOS) to `O_APPEND` are atomic at the OS level; concurrent `runners emit` invocations interleave correctly.
- **Replayable for free** — restart the orchestrator, re-scan, resume.

### 5.2 Schema

```jsonc
{
  "id": "01HG3K1YRG7RQ3N9...",     // ULID: time-sortable, monotonic within ms
  "ts": "2026-04-21T12:34:56.123Z",
  "crew_id": "01HG...",
  "mission_id": "01HG...",
  "from": "coder",                  // runner name | "human" | "orchestrator"
  "to": null,                       // null = broadcast, or target runner name
  "type": "review_requested",
  "payload": { "...": "..." },
  "correlation_id": null,           // groups events in one conversation
  "causation_id": null              // which event caused this one
}
```

- **ULID** — sortable, embeds a ms timestamp, deterministic ordering within a millisecond.
- **`correlation_id`** — shared by all events in one conversation (e.g. every event in one review cycle). Set by the first emitter; propagated by the orchestrator to events it generates in response.
- **`causation_id`** — the immediate trigger. Together with correlation, reconstructs the event DAG.

### 5.3 How runners emit events

The three-layer mechanism that answers "how does the agent know to append?"

#### Layer 1 — system prompt tells the convention

The composed prompt (§4.3) includes a Coordination section describing the `runners emit` CLI and listing allowed event types. LLM agents already know how to read CLI docs and invoke tools — it's the same capability they use for `git`, `gh`, `npm`.

#### Layer 2 — the CLI exists on PATH with context in env

The backend prepends `$APPDATA/runners/bin/` to PATH and drops the `runners` binary there at first run. At session spawn, env vars point at the mission's log and identify the runner.

On invocation, the CLI:
1. Reads env vars; errors if missing.
2. Builds an event (ULID, timestamps, `from` = `$RUNNERS_RUNNER_NAME`, `crew_id`, `mission_id`).
3. Validates `type` against the allowlist sidecar at `$APPDATA/runners/crews/{crew_id}/event_types.json`.
4. Appends one JSON line to `$RUNNERS_EVENT_LOG` via `open(O_APPEND | O_WRONLY)` + `write_all` + close.
5. Exits 0 on success.

Each invocation writes ≤ one 4KB line in one `write` syscall, so concurrent emitters interleave safely.

#### Layer 3 — role briefs reinforce usage

User-authored briefs include examples at the moments where emission matters. We ship sensible defaults per-runtime.

#### Why robust

- No in-band protocol in the PTY stream. Not parsing stdout for magic markers.
- Works for any CLI agent (MCP or not). Only requirement: can run shell commands.
- Fails visibly. If the agent forgets to emit, the orchestrator sees nothing and the user sees an idle runner.
- Fully observable. `$ runners emit review_requested` shows up literally in the terminal pane; the resulting event shows up in the timeline.

#### Failure mode: hallucinated event types

CLI validates against the allowlist; unknown types exit non-zero with a clear stderr message. Agent reads the error from shell history and self-corrects.

### 5.4 Consumers

Two subscribers to the NDJSON file, both via `notify`:

- **Orchestrator** — deserializes each new line, runs policy, dispatches actions, updates the fact projection when `fact_recorded` appears.
- **UI** — the backend re-emits each line as a `mission:{id}:event` Tauri event. Frontend renders timeline, HITL cards, and fact view.

#### Startup replay

On orchestrator boot (triggered by mission start or app restart mid-mission): open the mission's file, fold events into in-memory state (fact projection, pending asks, correlation tracking), then switch to tailing. The file *is* the state.

### 5.5 Orchestrator actions

| Action | Effect | Emits event? |
|---|---|---|
| `inject_stdin` | write template + `\r` to target runner's PTY writer | `stdin_injected` |
| `ask_human` | add card to HITL panel; wait for click | `human_question`, then `human_response` |
| `notify_human` | fire a toast | `human_notified` |
| `pause_runner` | SIGSTOP to target PTY | `runner_paused` |
| `resume_runner` | SIGCONT to target PTY | `runner_resumed` |

Emitted events have `causation_id` = the triggering event's `id`. Full chain is reconstructable.

**Crash correctness:** emit the event *before* performing the action. Worst case on crash+replay is a duplicate action, which is recoverable (stdin seen twice; HITL cards deduped by event id). Silent loss is not.

### 5.6 Who does delivery

Runners never address other runners. They emit; the orchestrator routes. This gives us:

- **Decoupled runners** — Coder doesn't know Reviewer exists. Swap Reviewer without touching Coder's brief.
- **Single policy location** — every "when X, do Y" lives on the crew row.
- **Orchestrator is the only side-effecting component** outside runner processes. Easy to reason about.

### 5.7 Known failure modes

| Failure | Mitigation |
|---|---|
| Orchestrator crashes mid-action | Emit event before action; replay on boot; accept duplicates |
| Two runners ask human at once | HITL queues both; user answers in order |
| Event storm (runner bug-looping) | Surface events/sec warning; no rate limit in v0 |
| Malformed NDJSON line | Skip and warn; file stays valid |
| NDJSON file grows large | End the mission; new mission = new file |
| Hallucinated event type | Allowlist validation + clear stderr for self-correction |

## 6. Shared context (mission-scoped)

Three layers, different mechanics. All scoped to the mission — each new mission starts empty on all three.

### 6.1 Mission brief (read-only, prompt-injected)

`missions.goal_override` or falls back to `crews.goal`. Injected into the composed prompt at spawn (§4.3). Never changes during a mission.

### 6.2 Roster (read-only, prompt-injected)

Rendered from `crew.runners` at mission start into each runner's prompt as `== Your crewmates ==` with name, role, and a one-line brief summary. Never changes during a mission (v0 constraint: no mid-mission crew edits).

This is how the Reviewer knows there's a Coder.

### 6.3 Facts — the shared whiteboard (mutable, event-backed)

A KV store any runner reads/writes during the mission, via `runners ctx`. Implemented on the event log — no second store.

**Write:**
```
runners ctx set <key> <value>    → emits { type: "fact_recorded", payload: {key, value} }
runners ctx unset <key>          → emits { type: "fact_recorded", payload: {key, value: null} }
```

**Read:**
```
runners ctx get <key>            → prints value
runners ctx list                 → prints all current key/value pairs
```

Last-writer-wins per key. Flat namespace in v0.

**Projection:** the orchestrator maintains a `HashMap<String, Value>`, updated on every `fact_recorded`. Rebuilt from scratch on boot replay. The UI's fact view is driven by this projection via a Tauri event.

**Read path in v0:** the CLI re-scans the log (small, per-mission). If this ever becomes slow, add a localhost HTTP endpoint on the orchestrator.

**Why log-structured:**
- Single source of truth — facts are events.
- Auditable — every write is a durable event with who/when.
- Replayable — projection rebuilds from the log.
- Atomic — one append per write.
- Observable — fact updates appear in the timeline.

**Optional snapshot** at `missions/{mission_id}/context.json`, rebuilt from events for `cat`-ability. Not load-bearing.

### 6.4 The `runners` CLI surface

```
runners emit <type> [--payload <json>] [--correlation-id <id>] [--causation-id <id>]
runners ctx  get <key> | set <key> <value> | unset <key> | list
runners help
```

One binary, two verbs, bundled with the app. Context always from env.

## 7. Data model

### 7.1 SQLite (config + session lifecycle)

```sql
crews (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  goal TEXT,
  orchestrator_policy TEXT,           -- JSON: [{ when, do }]
  event_types TEXT,                   -- JSON array: allowlist
  created_at TEXT, updated_at TEXT
);

runners (
  id TEXT PRIMARY KEY,
  crew_id TEXT REFERENCES crews(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  role TEXT NOT NULL,
  runtime TEXT NOT NULL,
  command TEXT NOT NULL,
  args_json TEXT,
  working_dir TEXT,
  system_prompt TEXT,
  env_json TEXT,
  created_at TEXT, updated_at TEXT
);

missions (
  id TEXT PRIMARY KEY,
  crew_id TEXT REFERENCES crews(id) ON DELETE CASCADE,
  status TEXT NOT NULL,               -- running | completed | aborted
  goal_override TEXT,
  started_at TEXT NOT NULL,
  stopped_at TEXT
);

sessions (
  id TEXT PRIMARY KEY,
  mission_id TEXT REFERENCES missions(id) ON DELETE CASCADE,
  runner_id TEXT REFERENCES runners(id) ON DELETE CASCADE,
  status TEXT NOT NULL,               -- running | stopped | crashed
  started_at TEXT, stopped_at TEXT
);
```

### 7.2 Filesystem

```
$APPDATA/runners/
├── bin/
│   └── runners                              # the CLI (emit + ctx)
├── runners.db                               # SQLite
└── crews/
    └── {crew_id}/
        ├── event_types.json                 # CLI allowlist sidecar
        └── missions/
            └── {mission_id}/
                ├── events.ndjson            # per-mission event log
                ├── context.json             # optional fact snapshot
                └── sessions/
                    └── {session_id}.log     # scrollback overflow
```

macOS: `$APPDATA` = `~/Library/Application Support/com.wycstudios.runners`. Dev builds use `-dev` suffix.

## 8. Process and thread model

```
Tauri main thread
  ├── Tauri async runtime (tokio)
  │     ├── MissionManager (async)
  │     ├── Orchestrator task per live mission (notify + dispatch)
  │     └── Tauri command handlers (CRUD)
  ├── Thread per active session (blocking PTY reader)
  └── Webview process (React + xterm.js)
```

For v0 scale (one live mission, ≤ ~10 sessions): fine.

## 9. Out of scope for v0

- Concurrent live missions per crew
- Cross-mission memory (fact or prompt carryover)
- Remote runners / SSH
- Sandboxing beyond the child's own permissions
- MCP-based event emission
- Auto-restart on crash
- Event log rotation (solved implicitly by per-mission files)
- LLM-based orchestrator rules
- Typed event schemas per type
- Multi-user / multi-machine event bus

## 10. Architectural bets

1. **Mission is the runtime unit.** Crew is config; mission is a run. Scopes event log, HITL queue, facts, orchestrator state.
2. **PTY, not pipes.** Required for TUI fidelity.
3. **NDJSON file per mission, not broker.** Debuggability and crash-durability.
4. **CLI wrapper, not MCP.** Works with every agent today.
5. **Orchestrator is the only router.** Runners stay decoupled.
6. **Facts on the event log, not a separate store.** Single source of truth.
7. **Prompt composition at spawn time.** Replaces runtime handshakes.
8. **xterm.js for rendering.** Don't reinvent the terminal emulator.
9. **ULID for event IDs.** Sortable, monotonic within ms.

## 11. Open questions

1. CLI installation: bundled with `.app`, copied to `$APPDATA/runners/bin/` on first run — ok?
2. Fact reads: CLI re-scans log (v0) vs orchestrator HTTP endpoint (v0.x if needed).
3. Resize debounce: 100ms is a guess; tune with a real TUI.
4. Event type allowlist: per-crew only (current), or global defaults + per-crew overrides?
5. `from` field: locked to env (v0), or `--from` override?
6. Mid-mission fact-change notifications (push into stdin vs pull-only): v0 is pull-only.

## 12. What would break this architecture

- A runtime with no way to inject a system prompt at spawn (we'd type into stdin post-spawn — ugly but doable).
- An agent that won't learn to call CLI tools (hasn't happened with any modern coding agent).
- NDJSON append atomicity breaking on an exotic filesystem (NFS, iCloud-synced). v0: document that app data must be on a local POSIX filesystem.
