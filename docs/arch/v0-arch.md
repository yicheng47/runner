# Runners — v0 Architecture

> Companion to `v0-prd.md`. The PRD defines *what* v0 ships; this doc defines *how* it works.

## 1. Overview

Runners is a local desktop app. A user configures a **crew** of CLI coding agents, launches a **mission** to activate it, and watches the crew coordinate in real time. The app is a Tauri 2 binary: Rust backend, React webview, SQLite for config, and a per-mission NDJSON file for live state.

### 1.1 Runtime picture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Tauri process (runner desktop app)                                          │
│                                                                             │
│  ┌──────────────────────┐   ┌──────────────────────┐   ┌─────────────────┐  │
│  │ MissionManager       │   │ SessionManager       │   │ EventBus        │  │
│  │  - mission lifecycle │   │  - PTY spawn/kill    │   │  - tail NDJSON  │  │
│  │  - compose prompts   │   │  - reader threads    │   │  - notify watch │  │
│  │  - roster + brief    │   │  - scrollback rings  │   │  - projections  │  │
│  └────────┬─────────────┘   └────────┬─────────────┘   └────────┬────────┘  │
│           │                          │                          │           │
│           │                          │                          ▼           │
│           │                          │               ┌──────────────────┐   │
│           │                          │               │ Signal router    │   │
│           │                          │               │  - fixed handlers│   │
│           │                          │               │  - status map    │   │
│           │                          │               └────────┬─────────┘   │
│           │                          │                        │             │
│           │      inject_stdin / human_question / status        │             │
│           └─────────────────────────►│◄───────────────────────┘             │
│                                      ▼                                      │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │ Runner session (one per runner × mission)                            │   │
│  │   ┌──────────┐   PTY   ┌─────────────────────────────────────────┐   │   │
│  │   │  master  │ ◄────►  │  child: claude-code / codex / shell     │   │   │
│  │   └──────────┘         │  env: RUNNER_CREW_ID,                   │   │   │
│  │                        │       RUNNER_MISSION_ID,                │   │   │
│  │                        │       RUNNER_HANDLE,                    │   │   │
│  │                        │       RUNNER_EVENT_LOG, PATH=…          │   │   │
│  │                        └─────┬───────────────────────────────────┘   │   │
│  └─────────────────────────────┼──────────────────────────────────────┘   │
│                                │ runs `runner signal` / `runner msg`      │
│                                ▼                                          │
│                  ┌─────────────────────────────┐                          │
│                  │  events.ndjson (per mission)│                          │
│                  └──────────────┬──────────────┘                          │
│                                 │ notify → EventBus → Router + UI         │
└─────────────────────────────────┼─────────────────────────────────────────┘
                                  ▼
                         ┌───────────────────────┐
                         │ React + xterm.js      │
                         │  terminals, messages, │
                         │  signals, HITL        │
                         └───────────────────────┘
```

### 1.2 The one-paragraph story

The user defines a **crew** (configuration: runners + signal allowlist). They click **Start Mission**, which creates a **mission** (runtime container), spawns one PTY-backed **session** per runner, and composes each runner's system prompt with the mission brief, the crew roster, and coordination instructions. Runners run real CLI binaries inside PTYs. They coordinate through two primitives — **signals** (typed events for the parent process to route on) and **messages** (flat prose stream for runner-to-runner conversation) — both carried through a bundled `runner` CLI that appends to the mission's NDJSON file. The **signal router** tails that file and applies fixed built-in handlers for bootstrap, wake-ups, HITL cards, and runner availability. The lead runner, not the router, owns coordination judgment.

## 2. Concepts

Domain objects split cleanly into two layers:

- **Configuration** — persistent, user-edited. Outlives missions. Crew, Runner, signal allowlist.
- **Runtime** — created at mission start, torn down at mission end. Everything here is scoped to a mission: the Mission itself, its Sessions, its coordination primitives (Signals, Messages), and the router's in-memory state.

The key insight: **Runner is config; Session is its runtime instance** — the same pattern as Crew (config) → Mission (runtime). A runner never runs on its own. A runner runs *inside a mission* as a session. The session is born when the mission starts, lives while the mission runs, and dies when the mission ends.

### 2.1 Relationship diagram

```
┌─ Configuration (persistent) ─────────┐    ┌─ Runtime (mission-scoped) ──────────────┐
│                                      │    │                                         │
│   Crew ─┬── Runner ──────────────────┼────┼──► Session ─► PTY process               │
│         │      (describes a role,    │    │     (one instance per runner per         │
│         │       binary, brief)       │    │      mission; lives & dies with the      │
│         │                            │    │      mission)                            │
│         │                            │    │                                         │
│         └── Signal allowlist         │    │     ▲                                    │
│                                      │    │     │  spawned & owned by                │
│                                      ├────┼──► Mission ─── events.ndjson             │
│                                      │    │     │              │                    │
│                                      │    │     │              ├─► Signal   [v0]    │
│                                      │    │     │              ├─► Message  [v0]    │
│                                      │    │     │              ├─► Thread   [v0.x]  │
│                                      │    │     │              └─► Fact     [v0.x]  │
│                                      │    │     │                                   │
│                                      │    │     ├─► Router in-memory state          │
│                                      │    │     │    (pending asks, runner status)  │
│                                      │    │     │                                   │
│                                      │    │     └─► Shared context:                 │
│                                      │    │           brief + roster (v0)           │
│                                      │    │           + facts (v0.x)                │
└──────────────────────────────────────┘    └─────────────────────────────────────────┘
```

A mission is a container. Everything in the runtime column is either the container itself (Mission) or an object whose lifecycle is scoped by it. **Sessions are first-class members of this container** alongside the coordination bus and the router state — not a side effect of spawning runners.

### 2.2 Crew — *a configured team*

The persistent "who's on the team and how they work together" record. A crew has a name, a default mission goal, a list of runners, and a signal-type allowlist. It does not run. It is blueprint.

**Every crew must have exactly one lead runner.** The lead is the human's counterpart in the crew: the mission goal and human-originated broadcast signals route to the lead by default, and the lead dispatches work to the other runners via directed messages. This is a hard invariant — a crew with zero runners or zero leads is invalid and cannot start a mission. The first runner added to a new crew becomes lead automatically; the user can reassign lead between existing runners, but cannot remove the lead runner without first designating a replacement. The lead is a routing convention, not a privileged capability: any runner can emit signals and post directed messages. Lead only governs *where inbound-from-human signals land by default*.

**Lead is also the default HITL gateway.** When a worker needs human input, it does not ask the human directly — it emits an `ask_lead` signal whose payload carries the question. The router wakes the lead (signals trigger fixed handlers; messages don't — see §5.5), who decides whether to answer from their own context or escalate to the human via `ask_human`. If the lead escalates, the human's answer flows back to the lead, who forwards it to the original worker as a directed message that the worker picks up on its next `runner msg read`. This keeps the human's attention focused on one interlocutor and lets the lead absorb, filter, or batch worker questions. See §5.5 for the protocol details (the `ask_lead` signal, the `on_behalf_of` payload field on `ask_human`, and the forwarding flow). Workers *may* emit `ask_human` directly as a fallback — it's not forbidden at the protocol layer — but the default runner system prompt instructs them to go through the lead.

Lifecycle: created by the user, edited freely, deleted when no longer needed. Persisted in SQLite.

### 2.3 Runner — *one configured agent*

An individual CLI agent: what binary to run, with what args, in what working directory, with what system prompt (the role's brief). Persistent config. A runner doesn't run on its own; it describes a process that will be spawned — either as a session inside a mission (the normal path), or as a standalone direct-chat session (see §2.6).

A runner has two identifying fields:

- **`handle`** — a lowercase slug (e.g. `coder`, `reviewer`, `tester`). Required, immutable once set, **globally unique** across the app. Used everywhere addressing is needed: as `from` and `to` in events, as `--to <handle>` on the CLI, and in router state. Global uniqueness means `@impl` names the same runner whether it's seen in crew A's event log or crew B's.
- **`display_name`** — a free-form label for the UI (e.g. "Coder", "Lead Reviewer"). Editable; not used in event fields or addressing.

Keeping these separate means renaming a runner for the UI doesn't break briefs, rules, or historical events. `handle` is the identity; `display_name` is just presentation.

**Runners are top-level config; crews compose them.** A runner is its own entity, not a child of a crew. The same runner can be a member of multiple crews — `@architect` can sit in `runners-feature` and `runners-ops` simultaneously. Crew membership lives in the `crew_runners` join table, which carries the per-crew `position` and `lead` flag (see §7.1). This is a deliberate post-C3 product change: a user typically curates a small stable of runners and reuses them across crews and ad-hoc direct chats; tying a runner to a single crew would force them to clone configs every time.

Exactly one runner per crew carries the per-crew `lead` flag (see §2.2). The router treats the lead as the default recipient of human broadcast messages and the mission-goal inject at startup; other crew members receive traffic only when directly addressed. The flag lives on `crew_runners`, enforced by a unique partial index: `UNIQUE(crew_id) WHERE lead = 1`. Lead is per-crew, not per-runner — the same runner can be lead in one crew and a worker in another.

### 2.4 Signal allowlist — *the crew's protocol vocabulary*

A JSON array of signal type strings attached to the crew and exported beside the mission log. The CLI validates emitted signals against this allowlist so typos fail visibly. MVP uses the built-in set only; user-defined signal types and policy-driven routing are v0.x.

### 2.5 Mission — *one activation of the crew, and the runtime container*

A mission is the only runtime container in the system. Everything alive at runtime lives *inside* a mission and dies with it:

- A **Session** per runner (the PTY processes — see §2.6).
- The **coordination bus** — the NDJSON event log carrying signals and messages.
- The **router's in-memory state** — pending HITL asks and latest runner availability. Per-runner read watermarks belong to the event bus projection.
- The **shared context** injected into each runner's composed prompt — the mission brief and the roster.

Lifecycle:
- **Start**: user clicks Start Mission on a crew. A mission row is created, one session is spawned per runner in the crew, the router boots with fresh state, and an NDJSON file is opened.
- **End**: explicit stop, or all sessions exited. Every session is killed, the router stops, the mission row is closed out.

This framing matters: when we say "the coordination bus is mission-scoped" or "the fact whiteboard is mission-scoped," we're saying the same thing as "sessions are mission-scoped." They all share one lifecycle because they all belong to the same container.

v0: concurrent missions on the same crew are allowed — a crew is a reusable template, and per-mission state (sessions, the coordination bus, the runner-CLI shim path, the roster sidecar) is fully namespaced by `mission_id`. Per-crew throttle / rate-limit is a future consideration.

### 2.6 Session — *one runner's PTY process*

The runtime instance of a Runner. A Session is to a Runner what a Mission is to a Crew: the *run* of a *configuration*.

A session has two flavors, distinguished by whether `mission_id` is set:

- **Mission session** — spawned when a mission starts; one session per crew member. It dies with the mission. The runner participates in the crew's coordination bus, sees broadcasts, can receive `inject_stdin` from the router, etc. This is the path the v0 demo flow exercises.
- **Direct-chat session** — spawned ad-hoc from the Runners page (see §2.3) without a parent mission. `mission_id` is null and the working directory lives on the session row directly. The runner is **not on any coordination bus** — there's no event log, no router, no inbox; it's just a one-on-one PTY between the human and the runner's CLI. Useful for "I just want to ask `@architect` something quickly" without spinning up a full crew.

A session owns:
- A PTY master handle (the only object in the system with a file descriptor to a running child process).
- A blocking reader thread that drains the PTY and pushes to the scrollback ring.
- A writer for stdin injection (used by the human and by the router's fixed handlers).
- A ring buffer (~10k lines) for scrollback that survives frontend tab-switches and app restarts within the mission.
- An exit status once the child has terminated.

A session is the only object in the system that actually *executes* code — everything else is metadata, a coordination channel, or a projection over the event log.

### 2.7 Coordination primitives — *what flows between runners*

Runners don't share a programming model; they share an IM-like surface. The same way Slack/Lark/Teams gave humans a small vocabulary of coordination (messages, threads, pings, pinned canvas), Runners gives agents a parallel vocabulary. We ship a subset in each milestone.

| Primitive | Role | v0 | v0.x | v1+ |
|---|---|:---:|:---:|:---:|
| **Signal** | Typed notification; the router handles built-ins. Verb grammar. | ✅ | | |
| **Message** | Prose, broadcast or directed to a specific runner. | ✅ | | |
| **Inbox** | Per-runner projection: broadcasts + messages addressed to me. | ✅ | | |
| **Thread** | Scoped sub-conversation within a mission. | | ✅ | |
| **Fact** | KV whiteboard; "what is currently true in this mission." | | ✅ | |
| **Mention** | Targeted `@name` inside a message's prose (lighter-weight than `--to`). | | | ✅ |
| **Reaction** | Lightweight signal attached to a message (`👍`, `🔍`, `blocking`). | | | ✅ |

#### 2.7.1 Signal — *"something happened, please wake the right surface"*

Short, typed, router-visible. Grammar: past-tense verb.

Examples: `review_requested`, `changes_requested`, `approved`, `blocked`.

Signals are machine-readable by design. The router has fixed handlers keyed to built-in signal types. Runners emit them when they need parent-process plumbing: wake the lead, show a human card, or report availability.

A signal carries an optional `payload` (JSON) for the router and UI. Human-readable conversation belongs in messages.

#### 2.7.2 Message — *"here's what I think"*

Prose, addressed either to the mission (broadcast) or to a specific crewmate (direct). Runner-to-runner (and human-readable). Grammar: sentence.

Two shapes:
- **Broadcast** — `runner msg post "<text>"`. Goes to everyone's inbox. Use for status updates, open questions, mission-wide announcements.
- **Direct** — `runner msg post --to <runner> "<text>"`. Goes to that runner's inbox only. Use for targeted questions, replies, or private back-and-forth.

Examples:
- broadcast: `"Branch feat/x is ready. Touched auth.rs and session.rs."`
- direct: `runner msg post --to reviewer "Line 47 in auth.rs: null check missing when the token is expired."`
- direct reply: `runner msg post --to coder "Kept the 30s timeout — provider is slow on cold start."`

Messages are **flat in v0** — one stream per mission, no thread scoping. Each runner consumes messages through their **inbox** (§2.7.5): broadcasts plus directly-addressed messages.

Messages and signals are separate for good reasons:
- Signals are typed and small; router handlers key off them. Messages are prose; the router doesn't parse them.
- A signal without prose works ("approved"). Prose without a signal works too ("I noticed X"). Conflating them forces every signal to carry prose and every note to carry a type.
- Runners (LLM agents) already know how to use both: signals are like exit codes, messages are like comments. The CLI keeps them linguistically separate.
- Direct messages enable real conversation between runners without forcing every interaction through parent-process wake-ups.

#### 2.7.3 Inbox — *"what's in my mailbox"*

Every runner has an **inbox**: the subset of the mission's messages that are relevant to it. The inbox is a **projection** over the event log, not a separate data structure. For the runner with handle `h`:

```
inbox(h) = all events in the mission where kind = "message" AND (to = null OR to = h)
```

`runner msg read` returns the calling runner's inbox, sorted by ULID (chronological). `--since <ts>` restricts to messages newer than a given ULID/timestamp so agents can poll without re-reading history.

This design keeps the storage model simple (one event log per mission, same as before) while giving each runner a clean "what for me" view. Broadcasts end up in everyone's inbox; direct messages end up in exactly one.

**The inbox is pull-based.** Messages are read when the recipient runs `msg read`; the system does not automatically interrupt a busy runner every time mail arrives. This is deliberate — not every direct message is urgent, and auto-interrupting on every DM would blur the signal/message split (§2.7.1 vs §2.7.2) and corrupt in-flight tool calls.

The recipient learns to read its inbox through two mechanisms:

1. **Convention** — the composed system prompt (§4.3) instructs every runner to check its inbox at natural task boundaries.
2. **Signals as the urgent wake-up** — if a sender needs the recipient to drop everything, they emit a signal in addition to (or instead of) the message. The signal goes through the router's fixed handlers, which may inject stdin. In MVP the injection is not enriched with inbox summaries; the recipient calls `runner msg read` when it wants the related conversation context.

The inbox is not a queue in the delete-on-read sense — messages stay in the log forever (well, for the mission). The "read" in `msg read` is lookup, not consumption.

#### 2.7.4 Thread *(v0.x)* — *scoped conversation*

When a mission has 3+ runners or runs for long enough to develop sub-topics, the flat message stream gets noisy. Threads add a scoping layer: messages can be posted to a named thread; runners can `msg read <thread>` to get just that conversation.

Cut from v0 because the v0 demo is two runners on one loop — the whole mission *is* the thread.

#### 2.7.5 Fact *(v0.x)* — *queryable state*

A KV whiteboard. Any runner can `ctx set key value` and `ctx get key`. Mission-scoped; each mission starts with an empty whiteboard. Backed by the event log as a `fact_recorded` event type, projected in-memory by the backend for O(1) reads.

Facts differ from messages and signals: they're **current state**, not events. Reading a fact answers "what is true right now?" not "what happened?" Cut from v0 because the demo doesn't need a dashboard-style current-state view.

### 2.8 Events — *the unifying transport*

Every coordination primitive is persisted as an **event** — one line in the per-mission NDJSON file. An event has: `{id, ts, crew_id, mission_id, kind, from, to, type, payload}`.

The `kind` field is the primitive discriminator — `signal`, `message`, and (later) `fact`, `thread_opened`. For `kind: "signal"`, the `type` field carries the signal's semantic verb (`runner_status`, `ask_lead`, etc.); for `kind: "message"`, `type` is omitted and the prose lives in `payload.text`. The router and UI project events into primitive-specific views based on `kind`.

This is a transport detail — runners interact through the CLI verbs (`runner signal`, `runner msg`), not the event schema directly. There is no separate `signal_emitted` or `message_posted` event type; the `kind` field is authoritative.

## 3. Mission lifecycle

### 3.1 Start

```
user clicks Start Mission on a crew
  └─► MissionManager.start(crew_id):
        ├─ insert `missions` row (status=running, mission_id = ULID)
        ├─ mkdir $APPDATA/runner/crews/{crew_id}/missions/{mission_id}/
        ├─ touch events.ndjson
        ├─ for each runner in crew:
        │     composed_prompt = compose(runner.system_prompt,
        │                                mission.brief,
        │                                roster(crew),
        │                                coordination_notes(crew.signal_types))
        │     SessionManager.spawn(mission_id, runner, composed_prompt)
        ├─ Router.start(mission_id)  ← fresh in-memory state
        │     open events.ndjson, read history (empty), tail via notify
        └─ emit Tauri event: mission:{id}:started
```

### 3.2 End

```
user clicks End Mission  (or all sessions have exited)
  └─► MissionManager.end(mission_id, status):
        ├─ SessionManager.kill_all_in_mission(mission_id)
        ├─ Router.stop(mission_id)
        ├─ update `missions` row: status (completed/aborted), stopped_at
        └─ emit Tauri event: mission:{id}:ended
```

### 3.3 v0 constraint

Concurrent missions on the same crew are allowed. Each mission gets its own session set, coordination bus, event log, and runner-CLI shim path; the crew row's `signal_types` allowlist is shared but immutable mid-mission. Per-crew throttle / rate-limit is a future consideration.

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
  PATH                = $APPDATA/runner/bin:<original PATH>
  RUNNER_CREW_ID      = <ulid>
  RUNNER_MISSION_ID   = <ulid>
  RUNNER_HANDLE       = coder
  RUNNER_EVENT_LOG    = $APPDATA/runner/crews/<crew>/missions/<mission>/events.ndjson

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
4. **Coordination notes** — how to use `runner signal` and `runner msg`, the crew's allowed signal types, and conventions for inbox checking.

Example for the `reviewer` runner (display name "Reviewer"):

```
You are `reviewer` (Reviewer), a runner in crew "Feature Ship".
Your role: code review.

== Your brief ==
When `coder` requests review, read their messages and the diff,
then either approve or request changes with specific feedback.

== Mission ==
Goal: Implement feature X with tests and a clean PR.

== Your crewmates ==
- `coder` (Coder, implementation): Writes code. Will signal review_requested
  and post messages explaining what changed.

Handles (the lowercase names in backticks above) are what you use to
address crewmates. Display names are shown for readability only.

== Coordination ==
- Signal milestones with `runner signal <type>`.
  Signal types: review_requested, changes_requested, approved, blocked.
- Post prose with `runner msg post "<text>"` (broadcast) or
  `runner msg post --to <handle> "<text>"` (direct to one crewmate).
- Your inbox is `runner msg read` (broadcasts + messages addressed to you).
  Check it at natural task boundaries:
    * before starting a new task,
    * before emitting a signal that affects another runner,
    * whenever you're waiting on something (poll with `--since <last_ulid>`).
  Urgent items will also arrive via stdin when someone signals — but by
  default, the inbox is pull-based.
```

### 4.4 Frontend wiring and human takeover

- On first view: fetch the session's scrollback ring; write to xterm.js to restore history.
- Subscribe to `session:{id}:out` for live output.
- xterm.js `onData` → `send_input(session_id, bytes)` → `master.writer.write_all(bytes)`.
- Frontend window resize → debounced (~100ms) `master.resize(rows, cols)` → SIGWINCH to child. Non-optional; without it, TUIs mis-render.

**Human takeover is a first-class capability.** At any moment, the human can type directly into any runner's stdin — the same writer the router uses for stdin pushes. This is deliberate: the human can step in to answer a prompt the agent is stuck on, correct a bad plan, kill a runaway tool call, or just chat with the agent mid-flight.

The UI surface for this is the xterm pane itself — it's a real terminal, not a log viewer. Typing sends keystrokes through untouched, including special keys (arrows, Enter, Ctrl-C). The agent on the other end can't tell whether the bytes came from the router, the human, or its normal terminal input — which is the whole point.

### 4.5 Sessions outlive the UI

Sessions live in the Rust backend and belong to the mission, not to any webview or tab. Closing the mission control window does *not* kill the sessions — the agents keep running, events keep flowing into the NDJSON file, and the router keeps handling live signals. Re-opening the window re-attaches: the frontend fetches each session's scrollback ring to rebuild xterm state, then subscribes to live output from wherever it was.

The only things that end a session in v0 are: user clicks End Mission, the child process exits, or the app itself quits. A closed webview window is none of those.

**Why this matters for human takeover:** if the only way to type into a runner required the UI to be visible, then minimizing or closing the mission view to focus on something else would silently cut the human out of the loop. That's wrong — the human should be able to close the monitor and still inject stdin (or let the router do it) without anything changing about how agents run.

### 4.6 Writer serialization

The PTY master writer is shared between the human (via `send_input` command) and the router (via stdin pushes). Concurrent writers could interleave bytes mid-line, which would confuse the TUI on the other end.

Solution: wrap each session's writer in a `tokio::sync::Mutex`. Every write is one `write_all` call under the lock. Small writes (keystrokes, short prompts) are fast enough that contention is invisible.

### 4.7 Threads, not async

`portable-pty`'s reader is blocking. Spawn an OS thread per session. Writers stay on the Tauri async runtime (writes are short).

### 4.8 Scrollback in Rust

`VecDeque<String>` ring (~10k lines) per session in SessionManager, so scrollback survives tab-switches and app restarts. Overflow lines append to `missions/{mission_id}/sessions/{session_id}.log`. The ring sees raw bytes including alt-screen toggles — acceptable v0 scuff.

### 4.9 Death and kill

Reader thread owns the child handle. On EOF, it calls `wait()`, emits `session:{id}:exit`, updates the sessions row. No auto-restart in v0.

Kill: drop master → SIGHUP via `portable-pty`; escalate to SIGKILL if child lingers. v0 targets macOS; Linux best-effort; Windows deferred.

## 5. Coordination bus

### 5.1 Transport

```
$APPDATA/runner/crews/{crew_id}/missions/{mission_id}/events.ndjson
```

One line per event. Append-only. **Each mission has its own file** — scopes log rotation, crash-replay, and deletion.

Why a file instead of an in-memory bus:
- **Debuggable** — `tail -f events.ndjson | jq .`.
- **Crash-durable** — whatever's on disk survived the crash.
- **Atomic** under explicit guards (see §5.1.1) — concurrent `runner` invocations interleave correctly at line granularity.
- **Replayable for projections** — restart the router, re-scan pending asks and runner status, resume live tail.

#### 5.1.1 Concurrent-write correctness

Multiple runners can invoke `runner signal` / `runner msg` at the same time from different PTYs. We need line-granular atomicity regardless of filesystem. The approach:

1. Open the log with `O_APPEND | O_WRONLY | O_CREAT`.
2. Acquire an advisory exclusive lock: `flock(fd, LOCK_EX)`.
3. Emit exactly one `write(2)` call with the serialized JSON line including the trailing `\n`.
4. `close(fd)`, which releases the lock.

This gives us:
- **Ordering**: `O_APPEND` guarantees the write lands at end-of-file at the moment the kernel performs it.
- **Atomicity across writers**: `flock(LOCK_EX)` serializes writers. Without it we cannot safely rely on filesystem-level write atomicity — `PIPE_BUF` applies to pipes, not regular files, and small-write atomicity on regular files is filesystem-specific.
- **No partial lines**: a single `write(2)` call of the full line + `\n` means either the whole line lands or none of it does (the kernel writes sequentially under the lock).

**Filesystem requirements.** The app data directory must be on a local POSIX filesystem (APFS, ext4, XFS, etc.). Network filesystems (NFS, SMB) and iCloud-synced volumes may not honor `flock()` or may re-order appends across clients; v0 documents this and checks at app startup.

Writers: the bundled `runner` CLI writes runner-authored events to the log; the Rust backend writes router-generated events such as `human_question` and `mission_warning` through the same `flock`-guarded path. No other process should write to this file.

### 5.2 Event schema

```jsonc
{
  "id": "01HG3K1YRG7RQ3N9...",     // ULID: time-sortable, monotonic within ms
  "ts": "2026-04-21T12:34:56.123Z",
  "crew_id": "01HG...",
  "mission_id": "01HG...",
  "kind": "signal",                 // signal | message  (v0.x adds: fact, thread_opened, ...)
  "from": "coder",                  // runner handle | "human" | "router"
  "to": null,                       // null = broadcast; runner handle = directed (messages);
                                    //   for signals, always null in v0 (policy decides routing)
  "type": "review_requested",       // for kind=signal; omitted for kind=message
  "payload": { "...": "..." }       // kind-specific (e.g. { "text": "..." } for messages)
}
```

For a signal event: `kind=signal`, `type` is set, payload optional.
For a message event: `kind=message`, `payload.text` is the prose.

- **ULID `id`** — sortable, embeds a ms timestamp. Ordering *is* the graph in v0.

**On `correlation_id` / `causation_id` (deferred).** An earlier draft carried both fields on every event. They're dropped from v0:

- The only real use case in v0 is matching an `ask_human` card to the `human_response` the operator clicks — that's handled in-payload via `human_response.payload.question_id` (§5.5.0).
- No runner-authored event has a reliable prior cause to cite; exposing `--correlation-id` / `--causation-id` as CLI flags just invites agents to leave them null or hallucinate values.
- "Groups events in one conversation" has no referent until threads (v0.x) define what a conversation is. Ship the concept with the thing that needs it.

v0.x will reintroduce explicit event-DAG fields when threads and richer routing require them. Until then, HITL causality is carried by `human_question.payload.triggered_by` and `human_response.payload.question_id`.

### 5.3 How runners emit signals and messages

Three layers — answers "how does the agent know to append to the event log?"

#### Layer 1 — system prompt tells the convention

The composed prompt (§4.3) includes a Coordination section describing the `runner` CLI and listing allowed signal types. LLM agents already know how to read CLI docs and invoke tools — same capability they use for `git`, `gh`, `npm`.

#### Layer 2 — the CLI exists on PATH with context in env

The backend prepends `$APPDATA/runner/bin/` to PATH and drops the `runner` binary there at first run. At session spawn, env vars point at the mission's log and identify the runner.

On invocation, the CLI:
1. Reads env vars; errors if missing.
2. Builds an event (ULID, timestamps, `from` = `$RUNNER_HANDLE`, `crew_id`, `mission_id`, `kind`).
3. For directed messages, validates `--to <handle>` against the crew's runner handles (rejects unknown handles with a clear stderr message).
3. For signals: validates `type` against the allowlist sidecar at `$APPDATA/runner/crews/{crew_id}/signal_types.json`.
4. Appends one JSON line to `$RUNNER_EVENT_LOG` via `open(O_APPEND | O_WRONLY)` + `write_all` + close.
5. Exits 0.

Each invocation writes ≤ one 4KB line in one `write` syscall; concurrent emitters interleave safely.

#### Layer 3 — role briefs reinforce usage

User-authored briefs include examples at the moments where signaling or messaging matters. We ship sensible defaults per-runtime.

#### Why robust

- No in-band protocol in the PTY stream. Not parsing stdout for magic markers.
- Works for any CLI agent (MCP or not). Only requirement: can run shell commands.
- Fails visibly. If the agent forgets to signal, the router sees nothing and the user sees the runner's last reported status.
- Fully observable. `$ runner signal review_requested` shows up literally in the terminal pane; the resulting event shows up in the timeline and messages panel.

#### Failure mode: hallucinated signal types

CLI validates against the allowlist; unknown types exit non-zero with a clear stderr message. Agent reads the error from shell history and self-corrects.

Messages have no allowlist — they're prose.

### 5.4 Consumers

Two subscribers to the NDJSON file, both via `notify`:

- **Signal router** — deserializes each new line. For built-in signals, runs a fixed handler. For messages, no-op by design (v0.x: threads, routing-by-mention).
- **UI** — the backend re-emits each line as a `mission:{id}:event` Tauri event. Frontend splits by `kind` into the messages panel and the signal/timeline panel.

#### Startup replay

On router boot: open the mission's file, fold `ask_human` / `human_response` and `runner_status` rows into in-memory state, then switch to tailing from the current end of the log. Replay rebuilds projections; it does not re-run historical stdin pushes.

### 5.5 Signal router handlers

The router is a flat dispatcher, not a policy engine. There is no per-crew `{when, do}` rule list in MVP. The lead runner owns coordination judgment; the router owns parent-process plumbing that a child PTY cannot do itself.

Stdin pushes are deliberately silent: the router writes bytes into the target PTY but does not synthesize `stdin_injected` audit events. The event log records the signal that caused the push, plus `human_question` / `human_response` for HITL cards.

| Signal type | Fixed handler |
|---|---|
| `mission_goal` | Compose launch prompt and inject it to the lead's stdin. |
| `human_said` | Inject `payload.text` to `payload.target` if present, otherwise to the lead. |
| `ask_lead` | Inject the worker's `{ question, context }` to the lead. |
| `ask_human` | Append a `human_question` event for the UI. |
| `human_response` | Look up the matching `question_id` and inject the answer to the runner that emitted the original `ask_human`. |
| `runner_status` | Update the latest-status map from `payload.state`. If a non-lead reports `idle`, inject a short availability update to the lead. |

#### 5.5.0 `ask_human` — payload shapes and matching

`ask_human { prompt, choices }` produces two correlated signals:

```jsonc
// When the card is shown:
{
  "id": "01HG...",                             // canonical question_id (use this in human_response)
  "kind": "signal",
  "type": "human_question",
  "from": "router",
  "payload": {
    "triggered_by": <triggering-signal.id>,    // e.g. the changes_requested signal's id
    "prompt": "Reviewer requested changes. Accept or override?",
    "choices": ["accept", "override"],
    "on_behalf_of": "@impl"                    // optional; see "Lead-mediated asks" below
  }
}

// When the human clicks a choice:
{
  "kind": "signal",
  "type": "human_response",
  "from": "human",
  "payload": {
    "question_id": <human_question.id>,       // = the card event's `id` field
    "choice": "accept"                         // the clicked value (always one of choices[])
  }
}
```

Causality is carried in-payload rather than on the envelope: `human_question.payload.triggered_by` records which signal opened the card, and `human_response.payload.question_id` records which card the human clicked (set to the `human_question` event's own `id`). The router needs no additional schema fields to match them.

**Implementation note.** `human_question` does *not* echo `question_id` into its own payload. The canonical id is the event's own `id` field, and the parent process can't know that id until after the flock-guarded log append assigns it (pre-allocating would break the cross-process monotonic-ULID invariant in §5.1.1). UI consumers should read `human_question.id` and pass it through as `human_response.payload.question_id`.

If two `ask_human` prompts are ever outstanding at once and we need richer discrimination, v0.x will add event-DAG fields. v0 ships the simple case; concurrent prompts are out of scope.

**Lead-mediated asks (the canonical pattern).** By convention (§2.2), workers do not escalate to the human directly. The flow is entirely signal-driven — never message-triggered — so it does not violate the pull-based rule below.

1. **Worker asks the lead.** Worker emits an `ask_lead` signal with the question in its payload:

   ```
   runner signal ask_lead --payload '{"question": "Should I add notify-debouncer-full?", "context": "Pros: … Cons: …"}'
   ```

   `ask_lead` is a built-in signal type. Its fixed handler is `ask_lead → inject_stdin @lead` (payload rendered into the injection template). The worker's stdin stays blocked waiting; the lead wakes.

2. **Lead decides.**
   - **Answer from own context.** Lead posts a directed message back to the worker via `runner msg post --to @impl "…"`. The worker picks it up on its next `runner msg read`. Pull-based; no new wake-up needed because the worker is already polling between turns per its system prompt.
   - **Escalate to human.** Lead emits `ask_human` with `payload.on_behalf_of: "@impl"` (the original asker's handle) and a `prompt` that restates the question for the human. The router appends `human_question`; the UI uses `on_behalf_of` to show the attribution chain (*@impl → @architect → you*).

3. **Human responds.** On click, `human_response` fires. The router injects the result into **the lead's stdin** (the lead was the asker of record for that `question_id`). The lead then forwards the answer onward via a directed message:

   ```
   runner msg post --to @impl "Human approved: use notify-debouncer-full."
   ```

   The worker picks up the answer on its next `runner msg read`.

This is not a new protocol — it is `ask_lead` + `ask_human` + directed messages composed. The only schema additions are the `ask_lead` signal type and the optional `on_behalf_of` field on `human_question`.

**Why route through the lead.** The lead can absorb, filter, or batch worker questions, and the human's attention stays focused on one interlocutor. The tradeoff is added latency and the possibility of the lead paraphrasing the human's answer imprecisely — both acceptable for v0. The full chain is always visible in the event log for audit.

**Worker-initiated asks.** A worker *may* emit `ask_human` directly (with no `on_behalf_of`); the router will route `human_response` back to that worker's stdin as in the direct flow. This is a fallback for cases where the lead is paused or unavailable and the system prompt explicitly permits it. It is not the default path.

**Messages do not trigger router actions in v0.** The inbox is pull-based (§2.7.3). If a sender needs the recipient to drop everything, they emit a signal — signals are the urgent wake-up mechanism; direct messages are async conversation.

#### 5.5.1 `runner_status` — availability reporting

Workers report capacity explicitly instead of making the parent process infer intent from PTY output:

```jsonc
{
  "kind": "signal",
  "type": "runner_status",
  "from": "impl",
  "payload": {
    "state": "idle",                         // "busy" | "idle" in MVP
    "note": "ready for next task"             // optional
  }
}
```

The router updates an in-memory status map and the UI projects the latest value in the runners rail. If a non-lead reports `idle`, the router also injects a short availability update to the lead. The lead decides whether to assign work; the router does not run a dispatch policy.

### 5.6 Who does delivery

Two different delivery models, by primitive kind:

- **Signals are router-routed.** Runners never address other runners with a signal. A signal is emitted into the bus; the router's fixed handler decides the parent-process side effect, if any. This keeps urgent wake-up plumbing in one place while leaving work assignment decisions to the lead.
- **Messages support both broadcast and direct addressing, but are pull-based.** A runner can `msg post` (everyone's inbox) or `msg post --to <runner>` (that runner's inbox only). The router is *not* in the message delivery path — messages sit in the inbox until the recipient runs `msg read`. If a sender needs immediate attention, they emit a signal; signals are the urgent-wake-up channel.

The split:

| | Sender addresses recipient? | Delivery timing | Router in path? |
|---|:---:|---|:---:|
| Signal | No — fixed handler decides | Immediate for wake-up handlers | Always |
| Broadcast message | No | On recipient's `msg read` | No |
| Direct message | Yes (`--to`) | On recipient's `msg read` | No |

- **Decoupled wake-up plumbing** — a worker can ask the lead or report idle without knowing anything about the parent process's PTY writers.
- **Coupled content flow where it's natural** — if Coder wants to ask Reviewer a specific question, it can just `msg post --to reviewer ...`. The roster injection (§4.3) already tells each runner the current names of its crewmates, so direct addressing works without extra config.
- **Single parent-process routing location** — every urgent wake-up mechanism lives in one fixed router, not scattered across runners.
- **No auto-interrupt for messages** — agents check their inboxes on convention and on signal-triggered wake-ups. This preserves the signal/message split (urgent vs async) and keeps in-flight tool calls uncorrupted.

### 5.7 Known failure modes

| Failure | Mitigation |
|---|---|
| Router crashes mid-stdin push | Replay rebuilds pending asks and runner status, but does not replay historical stdin pushes; the lead or human may need to retry the lost wake-up. |
| Two runners ask human at once | Concurrent prompts are out of scope for MVP; UI should render both if present, but the default prompt tells workers to go through the lead. |
| Event storm | Surface events/sec warning; no rate limit in v0 |
| Malformed NDJSON line | Skip and warn; file stays valid |
| NDJSON grows large | End the mission; new mission = new file |
| Hallucinated signal type | Allowlist validation + clear stderr |
| Runner posts messages nobody reads | Surface "unread by crewmate X" indicator in UI (v0.x) |

## 6. Shared context (mission-scoped)

Two layers in v0.

### 6.1 Mission brief (read-only, prompt-injected)

`missions.goal_override` or falls back to `crews.goal`. Injected into the composed prompt at spawn (§4.3). Never changes during a mission.

### 6.2 Roster (read-only, prompt-injected)

Rendered from `crew.runners` at mission start into each runner's prompt as `== Your crewmates ==` with name, role, and one-line brief summary. Never changes during a mission.

This is how the Reviewer knows there's a Coder.

### 6.3 The `runner` CLI surface in v0

```
runner signal <type> [--payload <json>]
runner msg    post <text> [--to <runner>]
runner msg    read [--since <ts>] [--from <runner>]
runner help
```

One binary. Two verbs. Context always from env. No event-DAG flags in v0 — causality is implicit (ordering in the log) or in-payload where it needs to be explicit (`human_response.payload.question_id`).

- `msg post` with no `--to` → broadcast.
- `msg post --to <runner>` → directed; lands in that runner's inbox only.
- `msg read` → the calling runner's inbox (broadcasts + directs addressed to me), sorted by ULID.
- `msg read --from <runner>` → filter to messages authored by a specific sender.
- `msg read --since <ts>` → only messages newer than `ts` (for polling without re-reading history).

## 7. Data model

### 7.1 SQLite (config + session lifecycle)

```sql
crews (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  purpose TEXT,                       -- short prose shown in Crew Detail; optional
  goal TEXT,                          -- default mission goal
  orchestrator_policy TEXT,           -- JSON: [{ when, do }]
  signal_types TEXT,                  -- JSON array: allowlist
  created_at TEXT, updated_at TEXT
);

runners (
  id TEXT PRIMARY KEY,
  handle TEXT NOT NULL UNIQUE,        -- globally unique slug; see §2.3
  display_name TEXT NOT NULL,         -- free-form UI label
  role TEXT NOT NULL,
  runtime TEXT NOT NULL,
  command TEXT NOT NULL,
  args_json TEXT,
  working_dir TEXT,
  system_prompt TEXT,
  env_json TEXT,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);

-- Crew membership lives here, not on `runners`. One runner can join many
-- crews; `lead` and `position` are per-crew (§2.3).
crew_runners (
  crew_id TEXT NOT NULL REFERENCES crews(id) ON DELETE CASCADE,
  runner_id TEXT NOT NULL REFERENCES runners(id) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  lead INTEGER NOT NULL DEFAULT 0,
  added_at TEXT NOT NULL,
  PRIMARY KEY (crew_id, runner_id),
  UNIQUE (crew_id, position)
);

-- Enforces the lead invariant (§2.2): exactly one lead per crew.
CREATE UNIQUE INDEX one_lead_per_crew ON crew_runners(crew_id) WHERE lead = 1;

missions (
  id TEXT PRIMARY KEY,
  crew_id TEXT NOT NULL REFERENCES crews(id) ON DELETE CASCADE,
  title TEXT NOT NULL,                -- short label shown in missions list + event log
  status TEXT NOT NULL,               -- running | completed | aborted
  goal_override TEXT,                 -- null means inherit crews.goal
  cwd TEXT,                           -- mission working dir; exposed as $MISSION_CWD
  started_at TEXT NOT NULL,
  stopped_at TEXT
);

sessions (
  id TEXT PRIMARY KEY,
  -- Nullable: direct-chat sessions exist without a mission (§2.6). For
  -- mission sessions, deleting the mission detaches the session
  -- (`SET NULL`) so historical session rows survive for activity stats.
  mission_id TEXT REFERENCES missions(id) ON DELETE SET NULL,
  runner_id TEXT NOT NULL REFERENCES runners(id) ON DELETE CASCADE,
  cwd TEXT,                           -- working dir; carried for direct-chat sessions
  status TEXT NOT NULL,               -- running | stopped | crashed
  pid INTEGER,                        -- OS process id once spawned; null while pending
  started_at TEXT, stopped_at TEXT
);
```

### 7.2 Filesystem

```
$APPDATA/runner/
├── bin/
│   └── runner                               # the CLI (signal + msg)
├── runner.db                                # SQLite
└── crews/
    └── {crew_id}/
        ├── signal_types.json                # CLI allowlist sidecar
        └── missions/
            └── {mission_id}/
                ├── events.ndjson            # per-mission event log
                └── sessions/
                    └── {session_id}.log     # scrollback overflow
```

macOS: `$APPDATA` = `~/Library/Application Support/com.wycstudios.runner`. Dev builds use `-dev` suffix.

## 8. Process and thread model

```
Tauri main thread
  ├── Tauri async runtime (tokio)
  │     ├── MissionManager (async)
  │     ├── Router task per live mission (notify + fixed handlers)
  │     └── Tauri command handlers (CRUD)
  ├── Thread per active session (blocking PTY reader)
  └── Webview process (React + xterm.js)
```

For v0 scale (a handful of concurrent missions, ≤ ~10 sessions in total): fine.

## 9. Out of scope for v0

- Threads (v0.x)
- Facts / shared whiteboard (v0.x)
- Mentions, reactions (v1)
- Cross-mission memory
- Remote runners / SSH
- Sandboxing beyond the child's own permissions
- MCP-based signal emission
- Auto-restart on crash
- Event log rotation (solved implicitly by per-mission files)
- LLM-based signal routing
- Multi-user / multi-machine coordination bus

## 10. Next milestones (vision)

### v0.x — Threads and Facts

**Threads** — when missions grow past 2 runners or 1 hour, messages get noisy. Add:
- `runners thread open <name>` → returns thread_id
- `runner msg post --thread <id> <text>`
- `runner msg read --thread <id>`
- Router can gain "open thread on signal X" handling
- UI splits message stream by thread

**Facts** — the shared whiteboard. Add:
- `runners ctx set/get/unset/list`
- `fact_recorded` event type; last-writer-wins projection in the backend
- UI gains a facts panel
- Solves "current state of the mission" at a glance

Both live on the same event log as new `kind` values. No transport changes.

### v1 — Mentions, reactions, richer routing

- `@runner` mentions inside messages → router can route on them
- Reactions (`👍`, `blocking`) on messages — lightweight signals
- Cross-mission memory / "crew memory"
- Concurrent missions per crew

## 11. Architectural bets

1. **Mission is the runtime unit.** Crew is config; mission is a run.
2. **PTY, not pipes.** TUI fidelity is non-negotiable.
3. **NDJSON file per mission, not broker.** Debuggable and crash-durable.
4. **CLI wrapper, not MCP.** Works with every agent today.
5. **Signals and messages as distinct primitives.** Keeps the router simple and prose natural.
6. **The signal router is the only urgent wake-up path.** Runners stay decoupled.
7. **Prompt composition at spawn time.** Replaces runtime handshakes.
8. **Incremental vocabulary.** v0 = signals + messages; v0.x adds threads + facts; v1 adds mentions + reactions.
9. **xterm.js for rendering.** Don't reinvent the terminal emulator.
10. **ULID for event IDs.** Sortable, monotonic within ms.

## 12. Open questions

1. CLI installation: bundled with `.app`, copied to `$APPDATA/runner/bin/` on first run — ok?
2. Resize debounce: 100ms is a guess; tune with a real TUI.
3. Signal type allowlist: per-crew only (current), or global defaults + per-crew overrides?
4. `from` field: locked to env (v0), or `--from` override?
5. Does `runner msg read` return everything or paginate? v0: everything, sorted by ULID.
6. Does the router include recent messages when injecting stdin on a signal? v0: no — the recipient calls `runner msg read` when it wants inbox context.

## 13. What would break this architecture

- A runtime with no way to inject a system prompt at spawn (we'd type into stdin post-spawn — ugly but doable).
- An agent that won't learn to call CLI tools.
- NDJSON append atomicity breaking on an exotic filesystem (NFS, iCloud-synced). v0: document that app data must be on a local POSIX filesystem.
