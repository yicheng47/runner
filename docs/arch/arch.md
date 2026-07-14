# Runner — Architecture

> Companion to [`../product/vision.md`](../product/vision.md). The vision doc defines *what* we're building and why; this doc defines *how* it works — tech stack, the model concepts the code is built around, and the protocol / schema decisions that make the model work.

## 1. Overview

Runner is a local desktop app. A user configures a **crew** of CLI coding agents, launches a **mission** to activate it, and watches the crew coordinate in real time. The app is a Tauri 2 binary: Rust backend, React + xterm.js webview, SQLite for config, and a per-mission NDJSON file for live state.

### 1.1 Runtime picture

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ Tauri process (Runner desktop app)                                           │
│                                                                              │
│                        ┌──────────────────────────┐                          │
│                        │ MissionManager           │                          │
│                        │   (orchestration —       │                          │
│                        │    lifecycle only)       │                          │
│                        │                          │                          │
│                        │  - mission lifecycle     │                          │
│                        │  - compose prompts       │                          │
│                        │  - remount on restart    │                          │
│                        └────────────┬─────────────┘                          │
│                                     │  spawns sessions + mounts router/bus   │
│                                     ▼                                        │
│   ┌─────────────────────────┐               ┌─────────────────────────┐      │
│   │ SessionManager          │               │ EventBus                │      │
│   │   (PTY runtime —        │               │   (NDJSON tailer —      │      │
│   │    the hot path)        │               │    projects for UI)     │      │
│   │                         │               │                         │      │
│   │  - PTY spawn / kill     │               │  - tail NDJSON          │      │
│   │  - reader threads       │               │  - notify watch         │      │
│   │  - scrollback rings     │               │  - projections          │      │
│   │  - resume stopped rows  │               │                         │      │
│   └────────────┬────────────┘               └────────────┬────────────┘      │
│                │                                         │                   │
│                │                                         ▼                   │
│                │                            ┌─────────────────────────┐      │
│                │                            │ Signal router           │      │
│                │                            │  - fixed handlers       │      │
│                │                            │  - status map           │      │
│                │                            └────────────┬────────────┘      │
│                │                                         │                   │
│                │  inject_stdin / human_question /        │                   │
│                │  status                                 │                   │
│                │◄────────────────────────────────────────┘                   │
│                ▼                                                             │
│   ┌──────────────────────────────────────────────────────────────────────┐   │
│   │ Runner session (one per slot × mission)                              │   │
│   │                                                                      │   │
│   │  ┌──────────┐   PTY   ┌─────────────────────────────────────┐        │   │
│   │  │  master  │ ◄─────► │ child: claude-code / codex / shell  │        │   │
│   │  └──────────┘         │   env: RUNNER_CREW_ID,              │        │   │
│   │                       │        RUNNER_MISSION_ID,           │        │   │
│   │                       │        RUNNER_HANDLE,               │        │   │
│   │                       │        RUNNER_EVENT_LOG, PATH=…     │        │   │
│   │                       └─────────────────┬───────────────────┘        │   │
│   └─────────────────────────────────────────┼──────────────────────────────┘ │
│                                             │ runs `runner` CLI              │
│                                             ▼                                │
│                    ┌──────────────────────────────────┐                      │
│                    │ events.ndjson  (per mission)     │                      │
│                    └────────────────┬─────────────────┘                      │
│                                     │ notify ──► EventBus (above)            │
└─────────────────────────────────────┼────────────────────────────────────────┘
                                      ▼
                          ┌──────────────────────────────┐
                          │ React + xterm.js  (webview)  │
                          │  terminals, event feed,      │
                          │  HITL cards                  │
                          └──────────────────────────────┘
```

**Two layers inside the box.** Everything runs in one Tauri 2 binary, but the components split into two distinct roles:

*Orchestration (top of the picture, lifecycle only).*

- **MissionManager** — mission lifecycle (start / stop / archive / reset), composes the per-runner system prompt at spawn, and re-mounts router + bus state for `running` missions on app restart. Under the in-process PTY runtime, child agents die with the app process; startup cleanup demotes stale `running` session rows to `stopped` so the UI can offer Resume. Once a mission is up, MissionManager goes quiet until the next lifecycle event — it is *not* in the runtime data path.

*Runtime (the hot path — the row below MissionManager).*

- **SessionManager** — the per-session PTY runtime. Holds each PTY master, runs the blocking reader thread, keeps the scrollback ring, and serializes writes. Resume is a fresh spawn against the same session row; for claude-code/codex, `agent_session_key` lets the agent CLI continue its own conversation when supported.
- **EventBus** — tails the per-mission NDJSON file with `notify`, parses each new line, and republishes it as Tauri events the webview and the router can subscribe to. "Projections" are the in-memory rollups it computes on the fly — inbox, pending HITL cards, status map — all derived from the same event stream.

**The Signal router** sits downstream of the EventBus. When a parsed line is a built-in signal type, the router runs a fixed handler. The "inject_stdin / human_question / status" arrow into SessionManager covers the three things a handler can do: write bytes into a specific session's PTY master (`inject_stdin` — launch prompt to lead on `mission_goal`, human choice on `human_response`, worker question on `ask_lead`), append a new event back to the NDJSON log so the UI renders a HITL card (`human_question`), or update the in-memory status map (`runner_status` events from the forwarder feed this).

**One session.** The session row in the middle is one slot's PTY process: SessionManager holds the master file descriptor; the child runs the agent binary with a real tty on its stdin/stdout/stderr (the slave end). The env vars are what make the bundled `runner` CLI work inside that child — when the agent runs `runner msg post …`, the CLI reads `RUNNER_MISSION_ID` + `RUNNER_EVENT_LOG` from its environment, builds the JSON line, and `flock`-appends to the right file. No daemon, no socket; the CLI just opens the file directly.

**Closing the loop.** Child invokes `runner` CLI → CLI appends a line to `events.ndjson` → `notify` wakes the EventBus → EventBus fans the line out to (a) the Signal router for handler dispatch and (b) the webview as a Tauri event. If the handler needs a wake-up, it calls back into SessionManager's writer to push bytes into a session's stdin — that's the upward arrow on the convergence point. The bus is the spine: all coordination flows through one append-only file, which is why it's debuggable with `tail -f | jq`.

**The webview** is downstream of everything. It renders each session's PTY output (subscribes to `session:{id}:out`) and the event feed + HITL cards + signal log (subscribes to `mission:{id}:event`).

**What's not in the picture.** The SQLite DB. That's deliberate — SQLite holds configuration and session-lifecycle metadata only (runners, crews, slots, mission rows, session rows with PID + runtime metadata). It is not on the runtime hot path. All live coordination state lives in the NDJSON file or in the router's in-memory map.

**The invariant this picture encodes.** There is exactly one piece of mutable shared state per mission: `events.ndjson`. Every other component is either a writer to it (the `runner` CLI; the router for `human_question`), a reader of it (EventBus → router + UI), or a per-session PTY pipeline that doesn't touch it directly. That's what makes mission coordination crash-durable and replayable — on restart, Runner re-opens the file and reconstructs router/feed projections from replay. PTY children themselves do not survive app restart under the in-process runtime; their rows become resumable stopped sessions.

## 2. Tech stack

| Layer | Choice | Why |
|---|---|---|
| Desktop shell | **Tauri 2** | Native binary + WebKit2 webview. Smaller than Electron, Rust-native plugin surface, ships dmg/AppImage cleanly. |
| Backend language | **Rust** | One language for the PTY layer, the NDJSON writer, the router, and the Tauri commands. No FFI churn. |
| Frontend | **React 19 + TypeScript** | Familiar, fast, no SSR concerns inside Tauri. |
| Styling | **Tailwind 4** + tokenized CSS variables | Design palette lives in `:root` vars; `<html data-theme>` swaps the active appearance variant. Terminal themes are separate from app chrome themes. |
| Terminal emulator | **xterm.js** (+ `xterm-addon-webgl`, fit, search, hyperlinks) | Mature, accurate ANSI/alt-screen rendering. WebGL backend keeps redraws cheap. |
| PTY runtime | **`portable-pty`** (in-process) | One OS thread per session reads bytes off the PTY master; writes go back through a tokio-mutex-guarded writer. Earlier tmux-backed runtime retired (see `docs/impls/archive/0011-pty-host-terminal-runtime.md`). |
| Persistence | **SQLite via `rusqlite`** (WAL mode) | Config + session lifecycle only; coordination state lives in the NDJSON event log. |
| Event transport | **Append-only NDJSON per mission** | Tailable with `tail -f \| jq`, crash-durable, replayable. `flock(LOCK_EX)` for cross-process append atomicity. |
| File watching | **`notify`** crate | The bus tails the NDJSON file and republishes lines as Tauri events. |
| Bundled CLI | **`runner` binary** | The agents talk to the bus through this — `runner signal …`, `runner msg post …`, `runner msg read`. Bundled with the app, dropped at `$APPDATA/runner/bin/runner` on first run, PATH-prepended per spawn. |
| Logging | **`tauri-plugin-log`** + a Rust panic hook | Writes to the OS log dir for the bundle; backtraces captured to the same file. |
| Auto-update | **`tauri-plugin-updater`** | Signed updates from the GitHub Releases manifest. Settings → About drives the check → download → restart ladder; a sidebar prompt card surfaces ready-to-install (impl 0025). |
| MCP | **`rmcp` + Unix socket + `runner-mcp` bridge** | Runner.app owns stateful tool execution; external MCP clients spawn `runner-mcp`, which bridges stdio to the app's local Unix socket. |

**Platform targets.** macOS (Apple Silicon + x64) primary; Linux (x64) best-effort. Windows is deferred — `portable-pty` works there but no one is on the validation loop.

## 3. Domain model

Domain objects split into two layers:

- **Configuration** — persistent, user-edited. Outlives missions. Runner, Crew, Slot, crew addendum.
- **Runtime** — created at mission start, torn down at mission end. Mission, Session, the in-memory router state, the per-mission shared context.

The key insight: **a Runner is config; a Session is its runtime instance** — same pattern as Crew (config) → Mission (runtime). A runner never runs on its own. A runner runs *inside a mission* as a session (or as a one-off direct-chat session outside any mission).

### 3.1 Relationship diagram

```
┌─ Configuration (persistent) ─────────┐    ┌─ Runtime (mission-scoped) ──────────────┐
│                                      │    │                                         │
│   Runner ──── Slot (per-crew handle, │    │   Session ─► PTY process                │
│      ▲             lead flag)        ┼────┼─►  (one per slot per mission;            │
│      │             ▲                 │    │      lives & dies with the mission)     │
│      │             │                 │    │                                         │
│      │             │ composes        │    │     ▲                                   │
│      │           Crew                │    │     │  spawned & owned by               │
│      │             │                 │    │     │                                   │
│      │             ├── system_prompt │    │   Mission ─── events.ndjson             │
│      │             │   addendum      │    │     │              │                    │
│      │             └── default goal  │    │     │              ├─► Signal           │
│      │                               │    │     │              └─► Message          │
│      │                               │    │     │                                   │
│      └─ direct chat session (off-bus, no mission, no router) ◄───────────────────── │
│                                      │    │     │                                   │
│                                      │    │     ├─► Router in-memory state          │
│                                      │    │     │    (pending asks, status map)    │
│                                      │    │     │                                   │
│                                      │    │     └─► Shared context: brief + roster  │
└──────────────────────────────────────┘    └─────────────────────────────────────────┘
```

A mission is a container. Everything in the runtime column is either the container itself (Mission) or an object whose lifecycle is scoped by it.

### 3.2 Runner — *one configured agent*

A reusable template: handle, display name, runtime (`claude-code | codex` today), command + args, working dir, system prompt (persona), env. **Top-level, not nested under a crew.** The same runner template can be used by many crews simultaneously, and can also be the subject of standalone direct-chat sessions.

A runner has two identifying fields:

- **`handle`** — a lowercase slug (`coder`, `reviewer`). Required, **globally unique**, immutable once set. The handle is the runner's identity in direct chats and in `from` fields when the session is not in a crew.
- **`display_name`** — free-form UI label. Editable; presentation-only.

Keeping these separate means renaming a runner for the UI doesn't break briefs or historical events.

### 3.3 Crew — *a configured team, composed of slots*

A named, persistent group of **slots**. Carries the default mission goal and the optional team-conventions addendum. It does not run. It is blueprint.

Crews are composed of **slots**, not runners directly. A slot is the indirection that lets the same runner template participate in many crews:

- **`slot_handle`** — the slot's in-crew handle (`@impl`, `@lead`). Required, unique within the crew. This is what crewmates address each other by — `runner msg post --to impl`. Different crews can carry slots with the same slot_handle filled by different runner templates, and the same runner template can carry different slot_handles in different crews.
- **`runner_id`** — which runner template fills the slot.
- **`position`** — display order within the crew.
- **`lead`** — exactly one slot per crew carries `lead = 1`, enforced by a unique partial index (§10.1).

**Why slot vs runner.** Users curate a small library of runner templates and re-use them across crews and direct chats. Tying the in-crew handle and lead flag to the runner template would force duplicating configs every time a runner shows up in a new crew.

**Lead is also the default HITL gateway.** When a worker needs human input, it does not ask the human directly — it emits an `ask_lead` signal. The router wakes the lead, who decides whether to answer from their own context or escalate via `ask_human`. The human's answer flows back to the lead, who forwards it to the original worker as a directed message. See §8.5 for the full protocol. Workers *may* emit `ask_human` directly as a fallback — the protocol doesn't forbid it — but the worker preamble (§6) instructs them to go through the lead.

### 3.4 Mission — *one activation of the crew, and the runtime container*

A mission is the only runtime container in the system. Everything alive at runtime lives *inside* a mission and dies with it:

- A **Session** per slot (the PTY processes — §3.5).
- The **coordination bus** — the NDJSON event log carrying signals and messages.
- The **router's in-memory state** — pending HITL asks and latest runner availability.
- The **shared context** — composed system prompts (brief, roster, coordination notes, optional team conventions) injected at spawn.

Lifecycle:
- **Start**: user clicks Start Mission on a crew. A mission row is created (with its own `cwd` and an optional per-mission `goal_override`), one session is spawned per slot, the router boots with fresh state, and an NDJSON file is opened.
- **Stop**: user pauses the mission. Live PTYs are killed, but the mission row remains `running`; router/bus state stays mounted and stopped slots can be resumed.
- **Archive**: user ends the mission. Runner appends `mission_stopped`, marks the row `completed`, sets `archived_at`, kills any live PTYs, and unmounts router/bus state. Archived missions are hidden from active lists and render read-only by direct URL.

**Mission cwd is authoritative.** Each mission carries its own `cwd` column. Spawned slots inherit `mission.cwd` regardless of what the runner template's `working_dir` says — that field is only used in direct chats (where there is no mission). This makes "start two missions on the same crew but in different repos" trivial.

Concurrent missions on the same crew are allowed — a crew is a reusable template, and per-mission state (sessions, the coordination bus, the runner-CLI shim path, the roster sidecar) is fully namespaced by `mission_id`.

### 3.5 Session — *one slot's PTY process*

The runtime instance of a slot inside a mission, a runner-backed direct chat, or a runtime-only direct chat. A Session is to a slot/runner/runtime what a Mission is to a Crew: the *run* of a *configuration*.

Two flavors, distinguished by whether `mission_id` is set on the session row:

- **Mission session** — spawned when a mission starts; one session per slot. It dies with the mission. The session participates in the crew's coordination bus, sees broadcasts, can receive `inject_stdin` from the router. The `RUNNER_HANDLE` env var carries the *slot* handle, not the runner template's global handle — so a runner template used as `@impl` in one crew sees `RUNNER_HANDLE=impl` there.
- **Direct-chat session** — spawned ad-hoc without a parent mission. It can be backed by a runner template or by a bare runtime selection. `mission_id` is null and the working directory lives on the session row directly. The agent is **not on any coordination bus** — there's no event log, no router, no inbox; it's just a one-on-one PTY between the human and the agent CLI. Runner-backed chats keep `runner_id`; runtime-only chats store `runner_id = NULL` plus `agent_runtime` / `agent_command`.

A session owns:
- A PTY master handle (the only object in the system with a file descriptor to a running child process).
- A blocking reader thread that drains the PTY and pushes bytes to the scrollback ring + a Tauri event stream.
- A writer for stdin injection (used by the human and by the router's fixed handlers), serialized through a tokio mutex.
- A bounded scrollback ring that survives frontend tab switches and route changes while the app process is alive.
- An exit status once the child has terminated.

A session is the only object in the system that actually *executes* code — everything else is metadata, a coordination channel, or a projection over the event log.

### 3.6 Surface hierarchy — *Project → Window → Folder → Tab → Pane*

How sessions are displayed spans durable organization and ephemeral view state. The concepts must never be blurred in code, docs, or UI copy:

- **Project** — a global, cwd-bound container that groups both missions and direct-chat tabs in the sidebar. Starting work from a project copies its cwd into the new mission/session row and records nullable `project_id`; runtime cwd precedence stays unchanged after that point. Project identity and ordering are durable, while collapse and the active project that scopes new-chat creation are per-window view state. Deleting a project only unbinds its work through `ON DELETE SET NULL`; it never archives chats/missions or touches the directory on disk.
- **Window** — a real OS window (⌘N, `File → New Window`, impl 0018). The backend's per-window subject registry (`src-tauri/src/windows.rs`) tracks every visible direct-chat subject, focus recency for duplicate-session ownership, and explicit current focus for viewed-attention semantics; it knows no tab layout beyond the reported session subjects.
- **Folder** — a user-created, collapsible sidebar group containing tabs. Folder identity, name, and order are persisted in SQLite; collapse is per-window view state. Ungrouped tabs have no folder and render below every folder.
- **Tab** — one stable, ULID-keyed group of panes rendered as exactly one sidebar row. SQLite persists its folder membership, name, order, JSON layout, and nullable completion/viewed watermarks; the layout picker mutates the same row without resetting attention state. Every active direct-chat session belongs to exactly one tab, including single-pane chats. Per-window active-tab selection remains ephemeral.
- **Pane** — one slot inside a tab, holding exactly one chat session (move-not-copy). Panes are filled from a pane's own New chat button or a sidebar pick into a focused empty pane; `⌘[` / `⌘]` cycle pane focus, `⌘W` closes the focused pane without stopping its session.

Sessions exist independently of the display tree: closing a pane or window never kills a PTY. Archiving a tab removes the tab row and archives its member sessions. Deleting a folder performs that archive for every member tab in one transaction; the `tabs.folder_id` foreign key is `ON DELETE RESTRICT`, so tabs never fall silently to the ungrouped level.

Projects and chat folders deliberately coexist. A project is the cwd-bound owner-facing grouping for missions and chats; a feature-38 folder remains chat-tab-only organization with its existing archive-on-delete lifecycle. Assigning a tab to a project does not rewrite `tabs.folder_id`, so removing the project assignment reveals the tab in its prior chat folder (or the ungrouped CHAT list).

Disambiguation: the mission workspace's per-slot terminal switcher (feature 33's "terminal tabs") predates this hierarchy and is a different, mission-scoped UI element — not a Tab in the sense above. If the mission surface ever adopts the tab/pane model, that is feature 19's deferred scope.

### 3.7 Settings surface (frontend only)

Settings is a full-window route, `/settings/:pane?`, rendered outside the app shell: its own sidebar (grouped nav + label search + "Back to app") replaces the app sidebar in the same slot — resizable and sharing the app sidebar's persisted width (`runner.sidebar.width`) so the takeover reads as continuous — with card-grouped panes in the content column (impl 0025, superseding the earlier modal). Entry points — the sidebar Settings row, the command palette entry, and `⌘,` — navigate to the route, threading the caller's location through state so the back button returns there. All settings persist to `localStorage` through the typed helpers in `src/lib/settings.ts`; there is no backend settings store yet.

Two pieces worth naming:

- **Keyboard shortcuts pane** — read-only view over the static registry in `src/lib/keymap.ts` (feature 257 v1). Handlers keep their hardcoded keys; each carries a one-line pointer back at the registry. Rebinding is a designed follow-up.
- **Update flow** — Updates merged into About: the hero card walks a five-state button ladder (check → download → restart) over the shared `useUpdate()` context, auto-checking on pane mount. When an update is ready to install, `UpdatePromptCard` floats above the app sidebar's Settings row (per-launch dismissable); the old top-center toast is gone.

## 4. Coordination primitives — *what flows between runners*

Runners don't share a programming model; they share an IM-like surface. We ship a subset in each milestone.

| Primitive | Role | Shipped | Planned |
|---|---|:---:|:---:|
| **Signal** | Typed notification; the router handles built-ins. Verb grammar. | ✅ | |
| **Message** | Prose, broadcast or directed to a specific slot. | ✅ | |
| **Inbox** | Per-slot projection: broadcasts + messages addressed to me. | ✅ | |
| **Thread** | Scoped sub-conversation within a mission. | | next |
| **Fact** | KV whiteboard; "what is currently true in this mission." | | next |
| **Mention** | Targeted `@handle` inside a message's prose. | | later |
| **Reaction** | Lightweight signal attached to a message (`👍`, `🔍`, `blocking`). | | later |

### 4.1 Signal — *"something happened, please wake the right surface"*

Short, typed, router-visible. Grammar: past-tense verb (or asker verbs like `ask_lead`, `ask_human`).

Signals are machine-readable by design. The router has fixed handlers keyed to built-in signal types (§8.5). Runners emit them when they need parent-process plumbing: wake the lead, show a human card, escalate.

A signal carries an optional `payload` (JSON) for the router and UI. Human-readable conversation belongs in messages.

### 4.2 Message — *"here's what I think"*

Prose, addressed either to the mission (broadcast) or to a specific crewmate (direct). Runner-to-runner *and* runner-to-human (via the reserved virtual handle `human` — see §8.5).

Two shapes:
- **Broadcast** — `runner msg post "<text>"`. Goes to everyone's inbox.
- **Direct** — `runner msg post --to <slot_handle> "<text>"`. Goes to that slot's inbox only. `--to human` reaches the workspace operator (rendered in the event feed).

Messages are **flat by design** — one stream per mission, no message-thread scoping, and no separate fact primitive. Each runner consumes messages through their **inbox** (§4.3). Durable conclusions belong in project files, code, commits, or normal message prose instead of a second coordination object model.

Messages and signals stay separate because:
- Signals are typed and small; router handlers key off them. Messages are prose; the router doesn't parse them.
- A signal without prose works (`approved`). Prose without a type works too. Conflating them forces every signal to carry prose and every note to carry a type.
- LLM agents already know how to use both: signals are like exit codes, messages are like comments.

### 4.3 Inbox — *"what's in my mailbox"*

Every slot has an **inbox**: the subset of the mission's messages relevant to it. The inbox is a **projection** over the event log, not a separate data structure. For the slot with handle `h`:

```
inbox(h) = all events in the mission where
          kind = "message" AND (to = null OR to = h)
```

`runner msg read` returns the calling slot's inbox, sorted by ULID (chronological). `--since <ts>` restricts to messages newer than a given ULID/timestamp so agents can poll without re-reading history.

**The inbox is pull-based.** Messages are read when the recipient runs `msg read`; the system does not automatically interrupt a busy runner every time mail arrives. Not every direct message is urgent, and auto-interrupting on every DM would blur the signal/message split (urgent vs async) and risk corrupting in-flight tool calls.

The recipient learns to read its inbox through two mechanisms:

1. **Convention** — the platform-injected worker preamble (§6 Layer 1) instructs every runner to check its inbox at natural task boundaries.
2. **Signals as the urgent wake-up** — if a sender needs the recipient to drop everything, they emit a signal in addition to (or instead of) the message. The signal goes through the router's fixed handlers, which may inject stdin.

### 4.4 Event — *the unifying transport*

Every coordination primitive is persisted as an **event** — one line in the per-mission NDJSON file. An event has:

```jsonc
{
  "id":         "01HG3K1YRG7RQ3N9...",  // ULID: time-sortable, monotonic within ms
  "ts":         "2026-04-21T12:34:56.123Z",
  "crew_id":    "01HG...",
  "mission_id": "01HG...",
  "kind":       "signal",                // signal | message
  "from":       "coder",                 // slot handle | "human" | "router"
  "to":         null,                    // null = broadcast; slot handle = directed
  "type":       "review_requested",      // for kind=signal; omitted for kind=message
  "payload":    { "...": "..." }         // kind-specific (e.g. { "text": "..." } for messages)
}
```

The `kind` field discriminates. For `kind: "signal"`, `type` carries the signal's semantic verb; for `kind: "message"`, `type` is omitted and the prose lives in `payload.text`. The router and UI project events into primitive-specific views based on `kind`.

Runners interact through CLI verbs (`runner signal`, `runner msg`), not the event schema directly — there is no separate `signal_emitted` or `message_posted` event type.

## 5. PTY session runtime

A short primer for readers without an OS-internals background. A **pseudo-terminal (PTY)** is a kernel-emulated terminal device. To the child process, it looks like a real TTY — `isatty()` returns true, `ioctl(TIOCGWINSZ)` reports a window size, signals route correctly — but the other end is just a file descriptor held by a controlling process, not a hardware terminal. The kernel exposes the pair as two endpoints:

- **slave** — what the child opens as `stdin` / `stdout` / `stderr`. Indistinguishable from a real `/dev/tty`.
- **master** — what the controlling process (Runner, here) reads from to see what the child wrote and writes to to push keystrokes into the child's stdin.

It's the same primitive `ssh`, `tmux`, and every terminal emulator (iTerm2, Alacritty, …) use under the hood.

For the rigorous treatment, see Stevens & Rago, *Advanced Programming in the UNIX Environment*, **chapter 19 ("Pseudo Terminals")** — line discipline, packet mode, the `forkpty` / `openpty` helpers, and the gotchas around signal forwarding and window-size propagation. Runner doesn't reimplement any of that; we use the `portable-pty` Rust crate, which wraps the same POSIX primitives.

### 5.1 Topology at a glance

Per session, two parallel data paths flow through one PTY master / slave pair.

Output (agent → UI + idle inference):

```
   Child ──► PTY slave ──► PTY master ──► Reader thread ─┬─► Scrollback ring (~10k lines)
   (tty stdout                            (blocking      ├─► xterm.js  (session:{id}:out)
    + stderr)                              OS thread)    └─► Idle detector
                                                              └─► runner_status event
                                                                  (source: forwarder)
```

Input (UI + router → agent):

```
   xterm.js (onData) ──┐
                       ├─► Writer ──► PTY master ──► PTY slave ──► Child
   Signal router ──────┘   (tokio::Mutex,                          (tty stdin)
   (inject_stdin)           lock-serialized,
                            one write_all per turn)
```

The PTY master is the hinge: held in Rust by SessionManager, written to by the mutex-guarded Writer, read from by the blocking reader thread. Master ↔ slave runs in-process via `portable-pty` (no external multiplexer).

Bus side (orthogonal to the PTY, drawn for completeness):

```
   Child ──► `runner` CLI on PATH ──► events.ndjson ──► notify ──► Signal router
                                       (per mission,                  │
                                        flock + append)               └─► back to Writer
                                                                          on wake-up signals
```

The whole system runs many of these side-by-side — one per slot per live mission, plus one per active direct chat — each with its own reader thread, writer mutex, and scrollback ring. The router and the bus are mission-scoped, so they fan out across every session in the same mission.

### 5.2 Why PTY (not pipes)

Claude Code and Codex are TUIs. They check `isatty()`; if false, they degrade (no colors, no spinner, sometimes outright refuse). Their output is a stream of escape sequences (`\x1b[2K`, alt-screen toggles) that only a terminal emulator can render.

A pseudo-terminal gives the child a real terminal on stdin/stdout/ stderr (full TUI mode) and hands us the master end as a byte stream that we forward to **xterm.js** in the webview.

### 5.3 Spawn

`portable-pty` is the in-process PTY library. The session runtime is encapsulated behind a `SessionRuntime` trait so the storage layer (SessionManager) doesn't know whether the runtime is in-process PTY, a tmux multiplexer, or anything else; today only `PtyRuntime` is shipped.

```
portable_pty::openpty(rows, cols)
  ├─ master handle  → kept by SessionManager
  └─ slave handle   → given to child via spawn_command()

Child inherits (mission session):
  PATH              = $APPDATA/runner/bin:<login-shell PATH>
  RUNNER_CREW_ID    = <ulid>
  RUNNER_MISSION_ID = <ulid>
  RUNNER_HANDLE     = <slot_handle>
  RUNNER_EVENT_LOG  = $APPDATA/runner/crews/<crew>/missions/<mission>/events.ndjson
  TERM              = xterm-256color
  COLORTERM         = truecolor
  <login-shell proxy env: HTTP_PROXY/HTTPS_PROXY/NO_PROXY>

Reader thread (blocking):
  loop {
    read(master)
      → emit session:{id}:out event,
        push to scrollback ring,
        feed PTY-silence idle detector
  }
  on EOF: wait(child) → emit session:{id}:exit { code } → update sessions row
```

System prompt content is delivered to the runtime via its native flag for the lead (`--append-system-prompt` for claude-code, the equivalent for each runtime) and via a positional-argv first-turn body for workers when the runtime accepts one. The runtime adapter in `router::runtime` owns the per-runtime mapping.

### 5.4 Frontend wiring and human takeover

- On first view: fetch the session's scrollback ring; write to xterm.js to restore history.
- Subscribe to `session:{id}:out` for live output.
- xterm.js `onData` → `send_input(session_id, bytes)` → `master.writer.write_all(bytes)`.
- Frontend window resize → debounced `master.resize(rows, cols)` → SIGWINCH to child. Non-optional; without it, TUIs mis-render.

**Human takeover is a first-class capability.** At any moment, the human can type directly into any runner's stdin — the same writer the router uses for stdin pushes. The human can step in to answer a prompt the agent is stuck on, correct a bad plan, kill a runaway tool call, or just chat with the agent mid-flight.

The xterm pane is a real terminal, not a log viewer. Special keys (arrows, Enter, Ctrl-C) pass through untouched. The agent on the other end can't tell whether the bytes came from the router, the human, or its normal terminal input — which is the point.

### 5.5 Sessions outlive the UI, not the app process

Sessions live in the Rust backend and belong to the mission, not to any webview or tab. Closing the mission control window does *not* kill the sessions — the agents keep running, events keep flowing into the NDJSON file, and the router keeps handling live signals. Re-opening the window re-attaches: the frontend fetches each session's scrollback ring to rebuild xterm state, then subscribes to live output from wherever it was.

**Rows persist across app restart; PTY children do not.** With the in-process `portable-pty` runtime, child agents die with Runner. On next launch, Runner re-mounts router/bus state for `running` missions, replays the NDJSON log, then demotes stale `running` session rows to `stopped`. The workspace can still show durable mission context and Resume controls, but Resume spawns a fresh PTY against the same session row.

The things that end a live PTY are: user clicks Stop/Archive, the child process exits, the app quits, or Runner explicitly kills the process tree.

### 5.6 Writer serialization

The PTY master writer is shared between the human (via `send_input` command) and the router (via stdin pushes). Concurrent writers could interleave bytes mid-line, confusing the TUI on the other end. Each session's writer is wrapped in a `tokio::sync::Mutex`; every write is one `write_all` call under the lock.

### 5.7 Threads, not async

`portable-pty`'s reader is blocking. Spawn one OS thread per session for the read side. Writers stay on the Tauri async runtime (writes are short and contended only at the millisecond scale).

### 5.8 Scrollback

Bounded raw-byte ring per session in SessionManager. It survives tab switches, route changes, and late workspace attachment while the app process is alive. It does not survive app restart, and there is no on-disk scrollback overflow today. The ring sees raw bytes including alt-screen toggles — acceptable because the frontend replays through xterm.js which can absorb them.

Resume preserves the ring for claude-code (impl 0024): it paints inline into the main screen, so kept scrollback + resume banner + tail repaint is what a physical terminal would show, and a later remount replay keeps the pre-resume conversation. The ring stays bounded and process-local as before — old and new bytes share the same cap. Codex still purges on resume: it repaints its whole frame (and its own resume replay restores a deep conversation tail), so retained scrollback under the new frame stacks garbled content. Either way `resume` stamps a seq watermark first; the frontend's starting/resuming pills only honor TUI-ready escapes above it, so retained pre-resume bytes can't clear an overlay that's waiting on the new PTY.

### 5.9 Death and kill

Reader thread owns the child handle. On EOF, it calls `wait()`, emits `session:{id}:exit`, updates the sessions row. No auto-restart.

Kill: drop master → SIGHUP via `portable-pty`; escalate to SIGKILL if the child lingers.

### 5.10 Busy / idle inference

Per-runner busy/idle is inferred from PTY-byte silence by the session forwarder, not reported by the agent via `runner status`. For mission sessions the forwarder appends a `runner_status` event with `source: "forwarder"` to the mission log, and the router maps it into the workspace status projection. Direct chats stay off-bus: `SessionManager` retains their latest live activity, exposes it through `session_activity_snapshot`, and emits `session/status` transitions to every window. Direct spawn/resume stores and emits an initial busy state before the first PTY byte; teardown removes the snapshot entry without synthesizing a completion.

The sidebar subscribes to `session/status` before hydrating the snapshot and replays any transitions that raced with the request. It aggregates activity at the durable tab level: any busy running member shows the spinner, and the final busy-to-idle transition records `tabs.last_completed_at`. A focused window displaying any tab member records the same completion as viewed; otherwise `last_completed_at > last_viewed_at` restores the unread dot across navigation, windows, and restart. Tab activation reports the target tab's full subject set and advances its viewed watermark in one backend command, while `chat/tab-attention-changed` rehydrates every window without masquerading as a layout mutation.

The `runner status busy|idle` CLI verb is kept as a back-compat alias only — it stamps `source: "agent"` so debug tooling can tell agent-reported events apart from forwarder inference, prints a stderr deprecation notice, and is slated for removal.

## 6. System prompt composition

Every spawned session receives a composed system prompt — different shape for workers, the lead, and direct chats. The composition is mechanical: pure functions over slot + crew + mission inputs, no LLM in the loop. Source of truth lives in `src-tauri/src/router/prompt.rs`.

### 6.1 The three layers

The mission spawn path composes each runner's effective prompt from three layers, applied in this order:

1. **Layer 1 — platform preamble** (code-owned, not editable). For non-lead workers: a fixed block describing the `runner` CLI verbs (`msg read`, `msg post`, `signal ask_lead`), how to reply to the human (`runner msg post --to human "…"`), and the pull-based inbox convention. For the lead: the launch prompt composed at `mission_goal` time (§6.3), including the goal, the roster, and the allowed-signals list.
2. **Layer 2 — crew team conventions** (data-owned, optional — `crews.system_prompt_addendum`). Spliced under a `== Team conventions ==` section between Layer 1 and Layer 3. Empty / NULL = no splice. Lets a crew share house rules without editing every runner template.
3. **Layer 3 — runner persona** (data-owned — `runners.system_prompt`). The role brief: who the runner is and what they do. Spliced under `== Your brief ==`.

### 6.2 What each session sees

| Session kind | Layer 1 | Layer 2 | Layer 3 | Delivery |
|---|:---:|:---:|:---:|---|
| Mission worker | preamble | if set | persona | first-turn body (argv when the runtime accepts it, otherwise stdin paste) |
| Mission lead | launch prompt (composed by router on `mission_goal`) | if set | persona | runtime's append-system-prompt flag at spawn + router-injected launch body to stdin on `mission_goal` |
| Direct chat | — | — | persona | runtime's append-system-prompt flag at spawn |

Direct chats see *only* Layer 3 — the worker preamble's verbs and the team conventions don't make sense off-bus.

Example for a worker slot `reviewer` filled by a `reviewer` runner template:

```
You are a worker in a crew coordinated by the bundled `runner` CLI…
[Layer 1 preamble: verbs, inbox convention, replying to human]

== Team conventions ==        ← Layer 2, if crew.system_prompt_addendum set
…

== Your brief ==              ← Layer 3
When `coder` requests review, read their messages and the diff,
then either approve or request changes with specific feedback.
```

### 6.3 The lead's launch prompt

The lead's startup prompt is short — just the runner's persona via the runtime's append-system-prompt flag. The full mission picture arrives later: once `mission_goal` fires, the router composes a launch-prompt body (goal, roster, allowed signals, addendum) and writes it to the lead's stdin. This separation keeps the spawn fast and lets the user edit the goal up to the moment they click Start Mission.

The composed launch prompt covers:

- The lead's identity (`You are <slot_handle> (Display Name), the lead runner in crew "<crew name>"`).
- The mission goal (from `missions.goal_override` or `crews.goal`).
- The roster — every crewmate's slot_handle, display name, and lead/worker tag.
- The team-conventions addendum (Layer 2), if set.
- The known signal types (from `runner_core::model::KnownSignalType`) as the coordination vocabulary.
- A reminder of the lead's job: dispatch via directed messages, absorb `ask_lead` traffic, escalate via `ask_human` only when needed.

## 7. Coordination bus

### 7.1 Transport

```
$APPDATA/runner/crews/{crew_id}/missions/{mission_id}/events.ndjson
```

One line per event. Append-only. Each mission has its own file — scopes log rotation, crash-replay, and deletion.

Why a file instead of an in-memory bus:
- **Debuggable** — `tail -f events.ndjson | jq .`.
- **Crash-durable** — whatever's on disk survived the crash.
- **Atomic** under explicit guards (§7.1.1) — concurrent `runner` invocations interleave correctly at line granularity.
- **Replayable for projections** — restart the router, re-scan pending asks and runner status, resume live tail.

#### 7.1.1 Concurrent-write correctness

Multiple runners can invoke `runner signal` / `runner msg` at the same time from different PTYs. We need line-granular atomicity regardless of filesystem:

1. Open the log with `O_APPEND | O_WRONLY | O_CREAT`.
2. Acquire an advisory exclusive lock: `flock(fd, LOCK_EX)`.
3. Emit exactly one `write(2)` call with the serialized JSON line including the trailing `\n`.
4. `close(fd)`, which releases the lock.

This gives us:
- **Ordering**: `O_APPEND` guarantees the write lands at end-of-file at the moment the kernel performs it.
- **Atomicity across writers**: `flock(LOCK_EX)` serializes writers. Small-write atomicity on regular files is filesystem-specific; we don't rely on it.
- **No partial lines**: a single `write(2)` of the full line + `\n` under the lock means the whole line lands or none of it does.

**Filesystem requirements.** The app data directory must be on a local POSIX filesystem (APFS, ext4, XFS, …). Network filesystems (NFS, SMB) and iCloud-synced volumes may not honor `flock()` or may re-order appends across clients; we document this and check at app startup.

Writers: the bundled `runner` CLI writes runner-authored events; the Rust backend writes router-generated events (`human_question`, `mission_warning`, `runner_status` from the forwarder, …) through the same `flock`-guarded path. No other process should write to this file.

### 7.2 Consumers

Two subscribers to the NDJSON file, both via `notify`:

- **Signal router** — deserializes each new line. For built-in signals, runs a fixed handler. For messages, no-op; messages stay flat and are not routed into thread/fact projections.
- **EventBus → UI** — the backend re-emits each line as a `mission:{id}:event` Tauri event. Frontend splits by `kind` into the event feed and the HITL/signal projections.

#### Startup replay

On router boot: open the mission's file, fold `human_question` / `human_response` and `runner_status` rows into in-memory state, then switch to tailing from the current end of the log. Replay rebuilds projections; it does not re-run historical stdin pushes.

## 8. Signal router

The router is a flat dispatcher, not a policy engine. There is no per-crew `{when, do}` rule list. The lead runner owns coordination judgment; the router owns parent-process plumbing that a child PTY cannot do itself.

Stdin pushes are deliberately silent: the router writes bytes into the target PTY but does not synthesize `stdin_injected` audit events. The event log records the signal that caused the push, plus `human_question` / `human_response` for HITL cards.

### 8.1 Fixed handler table

| Signal type | Fixed handler |
|---|---|
| `mission_goal` | Compose the launch prompt and inject it to the lead's stdin. |
| `human_said` | Inject `payload.text` to `payload.target` if present, otherwise to the lead. |
| `ask_lead` | Inject the worker's `{ question, context }` to the lead. |
| `ask_human` | Append a `human_question` event for the UI. |
| `human_response` | Look up the matching `question_id` and inject the answer to the runner that emitted the original `ask_human`. |
| `runner_status` | Update the latest-status map from `payload.state`. If a non-lead reports `idle`, inject a short availability update to the lead. |
| `inbox_read` | Internal — used by the event-feed projection to track read watermarks. Not user-visible. |

### 8.2 `ask_human` — payload shapes and matching

`ask_human { prompt, choices }` produces two correlated signals:

```jsonc
// When the card is shown:
{
  "id":   "01HG...",                            // canonical question_id (use this in human_response)
  "kind": "signal",
  "type": "human_question",
  "from": "router",
  "payload": {
    "triggered_by": <triggering-signal.id>,     // e.g. the changes_requested signal's id
    "prompt":       "Reviewer requested changes. Accept or override?",
    "choices":      ["accept", "override"],
    "on_behalf_of": "@impl"                     // optional; see "Lead-mediated asks" below
  }
}

// When the human clicks a choice:
{
  "kind": "signal",
  "type": "human_response",
  "from": "human",
  "payload": {
    "question_id": <human_question.id>,         // = the card event's `id` field
    "choice":      "accept"                     // the clicked value (always one of choices[])
  }
}
```

Causality is carried in-payload rather than on the envelope. The canonical `question_id` is the `human_question` event's own `id` field, assigned at flock-guarded log-append time.

### 8.3 Lead-mediated asks (the canonical pattern)

By convention (§3.3), workers do not escalate to the human directly:

1. **Worker asks the lead.** Worker emits `ask_lead` with the question in its payload. The router's fixed handler injects the worker's `{ question, context }` to the lead.
2. **Lead decides.**
   - **Answer from own context.** Lead posts a directed message to the worker via `runner msg post --to <handle> "…"`. The worker picks it up on its next `runner msg read`. Pull-based; no new wake-up needed.
   - **Escalate to human.** Lead emits `ask_human` with `payload.on_behalf_of: "<handle>"`. The router appends `human_question`; the UI uses `on_behalf_of` to show the attribution chain (*@impl → @architect → you*).
3. **Human responds.** The router injects the result into the lead's stdin (the lead was the asker of record). The lead forwards the answer via a directed message to the original worker.

This is not a new protocol — it is `ask_lead` + `ask_human` + directed messages composed. The only schema additions are the `ask_lead` signal type and the optional `on_behalf_of` field on `human_question`.

### 8.4 The reserved `human` handle

`human` is a reserved virtual recipient. Runners reply to the human via `runner msg post --to human "<text>"` — the event appears in the workspace feed and is what humans read. This is how workers reply in-feed without needing a `human_said`-style inverse signal.

### 8.5 Who does delivery

| | Sender addresses recipient? | Delivery timing | Router in path? |
|---|:---:|---|:---:|
| Signal | No — fixed handler decides | Immediate for wake-up handlers | Always |
| Broadcast message | No | On recipient's `msg read` | No |
| Direct message | Yes (`--to`) | On recipient's `msg read` | No |

**Messages do not trigger router actions.** The inbox is pull-based. If a sender needs the recipient to drop everything, they emit a signal — signals are the urgent wake-up channel, messages are async conversation.

## 9. The `runner` CLI

The bundled CLI is the agent-facing surface for everything in §4–§8. Spawned children invoke it directly (it's prepended onto their `PATH` at spawn — §5.3) to participate in the bus: emit signals, post messages, read their inbox. There is no other supported way for an agent to talk to the rest of the crew; the CLI is the "communication infrastructure" from the agent's point of view.

### 9.1 Surface

```
runner signal <type> [--payload <json>]
runner msg    post <text> [--to <handle>]
runner msg    read [--since <ts>] [--from <handle>]
runner status busy|idle [--note <text>]      (deprecated)
runner help
```

One binary, two real verbs (`signal`, `msg`) plus the deprecated `status` alias and a `help` entry point. Context always comes from env vars injected at spawn (`RUNNER_CREW_ID`, `RUNNER_MISSION_ID`, `RUNNER_HANDLE`, `RUNNER_EVENT_LOG`); the CLI is otherwise stateless and side-effect-free outside of the one log append.

### 9.2 Verb-by-verb

- **`signal <type> [--payload <json>]`** — append a `kind: signal` event to the mission log. The router picks it up via §7.2's notify tailer and runs its fixed handler (§8.1). `--payload` is free-form JSON; the router interprets it per signal type.
- **`msg post <text>`** — broadcast: append a `kind: message` event with `to: null`. Lands in every slot's inbox.
- **`msg post --to <handle> <text>`** — directed: append a `kind: message` event with `to: <handle>`. Lands in that slot's inbox only. `--to human` reaches the workspace operator (the reserved virtual recipient, §8.4).
- **`msg read [--since <ts>] [--from <handle>]`** — the inbox-read projection (§4.3). Returns broadcasts plus directs addressed to me, sorted by ULID. `--since` filters by ULID cutoff for poll-without-rewind; `--from` filters by sender.
- **`status busy|idle [--note <text>]`** — **deprecated.** Busy/idle is now inferred by the session forwarder from PTY-byte silence (§5.10). The verb is kept as a back-compat alias (the event is stamped `source: "agent"` so debug tooling can tell agent-reported events apart from forwarder-inferred ones) and prints a stderr deprecation notice. Bundled templates no longer call it; slated for removal in a future release.
- **`help`** — long-form usage from `cli/src/help.rs`. Mirrors this section.

### 9.3 What the CLI does *not* do

- **No event-DAG flags.** No `--correlation-id`, no `--causation-id`. Causality is implicit in ULID ordering, or in-payload where it has to be explicit (e.g. `human_response.payload.question_id` matches a `human_question` card's `id` — §8.2).
- **No daemon, no socket.** Each invocation is a one-shot process: read env, build the event, `flock` + append to `RUNNER_EVENT_LOG`, exit. The bus is the file; nothing else needs to be alive.
- **No per-crew allowlist.** The CLI validates `<type>` against the closed `runner_core::model::KnownSignalType` enum — one place to add a built-in signal type, no DB column or sidecar to keep in sync (feature 20).

### 9.4 Direct chats: the CLI is absent

Direct-chat sessions (§3.5) don't get the bundled CLI on PATH — there is no bus to write to, no router to wake, no inbox to read from. If an agent in a direct chat were to invoke `runner …` (e.g. because its system prompt was copied from a mission template), the command simply isn't found. This is deliberate: direct chats are one-on-one with the human, so the coordination verbs would be misleading.

## 10. Data model

### 10.1 SQLite (config + session lifecycle)

```sql
crews (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  purpose TEXT,                       -- short prose shown in Crew Detail; optional
  goal TEXT,                          -- default mission goal
  orchestrator_policy TEXT,           -- DEPRECATED (#247): superseded by system_prompt_addendum; retained but unused
  system_prompt_addendum TEXT,        -- Layer-2 team conventions; nullable
  created_at TEXT, updated_at TEXT
);

runners (
  id TEXT PRIMARY KEY,
  handle TEXT NOT NULL UNIQUE,        -- globally unique slug; see §3.2
  display_name TEXT NOT NULL,
  runtime TEXT NOT NULL,              -- first-class runtime key; claude-code | codex today
  command TEXT NOT NULL,
  args_json TEXT,
  working_dir TEXT,                   -- direct-chat working dir; missions override via mission.cwd
  system_prompt TEXT,                 -- Layer 3 persona
  env_json TEXT,
  model TEXT,                         -- optional model override (e.g. codex GPT-5 effort)
  effort TEXT,                        -- optional effort override
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

-- Crew membership lives on slots; each slot is one position inside
-- one crew, filled by one runner template. The same runner can fill
-- slots in many crews (§3.3).
slots (
  id TEXT PRIMARY KEY,
  crew_id TEXT NOT NULL REFERENCES crews(id) ON DELETE CASCADE,
  runner_id TEXT NOT NULL REFERENCES runners(id) ON DELETE CASCADE,
  slot_handle TEXT NOT NULL,          -- the in-crew handle (@impl, @reviewer); unique within crew
  position INTEGER NOT NULL,          -- display order; unique within crew
  lead INTEGER NOT NULL DEFAULT 0,
  added_at TEXT NOT NULL,
  UNIQUE (crew_id, slot_handle),
  UNIQUE (crew_id, position)
);

-- Exactly one lead per crew.
CREATE UNIQUE INDEX one_lead_per_crew ON slots(crew_id) WHERE lead = 1;

projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  cwd TEXT NOT NULL,
  position INTEGER NOT NULL,
  created_at TEXT NOT NULL
);

missions (
  id TEXT PRIMARY KEY,
  crew_id TEXT NOT NULL REFERENCES crews(id) ON DELETE CASCADE,
  project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
  title TEXT NOT NULL,
  status TEXT NOT NULL,               -- running | completed | aborted
  goal_override TEXT,                 -- null → inherit crews.goal
  cwd TEXT,                           -- authoritative working dir for slot spawns
  started_at TEXT NOT NULL,
  stopped_at TEXT,
  archived_at TEXT,                   -- non-null → read-only history; hidden from search
  pinned_at TEXT
);

sessions (
  id TEXT PRIMARY KEY,
  -- Nullable: direct-chat sessions exist without a mission (§3.5).
  -- For mission sessions, deleting the mission detaches the session
  -- (SET NULL) so historical session rows survive for stats.
  mission_id TEXT REFERENCES missions(id) ON DELETE SET NULL,
  project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
  runner_id TEXT REFERENCES runners(id) ON DELETE CASCADE,
  slot_id TEXT,                       -- back-reference to the slot a mission session filled
  cwd TEXT,                           -- direct-chat working dir
  status TEXT NOT NULL,               -- running | stopped | crashed
  pid INTEGER,
  started_at TEXT, stopped_at TEXT,
  -- Runtime metadata for the live in-process PTY handle. The legacy
  -- socket/window/pane columns are retained but not written by new rows.
  runtime TEXT,                       -- which runtime owns the live handle (native-pty)
  runtime_socket TEXT,
  runtime_session TEXT,
  runtime_window TEXT,
  runtime_pane TEXT,
  runtime_cursor INTEGER,
  -- Agent-side resume key captured at spawn so Resume can ask the CLI to
  -- continue the prior conversation after a stop or app restart.
  agent_session_key TEXT,
  agent_runtime TEXT,                 -- runtime-only direct chat identity
  agent_command TEXT,
  archived_at TEXT,
  title TEXT,                         -- direct-chat title; null for mission sessions
  pinned_at TEXT
);

folders (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  position INTEGER NOT NULL,
  created_at TEXT NOT NULL
);

tabs (
  id TEXT PRIMARY KEY,
  folder_id TEXT REFERENCES folders(id) ON DELETE RESTRICT,
  name TEXT NOT NULL,
  position INTEGER NOT NULL,
  layout TEXT NOT NULL,              -- JSON: preset, slot assignments, split sizes
  created_at TEXT NOT NULL,
  last_completed_at TEXT,
  last_viewed_at TEXT
);
```

Migrations live in `src-tauri/migrations/`. Schema changes are forward-only — no down migrations. Pre-release migrations were squashed into `0001_init.sql`; subsequent migrations are individual files.

### 10.2 Filesystem

```
$APPDATA/runner/
├── bin/
│   ├── runner                                # bundled agent CLI (signal + msg)
│   └── runner-mcp                            # stdio MCP bridge
├── mcp.sock                                  # local MCP socket while app is running
├── runner.db                                 # SQLite (WAL)
└── crews/
    └── {crew_id}/
        └── missions/
            └── {mission_id}/
                └── events.ndjson             # per-mission event log
```

macOS: `$APPDATA` = `~/Library/Application Support/com.wycstudios.runner`. Dev builds use a `-dev` suffix so dev and prod data are isolated.

**Mission sessions** share their mission's directory: their scrollback rings live in memory in SessionManager (no on-disk overflow today), and the only durable artifact for the mission is the `events.ndjson` log.

**Direct chats are off-disk.** Because a direct-chat session has no mission, no event bus, and no router, it has no filesystem footprint beyond its row in `sessions` (SQLite). The scrollback ring stays in memory; the PTY child writes nothing of its own to disk that Runner manages. This is why a direct chat can be started, used, and ended without ever touching `$APPDATA/runner/crews/`.

**Log files (out-of-band).** `tauri-plugin-log` writes to the OS log dir, not into `$APPDATA/runner/`. On macOS that's `~/Library/Logs/com.wycstudios.runner/runner.log`; the panic hook writes backtraces to the same file. Dev builds use a separate `com.wycstudios.runner-dev` subdir.

## 11. Process and thread model

Runner is a two-process system at runtime: the **Tauri backend process** (Rust) and the **webview process** (WebKit running our React bundle). They communicate through Tauri's IPC bridge — commands one way, events the other. Everything in this section is about the backend; the webview is single-threaded JavaScript and doesn't need separate explanation.

### 11.1 The shape inside the backend

```
Tauri backend process
  ├── Tauri main thread
  │     └── Tauri command dispatch + event emission
  │
  ├── Tokio async runtime (multi-threaded)
  │     ├── MissionManager (async; mostly idle between lifecycle events)
  │     ├── SessionManager command surface (async writers, command handlers)
  │     ├── Per live mission:
  │     │     ├── EventBus tailer task (notify watcher → broadcast)
  │     │     └── Signal router task (consumes the bus, runs handlers)
  │     └── Tauri command handlers (CRUD, session_spawn, mission_start, …)
  │
  └── OS threads (blocking, one per active session)
        └── Per session: blocking PTY reader thread
              (reads from PTY master → scrollback ring,
               session:{id}:out Tauri event, idle detector)

⇕ IPC bridge ⇕

Webview process (single-threaded JS)
  └── React + xterm.js
        ├── Subscribes to session:{id}:out, mission:{id}:event
        └── Sends Tauri commands on user interaction
```

### 11.2 Why a thread per PTY reader (not async)

`portable-pty`'s read side is **blocking** — there's no `AsyncRead` adapter that's correct on macOS without polling hacks. Putting it on the tokio runtime would either (a) block a runtime thread indefinitely (starving other tasks) or (b) require spinning a timer to poll, which costs latency and CPU. An OS thread that does one blocking `read(2)` in a loop is the right shape: the kernel parks it cheaply when there are no bytes, wakes it instantly when there are, and the cost of one OS thread per session is negligible at our scale.

The write side stays async. Writes are short (keystrokes, stdin pushes are tens of bytes), the `tokio::sync::Mutex` around the writer is fine on the async runtime, and there's no read-side blocking concern.

### 11.3 What runs per-mission vs. app-wide

| Lifetime | Components |
|---|---|
| App-wide (one of each) | Tauri main thread, tokio runtime, MissionManager, SessionManager, the SQLite connection pool, the webview process, the `tauri-plugin-log` writer. |
| Per live mission | One EventBus tailer task + one Signal router task + the `notify` watcher feeding them, all wired to that mission's NDJSON file. |
| Per active session | One blocking OS thread (the PTY reader) + the per-session writer mutex + the scrollback ring. |

The "per live mission" tasks come up at mission start (or app restart for missions in `status='running'`) and shut down when the mission is archived. A reversible Stop kills PTYs but leaves router/bus state mounted. Direct chats don't have these — they have only the per-session thread + writer + ring while their PTY is live.

### 11.4 Cost model

The target scale is one operator with a handful of concurrent missions and ≤ ~10 sessions in total. At that scale:

- ~10 blocking OS threads — well under any OS limit; cheap.
- 1 tokio runtime with the default multi-threaded scheduler.
- A few notify watchers (one per live mission) — cheap.
- One SQLite connection in WAL mode for the whole app — reads happen on whatever task needs them, the connection's own lock serializes.

The model scales until the **OS threads × stack size** product or the **notify watcher count** becomes meaningful, which neither will at this product's footprint. If we ever needed to host tens of missions per process, the right move is to put the PTY reads behind an event loop (`kqueue` / `epoll` polling against the PTY master's fd) and drop the per-session OS thread. We don't need that today.

### 11.5 Failure isolation

A panic in a PTY reader thread only affects that one session — the reader thread's job is to push bytes into channels; if it dies, the channels close and the session is marked stopped. A panic in a tokio task only affects its task (Tokio doesn't unwind across tasks). A panic on the Tauri main thread takes the whole app down, and is what the panic hook (§10.2) is set up to capture to the log file before exit.

## 12. Architectural bets

1. **Mission is the runtime unit.** Crew is config; mission is a run.
2. **Slot is the indirection** that lets one runner template participate in many crews and direct chats without duplication.
3. **PTY in-process via `portable-pty`, not pipes, not tmux.** TUI fidelity is non-negotiable; the multiplexer adds operational surface area we no longer need.
4. **NDJSON file per mission, not a broker.** Debuggable and crash-durable.
5. **CLI wrapper for spawned agents; MCP for external controllers.** Mission agents communicate through the bundled `runner` CLI on PATH. Outside agents/tools use the MCP bridge to inspect and operate Runner itself.
6. **Signals and messages as distinct primitives.** Keeps the router simple and prose natural.
7. **The signal router is the only urgent wake-up path.** Runners stay decoupled.
8. **Prompt composition at spawn time (Layer 1/2/3).** Replaces runtime handshakes.
9. **Small vocabulary.** Signals + messages are the coordination model. Thread and fact primitives are intentionally not supported; add new vocabulary only when it unlocks a concrete shipped workflow.
10. **xterm.js for rendering.** Don't reinvent the terminal emulator.
11. **ULID for event IDs.** Sortable, monotonic within ms.
12. **Mission state outlives the app process; PTYs do not.** The event log and session rows are the authoritative continuation point, and Resume creates fresh child processes.

## 13. What would break this architecture

- A runtime with no way to inject a system prompt at spawn (we'd type into stdin post-spawn — ugly but doable).
- An agent that won't learn to call CLI tools.
- NDJSON append atomicity breaking on an exotic filesystem (NFS, iCloud-synced). App data must be on a local POSIX filesystem.
- A target platform where `portable-pty` semantics differ meaningfully from POSIX PTYs (Windows is the standing example — why it's deferred).
