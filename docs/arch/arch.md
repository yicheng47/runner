# Runner — Architecture

> Companion to [`../product/vision.md`](../product/vision.md). The vision doc defines *what* we're building and why; this doc defines *how* it works — tech stack, the model concepts the code is built around, and the protocol / schema decisions that make the model work. Rewritten 2026-08-22 (M6.5) for the native GPUI app (shipped as `v0.6.0` on 2026-08-23; §14 updated then); the Tauri + xterm.js era version is in history (`git show 0e5ea18:docs/arch/arch.md`) and the port that replaced it is recorded in [`../impls/archive/gpui-rewrite/`](../impls/archive/gpui-rewrite/README.md).

## 1. Overview

Runner is a local desktop app for macOS and Windows. A user configures a **crew** of CLI coding agents, launches a **mission** to activate it, and watches the crew coordinate in real time. The app is one native process: a GPUI user interface, a Rust application core (`crates/runner-backend`), an `alacritty_terminal` grid per live session, SQLite for configuration, and a per-mission NDJSON file for live coordination state. There is no webview, no IPC bridge, and no serialization between the PTY and the screen.

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
│   │  per mission     │  │ (ops::mission│  │   / trae / copilot / pi /  │     │
│   └──────▲───────────┘  │  lifecycle)  │  │   shell                    │     │
│          │ flock append └──────────────┘  │  env: RUNNER_*, PATH=…     │     │
│          │                                └───────┬────────────────────┘     │
│          └────────────────────────────────────────┘ runs `runner` CLI        │
│                                                                              │
│   MCP server (rmcp, Unix socket $APPDATA/mcp.sock) ◄── runner CLI transport                    │
│   SQLite runner.db (rusqlite + r2d2, WAL) — config + session lifecycle, off the hot path │
└──────────────────────────────────────────────────────────────────────────────┘
```

**Three layers inside the box.**

*Orchestration (lifecycle only).* **MissionManager** (`ops::mission`) starts, stops, archives and resets missions, composes each role's system prompt at spawn, and re-mounts router + bus state for `running` missions on launch. Once a mission is up it goes quiet; it is not in the runtime data path.

*Runtime (the hot path).* **SessionManager** owns each PTY master, the blocking reader thread, the serialized writer, the idle detector and the session's last applied size. **EventBus** tails the per-mission NDJSON file with `notify`, parses each new line, hands it to the **Router** for handler dispatch and republishes a `mission/changed` notification for the UI. "Projections" — inbox, pending HITL cards, status map — are in-memory rollups over the same event stream.

*Presentation (the app crate).* **`AppStore`** holds read snapshots of the rows the UI renders and turns `AppEvent`s into scoped GPUI notifications. **`TerminalBridge`** (in `crates/runner-terminal`) owns one `alacritty_terminal::Term` per live session for the session's lifetime; panes borrow the terminal, they never own it. The terminal element paints the grid, and keys, IME composition and mouse events go straight back to the PTY writer.

**Two channels out of the core, deliberately different.** Terminal bytes take the synchronous path: the PTY reader thread calls `SessionEvents::output` and the bridge feeds the session's `Term` under its lock — no queue, no encoding, nothing to lag. Everything else (row changes, mission events, session lifecycle, router warnings) goes through a `tokio::sync::broadcast` of `AppEvent`s consumed by the `native-app-events` thread, which updates `AppStore` and wakes GPUI. Names in use today: `mission/changed`, `mission/resync`, `session/spawned`, `session/exit`, `session/updated`, `session/archived`, `session/status`, `session/warning`, `role/changed`, `role/activity`, `crew/changed`, `slot/changed`, `project/changed`, `chat/layout-changed`, `router/delivery-blocked`, `app/woke`.

**One session.** The session row is one slot's PTY process: SessionManager holds the master file descriptor; the child runs the agent binary with a real tty on stdin/stdout/stderr. The env vars are what make the bundled `runner` CLI work inside that child — when the agent runs `runner msg post …`, the CLI reads `RUNNER_MISSION_ID` + `RUNNER_EVENT_LOG` from its environment, builds the JSON line, and `flock`-appends to the right file. No daemon, no socket; the CLI opens the file directly.

**Closing the loop.** Child invokes `runner` CLI → CLI appends a line to `events.ndjson` → `notify` wakes the EventBus → the line goes to (a) the Router and (b) the UI as `mission/changed`. If a handler needs to wake a session, it writes bytes into that session's PTY through SessionManager's writer. The bus is the spine: all coordination flows through one append-only file, which is why it's debuggable with `tail -f | jq`.

**What's not in the hot path.** SQLite holds configuration and session-lifecycle metadata (roles, crews, slots, projects, the sidebar tree, mission rows, session rows with PID, runtime metadata and last size). Live coordination state lives in the NDJSON file or in the router's memory; live screen state lives in the `Term`s.

**The invariant this picture encodes.** There is exactly one piece of mutable shared state per mission: `events.ndjson`. Every other component is a writer to it (the `runner` CLI; the router for `human_question` / `mission_warning`), a reader of it (EventBus → router + UI), or a per-session PTY pipeline that doesn't touch it. On restart Runner re-opens the file and reconstructs router/feed projections from replay. PTY children do not survive app restart; their rows become resumable stopped sessions, and sessions flagged `resume_on_launch` are re-spawned at startup.

### 1.2 Code map

Crate boundaries are in [`AGENTS.md`](../../AGENTS.md); this is the shape *inside* `crates/runner-app/src/surfaces/`, where most of the UI lives.

**One directory per surface, one file per concern.** A surface that outgrows a single file becomes a directory: `mod.rs` holds the types, the state struct and the `Render` impl; every other file owns one concern and carries its own `use` block; tests live in `tests.rs`. `settings/` set the pattern and the four largest surfaces followed it in [#478](https://github.com/yicheng47/runner/issues/478) on 2026-09-13.

| Surface | Files | Largest | Concerns beyond `mod.rs` and `tests.rs` |
|---|---|---|---|
| `mission_workspace/` | 14 | 884 | state, routing, attach, drawer, events, actions, input, view, feed, composer, terminal pane, rail |
| `sidebar/` | 13 | 1049 | state, activation, archive, rows, shortcuts, menus, project, drag, view, row renders, elements |
| `crews/` | 10 | 777 | list, editor, editor sections, create, add slot, slots, overlays, logic |
| `roles/` | 10 | 570 | forms, create, edit, delete, list, detail, menu, logic |

`chat.rs`, `panes.rs`, `settings_page.rs` and `start_chat.rs` are still single files. The ones that have since outgrown the shape are tracked in [#582](https://github.com/yicheng47/runner/issues/582).

**Three privacy rules, one per error, and they are not interchangeable.** A **field** declared in `mod.rs` is visible to every child, so `self.field` from a concern file needs nothing. A private **method** moved into one child and called from a sibling is `E0624`; it takes `pub(super)`, applied only where the compiler asks, and needing `pub(crate)` means the item is reached from outside the directory and the cut is wrong. A private **field of a type declared in a child**, read from a sibling, is `E0616`; `pub(super)` on the type does not expose its fields, so the type moves to `mod.rs` instead.

**Paths into a surface stay stable.** `panes.rs` and `chat.rs` reach into `sidebar::` directly, so every item that is `pub(crate)` keeps resolving at `crate::surfaces::<surface>::<name>` through a `pub(crate) use` in `mod.rs`. The proof that a split preserved this is that the calling modules do not appear in its diff.

**Splitting one of these is a move, not a refactor.** Nothing about behavior, signatures or formatting changes; the permitted mechanical differences are imports, module declarations, `impl` wrappers, compiler-driven `pub(super)`, and the rustfmt reflow a widened signature forces. The four archived briefs under [`../impls/archive/`](../impls/archive/) carry the method: a `syn` item audit keyed by `Type::method`, validated by negative controls that must make it fail, plus an unchanged workspace test count.

## 2. Tech stack

| Layer | Choice | Why |
|---|---|---|
| UI framework | **GPUI** (`gpui-ce` 0.3.3, Metal) | Zed's retained-mode Rust UI: entities + elements, one process with the core, native text shaping and IME. Replaced Tauri + React in the 2026-08 rewrite. |
| Terminal model | **`alacritty_terminal` 0.26** | Grid, VTE parser, scrollback with reflow, selection, mouse/alt-screen modes. The same model Zed embeds. |
| Terminal renderer | custom GPUI element (`runner-app/src/terminal/element.rs`) | Walks the `Term` grid per frame, shapes runs through GPUI's text system; bundled JetBrainsMono Nerd Font Mono is the default face, Menlo the alternative. |
| Application core | **Rust** crate `runner-backend`, UI-agnostic | SQLite, session manager, event bus, router, MCP server. The same crate could host another front end; the app crate is a consumer. |
| PTY runtime | **`portable-pty`** (in-process) | One blocking OS thread per session reads the master; writes are serialized per session. |
| Persistence | **SQLite via `rusqlite`** + `r2d2` pool, WAL | Config + session lifecycle only. Migrations in `crates/runner-backend/migrations/` (0001–0023). |
| Event transport | **Append-only NDJSON per mission** | Tailable, crash-durable, replayable; `flock(LOCK_EX)` for cross-process append atomicity. |
| File watching | **`notify`** | The bus tails the NDJSON file and republishes lines. |
| Bundled CLI | **`runner`** (`crates/runner-cli/`) | Agents talk to the bus through it — `runner signal …`, `runner msg post …`, `runner msg read`. Dropped at `$APPDATA/bin/runner` on first run, PATH-prepended per spawn. |
| MCP | **`rmcp`** server over local IPC | Runner.app owns stateful tool execution (crews, roles, slots, projects, missions, direct sessions); the bundled CLI uses `$APPDATA/mcp.sock` on Unix or `\\.\pipe\com.wycstudios.runner[-dev]` on Windows as its transport. |
| Logging | **`tracing`** + rotating file layer + panic hook | `~/Library/Logs/com.wycstudios.runner/runner.log`; release filter `info`, debug builds `debug`, `RUST_LOG` overrides. |
| Updater | **Sparkle 2.9.5** via `objc2` (`updater` feature) | `SPUStandardUpdaterController`, EdDSA-signed appcasts on GitHub Releases, with separate production and nightly feeds — see §14. |
| Packaging | `script/bundle-mac` | `.app` assembly, Developer ID codesign, notarization, DMG; `CFBundleVersion` is the build stamp. |
| Input | GPUI key dispatch + native IME (`terminal_ime.rs`) | Pinyin composition in the terminal was the hard requirement of the rewrite. |

**Platform target.** macOS on Apple Silicon and Windows x64. Intel Macs are not supported and no Intel build is planned: universal packaging shipped in `v0.6.0` and `v0.6.1` and was dropped for download size — 43 MB universal against 19 MB arm64 (Jason, 2026-08-25). Windows has shipped since `0.8.0`: every production release carries a signed `Runner-Setup-…-x64.exe` beside the arm64 DMG. Both platforms develop from `main` and both CI jobs run on every pull request, but only `Rust / macOS` is a required check, so a Windows break does not block the merge button — read that job before merging. See [Windows development](./windows.md) and §14. Platform window chrome lives in `crates/runner-app/src/platform_ui/{macos,windows}.rs` with fonts in the adjacent `fonts_{macos,windows}.rs`, selected at compile time; everything else is shared, and a Unix-only mechanism needs a `cfg` arm rather than an assumption. Linux is out of scope.

## 3. Domain model

Domain objects split into two layers:

- **Configuration** — persistent, user-edited. Outlives missions. Role, Crew, Slot, crew addendum, Project, the sidebar tree.
- **Runtime** — created at mission start, torn down at mission end. Mission, Session, the in-memory router state, the per-mission shared context.

The key insight: **a Role is config; a Session is its runtime instance** — same pattern as Crew (config) → Mission (runtime). A role is instantiated as a session *inside a mission* or as a one-off direct-chat session outside any mission.

### 3.1 Relationship diagram

```
┌─ Configuration (persistent) ─────────┐    ┌─ Runtime (mission-scoped) ──────────────┐
│                                      │    │                                         │
│   Role   ──── Slot (per-crew handle, │    │   Session ─► PTY process                │
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

### 3.2 Role — *one configured agent*

A reusable template: handle, display name, runtime, command + args, working dir, system prompt (persona), env, optional model and effort. Known runtimes use the `Runtime` enum in code (`ClaudeCode`, `Codex`, `Trae`, `Copilot`, `Pi`, `Shell`); SQLite runtime names remain plain strings, and legacy or arbitrary names, including `qoder` rows from before v0.6.7, stay unchanged and readable without a migration. Dispatch parses those names while preserving the existing behavior for unknown runtimes. **Top-level, not nested under a crew.** The same role can be used by many crews simultaneously, and can also be the subject of standalone direct-chat sessions.

A role has two identifying fields:

- **`handle`** — a lowercase slug (`coder`, `reviewer`). Required, **globally unique**, immutable once set. The handle is the role's identity in direct chats and in `from` fields when the session is not in a crew.
- **`display_name`** — free-form UI label. Editable; presentation-only.

Keeping these separate means renaming a role for the UI doesn't break briefs or historical events.

Runtime argv is composed by the adapter in `router/runtime.rs` from the stored role args, and is not itself stored: the permission mode (`--permission-mode` / Codex `--ask-for-approval` + `--sandbox` / Copilot `--yolo`) follows `MissionPermissionMode` for mission slots, while attended chats strip permission flags without replacement; the adapter adds model and effort flags, Codex and Copilot's `--add-dir` grant for the mission directory, and the first-turn body. Pi has no permission flags: mission slots receive `--approve`, chats do not, every spawn sets `PI_SKIP_VERSION_CHECK=1`, and model/thinking map to `--model <value>` and lowercased `--thinking <value>`. Every Pi invocation also receives `-e <app data>/pi-hooks/runner-status.ts` before its system-prompt and goal arguments when the Runner-owned extension is installed. Claude Code additionally receives one compact `--settings` JSON that selects its alternate-screen renderer (`"tui":"fullscreen"`), installs the `/clear` rekey hook, and, when the effective mode is Bypass, acknowledges its bypass consent dialog (`"skipDangerousModePermissionPrompt":true`) so a mission slot never waits on it, unless the role's own args already pass `--settings`. Runner owns the renderer for the sessions it spawns; `--settings` outranks the user's `~/.claude/settings.json`. Copilot always receives `--no-auto-update` and an additive `--plugin-dir <app data>/copilot-hooks` backed by a Runner-owned plugin regenerated at startup; each hook entry carries a `command` (the `sh` reporter, run on macOS) and a `powershell` slot (an inline reporter, run on Windows), and Copilot picks the slot by OS. The plugin takes its per-session feed path and generation from the spawn environment, so repeated user `--plugin-dir` flags compose and no file under `~/.copilot` is changed. A user `disableAllHooks` setting produces no feed and leaves the terminal-activity baseline in control.

### 3.3 Crew — *a configured team, composed of slots*

A named, persistent group of **slots**. Carries the default mission goal and the optional team-conventions addendum. It does not run. It is blueprint.

Crews are composed of **slots**, not roles directly. A slot is the indirection that lets the same role participate in many crews:

- **`slot_handle`** — the slot's in-crew handle (`@impl`, `@lead`). Required, unique within the crew. This is what crewmates address each other by — `runner msg post --to impl`.
- **`role_id`** — which role fills the slot.
- **`position`** — display order within the crew.
- **`lead`** — exactly one slot per crew carries `lead = 1`, enforced by a unique partial index (§10.1).
- **`runtime_override`, `model_override`, `effort_override`** — per-slot deviations from the template, so one crew can run the same persona on two runtimes.

**Why slot vs role.** Users curate a small library of roles and re-use them across crews and direct chats. Tying the in-crew handle and lead flag to the role would force duplicating configs every time a role shows up in a new crew.

**Lead is also the default HITL gateway.** When a worker needs human input, it does not ask the human directly — it emits an `ask_lead` signal. The router wakes the lead, who decides whether to answer from their own context or escalate via `ask_human`. The human's answer flows back to the lead, who forwards it to the original worker as a directed message. See §8.3. Workers *may* emit `ask_human` directly as a fallback, but the worker preamble (§6) instructs them to go through the lead.

### 3.4 Mission — *one activation of the crew, and the runtime container*

A mission is the only runtime container in the system. Everything alive at runtime lives *inside* a mission and dies with it:

- A **Session** per slot (the PTY processes — §3.5).
- The **coordination bus** — the NDJSON event log carrying signals and messages.
- The **router's in-memory state** — pending HITL asks, latest session availability, per-slot delivery outboxes.
- The **shared context** — composed system prompts (brief, roster, coordination notes, optional team conventions) injected at spawn.

Lifecycle:

- **Start**: a mission row is created (with its own `cwd` and an optional per-mission `goal_override`), one session is spawned per slot, the router boots with fresh state, and an NDJSON file is opened. Missions can be started from the UI or through the MCP `mission_start` tool.
- **Stop**: live PTYs are killed, but the mission row remains `running`; router/bus state stays mounted and stopped slots can be resumed.
- **Archive**: Runner appends `mission_stopped`, marks the row `completed`, sets `archived_at`, kills any live PTYs (verified dead before the row flips), and unmounts router/bus state. Archived missions are hidden from active lists and render read-only.
- **Reset**: kills the slots and re-spawns them against the same mission row and log; forks read the persisted last size so they open at the width the pane had.

**Mission cwd is authoritative.** Each mission carries its own `cwd` column. Spawned slots inherit `mission.cwd` regardless of what the role's `working_dir` says — that field is only used in direct chats. Starting from a project copies the project's cwd into the row.

Concurrent missions on the same crew are allowed — a crew is a reusable template, and per-mission state (sessions, the bus, the runner-CLI shim path, the roster sidecar) is fully namespaced by `mission_id`.

### 3.5 Session — *one slot's PTY process*

The runtime instance of a slot inside a mission, a role-backed direct chat, or a runtime-only direct chat. A Session is to a slot/role/runtime what a Mission is to a Crew: the *run* of a *configuration*.

Two flavors, distinguished by whether `mission_id` is set on the session row:

- **Mission session** — spawned when a mission starts; one session per slot. It participates in the crew's bus, sees broadcasts, can receive stdin injection from the router. `RUNNER_HANDLE` carries the *slot* handle, not the role's global handle.
- **Direct-chat session** — spawned ad-hoc without a parent mission, backed by a role or a bare runtime selection. `mission_id` is null and the working directory lives on the session row. The agent is **not on any coordination bus**. Role-backed chats keep `role_id`; runtime-only chats store `role_id = NULL` plus `agent_runtime` / `agent_command`.

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
- **Pane** — one slot inside a tab, holding one chat or shell session (move-not-copy). `⌘[` / `⌘]` cycle pane focus, `⌘W` closes the focused pane without stopping its session. Every pane carries a split icon — Split Right `⌘D`, Split Down `⇧⌘D` — which turns that pane's leaf into a 50/50 split with a new pane on the chosen side; a single-pane tab shows the icon in its header instead. There is no pane ceiling, only a size floor: an item is disabled once either half would fall under 240 × 160 px at 1× zoom. A terminal tab has at least one session and every filled pane holds a shell; empty panes do not affect its kind. Splitting it immediately spawns and focuses a shell in the split-from session's stored cwd. A tab containing a chat still splits to an empty New chat stub. New terminal on a terminal tab fills an existing empty pane or splits right through the same size gate; terminal tabs keep the drawer icon hidden.

  A tab's layout persists as a **tree** in its `nodes.layout` JSON: `tree` is nested splits (`id`, `orientation`, `sizes`, `a`, `b`) and leaves (`id`, `session_id`), with leaf ids `p<n>` and split ids `s<n>` drawn from one per-tab counter. `slots` (leaf session ids in leaf order) and `drawer` stay beside it because the backend reconciler reads only those two. Dragging a pane within the tree moves its leaf without changing its id or session, and new split ids continue the counter. Rows written before the tree — `preset` + `slots` + `sizes`, one of six picker shapes — are rebuilt through a legacy reader into the tree that preset drew, and are rewritten in the new shape on their next save.

Sessions exist independently of the display tree: closing a pane or window never kills a PTY, and since M6.8 it does not even drop the session's `Term` — the grid keeps ingesting while hidden and is simply painted again when a pane shows it.

The mission workspace's per-slot terminal switcher predates this hierarchy and is a different, mission-scoped UI element — not a Tab in the sense above.

### 3.7 Settings surface

Settings is a full-window route rendered in place of the app shell, with its own grouped sidebar and card-grouped panes: Appearance (zoom, theme), Terminal (font, cursor, theme palette), Agents (runtime discovery and overrides, the login-shell probe outcome, model and effort), Skills (global skill visibility and editing), MCP (the global server catalog), Keyboard shortcuts (a view over the registry in `runner-app/src/keymap.rs`), Updates, Diagnostics (log path, open-log), About, and Archived. Entry points — the sidebar Settings row, the command palette, and `⌘,` — navigate to the route and return to the caller's location.

Settings → Skills reads each runtime's declared user roots. TRAE CLI contributes only `~/.trae/skills` (not its `.coco` or `.trae-cn` compatibility roots); its pane is a read/edit catalog with no Runner toggle, and its caption points to the skill frontmatter's `disable-model-invocation` switch.

Settings → MCP reads the union of Claude Code, Codex, TRAE CLI, and GitHub Copilot CLI's global server entries directly from their config files, with no Runner-side server store. A runtime dropdown selects a detected, enabled agent; toggles register or unregister the named server for that agent, copying the first registered entry in agent order when turning it on. A server named `runner` is an ordinary user-configured row with the same toggle and editor as every other server. The detail modal shows each agent's native entry and any conflicting definition; its JSON/TOML editor can also translate the change to the other registered agents while preserving their unmodelled keys. Each write changes only the named entry and preserves the rest of the file, including formatting and comments; adding servers and auth flows stay with the agents' own tooling. Reads refresh on entry, Refresh, and after writes; running sessions pick up changes on their next launch. Copilot's registration is `~/.copilot/mcp-config.json` under `mcpServers`, with a `local` entry containing `command`, empty `args`, and `tools: ["*"]`; the file is created when absent. The 0.11 upgrade step consults the legacy `initializedMcpClients` set once and removes only a `runner` entry with no arguments whose command is this installation's legacy bridge executable under `<app data>/bin`; another installation's entry and a hand edit are logged and left unchanged. A config that cannot be read or parsed is left untouched for that launch and logged as deferred; a second-read race or write failure leaves the step pending for the next launch. See [#555](../features/archive/555-mcp-settings.md).

Preferences persist in `$APPDATA/ui-settings.json`, read by the app at launch; a first launch starts from defaults (resume-on-launch off).

**Updates** is the slim form of `main`'s pane: check now, the automatic-checks toggle, last-check time. Sparkle's standard user driver owns the found/download/install dialogs. Not ported from `main`: the Arc-style "New Runner version available" pill above the sidebar Settings row (hover → card, per-launch dismiss, auto-install checkbox). It needs an `SPUUpdaterDelegate` so the app learns an update was found; tracked as M6.9 in [`../impls/gpui-rewrite/m6-consolidation.md`](../impls/archive/gpui-rewrite/m6-consolidation.md).

## 4. Coordination primitives — *what flows between crew members*

Crew members don't share a programming model; they share an IM-like surface.

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

- **Broadcast** — `runner msg post "<text>"`. Goes to every other crew member's inbox; a human-authored broadcast goes to every crew member.
- **Direct** — `runner msg post --to <slot_handle> "<text>"`. Goes to that slot's inbox only.

Messages are **flat by design** — one stream per mission, no thread scoping, no fact primitive. Durable conclusions belong in project files, code, commits, or message prose. Signals are typed and small; messages are prose the router does not parse — but it does *notice* them (§8.5).

### 4.3 Inbox — *"what's in my mailbox"*

Every slot has an **inbox**: the subset of the mission's messages relevant to it. The inbox is a **projection** over the event log, not a separate data structure. For the slot with handle `h`:

```
inbox(h) = all events in the mission where
          kind = "message" AND from != h AND (to = null OR to = h)
```

`runner msg read` returns the calling slot's inbox, sorted by ULID. `--since <ts>` restricts to messages newer than a given ULID/timestamp so agents can poll without re-reading history.

**The inbox is pull-based, with a nudge.** The body of a message is only ever read when the recipient runs `msg read`. What the router does on a new message is write one line into the recipient's PTY — `[inbox] new message from @coder — run \`runner msg read\` to view.` — subject to the delivery gate in §8.5, so an agent that is mid-turn or a human who is mid-draft is not interrupted. The recipient also learns to read its inbox through the platform preamble (§6 Layer 1), which instructs every crew member to check at natural task boundaries.

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

`kind` discriminates. For `kind: "signal"`, `type` carries the verb; for `kind: "message"`, the prose lives in `payload.text`. Crew members interact through CLI verbs (`runner signal`, `runner msg`), not the event schema directly. Event-log primitives (ULIDs with a monotonic floor, the `flock` append, tail repair of a torn last line) live in `crates/runner-core`.

## 5. PTY session runtime

A **pseudo-terminal (PTY)** is a kernel-emulated terminal device. To the child it looks like a real TTY — `isatty()` is true, `ioctl(TIOCGWINSZ)` reports a window size, signals route correctly — but the other end is a file descriptor held by a controlling process. The kernel exposes the pair as a **slave** (the child's stdin/stdout/stderr) and a **master** (what Runner reads from and writes to). It's the same primitive `ssh`, `tmux`, and every terminal emulator use. Runner uses the `portable-pty` crate, which wraps the POSIX primitives; for the rigorous treatment see Stevens & Rago, *APUE*, chapter 19.

### 5.1 Topology at a glance

Output (agent → screen + idle inference):

```
   Child ──► PTY slave ──► PTY master ──► Reader thread ─┬─► SessionEvents::output
   (tty stdout                            (blocking      │     └─► TerminalBridge ─► the session's Term
    + stderr)                              OS thread)    │            (alacritty, 10,000-line scrollback)
                                                         │            └─► wake GPUI if a pane is viewing it
                                                         └─► Idle detector ─► session_status (forwarder)
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

Claude Code, Codex, TRAE CLI, GitHub Copilot CLI, and pi are TUIs. They check `isatty()`; if false they degrade. Their output is escape sequences that only a terminal emulator can render. The PTY gives the child a real terminal; `alacritty_terminal` gives Runner a correct emulator — grid, VTE parser, alt screen, scrollback reflow, selection, mouse reporting modes — without reinventing one. GPUI paints the grid it maintains.

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

The composed prompt is split into the runtime's system-prompt and first-turn channels in `router/prompt.rs`, then delivered by the adapter in `router/runtime.rs`. Claude Code, Codex, TRAE and Copilot preserve their existing byte-identical first turns: a positional argument where accepted, `-i <body>` for Copilot, or a verified paste after a Windows batch wrapper is ready; genuine resumes suppress that first turn. Pi instead receives `--append-system-prompt <app data>/session-prompts/<Runner session id>.md` on every fresh spawn and resume. Runner rewrites that file from the current role, crew and roster rows before each spawn, removes it when the session ends, and sweeps leftovers at startup. A Pi direct chat and worker have no first turn; only the lead's `== Mission ==` section follows `--` as its first turn.

Every fresh mission-slot spawn carries the cold-start composed prompt: `compose_launch_prompt` for the lead (brief, crew conventions, roster, and latest mission goal), or `compose_worker_first_turn` for a worker (coordination preamble, crew conventions, and brief). This includes Restart and a manual Resume that falls back to fresh because a Claude, Copilot or Pi conversation file is missing or Codex/TRAE has no captured key. A genuine conversation resume sends no first turn; Pi still refreshes its system-prompt file so role edits take effect. Fresh Codex/TRAE respawns receive a new capture marker; Windows batch wrappers queue the same body through the existing first-turn delivery fallback.

A mission slot has three lifecycle actions ([542](../features/archive/542-slot-restart.md)): **Stop** synchronously kills and reaps only its PTY, leaving the mission and sibling slots running; **Resume** respawns its existing row and asks the agent CLI to restore its conversation; **Restart** kills a running PTY and respawns the same row as a fresh conversation with its cold-start first turn. Restart replaces the agent key while retaining the Runner session id, router mapping, tabs, and mission event history. Resume and Restart share a per-session claim so concurrent requests cannot spawn twice or kill an in-flight respawn. Both accept current pane dimensions ahead of the persisted size, including when a stopped pane was resized. A mission slot only resumes or restarts while its mission is running and unarchived.

Restart records a human `slot_restarted` signal followed by a `runner` message asking the lead to re-send the slot's task and context. If the lead is restarted, each other slot receives the note instead. The existing message router nudges the recipients' inboxes. A notification append failure leaves the successful restart intact and emits a session warning so the human knows to re-send context. Rail controls act immediately; the stopped-slot pane offers **Resume slot** and **Restart slot** while siblings are live. When all slots are stopped, the mission-wide **Resume** and **Archive** card remains. Only header **Stop all slots** asks for confirmation and names the running slot count ("Stop all N running slots?", or "Stop the running slot?" for one).

### 5.4 Native wiring and human takeover

- The terminal element renders the session's `Term` from the registry; a pane that mounts late simply paints the grid as it is — there is no history fetch.
- Keys are encoded from the `Term`'s mode (application cursor keys, bracketed paste, Shift+Enter as `ESC CR`, ⌥ as Meta); IME composition is native, with marked text drawn in the grid; mouse reporting modes 1000/1002/1003/1006 are honored and Shift bypasses them for selection and scrollback; selection and copy are the element's own.
- **Resize** is immediate. The pane that owns the terminal's size pushes each measured size from `prepaint`; `TerminalSession::resize` resizes the `Term` (reflowing history) and the PTY ioctl fires on the same call, every frame of a drag, for every runtime. A 175 ms settle thread only persists the final size once per storm. Nothing clears the grid on resize: the TUI's own SIGWINCH repaint plus alacritty's reflow is the whole story (M6.6 — the earlier "clear and replay" contract duplicated history, because `ESC[2J` on the primary screen is `clear_viewport`, which scrolls the viewport into scrollback).

**Human takeover is a first-class capability.** At any moment the human can type directly into any session's stdin — the same writer the router uses. The pane is a real terminal, not a log viewer: special keys pass through untouched, and the agent cannot tell whether bytes came from the router, the human, or its normal terminal input.

The mission feed is read-mostly: it renders coordination events and historical human-authored events, has no free-form composer, and its only input is the choice control on a pending `human_question` card. External orchestrators use the `runner` CLI to post `human_said` when they need to relay an operator instruction programmatically.

### 5.5 Sessions outlive views; terminals outlive panes

Sessions live in the core and belong to the mission, not to any window, tab or pane. Closing a window does *not* kill the sessions — the agents keep running, events keep flowing, the router keeps handling live signals.

Since M6.8 the same is true of the screen. `TerminalBridge` holds a strong `Arc<TerminalSession>` per live session: created on `session/spawned` (with a first-output fallback as an ordering safety net), fed from the first byte, released on `session/exit` and `session/archived`, replaced when a resume or reset spawns a new child under the same id. Panes take a *viewer lease* on the terminal they show; a hidden terminal keeps ingesting but does not wake GPUI. Tab switches, route changes and re-opened panes re-render an existing grid instead of rebuilding one. Consequence, recorded as a deviation from `main`: a stopped pane shows the Ended/Resume card over a neutral background, not the final screen; flip the release point from exit to archive if that is ever missed.

**Rows persist across app restart; PTY children do not.** On quit, `stop_running_sessions_on_quit` kills every process group (SIGHUP, then SIGKILL) and joins the forwarders; the startup orphan sweep is the crash fallback and a failing sweep is fatal at boot. On next launch Runner re-mounts router/bus state for `running` missions, replays the logs, demotes stale `running` session rows to `stopped`, and re-spawns sessions flagged `resume_on_launch`. Resume spawns a fresh PTY against the same session row; for claude-code/codex/trae/copilot/pi, `agent_session_key` lets the agent CLI continue its own conversation when supported.

Copilot assigns a UUID before spawning and persists it as `agent_session_key`, then passes `--session-id <uuid>` for both fresh starts and resumes. The conversation probe checks `$COPILOT_HOME` (otherwise `~/.copilot`), `session-state/<uuid>/events.jsonl`; a missing transcript starts fresh with the same id and includes the cold-start first turn. Before every Copilot spawn, Runner seeds the exact cwd in `config.json` `trustedFolders`, keeping the leading `//` header and unrelated values; `--yolo` alone does not bypass folder trust. Default and Accept edits role permission modes respectively write no permission flag and `--allow-tool=write`; Auto is not offered. Chats strip all Copilot permission flags.

Pi also assigns and persists a UUID before the first process spawn, then passes `--session-id <uuid>` on fresh and resumed sessions. Its conversation probe mirrors the child's effective environment: the role overrides Runner's inherited process environment, `PI_CODING_AGENT_SESSION_DIR` selects one flat directory, `PI_CODING_AGENT_DIR` selects `sessions/<slug>/` beneath the agent directory, and the fallback is `~/.pi/agent/sessions/<slug>/`; every layout names files `<ISO timestamp with colons and periods replaced by hyphens>_<uuid>.jsonl`. `sessionDir` in `settings.json` and a role's own `--session-dir` argument are not honoured. A missing file starts fresh under the same id, with only a lead receiving its goal again. The refreshed `--append-system-prompt` file applies current role edits even to a genuine resume.

Fork creates a new direct-chat row without writing the source row, key, PTY, or conversation file. The new row copies the source's project, role, cwd, runtime, command, model, and effort columns. Claude Code starts the visible TUI directly with `--resume <source> --fork-session --session-id <new>` and a caller-assigned key; Pi uses `--fork <source> --session-id <new>` with the fork row's own refreshed system-prompt file; Codex runs a bounded headless `exec fork` until it has a lineage-validated `thread.started` key, persists that key, marks the temporary row stopped, and starts the visible PTY through the ordinary resume path. A direct-spawn, materialization, or resume failure removes both the fork row and its tab. The runtime definition's `native_fork` capability enables Claude Code, Codex and Pi and excludes TRAE and Copilot. An untouched Claude fork is intentionally copy-on-write: manual resume falls back to a fresh chat and launch-time resume reports it unavailable; re-fork the source instead of adding recovery state.

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

The baseline detector infers activity from PTY byte traffic ([#124](../features/archive/13-pty-silence-idle-detection.md)). It is a heuristic: output silence cannot distinguish thinking from readiness inside an interactive agent, and foreground process ownership has the same limitation. Codex adds a narrow title hint inside this baseline: the backend PTY reader parses OSC titles for every Codex session, including a mission slot with no open pane, and a recognized Codex activity title takes precedence over bytes while the fallback owns the session.

The current sources and precedence are:

1. **`forwarder`.** `IdleDetector` reports Busy on PTY output and Idle after 2 s of silence. A 500 ms grace window re-armed on resize prevents resize output from waking an idle session. On Codex, two distinct frames from its ten-frame leading activity spinner confirm the title format; a matching title with that leading activity item removed is an Idle hint. The recorded thread-renaming suffix may keep an already established Idle hint through its own spinner and generated-name update, but a bare/custom/reset title, a status word, or a spinner embedded in unrelated text cannot establish Idle. Unknown formats immediately return to byte activity. A successful local, pasted, routed or automatic submission invalidates the hint, and a new PTY starts without title state. `CodexStartup` remains the separate untouched-launch readiness mechanism from [#687](../tests/687-codex-pre-hook-idle.md). An active hook adapter owns activity and makes `note_forwarder_transition` reject all forwarder status transitions. Without hook ownership, it rejects forwarder Busy while `suppress_local_input_busy` is set.
2. **`input-submit`.** Input submission can mark the session Busy and is not blocked by the forwarder guards. The sink clears local-input suppression on an accepted Idle report and deduplicates unchanged activity. This is a remaining status writer, not a complete lifecycle precedence model. `InputTracker` supplies input observations and influences local-input suppression; it does not own Busy/Idle.

Mission sessions and direct chats are both seeded Busy with source `spawn` at spawn, so a slot reads Working · estimated from its first byte. After that seed, the output forwarder appends mission `session_status` transitions with their source to the mission log and the router updates its status projection. Direct chats stay off-bus: SessionManager stores their latest activity and emits `session/status` to every window; sidebar aggregation drives activity and completion/unread indicators. False Busy can suppress idle-gated inbox reconciliation, while false Idle can permit a nudge during a turn. Shells still use byte activity in v0.8.9, so silent commands can incorrectly read Idle.

Claude Code, Codex, GitHub Copilot CLI, and Pi have lifecycle hook adapters on macOS and Windows ([#610](../features/archive/610-windows-hook-status.md), [#539](../features/archive/539-pi-runtime.md)). Hook reporters receive the status feed path with `/` separators on both platforms. On Windows, Claude Code runs the same `sh` reporter under its own Git Bash; Codex and Copilot run one PowerShell reporter that copies the payload as raw bytes and serializes its record under a per-feed mutex (Codex as a per-session `<feed>.ps1` run through `ScriptBlock::Create`, which keeps npm-shim command lines under cmd.exe's limit; Copilot inline in the plugin's `powershell` slot). Pi's `-e` extension reports directly from Node on both platforms, so it needs neither shell reporter. A hook environment without its required reporter writes nothing and keeps the baseline; TRAE remains on estimated byte activity. Before the first accepted hook, when hooks are unavailable, and after an explicit bridge failure, the Codex title hint can refine that baseline except while #687's `CodexStartup` owns the untouched prompt or protected automatic first turn. The first accepted hook clears title authority; neither quiet output nor missing later hook events returns a healthy hook-owned session to fallback. Bridge failure clears hook ownership and requires fresh post-failure title evidence.

The adapter contract is [#347, hook-based agent status](../features/archive/347-hook-based-session-status.md): supported Claude Code, Codex, Copilot, and Pi lifecycle adapters own activity, while output silence never overrides a healthy hook-driven turn. Only unresolved surfaced approvals or questions hold crew delivery; Working and Idle do not. Title-derived transitions keep source `forwarder`, observation source `baseline`, and the estimated UI vocabulary; they emit no completion outcome, interaction hold, draft state or delivery side effect. Byte activity remains unchanged for unsupported runtimes, unrecognized/customized titles, and bridge loss until a fresh supported Codex title is observed. The accepted Codex grammar is pinned to the upstream [default activity title and ten frames](https://github.com/openai/codex/blob/7f01a84effccef40d4726c3ca12e6c839ec98d7a/codex-rs/tui/src/chatwidget/status_surfaces.rs#L26-L36), [active-progress gating and animation](https://github.com/openai/codex/blob/7f01a84effccef40d4726c3ca12e6c839ec98d7a/codex-rs/tui/src/chatwidget/status_surfaces.rs#L1009-L1068), [thread-title progress spinner](https://github.com/openai/codex/blob/7f01a84effccef40d4726c3ca12e6c839ec98d7a/codex-rs/tui/src/chatwidget/thread_title_status.rs#L32-L49), and [OSC 0 writer](https://github.com/openai/codex/blob/7f01a84effccef40d4726c3ca12e6c839ec98d7a/codex-rs/tui/src/terminal_title.rs#L46-L90); `runner-terminal/fixtures/codex-title-working.ndjson` records the corresponding bytes and is the evidence for the literal `renaming... ` progress prefix. Claude's retained fixture remains regression evidence for byte activity, but no Claude title shape is accepted without an inspectable upstream producer. [#586](../features/586-shell-status-detection.md) separately tries process detection for shell command lifetime before optional semantic shell integration. [#587](../features/archive/587-terminal-provided-titles.md) displays terminal-provided titles independently of status and routing; `session/title.rs::provider_title` remains display-only.

## 6. System prompt composition

Every spawned session receives a composed prompt — different shape for workers, the lead, and direct chats. The composition and its split across system-prompt and first-turn channels are mechanical: pure functions over slot + crew + mission inputs, no LLM in the loop. Source of truth lives in `crates/runner-backend/src/router/prompt.rs`; delivery mechanics in `router/runtime.rs`.

### 6.1 The three layers

1. **Layer 1 — platform preamble** (code-owned). For workers: a fixed block describing the `runner` CLI verbs and the inbox convention. For the lead: the launch prompt composed before session registration (§6.3).
2. **Layer 2 — crew team conventions** (`crews.system_prompt_addendum`, optional). Spliced under `== Team conventions ==`.
3. **Layer 3 — role prompt** (`roles.system_prompt`). Spliced under `== Your brief ==`.

### 6.2 What each session sees

| Session kind | Layer 1 | Layer 2 | Layer 3 | Delivery |
|---|:---:|:---:|:---:|---|
| Mission worker | preamble | if set | persona | established runtimes: first turn; Pi: system-prompt file, no first turn |
| Mission lead | coordination + mission goal | if set | persona | established runtimes: first turn; Pi: all except `== Mission ==` in the system-prompt file, mission section as first turn |
| Direct chat | — | — | persona | established runtimes: first turn; Pi: system-prompt file, no first turn |

Direct chats see *only* Layer 3 — the worker preamble's verbs and the team conventions don't make sense off-bus.

### 6.3 The lead's launch prompt

Before registering mission sessions, MissionManager composes the lead's launch-prompt body — identity, the mission goal (`missions.goal_override` or `crews.goal`), the roster, the addendum, the known signal types from `runner_core::model::KnownSignalType`, and a reminder of the lead's job. The prompt is passed into the runtime adapter at spawn, so the opening `mission_goal` event remains a durable feed record but has no stdin-injection side effect. Pi's split makes the mission section the lead's only first turn; the rest is refreshed in its system-prompt file on later spawns.

## 7. Coordination bus

### 7.1 Transport

```
$APPDATA/crews/{crew_id}/missions/{mission_id}/events.ndjson
```

One line per event, append-only, one file per mission. Debuggable (`tail -f | jq .`), crash-durable, atomic under explicit guards, replayable for projections.

#### 7.1.1 Concurrent-write correctness

Multiple crew members can invoke `runner signal` / `runner msg` at the same time from different PTYs, and the core writes router-generated events (`human_question`, `mission_warning`, `session_status`) to the same file:

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

On router boot: open the mission's file, fold `human_question` / `human_response` and `session_status` rows into in-memory state, record the replay high-water mark, then tail from the current end. Replay rebuilds projections; it never re-runs historical stdin pushes or inbox nudges.

## 8. Signal router

The router is a flat dispatcher, not a policy engine. There is no per-crew `{when, do}` rule list. The lead owns coordination judgment; the router owns parent-process plumbing that a child PTY cannot do itself.

Stdin pushes are deliberately silent: the router writes bytes into the target PTY but does not synthesize `stdin_injected` audit events. The event log records the signal or message that caused the push, plus `human_question` / `human_response` for HITL cards and `mission_warning` when a delivery cannot happen.

### 8.1 Fixed handler table

| Event | Fixed handler |
|---|---|
| `mission_goal` | No runtime side effect; the launch prompt was composed before spawn and this event remains the durable goal record. |
| `human_said` | Inject MCP-provided `payload.text` to `payload.target` if present, otherwise to the lead. |
| `ask_lead` | Inject the worker's `{ question, context }` to the lead. |
| `ask_human` | Append a `human_question` event for the UI. |
| `human_response` | Look up the matching `question_id` and inject the answer to the session that emitted the original `ask_human`. |
| `session_status` | Update the latest-status map from `payload.state`; `runner_status`, the row's name before #632, is read the same way. If a non-lead reports `idle`, inject a short availability update to the lead. |
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

The mission feed answers what is happening across the crew; the selected terminal pane is where the operator talks to a crew member. Crew members cannot address a virtual `human` message recipient: `runner msg post --to human` fails with guidance to answer in TUI output. The feed keeps its render paths for historical `human_said`, `human_response`, and messages addressed to `human`, so old logs replay unchanged.

### 8.5 Who does delivery, and when

| | Sender addresses recipient? | What the router does | When |
|---|:---:|---|---|
| Signal | No — fixed handler decides | Injects the handler's text | Through the delivery gate |
| Broadcast message | No | One-line inbox nudge to every other slot | Through the delivery gate |
| Direct message | Yes (`--to`) | One-line inbox nudge to that slot | Through the delivery gate |

Message *bodies* are never pushed; recipients read them with `msg read`. What the router pushes is the wake-up line, and every push — handler text or nudge — goes through the per-slot **delivery gate and outbox** in `router/mod.rs`:

- **Draft-aware.** If the human is typing in that slot's pane (today a byte-level latch: printable input and pastes set it, Enter/Ctrl-C clear it, with no time bound — the 10-minute abandonment backstop in impl 0041 was never implemented, so only Enter, Ctrl-C, respawn or exit release it; M6.1 replaces the latch with an observed input state), the delivery waits in the outbox and the pane shows a "delivery waiting" pill (`router/delivery-blocked`).
- **Turn-boundary.** Deliveries are spaced by an 80 ms cooldown and a 30 s reconciliation re-nudge covers a nudge that landed while the agent was mid-tool-call; latest-wins absorbs bursts. The planned hook adapter in [#347](../features/archive/347-hook-based-session-status.md) will supply verified turn boundaries; v0.8.9 still uses inferred idle.
- **Queued until resume.** A delivery to a stopped or resuming slot is queued, announced once per outbox with a `mission_warning` ("queued until the session resumes"), flushed on `Respawned`, and dropped with a second warning if the session exits for good. Nothing is silently lost and nothing hard-fails (M6.8).
- **Never on replay.** Events at or below the replay high-water mark are not re-dispatched, so a restart does not re-nudge anyone with mail they already saw.

## 9. The `runner` CLI

The bundled CLI is Runner's external command surface for people, scripts, direct chats, and mission sessions. It connects to the app's local MCP socket for workspace and lifecycle operations; the socket protocol is an implementation detail. Mission sessions continue to use the same binary for direct event-log messaging.

### 9.1 Surface

```
# meta
runner status
runner help [agents | <noun>]
runner call <tool> [<json>]

# projects and roles
runner project list | show <project> | create <name> [--path <dir>] | rename <project> <name> | delete <project> [--force]
runner role list | show <handle>
runner role create <handle> --runtime <runtime> [--name <display name>] [--model <model>] [--effort <effort>]
                   [--permission <mode>] [--prompt <text> | --prompt-file <path | ->]
                   [--arg <arg>]... [--env KEY=VALUE]... [--cwd <dir>]
runner role update <handle> [the same optional flags] | delete <handle>

# crews; slots are addressed by handle
runner crew list | show <crew>
runner crew create <name> [--purpose <text>] [--goal <text>] [--conventions-file <path | ->]
runner crew update <crew> [--name <name>] [--purpose <text>] [--goal <text>] [--conventions-file <path | ->]
runner crew delete <crew>
runner crew add <crew> <role> [--as <handle>] [--runtime <runtime>] [--model <model>] [--effort <effort>]
runner crew set <crew> <handle> [--as <new handle>] [--runtime <runtime>] [--model <model>] [--effort <effort>]
runner crew remove <crew> <handle> | lead <crew> <handle> | order <crew> <handle> <handle>...

# missions, chats, and sessions
runner mission list [--crew <crew>] | show [<mission>]
runner mission start --crew <crew> [--goal <text> | --goal-file <path | ->] [--title <title>]
                     [--project <project> | --cwd <dir>]
runner mission stop | resume | archive | unarchive | pin | unpin [<mission>]
runner mission rename <mission> <title> | move [<mission>] (--project <project> | --unfile)
runner mission feed [<mission>] [--follow] [--since <offset>] [--limit <n>] [--oldest-first]
                    [--types <kind,...> | --all] [--from <handle>]
runner mission answer <mission> <question_id> <choice>
runner chat start (<role> | --runtime <runtime>) [--model <model>] [--effort <effort>] [--project <project> | --cwd <dir>]
runner session list | show <session> | stop <session> | archive <session> | resume <session> | restart <session>

# mission-scoped
runner msg post [--mission <mission>] [--as <handle>] [--to <handle>] <text>
runner msg read [--since <ulid>] [--from <handle>]
runner signal <type> [--mission <mission>] [--as <handle>] [--payload <json>]
runner ask <question> [--context <text>] [--mission <mission>] [--as <handle>]
runner ask --human <prompt> --choices <a,b,...> [--mission <mission>] [--as <handle>]
```

Every command accepts `--json` for JSON output and `-q` for result ids. stdout carries data; stderr carries diagnostics. Exit 0 is success, 1 means the tool refused the operation, 2 is usage or reference resolution, 3 means the app is not running, and 5 means the connection to the socket was denied although the socket exists, which is a command sandbox such as Codex's default one: the caller runs the command again outside the sandbox. 4 is reserved for #562. `spawn`, `ps`, `wait`, `stop <handle>`, and `done` are reserved for #562.

### 9.2 References and defaults

Roles resolve by their unique handle. Crews and projects resolve by id or exact name; an ambiguous name exits 2 and prints the matching ids. Missions and sessions resolve by id or unique id prefix. `session_list` contains direct chats only, so a mission session can be resumed or restarted only by its full 26-character id, which passes through for backend validation. Active mission lookup comes from `mission_list`; because archived missions are absent from that tool, any full 26-character mission id is validated with `mission_get` and then passed to the requested tool. An active-only operation on an archived mission is therefore a tool refusal (exit 1), while a missing name, prefix or full mission id remains an unresolvable reference (exit 2).

`mission start`, `chat start`, and `project create` default to the shell's current directory exactly unless a project or explicit directory is supplied. Relative paths are joined to that directory and normalized lexically by removing `.` and resolving `..`, without filesystem canonicalization. Long text accepts an inline flag or a file, and `-` reads stdin. On update/set commands, an empty optional value clears the field.

Default output is command-aware rather than a generic JSON projection. List commands expose only their identifying and operational columns; show commands use key-value blocks plus noun-specific sections; mission feed emits one chronological line per event. Table cells collapse whitespace, truncate at a fixed width with an ellipsis, and render null as `-`. `--json` preserves the tool's JSON text verbatim and `-q` prints ids only.

`mission feed --follow` prints the requested window, then polls `mission_feed` from its last `next_offset` every 3 seconds in oldest-first order. Three seconds reduces quiet polling sixfold from 500 ms while keeping questions and lifecycle changes within the next poll, plus IPC time. Polling and notification batching are separate: each event is flushed immediately, and pages through the snapshot's last event offset are drained without an extra poll delay. Each event is printed once. The 1 ms test-build interval seam remains.

Each poll first reads `mission_status`, replacing the old archive check every fourth poll (2 seconds then, 12 seconds at the new interval). This existing snapshot includes mission lifecycle, session rows and a last-event offset; it does scan the mission log, but avoids a new transport or a new backend contract. Taking the snapshot before reading the feed lets the follower drain final events before exiting. Ctrl-C, archive, completed mission state and all sessions stopped exit successfully. An aborted mission or terminal crashed sessions exit 1; a partial session crash is reported immediately while watching the surviving sessions. Empty startup rosters and busy/idle activity do not end the watch. `mission stop` leaves the mission row running for later resume, so the follower uses session lifecycle too. A crew message saying “done” is surfaced as a message, not interpreted as a lifecycle transition. Resume requires a new watcher.

Every stopped/crashed session transition and successful watch end is reported on stderr, independently of feed filters; session notices use roster handles when available. App disconnect exits 3. A watch request timing out after 30 seconds exits 1 and explicitly says Runner may still be running; the timeout is independent of the 3-second polling cadence so an ordinary slow reply does not end a watch. A missing mission or non-advancing feed cursor also exits 1. Errors identify the mission and last consumed cursor with show/re-follow arguments for the same Runner executable, preserving the development/installed app boundary. There is no silent retry. Host watch expiry is handled by the agent's watch rule, since the host can kill the CLI before it can report. Watch facilities must deliver stderr and process exit as well as stdout; Claude Code Monitor needs `2>&1` because only stdout produces notifications. That merged host stream can include plain-text diagnostics; the CLI's own stdout remains event NDJSON.

`--types` and `--from` filter client-side. Human feed output and every follow stream hide routine `session_status`/legacy `runner_status` and `inbox_read` unless `--all` is present or `--types` explicitly names them; status payloads with `status.lifecycle = "error"` or `status.observation.outcome = "failed"` remain visible by default. A failed outcome persists across later busy/idle rows, so each distinct failure-bearing event remains visible until the outcome clears; event-ID deduplication still applies. One-shot `--json` without a filter remains the exact tool result; follow JSON is one flushed event per NDJSON line and never prints the cursor. `--follow --limit 0` is rejected because it cannot deliver events or advance through a backlog.

`runner help agents` is compiled into the CLI and therefore matches its installed version. It summarizes the command tree, references, output modes, exit codes, identity rule, and mission flow. Starting delegated work requires arming exactly one ID-bound watcher that delivers events, errors and exit to the initiating agent during later work and while idle; merely redirecting to an unread log or waiting only for completion is insufficient. The CLI start command itself remains one-shot, with unchanged ID/JSON output and no new flag. If the host cannot deliver events, the agent must say automatic watching is unavailable and give the foreground follow command. Host-specific claims are limited to the installed documentation recorded in [#686's smoke plan](../tests/686-watch-cli-started-missions.md).

### 9.3 Two modes and one identity rule

Inside a mission, `RUNNER_CREW_ID`, `RUNNER_MISSION_ID`, `RUNNER_HANDLE`, and `RUNNER_EVENT_LOG` identify the caller. `msg post`, `msg read`, `signal`, and `ask` append or read the event log directly and do not start a Tokio runtime or connect to the app. Other commands use the socket, and mission commands with no explicit target default to the caller's mission. An explicit different mission uses the outside path.

Outside a mission, mission-scoped writes require `--mission`; `msg read` is inside-only and points outside callers to `mission feed`. The caller acts for the user, so posts and answers with no handle appear as the person, represented as `human`. `--as <handle>` becomes the socket tool's `from` after roster validation, but must only name a slot the caller holds, never another agent's slot; #562 adds the seats an outside agent can take. `ask` requires `--as` outside; `mission answer` is always the person's verb and posts `human_response` without a handle.

### 9.4 Direct chats

Direct chats remain off the mission bus, so they have no implicit mission, mission identity, event log, router, or inbox. Agent direct chats export `RUNNER_HANDLE` as their process label, but the CLI treats that handle-only environment as off-bus; it does not turn the label into a caller identity. Every chat, terminal, resume, and fork gets `<app data>/bin` first on PATH, so bare `runner` is the sidecar belonging to the app that spawned it even if the runtime loads a skill installed by another build; a mission slot's identity shim remains ahead of that folder. PATH does not select or suppress skills. The socket endpoint remains selected at compile time, with no environment override. Workspace commands go through the socket, mission-scoped writes name `--mission`, calls without `--as` act for the user, and `msg read` is unavailable. Until #562 adds outside seats, direct chats and terminals must not use `--as` to speak as a mission slot.

### 9.5 Socket transport

Each outside command opens `$APPDATA/mcp.sock`, bounds connection establishment to 500 ms and the MCP handshake separately to 3 s, performs the reference-list calls and one requested operation, prints the result, and exits. The backend registry and the tools reached by the CLI's exhaustive recorder test assert against one shared list of tool names. The stdio bridge is gone; the once-only upgrade step removes only the exact bridge registrations this installation wrote, leaves mismatched entries unchanged as final skips, and defers unreadable, unparseable, or concurrently changed configs for retry without changing them.

### 9.6 Agent discovery skill

Runner embeds a compact `SKILL.md` that points agents to `runner help agents`. Release builds install `runner` and prefer bare `runner` on PATH. Development builds install a separate `runner-dev` skill with a development-only frontmatter trigger and always invoke its quoted absolute sidecar, including for `help agents`, so it neither competes with the release skill nor follows a production `runner` from another PATH.

Both generated skills require start-and-watch without a separate user request, one watcher per exact returned mission ID, immediate handoffs/questions/failure reporting, and cleanup at lifecycle end. The development skill uses its quoted absolute sidecar for both start and feed. On an expired/broken watch, the agent reports the gap, checks mission state, and re-arms one watcher with the recovery cursor when possible; otherwise it catches up and deduplicates event IDs. Host limitations must be reported honestly. Claude Code Monitor is documented to deliver lines; the verified Codex and Copilot help does not establish idle delivery, and pi needs an already available forwarding extension. These are workflow instructions, not host configuration changes. Updated skills reach real agent homes only through an app build and the normal app-managed installation path.

The three user roots are `~/.claude/skills`, `~/.agents/skills` (shared by Codex, Copilot and pi), and `~/.trae/skills`. The `runnerSkillEnabled` switch in Settings → General is the truth and defaults on. When on, launch and every runtime-discovery refresh install or refresh the skill in each root read by a detected runtime; the Settings → Agents enable switch is deliberately irrelevant, and a marked folder deleted by hand returns. When off, Runner removes every owned folder from all three roots and does not reinstall it. A root no detected runtime reads is left alone.

Each Runner-owned folder has `.runner-managed`. Runner rewrites only absent or marked folders and never touches a foreign folder of the same name. A foreign folder is reported beneath the General switch; a healthy enabled switch and a disabled switch have no status line. Settings → Skills continues to show the folder in each runtime's ordinary catalog, with a `Managed by Runner` badge when the marker is present. The older `initializedSkillRoots` setting remains decodable but is no longer consulted.

### 9.7 Command installation

The command installed for a person's terminal is a link to the sidecar in `<app data>/bin`, never the app executable. Release builds call it `runner`; macOS development builds call it `runner-dev`. Runner owns a macOS command only when it is a symlink whose target is this build's sidecar. A regular file, a directory, or a symlink to anything else is foreign and is never replaced or removed. Status distinguishes not installed, installed at a path, foreign at a path, and an owned install shadowed by another executable earlier on the login PATH.

After login-shell discovery succeeds, the first-launch default chooses only `~/.local/bin` when that exact directory is on the login PATH, otherwise `/usr/local/bin` when it is on the login PATH and writable without escalation. It does nothing and records nothing when neither qualifies, so a later runtime refresh can try again. The default never prompts. Once it installs, finds an owned link, or finds a foreign entry, `initializedCommandInstall` records the decision. Explicit Install and Uninstall record it before acting, so a link removed by the user is not resurrected at the next launch.

Explicit Install uses the default target when one exists. Otherwise it tries `/usr/local/bin/<name>` directly, then runs `/usr/bin/osascript` with administrator privileges; cancellation is a normal unchanged outcome. Uninstall removes only an owned link and uses the same escalation path when a direct removal is not permitted. Settings → General and the command palette share these actions, and `runner status` reports the resulting state. That status check ignores the sidecar and mission-shim PATH entries Runner injects into its own sessions, so they cannot falsely shadow the user install.

On Windows, release builds add exactly the sidecar directory to `HKCU\Environment\Path`, preserving `REG_SZ` versus `REG_EXPAND_SZ`, then broadcast `WM_SETTINGCHANGE` for `Environment`. Comparison is case-insensitive and ignores a trailing backslash. Both Settings and `runner status` read that registry value as the ownership source; the current process PATH is used only to detect shadowing. Uninstall removes exactly that entry and broadcasts again. A Windows debug build does not install a command because adding its directory could only expose `runner.exe`, not a separate `runner-dev.exe`.

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

roles (
  id TEXT PRIMARY KEY,
  handle TEXT NOT NULL UNIQUE,        -- globally unique slug; §3.2
  display_name TEXT NOT NULL,
  runtime TEXT NOT NULL,              -- claude-code | codex | trae | copilot | pi | shell (qoder: legacy rows only)
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
  role_id TEXT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
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
  role_id TEXT REFERENCES roles(id) ON DELETE CASCADE,
  slot_id TEXT,
  cwd TEXT,
  status TEXT NOT NULL,               -- running | stopped | crashed
  pid INTEGER,
  started_at TEXT, stopped_at TEXT,
  runtime TEXT, runtime_socket TEXT, runtime_session TEXT,      -- live handle metadata
  runtime_window TEXT, runtime_pane TEXT, runtime_cursor INTEGER, -- (legacy tmux columns, unused)
  agent_session_key TEXT,             -- the agent CLI's own conversation id, for Resume
  agent_runtime TEXT, agent_command TEXT, agent_model TEXT, agent_effort TEXT,
  live_title TEXT,                    -- the agent's own conversation title, when it reports one
  last_cols INTEGER, last_rows INTEGER,   -- last applied PTY size (§5.8)
  resume_on_launch INTEGER NOT NULL DEFAULT 0,
  archived_at TEXT, title TEXT, pinned_at TEXT
);

session_attention (
  session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
  unread_since INTEGER,               -- ms epoch: finished while its tab was not viewed
  error_acknowledged_at TEXT
);
```

Migrations live in `crates/runner-backend/migrations/` (`0001_init.sql` … `0023_roles.sql`) and are forward-only: an older Runner cannot open a database a newer one has migrated, which the 0023 table rename accepted deliberately. Two rules from that rename hold for every table. Every SQL statement that names a table lives behind a `repo/` function, so `ops/`, `session/` and `mcp/` speak only in domain terms; the gate is a grep for the table and column names inside SQL strings, which must hit only `repo/` and the migrations. A persisted value whose spelling changes is written in the new form and read in both, with nothing frozen under the old name for compatibility's sake: `role-default` reads `runner-default`, the start-chat mode `role` reads `runner`, the saved route `/roles` reads `/runners`.

### 10.2 Filesystem

```
~/Library/Application Support/com.wycstudios.runner/      ($APPDATA; debug builds: …runner-dev)
├── runner.db                               # SQLite (WAL)
├── ui-settings.json                        # preferences (§3.7)
├── mcp.sock                                # MCP server socket while the app runs
├── bin/
│   └── runner                              # general CLI + direct mission-bus verbs
└── crews/{crew_id}/missions/{mission_id}/
    └── events.ndjson                       # per-mission event log (+ roster sidecar)

~/Library/Logs/com.wycstudios.runner/runner.log   # rotating app log + panic backtraces
```

The data directory has been the same since the Tauri app, so every upgrade finds its roles, crews, missions and sessions in place. Startup removes the stale legacy bridge executable from `bin/` best-effort. Direct chats are off-disk beyond their row in `sessions`; mission sessions share their mission's directory and the only durable artifact is `events.ndjson`. Screen state lives in memory (§5.8).

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
2. **Slot is the indirection** that lets one role participate in many crews and direct chats without duplication.
3. **PTY in-process via `portable-pty`, not pipes, not tmux.** TUI fidelity is non-negotiable.
4. **NDJSON file per mission, not a broker.** Debuggable and crash-durable.
5. **One CLI for people, scripts, and agents.** The local MCP socket is its internal transport.
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

Release-channel contract with one nightly release and shared app identity (#504/#505, following #502); the first live cut ran on 2026-09-08 and the old Windows release is gone:

- **`main`** is the native app and the only line of work; the Tauri + React line ended at `276a3a4` and its last release, `v0.5.2`, bridges into `v0.6.0` on its next update check. Work lands as a task branch → PR → the one required check (`Rust / macOS`) → merge → a docs landing commit. The human smoke-tests before the PR; crews do not launch the app.
- **Versions.** Nightly is a rolling development channel, shown as `Nightly (<short-sha>)`; it is independent of an official release number. Official releases alone get `vX.Y.Z` version tags. `CFBundleVersion` is the UTC build stamp `YYYYMMDD.HHMM` on both macOS channels, which Sparkle compares; `CFBundleShortVersionString` is the short commit on nightly and `X.Y.Z` on production. Sparkle supplies the `Runner` app name in its native alert; Runner’s own update displays format the commit as `Nightly (<sha>)`. Windows installers use `nightly.<sha>.<stamp>` and update by the trailing stamp, while the app and update offer show `Nightly (<sha>)`. The three lockstep crate versions remain ordinary package metadata (`0.8.2` at #502's baseline); nightlies do not bump them or require a `-nightly` suffix. `runner-core` and the CLI remain independently versioned. Unstamped `make run` builds show `<crate-version> (dev)`.
- **Nightly workflow.** `gh workflow run nightly.yml --ref main` is dispatch-only and defaults to `platform=both`; `-f platform=macos` or `-f platform=windows` selects one platform. `prepare` computes one UTC stamp, commit-based nightly identity, and dispatch SHA without consulting the official crate version. Both selected builds check out that SHA and pass artifacts from the current run to one `publish` job. One `nightly` concurrency group cancels the whole older run. Publishing requires every selected build to succeed, tolerates the unselected build being skipped, and waits for successful CI on the same SHA once. A failed or cancelled selected build, or missing/failed CI, prevents publication.
- **Nightly channels.** One public `nightly` prerelease serves both platforms and is excluded from latest stable. macOS publishes the signed/notarized `Runner-Nightly-<sha>.<stamp>-arm64.dmg`, then a signed `appcast.xml` with an Apple Silicon requirement. The `com.wycstudios.runner` bundle (`Runner.app`) reads `releases/download/nightly/appcast.xml` and offers updates through Sparkle. Windows reads `releases/tags/nightly` and selects the newest `Runner-Setup-nightly.<sha>.<stamp>-x64.exe` with its matching `.sig`, ignoring macOS assets. One combined notes file, `script/nightly-release-notes.md`, covers both platforms. The workflow verifies release flags and every selected asset’s anonymous download before pruning each platform independently to two builds by stamp; Windows installers and signatures are deleted together, and the appcast’s DMG is retained. Single-platform cuts leave the other platform’s assets and feed alone. After the CI gate passes, `publish` force-moves the annotated `nightly` tag to the built commit and writes the release notes: the commits since the latest official `v*` tag reachable from the commit (capped at one hundred, merges omitted) above the fixed install text in `script/nightly-release-notes.md`, so the list reads as what the nightly has beyond stable and resets at each release. `git checkout nightly` is therefore the shipped nightly, and the release page shows its commit. Uploads are not transactional: failure or cancellation after an upload means an incomplete cut, and upload/download failures prevent all pruning. The `/nightly run [both|macos|windows]` and `/nightly check [both|macos|windows]` skill paths use this contract.
- **Production channel.** Tag `vX.Y.Z` on a commit whose crate version is exactly `X.Y.Z` → `release.yml` builds the same ARM64 bundle for `com.wycstudios.runner` (`SUFeedURL` `releases/latest/download/appcast.xml`), generates a one-item signed appcast with an Apple Silicon hardware requirement, and attaches everything to a **draft** release; publishing is the human's switch, because it moves the `releases/latest` alias. `workflow_dispatch` with `dry_run` builds the same artifacts into a throwaway draft. One Sparkle EdDSA keypair (`packaging/sparkle-public-key`) serves both feeds. Intel packaging is not planned (§2); an Intel install left on the universal `v0.6.0`/`v0.6.1` stays there, correctly withheld by the appcast's Apple Silicon hardware requirement.
- **Tauri bridge, closed.** `v0.6.0` carried the Tauri updater files so a dormant 0.5.x install could hop to Sparkle; the hard cutoff at 0.7.0 has passed, and a 0.5.x install still dormant installs the DMG by hand.
- **Windows integration.** Both platforms develop from `main`, with CI on PRs and main pushes. `release.yml` computes one identity, builds the macOS artifacts and Windows x64 installer separately, and creates one draft only after both builds and CI pass. Windows binaries and installers are Authenticode-signed through Certum SimplySign in CI and carry minisign update signatures; see [Code signing](./windows.md#code-signing). `bundle-windows.ps1 -Channel production` bakes the stable release channel and base display version; the default nightly channel uses `nightly` and displays `Nightly (<sha>)`. The updater compares the trailing UTC stamp, including older bare-version/`X.Y.Z-nightly` installers and new commit-based nightlies. Both channels retain `%APPDATA%\com.wycstudios.runner`; see [Windows update behavior](./windows.md#update-behavior).
- **Isolation.** Nightly and production use the same macOS bundle id and `Runner.app` name, retaining the data directory in §10.2. Installing a nightly over Runner switches that install to the nightly feed; installing a stable DMG switches it back. Production Sparkle reads only `releases/latest/download/appcast.xml`; nightly Sparkle reads `releases/download/nightly/appcast.xml`. The feed address provides channel isolation. Public nightlies are discoverable on the releases page, but prereleases never become latest stable. Updates require the user’s install action; dispatch can lead to a nightly update offer and Windows background download, but does not itself restart the app.
- **One-time transition.** The `nightly-win` release and tag no longer exist, so a dormant Windows install that missed the hop reinstalls from `nightly`; the workflow never read or wrote the old release. On macOS the first unified DMG goes over `Runner.app` by hand and `Runner Nightly.app` is deleted, because Sparkle cannot install across bundle identifiers.

History of how this was decided: [`../impls/archive/gpui-rewrite/README.md`](../impls/archive/gpui-rewrite/README.md) (condensed) and [`../impls/archive/gpui-rewrite/plan.md`](../impls/archive/gpui-rewrite/plan.md) §Release channels (full); the M6 remainder queue, all landed by 2026-08-27: [`../impls/archive/gpui-rewrite/m6-remainder.md`](../impls/archive/gpui-rewrite/m6-remainder.md).
