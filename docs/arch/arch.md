# Runner — Architecture

> Companion to [`../product/vision.md`](../product/vision.md). The vision doc defines *what* we're building and why; this doc defines *how* it works — tech stack, the model concepts the code is built around, and the protocol / schema decisions that make the model work. Rewritten 2026-08-22 (M6.5) for the native GPUI app (shipped as `v0.6.0` on 2026-08-23; §14 updated then); the Tauri + xterm.js era version is in history (`git show 0e5ea18:docs/arch/arch.md`) and the port that replaced it is recorded in [`../impls/gpui-rewrite/`](../impls/gpui-rewrite/README.md).

## 1. Overview

Runner is a local macOS desktop app. A user configures a **crew** of CLI coding agents, launches a **mission** to activate it, and watches the crew coordinate in real time. The app is one native process: a GPUI user interface, a Rust application core (`crates/runner-backend`), an `alacritty_terminal` grid per live session, SQLite for configuration, and a per-mission NDJSON file for live coordination state. There is no webview, no IPC bridge, and no serialization between the PTY and the screen.

### 1.1 Runtime picture

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ Runner.app — one process                                                     │
│                                                                              │
│  UI (crates/runner-app, GPUI main thread)                                    │
│   windows · sidebar · tabs/panes · mission workspace + feed · settings       │
│   terminal element paints each pane's grid; keys/IME/mouse → PTY input       │
│          ▲ wake + AppEvents                       ▲ grid            │ bytes  │
│          │                                        │                 ▼        │
│  ┌───────┴────────────┐        ┌──────────────────┴──────────────────────┐   │
│  │ AppStore           │        │ TerminalBridge registry                 │   │
│  │  snapshots of rows │        │  (crates/runner-terminal)               │   │
│  │  + reactions       │        │  one alacritty Term per live session,   │   │
│  └───────▲────────────┘        │  fed raw bytes on the ingestion thread  │   │
│          │ AppEvent broadcast  └──────────────────▲──────────────────────┘   │
│          │ (mission/changed, session/*, …)        │ SessionEvents::output    │
│  ════════╪════════════════════════════════════════╪══════════════════════    │
│  Core (crates/runner-backend, AppCore)            │                          │
│   ┌──────┴───────────┐  ┌──────────────┐  ┌───────┴────────────────────┐     │
│   │ EventBus         │  │ Router       │  │ SessionManager             │     │
│   │  notify tailer   │─►│  handlers    │─►│  PTY runtime (hot path)    │     │
│   │  per mission     │  │  delivery    │  │  spawn/kill/resume, reader │     │
│   │  + projections   │  │  gate/outbox │  │  threads, writers, sizes   │     │
│   └──────▲───────────┘  └──────────────┘  └───────┬────────────────────┘     │
│          │                                         │ PTY master             │
│   ┌──────┴───────────┐  ┌──────────────┐  ┌───────┴────────────────────┐     │
│   │ events.ndjson    │◄─│ MissionMgr   │  │ child: claude-code / codex │     │
│   │  per mission     │  │ (ops::mission│  │   / trae / shell           │     │
│   └──────▲───────────┘  │  lifecycle)  │  │  env: RUNNER_*, PATH=…     │     │
│          │ flock append └──────────────┘  └───────┬────────────────────┘     │
│          └────────────────────────────────────────┘ runs `runner` CLI        │
│                                                                              │
│   MCP server (rmcp, Unix socket $APPDATA/mcp.sock) ◄── runner-mcp bridge ◄── external clients │
│   SQLite runner.db (rusqlite + r2d2, WAL) — config + session lifecycle, off the hot path │
└──────────────────────────────────────────────────────────────────────────────┘
```

**Three layers inside the box.**

*Orchestration (lifecycle only).* **MissionManager** (`ops::mission`) starts, stops, archives and resets missions, composes each runner's system prompt at spawn, and re-mounts router + bus state for `running` missions on launch. Once a mission is up it goes quiet; it is not in the runtime data path.

*Runtime (the hot path).* **SessionManager** owns each PTY master, the blocking reader thread, the serialized writer, the idle detector and the session's last applied size. **EventBus** tails the per-mission NDJSON file with `notify`, parses each new line, hands it to the **Router** for handler dispatch and republishes a `mission/changed` notification for the UI. "Projections" — inbox, pending HITL cards, status map — are in-memory rollups over the same event stream.

*Presentation (the app crate).* **`AppStore`** holds read snapshots of the rows the UI renders and turns `AppEvent`s into scoped GPUI notifications. **`TerminalBridge`** (in `crates/runner-terminal`) owns one `alacritty_terminal::Term` per live session for the session's lifetime; panes borrow the terminal, they never own it. The terminal element paints the grid, and keys, IME composition and mouse events go straight back to the PTY writer.

**Two channels out of the core, deliberately different.** Terminal bytes take the synchronous path: the PTY reader thread calls `SessionEvents::output` and the bridge feeds the session's `Term` under its lock — no queue, no encoding, nothing to lag. Everything else (row changes, mission events, session lifecycle, router warnings) goes through a `tokio::sync::broadcast` of `AppEvent`s consumed by the `native-app-events` thread, which updates `AppStore` and wakes GPUI. Names in use today: `mission/changed`, `mission/resync`, `session/spawned`, `session/exit`, `session/updated`, `session/archived`, `session/status`, `session/warning`, `runner/changed`, `runner/activity`, `crew/changed`, `slot/changed`, `project/changed`, `chat/layout-changed`, `router/delivery-blocked`, `app/woke`.

**One session.** The session row is one slot's PTY process: SessionManager holds the master file descriptor; the child runs the agent binary with a real tty on stdin/stdout/stderr. The env vars are what make the bundled `runner` CLI work inside that child — when the agent runs `runner msg post …`, the CLI reads `RUNNER_MISSION_ID` + `RUNNER_EVENT_LOG` from its environment, builds the JSON line, and `flock`-appends to the right file. No daemon, no socket; the CLI opens the file directly.

**Closing the loop.** Child invokes `runner` CLI → CLI appends a line to `events.ndjson` → `notify` wakes the EventBus → the line goes to (a) the Router and (b) the UI as `mission/changed`. If a handler needs to wake a runner, it writes bytes into that session's PTY through SessionManager's writer. The bus is the spine: all coordination flows through one append-only file, which is why it's debuggable with `tail -f | jq`.

**What's not in the hot path.** SQLite holds configuration and session-lifecycle metadata (runners, crews, slots, projects, the sidebar tree, mission rows, session rows with PID, runtime metadata and last size). Live coordination state lives in the NDJSON file or in the router's memory; live screen state lives in the `Term`s.

**The invariant this picture encodes.** There is exactly one piece of mutable shared state per mission: `events.ndjson`. Every other component is a writer to it (the `runner` CLI; the router for `human_question` / `mission_warning`), a reader of it (EventBus → router + UI), or a per-session PTY pipeline that doesn't touch it. On restart Runner re-opens the file and reconstructs router/feed projections from replay. PTY children do not survive app restart; their rows become resumable stopped sessions, and sessions flagged `resume_on_launch` are re-spawned at startup.

## 2. Tech stack

| Layer | Choice | Why |
|---|---|---|
| UI framework | **GPUI** (`gpui-ce` 0.3.3, Metal) | Zed's retained-mode Rust UI: entities + elements, one process with the core, native text shaping and IME. Replaced Tauri + React in the 2026-08 rewrite. |
| Terminal model | **`alacritty_terminal` 0.26** | Grid, VTE parser, scrollback with reflow, selection, mouse/alt-screen modes. The same model Zed embeds. |
| Terminal renderer | custom GPUI element (`runner-app/src/terminal/element.rs`) | Walks the `Term` grid per frame, shapes runs through GPUI's text system; bundled MesloLGS Nerd Font is the default face, Menlo the alternative. |
| Application core | **Rust** crate `runner-backend`, UI-agnostic | SQLite, session manager, event bus, router, MCP server. The same crate could host another front end; the app crate is a consumer. |
| PTY runtime | **`portable-pty`** (in-process) | One blocking OS thread per session reads the master; writes are serialized per session. |
| Persistence | **SQLite via `rusqlite`** + `r2d2` pool, WAL | Config + session lifecycle only. Migrations in `crates/runner-backend/migrations/` (0001–0020). |
| Event transport | **Append-only NDJSON per mission** | Tailable, crash-durable, replayable; `flock(LOCK_EX)` for cross-process append atomicity. |
| File watching | **`notify`** | The bus tails the NDJSON file and republishes lines. |
| Bundled CLI | **`runner`** (`cli/`) | Agents talk to the bus through it — `runner signal …`, `runner msg post …`, `runner msg read`. Dropped at `$APPDATA/bin/runner` on first run, PATH-prepended per spawn. |
| MCP | **`rmcp`** server over a Unix socket + `runner-mcp` stdio bridge | Runner.app owns stateful tool execution (crews, runners, slots, projects, missions, direct sessions); external clients spawn `runner-mcp`, which bridges stdio to `$APPDATA/mcp.sock`. |
| Logging | **`tracing`** + rotating file layer + panic hook | `~/Library/Logs/com.wycstudios.runner/runner.log`; release filter `info`, debug builds `debug`, `RUST_LOG` overrides. |
| Updater | **Sparkle 2.9.5** via `objc2` (`updater` feature) | `SPUStandardUpdaterController`, EdDSA-signed appcast on GitHub Releases (production only; the nightly channel has no feed — see §14). |
| Packaging | `script/bundle-mac` | `.app` assembly, Developer ID codesign, notarization, DMG; `CFBundleVersion` is the build stamp. |
| Input | GPUI key dispatch + native IME (`terminal_ime.rs`) | Pinyin composition in the terminal was the hard requirement of the rewrite. |

**Platform target.** macOS on Apple Silicon, and only macOS. Intel Macs are not supported and no Intel build is planned: universal packaging shipped in `v0.6.0` and `v0.6.1` and was dropped for download size — 43 MB universal against 19 MB arm64 (Jason, 2026-08-25). Linux and Windows are out of scope, so no cross-platform fallback paths are maintained and Unix-only mechanisms are used freely.

## 3. Domain model

Domain objects split into two layers:

- **Configuration** — persistent, user-edited. Outlives missions. Runner, Crew, Slot, crew addendum, Project, the sidebar tree.
- **Runtime** — created at mission start, torn down at mission end. Mission, Session, the in-memory router state, the per-mission shared context.

The key insight: **a Runner is config; a Session is its runtime instance** — same pattern as Crew (config) → Mission (runtime). A runner never runs on its own. A runner runs *inside a mission* as a session (or as a one-off direct-chat session outside any mission).

### 3.1 Relationship diagram

```
┌─ Configuration (persistent) ─────────┐    ┌─ Runtime (mission-scoped) ──────────────┐
│                                      │    │                                         │
│   Runner ──── Slot (per-crew handle, │    │   Session ─► PTY process                │
│      ▲             lead flag,        ┼────┼─►  (one per slot per mission;            │
│      │             runtime/model/    │    │      lives & dies with the mission)     │
│      │             effort overrides) │    │                                         │
│      │             ▲                 │    │     ▲                                   │
│      │             │ composes        │    │     │  spawned & owned by               │
│      │           Crew                │    │     │                                   │
│      │             │                 │    │   Mission ─── events.ndjson             │
│      │             ├── system_prompt │    │     │              │                    │
│      │             │   addendum      │    │     │              ├─► Signal           │
│      │             └── default goal  │    │     │              └─► Message          │
│      │                               │    │     │                                   │
│      └─ direct chat session (off-bus, no mission, no router) ◄───────────────────── │
│                                      │    │     │                                   │
│   Project (cwd) ── nodes tree        │    │     ├─► Router in-memory state          │
│     (projects, tabs, missions)       │    │     │    (pending asks, status, outbox) │
│                                      │    │     └─► Shared context: brief + roster  │
└──────────────────────────────────────┘    └─────────────────────────────────────────┘
```

A mission is a container. Everything in the runtime column is either the container itself (Mission) or an object whose lifecycle is scoped by it.

### 3.2 Runner — *one configured agent*

A reusable template: handle, display name, runtime (`claude-code | codex | trae` today, plus a bare shell; `qoder` rows from before v0.6.7 stay readable but cannot spawn), command + args, working dir, system prompt (persona), env, optional model and effort. **Top-level, not nested under a crew.** The same runner template can be used by many crews simultaneously, and can also be the subject of standalone direct-chat sessions.

A runner has two identifying fields:

- **`handle`** — a lowercase slug (`coder`, `reviewer`). Required, **globally unique**, immutable once set. The handle is the runner's identity in direct chats and in `from` fields when the session is not in a crew.
- **`display_name`** — free-form UI label. Editable; presentation-only.

Keeping these separate means renaming a runner for the UI doesn't break briefs or historical events.

Runtime argv is composed by the adapter in `router/runtime.rs`, not stored: the permission mode (`--permission-mode` / codex `--ask-for-approval` + `--sandbox`), model and effort flags, codex's `--add-dir` grant for the mission directory, the first-turn body, and — for claude-code — one compact `--settings {"tui":"fullscreen"}` pair that selects Claude Code's alternate-screen renderer unless the runner's own args already pass `--settings`. Runner owns the renderer for the sessions it spawns; `--settings` outranks the user's `~/.claude/settings.json`.

### 3.3 Crew — *a configured team, composed of slots*

A named, persistent group of **slots**. Carries the default mission goal and the optional team-conventions addendum. It does not run. It is blueprint.

Crews are composed of **slots**, not runners directly. A slot is the indirection that lets the same runner template participate in many crews:

- **`slot_handle`** — the slot's in-crew handle (`@impl`, `@lead`). Required, unique within the crew. This is what crewmates address each other by — `runner msg post --to impl`.
- **`runner_id`** — which runner template fills the slot.
- **`position`** — display order within the crew.
- **`lead`** — exactly one slot per crew carries `lead = 1`, enforced by a unique partial index (§10.1).
- **`runtime_override`, `model_override`, `effort_override`** — per-slot deviations from the template, so one crew can run the same persona on two runtimes.

**Why slot vs runner.** Users curate a small library of runner templates and re-use them across crews and direct chats. Tying the in-crew handle and lead flag to the runner template would force duplicating configs every time a runner shows up in a new crew.

**Lead is also the default HITL gateway.** When a worker needs human input, it does not ask the human directly — it emits an `ask_lead` signal. The router wakes the lead, who decides whether to answer from their own context or escalate via `ask_human`. The human's answer flows back to the lead, who forwards it to the original worker as a directed message. See §8.3. Workers *may* emit `ask_human` directly as a fallback, but the worker preamble (§6) instructs them to go through the lead.

### 3.4 Mission — *one activation of the crew, and the runtime container*

A mission is the only runtime container in the system. Everything alive at runtime lives *inside* a mission and dies with it:

- A **Session** per slot (the PTY processes — §3.5).
- The **coordination bus** — the NDJSON event log carrying signals and messages.
- The **router's in-memory state** — pending HITL asks, latest runner availability, per-slot delivery outboxes.
- The **shared context** — composed system prompts (brief, roster, coordination notes, optional team conventions) injected at spawn.

Lifecycle:

- **Start**: a mission row is created (with its own `cwd` and an optional per-mission `goal_override`), one session is spawned per slot, the router boots with fresh state, and an NDJSON file is opened. Missions can be started from the UI or through the MCP `mission_start` tool.
- **Stop**: live PTYs are killed, but the mission row remains `running`; router/bus state stays mounted and stopped slots can be resumed.
- **Archive**: Runner appends `mission_stopped`, marks the row `completed`, sets `archived_at`, kills any live PTYs (verified dead before the row flips), and unmounts router/bus state. Archived missions are hidden from active lists and render read-only.
- **Reset**: kills the slots and re-spawns them against the same mission row and log; forks read the persisted last size so they open at the width the pane had.

**Mission cwd is authoritative.** Each mission carries its own `cwd` column. Spawned slots inherit `mission.cwd` regardless of what the runner template's `working_dir` says — that field is only used in direct chats. Starting from a project copies the project's cwd into the row.

Concurrent missions on the same crew are allowed — a crew is a reusable template, and per-mission state (sessions, the bus, the runner-CLI shim path, the roster sidecar) is fully namespaced by `mission_id`.

### 3.5 Session — *one slot's PTY process*

The runtime instance of a slot inside a mission, a runner-backed direct chat, or a runtime-only direct chat. A Session is to a slot/runner/runtime what a Mission is to a Crew: the *run* of a *configuration*.

Two flavors, distinguished by whether `mission_id` is set on the session row:

- **Mission session** — spawned when a mission starts; one session per slot. It participates in the crew's bus, sees broadcasts, can receive stdin injection from the router. `RUNNER_HANDLE` carries the *slot* handle, not the runner template's global handle.
- **Direct-chat session** — spawned ad-hoc without a parent mission, backed by a runner template or a bare runtime selection. `mission_id` is null and the working directory lives on the session row. The agent is **not on any coordination bus**. Runner-backed chats keep `runner_id`; runtime-only chats store `runner_id = NULL` plus `agent_runtime` / `agent_command`.

A session owns, in the core:

- A PTY master handle (the only object in the system with a file descriptor to a running child process).
- A blocking reader thread that drains the PTY, hands each chunk to `SessionEvents::output`, and feeds the idle detector.
- A serialized writer for stdin (the human's keystrokes, pastes, and the router's injections all go through it).
- Its last applied PTY size (`last_cols`/`last_rows` on the row, the latest measurement in memory) and an exit status once the child has terminated.

And, in the app: one `alacritty_terminal::Term` in the `TerminalBridge` registry, created when the session spawns and released when it exits or is archived (§5.5). There is no separate scrollback buffer — the `Term` *is* the screen and the history.

A session is the only object in the system that actually *executes* code — everything else is metadata, a coordination channel, or a projection over the event log.

### 3.6 Surface hierarchy — *Project → Window → Tab → Pane*

How sessions are displayed spans durable organization and ephemeral view state. The concepts must never be blurred in code, docs, or UI copy:

- **Project** — the only durable container in the sidebar: a global, cwd-bound group for missions and direct-chat tabs. Starting work from a project copies its cwd into the new mission/session row and records nullable `project_id`. Deleting a project archives its chats and missions but never touches the directory on disk.
- **Window** — a real OS window (⇧⌘N, `File → New Window`). The core's per-window subject registry (`crates/runner-backend/src/windows.rs`) tracks every visible direct-chat subject, focus recency for duplicate-session ownership, and current focus for viewed-attention semantics. A session shown in two windows has one primary; the secondary window shows a duplicate-chat placeholder rather than a second live grid.
- **Tab** — one stable, ULID-keyed group of panes rendered as exactly one sidebar row. Tabs, projects and mission references are rows of the `nodes` tree (§10.1) with an optional project parent, name, order, JSON layout, pin position and completion/viewed watermarks. Every active direct-chat session belongs to exactly one tab. Per-window active-tab selection is ephemeral.
- **Pane** — one slot inside a tab, holding exactly one chat session (move-not-copy). `⌘[` / `⌘]` cycle pane focus, `⌘W` closes the focused pane without stopping its session.

Sessions exist independently of the display tree: closing a pane or window never kills a PTY, and since M6.8 it does not even drop the session's `Term` — the grid keeps ingesting while hidden and is simply painted again when a pane shows it.

The mission workspace's per-slot terminal switcher predates this hierarchy and is a different, mission-scoped UI element — not a Tab in the sense above.

### 3.7 Settings surface

Settings is a full-window route rendered in place of the app shell, with its own grouped sidebar and card-grouped panes: Appearance (zoom, theme), Terminal (font, cursor, theme palette), Agents (runtime discovery and overrides, the login-shell probe outcome), MCP, Keyboard shortcuts (a view over the registry in `runner-app/src/keymap.rs`), Updates, Diagnostics (log path, open-log), About, and Archived. Entry points — the sidebar Settings row, the command palette, and `⌘,` — navigate to the route and return to the caller's location.

Preferences persist in `$APPDATA/ui-settings.json`, read by the app at launch. They do **not** migrate from the Tauri app, which kept them in the webview's localStorage; a first native launch starts from defaults (both apps default resume-on-launch off).

**Updates** is the slim form of `main`'s pane: check now, the automatic-checks toggle, last-check time. Sparkle's standard user driver owns the found/download/install dialogs. Not ported from `main`: the Arc-style "New Runner version available" pill above the sidebar Settings row (hover → card, per-launch dismiss, auto-install checkbox). It needs an `SPUUpdaterDelegate` so the app learns an update was found; tracked as M6.9 in [`../impls/gpui-rewrite/m6-consolidation.md`](../impls/gpui-rewrite/m6-consolidation.md).

## 4. Coordination primitives — *what flows between runners*

Runners don't share a programming model; they share an IM-like surface.

| Primitive | Role | Shipped | Planned |
|---|---|:---:|:---:|
| **Signal** | Typed notification; the router handles built-ins. Verb grammar. | ✅ | |
| **Message** | Prose, broadcast or directed to a specific slot. | ✅ | |
| **Inbox** | Per-slot projection: broadcasts + messages addressed to me. | ✅ | |
| **Thread** | Scoped sub-conversation within a mission. | | not planned |
| **Fact** | KV whiteboard; "what is currently true in this mission." | | not planned |
| **Mention** | Targeted `@handle` inside a message's prose. | | later |
| **Reaction** | Lightweight signal attached to a message. | | later |

### 4.1 Signal — *"something happened, please wake the right surface"*

Short, typed, router-visible. Grammar: past-tense verb (or asker verbs like `ask_lead`, `ask_human`). The router has fixed handlers keyed to built-in signal types (§8.1). A signal carries an optional `payload` (JSON) for the router and UI. Human-readable conversation belongs in messages.

### 4.2 Message — *"here's what I think"*

Prose, addressed either to the mission (broadcast) or to a specific crewmate (direct).

- **Broadcast** — `runner msg post "<text>"`. Goes to every other runner's inbox; a human-authored broadcast goes to every runner.
- **Direct** — `runner msg post --to <slot_handle> "<text>"`. Goes to that slot's inbox only.

Messages are **flat by design** — one stream per mission, no thread scoping, no fact primitive. Durable conclusions belong in project files, code, commits, or message prose. Signals are typed and small; messages are prose the router does not parse — but it does *notice* them (§8.5).

### 4.3 Inbox — *"what's in my mailbox"*

Every slot has an **inbox**: the subset of the mission's messages relevant to it. The inbox is a **projection** over the event log, not a separate data structure. For the slot with handle `h`:

```
inbox(h) = all events in the mission where
          kind = "message" AND from != h AND (to = null OR to = h)
```

`runner msg read` returns the calling slot's inbox, sorted by ULID. `--since <ts>` restricts to messages newer than a given ULID/timestamp so agents can poll without re-reading history.

**The inbox is pull-based, with a nudge.** The body of a message is only ever read when the recipient runs `msg read`. What the router does on a new message is write one line into the recipient's PTY — `[inbox] new message from @coder — run \`runner msg read\` to view.` — subject to the delivery gate in §8.5, so an agent that is mid-turn or a human who is mid-draft is not interrupted. The recipient also learns to read its inbox through the platform preamble (§6 Layer 1), which instructs every runner to check at natural task boundaries.

### 4.4 Event — *the unifying transport*

Every coordination primitive is persisted as an **event** — one line in the per-mission NDJSON file:

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

`kind` discriminates. For `kind: "signal"`, `type` carries the verb; for `kind: "message"`, the prose lives in `payload.text`. Runners interact through CLI verbs (`runner signal`, `runner msg`), not the event schema directly. Event-log primitives (ULIDs with a monotonic floor, the `flock` append, tail repair of a torn last line) live in `crates/runner-core`.

## 5. PTY session runtime

A **pseudo-terminal (PTY)** is a kernel-emulated terminal device. To the child it looks like a real TTY — `isatty()` is true, `ioctl(TIOCGWINSZ)` reports a window size, signals route correctly — but the other end is a file descriptor held by a controlling process. The kernel exposes the pair as a **slave** (the child's stdin/stdout/stderr) and a **master** (what Runner reads from and writes to). It's the same primitive `ssh`, `tmux`, and every terminal emulator use. Runner uses the `portable-pty` crate, which wraps the POSIX primitives; for the rigorous treatment see Stevens & Rago, *APUE*, chapter 19.

### 5.1 Topology at a glance

Output (agent → screen + idle inference):

```
   Child ──► PTY slave ──► PTY master ──► Reader thread ─┬─► SessionEvents::output
   (tty stdout                            (blocking      │     └─► TerminalBridge ─► the session's Term
    + stderr)                              OS thread)    │            (alacritty, 10,000-line scrollback)
                                                         │            └─► wake GPUI if a pane is viewing it
                                                         └─► Idle detector ─► runner_status (forwarder)
```

Input (UI + router → agent):

```
   terminal element (keys, IME, paste, mouse) ──► TerminalSession ─┐
     direct chats: inline write                                     ├─► per-session writer ──► PTY master ──► child
     mission panes: queued input worker (off the render thread)     │
   Router (launch prompt, ask_lead, human_response, inbox nudges) ──┘
```

The PTY master is the hinge: held by SessionManager, written to by the serialized writer, read from by the blocking reader thread. Bus side, orthogonal to the PTY:

```
   Child ──► `runner` CLI on PATH ──► events.ndjson ──► notify ──► EventBus ──► Router ──► back to the writer on wake-up signals
```

The whole system runs many of these side-by-side — one per slot per live mission, plus one per active direct chat — each with its own reader thread, writer, idle detector and `Term`.

### 5.2 Why PTY (not pipes) and why alacritty

Claude Code, Codex, and TRAE CLI are TUIs. They check `isatty()`; if false they degrade. Their output is escape sequences that only a terminal emulator can render. The PTY gives the child a real terminal; `alacritty_terminal` gives Runner a correct emulator — grid, VTE parser, alt screen, scrollback reflow, selection, mouse reporting modes — without reinventing one. GPUI paints the grid it maintains.

### 5.3 Spawn

The session runtime is encapsulated behind a `SessionRuntime` trait so SessionManager doesn't know whether the runtime is in-process PTY or anything else; only `PtyRuntime` is shipped.

Login-shell discovery is startup-safe and shared: setup seeds SessionManager from the last successful `LoginShellEnv` snapshot, paints the app, then runs the configured shell probe on a background thread with a five-second deadline. A successful probe atomically swaps the environment used by future spawns and emits `runtime/changed`; a failure leaves the prior snapshot active and records the outcome for Settings → Agents.

Built-in runtime commands are resolved in Rust against the same PATH the child receives: a persisted runtime override, then the first executable found on the composed PATH, then the bare catalog command only while discovery is in flight. Once discovery completes, a missing executable fails before PTY creation with a pointer to Settings → Agents.

```
portable_pty::openpty(rows, cols)          rows/cols: explicit pane size > latest in-memory
  ├─ master handle  → kept by SessionManager        measurement > persisted last_cols/rows > 80×24
  └─ slave handle   → given to child via spawn_command()

Child inherits (mission session):
  PATH              = <mission shim>:$APPDATA/bin:<login-shell PATH>:<curated CLI dirs>:<process PATH>
  RUNNER_CREW_ID    = <ulid>
  RUNNER_MISSION_ID = <ulid>
  RUNNER_HANDLE     = <slot_handle>
  RUNNER_EVENT_LOG  = $APPDATA/crews/<crew>/missions/<mission>/events.ndjson
  TERM              = xterm-256color
  COLORTERM         = truecolor
  COLUMNS / LINES   = the spawn size
  <login-shell proxy env: HTTP_PROXY/HTTPS_PROXY/NO_PROXY>

install_handle:
  record the PTY, cache the runtime's policy flags, reconcile any size pushed while the
  PTY did not exist, emit session/spawned   ← the bridge creates the Term here
  start the forwarder (reader thread + idle detector)

Reader thread (blocking):
  loop { read(master) → SessionEvents::output(raw bytes) ; feed idle detector }
  on EOF: wait(child) → emit session/exit { code } → update sessions row
```

The first-turn body (persona, brief, launch prompt) is delivered through the runtime adapter in `router/runtime.rs`: as a positional argument for runtimes that accept one, otherwise as a verified paste after the TUI is ready. Claude Code's `--append-system-prompt` is SDK-only (requires `-p`), so interactive claude sessions get their brief as a first user turn. Resumed conversations suppress it.

### 5.4 Native wiring and human takeover

- The terminal element renders the session's `Term` from the registry; a pane that mounts late simply paints the grid as it is — there is no history fetch.
- Keys are encoded from the `Term`'s mode (application cursor keys, bracketed paste, Shift+Enter as `ESC CR`, ⌥ as Meta); IME composition is native, with marked text drawn in the grid; mouse reporting modes 1000/1002/1003/1006 are honored and Shift bypasses them for selection and scrollback; selection and copy are the element's own.
- **Resize** is immediate. The pane that owns the terminal's size pushes each measured size from `prepaint`; `TerminalSession::resize` resizes the `Term` (reflowing history) and the PTY ioctl fires on the same call, every frame of a drag, for every runtime. A 175 ms settle thread only persists the final size once per storm. Nothing clears the grid on resize: the TUI's own SIGWINCH repaint plus alacritty's reflow is the whole story (M6.6 — the earlier "clear and replay" contract duplicated history, because `ESC[2J` on the primary screen is `clear_viewport`, which scrolls the viewport into scrollback).

**Human takeover is a first-class capability.** At any moment the human can type directly into any runner's stdin — the same writer the router uses. The pane is a real terminal, not a log viewer: special keys pass through untouched, and the agent cannot tell whether bytes came from the router, the human, or its normal terminal input.

The mission feed is read-mostly: it renders coordination events and historical human-authored events, has no free-form composer, and its only input is the choice control on a pending `human_question` card. External orchestrators post `human_said` through MCP when they need to relay an operator instruction programmatically.

### 5.5 Sessions outlive views; terminals outlive panes

Sessions live in the core and belong to the mission, not to any window, tab or pane. Closing a window does *not* kill the sessions — the agents keep running, events keep flowing, the router keeps handling live signals.

Since M6.8 the same is true of the screen. `TerminalBridge` holds a strong `Arc<TerminalSession>` per live session: created on `session/spawned` (with a first-output fallback as an ordering safety net), fed from the first byte, released on `session/exit` and `session/archived`, replaced when a resume or reset spawns a new child under the same id. Panes take a *viewer lease* on the terminal they show; a hidden terminal keeps ingesting but does not wake GPUI. Tab switches, route changes and re-opened panes re-render an existing grid instead of rebuilding one. Consequence, recorded as a deviation from `main`: a stopped pane shows the Ended/Resume card over a neutral background, not the final screen; flip the release point from exit to archive if that is ever missed.

**Rows persist across app restart; PTY children do not.** On quit, `stop_running_sessions_on_quit` kills every process group (SIGHUP, then SIGKILL) and joins the forwarders; the startup orphan sweep is the crash fallback and a failing sweep is fatal at boot. On next launch Runner re-mounts router/bus state for `running` missions, replays the logs, demotes stale `running` session rows to `stopped`, and re-spawns sessions flagged `resume_on_launch`. Resume spawns a fresh PTY against the same session row; for claude-code/codex/trae, `agent_session_key` lets the agent CLI continue its own conversation when supported.

Fork creates a new direct-chat row without writing the source row, key, PTY, or conversation file. The new row copies the source's project, runner, cwd, runtime, command, model, and effort columns. Claude Code starts the visible TUI directly with `--resume <source> --fork-session --session-id <new>` and a caller-assigned key; Codex runs a bounded headless `exec fork` until it has a lineage-validated `thread.started` key, persists that key, marks the temporary row stopped, and starts the visible PTY through the ordinary resume path. A direct-spawn, materialization, or resume failure removes both the fork row and its tab. The runtime definition's `native_fork` capability enables Claude Code and Codex and excludes TRAE. An untouched Claude fork is intentionally copy-on-write: manual resume falls back to a fresh chat and launch-time resume reports it unavailable; re-fork the source instead of adding recovery state.

### 5.6 Writer serialization

The PTY writer is shared between the human and the router. Each session's writer is serialized, one `write_all` per turn. Mission panes send through a per-session **queued input worker** so that a delivery waiting on the draft gate (§8.5) can never park GPUI's render thread; direct chats write inline. Pastes are bracketed when the TUI has enabled bracketed paste, and the first-turn paste is verified against the grid before Enter is sent.

### 5.7 Threads, not async

`portable-pty`'s reader is blocking. One OS thread per session does one blocking `read(2)` in a loop; the kernel parks it cheaply. Writes are short and take a per-session lock. The core uses a small tokio runtime only where the libraries want one — the `rmcp` MCP server on its Unix socket and the broadcast channel behind `AppEvent`s.

### 5.8 Scrollback and size

Scrollback is the session's `alacritty_terminal::Term`, configured for 10,000 lines, reflowed on width change, and process-local: it does not survive app restart and there is no on-disk overflow. A resumed or reset session gets a fresh `Term`; the agent CLI's own resume restores conversation context on screen.

Each session row persists the last applied PTY `cols` and `rows`. Spawn and resume resolve their initial size as the explicit pane size, then the latest in-memory measurement (which can arrive before the PTY exists and is applied at `install_handle`), then the persisted dimensions, then 80×24 only for a session with no prior size. Mission forks and resets read the persisted size when it is newer than their hint, so a re-spawned slot opens at the width its pane has.

### 5.9 Death and kill

The reader thread owns the child handle. On EOF it calls `wait()`, emits `session/exit`, updates the sessions row. No auto-restart. Kill: SIGHUP to the process group via `portable-pty`; escalate to SIGKILL if the child lingers. Archive paths verify the child is dead before the row flips.

### 5.10 Busy / idle inference

Per-runner busy/idle is inferred from PTY-byte silence by the session forwarder's `IdleDetector` (2 s of silence = idle; a 500 ms grace window re-armed on every resize ioctl keeps SIGWINCH repaints from reading as work), not reported by the agent. For mission sessions the forwarder appends a `runner_status` event with `source: "forwarder"` to the mission log and the router maps it into the workspace status projection. Direct chats stay off-bus: SessionManager keeps their latest activity and emits `session/status` transitions to every window; the sidebar aggregates them at the tab level into spinners, `last_completed_at`, and the unread dot.

This is the fallback tier by design. M6.2 adds hook-based status from the agents' own turn signals (`working` / `waiting` / `done`) with the byte-flow detector demoted to heuristic; M6.1 replaces the byte-based "is the human typing" latch with an observed input state from the native seam.

## 6. System prompt composition

Every spawned session receives a composed system prompt — different shape for workers, the lead, and direct chats. The composition is mechanical: pure functions over slot + crew + mission inputs, no LLM in the loop. Source of truth lives in `crates/runner-backend/src/router/prompt.rs`; delivery mechanics in `router/runtime.rs`.

### 6.1 The three layers

1. **Layer 1 — platform preamble** (code-owned). For workers: a fixed block describing the `runner` CLI verbs and the inbox convention. For the lead: the launch prompt composed at `mission_goal` time (§6.3).
2. **Layer 2 — crew team conventions** (`crews.system_prompt_addendum`, optional). Spliced under `== Team conventions ==`.
3. **Layer 3 — runner persona** (`runners.system_prompt`). Spliced under `== Your brief ==`.

### 6.2 What each session sees

| Session kind | Layer 1 | Layer 2 | Layer 3 | Delivery |
|---|:---:|:---:|:---:|---|
| Mission worker | preamble | if set | persona | first-turn body (argv when the runtime accepts it, otherwise a verified stdin paste) |
| Mission lead | launch prompt (composed by router on `mission_goal`) | if set | persona | persona at spawn + router-injected launch body on `mission_goal` |
| Direct chat | — | — | persona | first turn |

Direct chats see *only* Layer 3 — the worker preamble's verbs and the team conventions don't make sense off-bus.

### 6.3 The lead's launch prompt

The lead's startup is short. Once `mission_goal` fires, the router composes a launch-prompt body — identity, the mission goal (`missions.goal_override` or `crews.goal`), the roster, the addendum, the known signal types from `runner_core::model::KnownSignalType`, and a reminder of the lead's job — and writes it to the lead's stdin once the TUI is ready. This keeps the spawn fast and lets the user edit the goal up to the moment they click Start Mission.

## 7. Coordination bus

### 7.1 Transport

```
$APPDATA/crews/{crew_id}/missions/{mission_id}/events.ndjson
```

One line per event, append-only, one file per mission. Debuggable (`tail -f | jq .`), crash-durable, atomic under explicit guards, replayable for projections.

#### 7.1.1 Concurrent-write correctness

Multiple runners can invoke `runner signal` / `runner msg` at the same time from different PTYs, and the core writes router-generated events (`human_question`, `mission_warning`, `runner_status`) to the same file:

1. Open the log with `O_APPEND | O_WRONLY | O_CREAT`.
2. `flock(fd, LOCK_EX)`.
3. Exactly one `write(2)` of the serialized JSON line including the trailing `\n`.
4. `close(fd)`, which releases the lock.

Ordering from `O_APPEND`, atomicity across writers from the lock, no partial lines from the single write. The app data directory must be on a local POSIX filesystem; network and iCloud-synced volumes may not honor `flock()`.

**No `fsync`, by decision** (2026-08-20): page-cache durability is the right trade for a local tool. The cost is a hard power loss mid-append; the log's tail repair already handles a torn last line on open, and coordination state is reconstructible from the agents' own session logs.

ULIDs carry a monotonic floor per log so two writers (the app and the CLI) never emit out-of-order ids within a millisecond.

### 7.2 Consumers

Two subscribers to each mission's file, both fed by one `notify` watcher:

- **Router** — deserializes each new line and dispatches it (§8).
- **UI** — the bus republishes the line's arrival as a `mission/changed` `AppEvent`; the mission workspace re-reads the feed from its cursor and projects events into the feed, the HITL cards and the status pills (incremental append is M6.4).

#### Startup replay

On router boot: open the mission's file, fold `human_question` / `human_response` and `runner_status` rows into in-memory state, record the replay high-water mark, then tail from the current end. Replay rebuilds projections; it never re-runs historical stdin pushes or inbox nudges.

## 8. Signal router

The router is a flat dispatcher, not a policy engine. There is no per-crew `{when, do}` rule list. The lead runner owns coordination judgment; the router owns parent-process plumbing that a child PTY cannot do itself.

Stdin pushes are deliberately silent: the router writes bytes into the target PTY but does not synthesize `stdin_injected` audit events. The event log records the signal or message that caused the push, plus `human_question` / `human_response` for HITL cards and `mission_warning` when a delivery cannot happen.

### 8.1 Fixed handler table

| Event | Fixed handler |
|---|---|
| `mission_goal` | Compose the launch prompt and inject it to the lead. |
| `human_said` | Inject MCP-provided `payload.text` to `payload.target` if present, otherwise to the lead. |
| `ask_lead` | Inject the worker's `{ question, context }` to the lead. |
| `ask_human` | Append a `human_question` event for the UI. |
| `human_response` | Look up the matching `question_id` and inject the answer to the runner that emitted the original `ask_human`. |
| `runner_status` | Update the latest-status map from `payload.state`. If a non-lead reports `idle`, inject a short availability update to the lead. |
| message (any) | Inject a one-line inbox nudge to the recipient (directed) or to every other roster member (broadcast); a message to the virtual `human` handle is rendered in the feed and not nudged. |
| `inbox_read` | Internal — owned by the bus's projection layer to track read watermarks. |

`mission_start`, `mission_stopped`, `human_question`, `mission_warning` are observed but not routed: they are events the router or the lifecycle itself emits.

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
    "triggered_by": <triggering-signal.id>,
    "prompt":       "Reviewer requested changes. Accept or override?",
    "choices":      ["accept", "override"],
    "on_behalf_of": "@impl"                     // optional; see §8.3
  }
}

// When the human clicks a choice:
{
  "kind": "signal",
  "type": "human_response",
  "from": "human",
  "payload": {
    "question_id": <human_question.id>,
    "choice":      "accept"
  }
}
```

Causality is carried in-payload rather than on the envelope. The canonical `question_id` is the `human_question` event's own `id`, assigned at flock-guarded append time.

### 8.3 Lead-mediated asks (the canonical pattern)

1. **Worker asks the lead.** Worker emits `ask_lead`; the router injects `{ question, context }` to the lead.
2. **Lead decides.** Answer from own context via a directed message (pull-based, plus the nudge), or escalate with `ask_human` and `payload.on_behalf_of: "<handle>"`; the UI shows the attribution chain (*@impl → @architect → you*).
3. **Human responds.** The router injects the result into the lead's stdin; the lead forwards it to the worker as a directed message.

### 8.4 Read-mostly mission feed

The mission feed answers what is happening across the crew; the selected terminal pane is where the operator talks to a runner. Runners cannot address a virtual `human` message recipient: `runner msg post --to human` fails with guidance to answer in TUI output. The feed keeps its render paths for historical `human_said`, `human_response`, and messages addressed to `human`, so old logs replay unchanged.

### 8.5 Who does delivery, and when

| | Sender addresses recipient? | What the router does | When |
|---|:---:|---|---|
| Signal | No — fixed handler decides | Injects the handler's text | Through the delivery gate |
| Broadcast message | No | One-line inbox nudge to every other slot | Through the delivery gate |
| Direct message | Yes (`--to`) | One-line inbox nudge to that slot | Through the delivery gate |

Message *bodies* are never pushed; recipients read them with `msg read`. What the router pushes is the wake-up line, and every push — handler text or nudge — goes through the per-slot **delivery gate and outbox** in `router/mod.rs`:

- **Draft-aware.** If the human is typing in that slot's pane (today a byte-level latch: printable input and pastes set it, Enter/Ctrl-C clear it, with no time bound — the 10-minute abandonment backstop in impl 0041 was never implemented, so only Enter, Ctrl-C, respawn or exit release it; M6.1 replaces the latch with an observed input state), the delivery waits in the outbox and the pane shows a "delivery waiting" pill (`router/delivery-blocked`).
- **Turn-boundary.** Deliveries are spaced by an 80 ms cooldown and a 30 s reconciliation re-nudge covers a nudge that landed while the agent was mid-tool-call; latest-wins absorbs bursts. With M6.2 the boundary comes from the agent's own `done` signal instead of inferred idle.
- **Queued until resume.** A delivery to a stopped or resuming slot is queued, announced once per outbox with a `mission_warning` ("queued until the session resumes"), flushed on `Respawned`, and dropped with a second warning if the session exits for good. Nothing is silently lost and nothing hard-fails (M6.8).
- **Never on replay.** Events at or below the replay high-water mark are not re-dispatched, so a restart does not re-nudge anyone with mail they already saw.

## 9. The `runner` CLI

The bundled CLI is the agent-facing surface for everything in §4–§8. Spawned children invoke it directly (it's prepended onto their `PATH` at spawn) to participate in the bus. There is no other supported way for an agent to talk to the rest of the crew.

### 9.1 Surface

```
runner signal <type> [--payload <json>]
runner msg    post <text> [--to <handle>]
runner msg    read [--since <ts>] [--from <handle>]
runner status busy|idle [--note <text>]      (deprecated)
runner help
```

Context always comes from env vars injected at spawn (`RUNNER_CREW_ID`, `RUNNER_MISSION_ID`, `RUNNER_HANDLE`, `RUNNER_EVENT_LOG`); the CLI is otherwise stateless and side-effect-free outside of the one log append. Message bodies are capped at 32 KB.

### 9.2 Verb-by-verb

- **`signal <type> [--payload <json>]`** — append a `kind: signal` event. The router runs its fixed handler. `<type>` is validated against the closed `runner_core::model::KnownSignalType` enum.
- **`msg post <text>`** — broadcast: `to: null`. **`msg post --to <handle> <text>`** — directed; the handle must be a slot in the mission roster.
- **`msg read [--since <ts>] [--from <handle>]`** — the inbox projection (§4.3), sorted by ULID; also records an `inbox_read` watermark.
- **`status busy|idle`** — **deprecated**; status is inferred (§5.10). Kept as an alias stamped `source: "agent"`, prints a deprecation notice.
- **`help`** — long-form usage from `cli/src/help.rs`.

### 9.3 What the CLI does *not* do

No event-DAG flags (causality is ULID order or in-payload), no daemon, no socket, no per-crew allowlist. Each invocation is a one-shot process: read env, build the event, `flock` + append, exit.

### 9.4 Direct chats: the CLI is absent

Direct-chat sessions don't get the bundled CLI on PATH — there is no bus, no router, no inbox. This is deliberate: direct chats are one-on-one with the human.

### 9.5 External control: MCP, not the CLI

Outside agents and tools operate Runner itself through the MCP server the app hosts on `$APPDATA/mcp.sock` (bridged from stdio by `runner-mcp`): `crew_*`, `runner_*`, `slot_*`, `project_*`, `mission_*` (start, stop, archive, reset, status, feed, post human message/signal, pin, rename) and `session_start_direct`. This is how a Claude Code session drives a crew mission from the outside — the loop the rewrite itself was built with.

## 10. Data model

### 10.1 SQLite (config + session lifecycle)

```sql
crews (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  purpose TEXT,
  goal TEXT,                          -- default mission goal
  system_prompt_addendum TEXT,        -- Layer-2 team conventions; nullable
  created_at TEXT, updated_at TEXT
);

runners (
  id TEXT PRIMARY KEY,
  handle TEXT NOT NULL UNIQUE,        -- globally unique slug; §3.2
  display_name TEXT NOT NULL,
  runtime TEXT NOT NULL,              -- claude-code | codex | trae | shell (qoder: legacy rows only)
  command TEXT NOT NULL,
  args_json TEXT,
  working_dir TEXT,                   -- direct-chat working dir; missions use mission.cwd
  system_prompt TEXT,                 -- Layer 3 persona
  env_json TEXT,
  model TEXT, effort TEXT,            -- optional overrides, composed into argv at spawn
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);

slots (
  id TEXT PRIMARY KEY,
  crew_id TEXT NOT NULL REFERENCES crews(id) ON DELETE CASCADE,
  runner_id TEXT NOT NULL REFERENCES runners(id) ON DELETE CASCADE,
  slot_handle TEXT NOT NULL,          -- in-crew handle; unique within crew
  position INTEGER NOT NULL,
  lead INTEGER NOT NULL DEFAULT 0,
  runtime_override TEXT, model_override TEXT, effort_override TEXT,
  added_at TEXT NOT NULL,
  UNIQUE (crew_id, slot_handle), UNIQUE (crew_id, position)
);
CREATE UNIQUE INDEX one_lead_per_crew ON slots(crew_id) WHERE lead = 1;

projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  cwd TEXT NOT NULL,
  position INTEGER NOT NULL,
  created_at TEXT NOT NULL
);

-- The sidebar tree: projects, tabs and mission references as one ordered
-- forest (migration 0014 replaced the folders/tabs tables).
nodes (
  id TEXT PRIMARY KEY,
  parent_id TEXT REFERENCES nodes(id) ON DELETE RESTRICT,   -- NULL = root
  position INTEGER NOT NULL,                                -- scoped to parent
  type TEXT NOT NULL,                                       -- 'project' | 'tab' | 'mission'
  name TEXT,                                                -- tab title
  ref_id TEXT,                                              -- projects.id / missions.id
  layout TEXT,                                              -- tab-only: pane layout JSON
  pinned_position INTEGER,                                  -- non-NULL = pinned
  last_completed_at TEXT, last_viewed_at TEXT,              -- tab attention watermarks
  created_at TEXT NOT NULL
);

missions (
  id TEXT PRIMARY KEY,
  crew_id TEXT NOT NULL REFERENCES crews(id) ON DELETE CASCADE,
  project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
  title TEXT NOT NULL,
  status TEXT NOT NULL,               -- running | completed | aborted
  goal_override TEXT,
  cwd TEXT,                           -- authoritative working dir for slot spawns
  started_at TEXT NOT NULL, stopped_at TEXT, archived_at TEXT, pinned_at TEXT
);

sessions (
  id TEXT PRIMARY KEY,
  mission_id TEXT REFERENCES missions(id) ON DELETE SET NULL,   -- NULL = direct chat
  project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
  runner_id TEXT REFERENCES runners(id) ON DELETE CASCADE,
  slot_id TEXT,
  cwd TEXT,
  status TEXT NOT NULL,               -- running | stopped | crashed
  pid INTEGER,
  started_at TEXT, stopped_at TEXT,
  runtime TEXT, runtime_socket TEXT, runtime_session TEXT,      -- live handle metadata
  runtime_window TEXT, runtime_pane TEXT, runtime_cursor INTEGER, -- (legacy tmux columns, unused)
  agent_session_key TEXT,             -- the agent CLI's own conversation id, for Resume
  agent_runtime TEXT, agent_command TEXT, agent_model TEXT, agent_effort TEXT,
  last_cols INTEGER, last_rows INTEGER,   -- last applied PTY size (§5.8)
  resume_on_launch INTEGER NOT NULL DEFAULT 0,
  archived_at TEXT, title TEXT, pinned_at TEXT
);
```

Migrations live in `crates/runner-backend/migrations/` (`0001_init.sql` … `0020_slot_effort_override.sql`). Forward-only; new migrations are allocated there and the Tauri app never sees them, which is why the cutover is a one-way door for the database (M6.3's index migration is deferred to cutover for that reason).

### 10.2 Filesystem

```
~/Library/Application Support/com.wycstudios.runner/      ($APPDATA; debug builds: …runner-dev)
├── runner.db                               # SQLite (WAL)
├── ui-settings.json                        # preferences (§3.7)
├── mcp.sock                                # MCP server socket while the app runs
├── bin/
│   ├── runner                              # bundled agent CLI (signal + msg)
│   └── runner-mcp                          # stdio MCP bridge
└── crews/{crew_id}/missions/{mission_id}/
    └── events.ndjson                       # per-mission event log (+ roster sidecar)

~/Library/Logs/com.wycstudios.runner/runner.log   # rotating app log + panic backtraces
```

The data directory is the one the Tauri app used, so a cutover install finds its runners, crews, missions and sessions in place; only the webview's localStorage preferences are left behind. Direct chats are off-disk beyond their row in `sessions`; mission sessions share their mission's directory and the only durable artifact is `events.ndjson`. Screen state lives in memory (§5.8).

## 11. Process and thread model

Runner is one process. There is no IPC boundary between the screen and the PTY.

### 11.1 The shape

```
Runner.app process
  ├── GPUI main thread
  │     render + layout, input dispatch, IME, window management,
  │     AppStore reactions → scoped cx.notify, Sparkle callbacks
  │
  ├── native-app-events thread
  │     AppEvent broadcast → AppStore snapshots → wake GPUI
  │
  ├── Per live session
  │     ├── blocking PTY reader thread  (read(2) → SessionEvents::output → Term, idle detector)
  │     ├── queued input worker         (mission panes; draft-gate waits live here)
  │     └── short-lived settle thread   (one per resize storm; persists last_size)
  │
  ├── Per live mission
  │     └── notify watcher → EventBus tail → Router dispatch; router cooldown / reconciliation timers
  │
  ├── tokio runtime (small)
  │     ├── rmcp MCP server on $APPDATA/mcp.sock
  │     └── AppEvent broadcast channel
  │
  └── login-shell probe thread (startup, five-second deadline)
```

### 11.2 Why a thread per PTY reader (not async)

`portable-pty`'s read side is blocking. An OS thread doing one blocking `read(2)` in a loop is the right shape: the kernel parks it cheaply when there are no bytes and wakes it instantly when there are. The same thread feeds the `Term` directly — the lock is the only synchronization between ingestion and painting, and GPUI only re-renders when a viewed terminal changed.

### 11.3 What runs per-mission vs. app-wide

| Lifetime | Components |
|---|---|
| App-wide | GPUI main thread, the events thread, AppStore, SessionManager, MissionManager, the SQLite pool, the MCP server, the tracing writer, the Sparkle controller. |
| Per live mission | One notify watcher + bus tail + router dispatch, wired to that mission's NDJSON file. |
| Per live session | The PTY reader thread, the writer and input worker, the idle detector, and one `Term` (10,000-line scrollback) in the bridge registry. |

Per-mission components come up at mission start (or app launch for `running` missions) and go down at archive. A reversible Stop kills PTYs but leaves router/bus state mounted.

### 11.4 Cost model

The target scale is one operator with a handful of concurrent missions and ≤ ~10 live sessions. That is ~10 reader threads, as many `Term`s (tens of MB each only when their scrollback is full), a few notify watchers and one SQLite pool. Rendering cost scales with *visible* terminals, not live ones. Per-frame work today re-walks the visible grid (M6.7 lists what is still wasteful); nothing in the model needs an event loop over PTY fds at this footprint.

### 11.5 Failure isolation

A panic in a PTY reader thread only affects that session: the forwarder ends, the session is marked stopped. A panic on the GPUI main thread takes the app down — the panic hook writes the backtrace to `runner.log` first, and the crash reporter's `.ips` lands in `~/Library/Logs/DiagnosticReports/`. The standing GPUI rules in `impl_log.md` exist because several of those were found the hard way (`window.current_view()` outside render, entity updates inside their own observers).

## 12. Architectural bets

1. **Mission is the runtime unit.** Crew is config; mission is a run.
2. **Slot is the indirection** that lets one runner template participate in many crews and direct chats without duplication.
3. **PTY in-process via `portable-pty`, not pipes, not tmux.** TUI fidelity is non-negotiable.
4. **NDJSON file per mission, not a broker.** Debuggable and crash-durable.
5. **CLI wrapper for spawned agents; MCP for external controllers.**
6. **Signals and messages as distinct primitives.** Keeps the router simple and prose natural.
7. **The signal router is the only urgent wake-up path**, and every push goes through one delivery gate.
8. **Prompt composition at spawn time (Layer 1/2/3).** Replaces runtime handshakes.
9. **Small vocabulary.** Signals + messages; no threads or facts.
10. **One native process; `alacritty_terminal` as the model, GPUI paints the grid.** Terminal bytes are never serialized, and the terminal outlives its views.
11. **ULID for event IDs.** Sortable, monotonic within ms.
12. **Mission state outlives the app process; PTYs do not.** The event log and session rows are the continuation point; Resume creates fresh child processes.
13. **The core is UI-agnostic.** `runner-backend` knows nothing about GPUI; the app is one consumer of `AppCore`, and the MCP server is another.

## 13. What would break this architecture

- A runtime with no way to take a first turn or a system prompt at spawn.
- An agent that won't learn to call CLI tools.
- NDJSON append atomicity breaking on an exotic filesystem (NFS, iCloud-synced). App data must be on a local POSIX filesystem.
- A target platform where `portable-pty` semantics differ meaningfully from POSIX PTYs (Windows).
- A GPUI API break: `gpui-ce` is pinned (0.3.3) and upgraded deliberately; the terminal element and IME integration are the surfaces most exposed to it.

## 14. Program state — line, landing, channels

As of 2026-08-23 (`v0.6.0` published):

- **`main`** is the native app and the only line of work; the Tauri + React line ended at `276a3a4` and its last release, `v0.5.2`, bridges into `v0.6.0` on its next update check. Work lands as a task branch → PR → the one required check (`Rust / macOS`) → merge → a docs landing commit. The human smoke-tests before the PR; crews do not launch the app.
- **Versions.** `CFBundleVersion` is always the UTC build stamp `YYYYMMDD.HHMM` on both channels — Sparkle compares only this; `CFBundleShortVersionString` is display: `X.Y.Z-nightly.<stamp>` on nightly, `X.Y.Z` on production. The crate version (`runner-app`, `runner-backend`, `runner-terminal` in lockstep) carries `-nightly` between releases and is hand-bumped to the bare version on the release commit; the bump back (`0.6.0` → `0.7.0-nightly`) is the first post-GA chore. In-app display is `<short version> (<sha>)`; `make run` shows `<crate version> (dev)`.
- **Nightly channel.** `gh workflow run nightly.yml --ref main` (dispatch-only; a push runs CI alone) builds, signs, notarizes, and uploads `Runner-Nightly-<short version>-arm64.dmg` to the rolling `nightly` GitHub **draft** release (draft since 2026-08-24), keeping the last ten. A draft is invisible without push access and its assets are not anonymously downloadable, so the nightly channel has no Sparkle feed and general users never see it on the public releases page. The nightly bundle is `com.wycstudios.runner.nightly` ("Runner Nightly"); its baked `SUFeedURL` `releases/download/nightly/appcast.xml` 404s by design, and installs are manual — `gh release download nightly` or a local `bundle-mac --channel nightly` build. The workflow refuses to publish unless CI is green for the sha, and fails afterwards if the release is not a draft or the new DMG is reachable anonymously. The `/nightly` skill runs this path.
- **Production channel.** Tag `vX.Y.Z` on a commit whose crate version is exactly `X.Y.Z` → `release.yml` builds the same ARM64 bundle for `com.wycstudios.runner` (`SUFeedURL` `releases/latest/download/appcast.xml`), generates a one-item signed appcast with an Apple Silicon hardware requirement, and attaches everything to a **draft** release; publishing is the human's switch, because it moves the `releases/latest` alias. `workflow_dispatch` with `dry_run` builds the same artifacts into a throwaway draft. One Sparkle EdDSA keypair (`packaging/sparkle-public-key`) serves both feeds. Intel packaging is not planned (§2); an Intel install left on the universal `v0.6.0`/`v0.6.1` stays there, correctly withheld by the appcast's Apple Silicon hardware requirement.
- **Tauri bridge (until 0.7.0).** `v0.6.0` carries `Runner.app.tar.gz` + its minisign `.sig` (key id `23fee1fa29746d59`, the one embedded in 0.5.x) + `latest.json`; every later 0.6.x release re-uploads the same `latest.json` (both platform keys at the absolute `v0.6.0` URL) so a dormant 0.5.x install hops Tauri → 0.6.0 → Sparkle. **Hard cutoff at 0.7.0**: no `latest.json`; a 0.5.x install still dormant installs the DMG by hand.
- **Isolation.** The two bundles share one data directory (§10.2) — one instance at a time. A nightly is invisible to production installs and to general users by construction: different feed URL, and a draft release never appears on the releases page or behind `releases/latest`.

History of how this was decided: [`../impls/gpui-rewrite/README.md`](../impls/gpui-rewrite/README.md) (condensed) and [`../impls/archive/gpui-rewrite/plan.md`](../impls/archive/gpui-rewrite/plan.md) §Release channels (full); what remains: [`../impls/gpui-rewrite/m6-remainder.md`](../impls/gpui-rewrite/m6-remainder.md).
