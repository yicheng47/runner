# Concurrency: processes, threads and async runtimes

How Runner's code is spread across processes and threads, which scheduler runs what, and how the pieces reach each other. [`arch.md` §11](arch.md#11-process-and-thread-model) has the thread diagram and the cost model; this document explains the executors and runtimes behind it.

## Two processes

```text
Runner app process (one PID, one address space)
├── main thread ────────────── GPUI foreground executor: UI, rendering, entity updates
├── GPUI background threads ── GPUI background executor: blocking work the UI hands off
├── runner-ipc threads ─────── tokio runtime: the socket server the runner CLI talks to
├── per-session threads ────── pty-reader-<session>, pty-idle-<session>, output forwarders
├── per-mission threads ────── event-bus-<mission>, inbox-reconcile-<mission>, router timers
└── short-lived helpers ────── login-shell probe, runtime and version probes, usage readers
          │
          └── all share one AppCore in memory: SQLite pool, SessionManager, event channel

runner CLI process (a new PID per command) ── connects to the app over mcp.sock, calls one tool, exits
```

The app is one process. Every thread in it sees the same memory, so GPUI code, tokio code and plain threads share state directly through `Arc<AppCore>`. They coordinate with ordinary thread-safety tools (`Arc`, `Mutex`, channels), never with IPC.

The `runner` CLI is the only other process, and the socket exists only for it. Each command starts a process, connects to `mcp.sock` in app data (a Unix domain socket on macOS and Linux, a named pipe on Windows, both in `runner-backend/src/ipc.rs`), speaks MCP to call one tool, prints the result and exits. The CLI runs that call on a single-threaded tokio runtime (`new_current_thread` in `runner-cli/src/command.rs`). Because live sessions, missions and the UI exist only inside the app, the CLI needs the app to be running.

## Threads inside the app

| Threads | Started by | Runs |
|---|---|---|
| Main thread | macOS or Windows, then GPUI | GPUI's foreground executor: rendering, input, and every task that touches an entity (`cx.spawn`) |
| GPUI background threads | GPUI | GPUI's background executor (`cx.background_spawn`): Grand Central Dispatch global queues on macOS, the system thread pool on Windows |
| `runner-ipc` | tokio, built in `NativeMcpServer::start` (`runner-app/src/bootstrap.rs`) | The socket server and its tool handlers. A multi-thread runtime: one worker per core, plus tokio's blocking pool for `spawn_blocking`, all named `runner-ipc` |
| `pty-reader-<session>`, `pty-idle-<session>`, output forwarders | The session layer, as `std` threads | Blocking PTY reads, idle detection, and feeding each session's terminal model (§11.2 of `arch.md` explains why these are threads, not async tasks) |
| `event-bus-<mission>`, `inbox-reconcile-<mission>`, router timers | The event bus and router, as `std` threads | Tailing a mission's NDJSON log and routing its messages |
| Short-lived helpers | Various, as `std` threads | Start-up probes (login shell, runtimes and versions, usage), each ending when its work is done |

## Two async executors, one OS scheduler

Runner runs two async executors side by side, and neither knows about the other.

- **GPUI's executors** drive the UI's `async` code. The foreground executor runs on the main thread: on macOS it posts to the main dispatch queue, and on Windows it posts messages to the UI thread. The background executor hands work to Grand Central Dispatch's global queues on macOS and to the system thread pool on Windows.
- **Tokio** drives only the socket server. It is a multi-thread runtime: many cheap tasks on a fixed set of worker threads, each with its own run queue, and idle workers steal from busy ones. Tasks switch only at `.await`.

The OS kernel schedules every thread in the process together, whichever executor or subsystem owns it. The executors only decide which of their own tasks runs next on their own threads. Both size themselves to the core count, so the process has more threads than cores, but most of them are idle most of the time.

Rust's `async`/`await` is language syntax; the executors are libraries that poll the resulting futures. The same `async fn` can run on either executor, but I/O types belong to one runtime: a tokio socket, timer or `tokio::spawn` only works inside a tokio runtime, because it registers with tokio's event loop. That is why `McpHandle::start` calls `rt.enter()` before binding the listener, and why tokio's I/O stays inside the IPC server.

## Where the two sides meet

Nothing crosses between the executors except shared state and runtime-neutral channels.

- **Operations.** `ops` functions are synchronous. The UI calls them from `cx.background_spawn`; the socket tools call them from their tokio handlers. Both reach the same SQLite pool (8 connections, a 5-second busy timeout) and the same `SessionManager`, so a CLI call and a UI click can contend for a connection or a lock like any two threads.
- **Events.** `AppCore.events` is a `tokio::sync::broadcast` channel. `tokio::sync` works on any thread and under any executor; only `tokio::net`, `tokio::time` and task spawning need the tokio runtime. An operation emits an event (`role/changed`, `chat/layout-changed`) from whichever thread ran it. A GPUI background task awaits the channel and forwards each refresh to a foreground task that updates the entities (`runner-app/src/app_store.rs`). That is how a change made through the CLI appears in an open window.

## Rules for new code

- **Tokio's I/O, timers and task spawning stay in the IPC server** (`ipc.rs`, `mcp/`). Anywhere else the code runs on GPUI's executor or a plain thread, where there is no tokio reactor, and those calls panic.
- **`tokio::sync` channels and locks are fine anywhere.**
- **Never block the main thread.** UI code sends database work, process spawning and file I/O through `cx.background_spawn`.
- **Blocking inside a tokio handler is fine for short work** such as an `ops` call that runs a few SQLite statements. Anything that waits on a process or the network goes through `tokio::task::spawn_blocking`, as the session and mission tools already do.
- **A blocking read that never ends gets its own thread**, the way PTY readers do. It would pin an executor worker.

## Inspecting a running app

`ps -M <pid>` on macOS lists every thread of the app, and Activity Monitor's Sample Process shows what each one is doing. The named threads (`runner-ipc`, `pty-reader-…`, `event-bus-…`) identify their owners; GPUI's background work appears on Grand Central Dispatch worker threads.
