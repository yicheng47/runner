# Concurrency: processes, threads and async runtimes

Runner separates the app, background service and command-line client. [Architecture §11](arch.md#11-process-and-thread-model) has the cost model; [Process model](process-model.md) maps terminal ownership and data paths.

## Three processes

```text
Runner.app
├── GPUI foreground executor: rendering, input, windows and entity updates
├── GPUI background executor: blocking client requests and UI work
├── socket reader/writer: client-protocol frames and terminal mirrors
└── managed reconnect worker: daemon recovery, pauses automatic restarts after three crashes in five minutes; Try again requests one start

runnerd (one per app-data directory)
├── AppCore: SQLite pool, SessionManager, buses, routers, usage and discovery
├── runner-ipc tokio runtime: runnerd.sock client protocol and accept-and-close older-app sentinel
├── per-session threads: PTY reader, idle detector, forwarder, terminal events and input
├── per-mission threads: event bus, inbox reconciliation and router timers
└── startup/version/usage helpers

runner CLI (a new process per command)
└── current-thread tokio runtime for a socket call; mission appends write the log directly
```

Production and development data each have their own daemon. The app and CLI use owner-only Unix sockets on macOS and named pipes on Windows. The CLI can start the daemon without the app, except from an SSH session or when Windows job breakaway is denied. A daemon keeps running without any client; UI-only usage polling and discovery refresh pause until a client reconnects.

## Executors and shared state

GPUI’s foreground executor owns every UI entity. Its background executor sends blocking typed requests through `DaemonClient`; render reads `AppStore` and mirrors only. The socket reader publishes runtime-neutral events and terminal frames, which GPUI tasks turn into entity updates and batched wakes. No app code reaches the daemon’s SQLite or session manager directly.

Tokio owns the daemon’s client-protocol server and older-app sentinel. Requests reach the same `AppCore` through synchronous `ops` functions; process and network waits run in its blocking pool. The daemon’s ordinary threads share the SQLite pool, manager and terminal registry through `Arc`, mutexes and channels. The app shares no address space with them.

The CLI uses a current-thread runtime, runs typed `DaemonClient` calls in the blocking pool, and receives pushed feed events when following a mission. Inside a mission, messages and signals append directly to `events.ndjson`; the daemon’s watcher and router continue consuming those writes while the app is closed.

## Terminal threads

`portable-pty` has a blocking reader, so each session gets a reader thread. A manager forwarder numbers chunks, publishes them to bounded client queues and feeds the authoritative model. The daemon’s terminal-event worker answers queries; its input worker serializes user input behind the delivery gate. The app’s socket reader feeds a separate mirror parser, and panes paint the mirror. All sessions parse while no app is attached.

A slow mirror never slows an agent: terminal queue overflow asks the client to resync from a snapshot. App wake batching affects painting, not daemon parsing or routing. Snapshot capture and stream subscription share the session’s sequence lock.

## Rules for new code

- Tokio I/O, timers and task spawning stay inside the daemon’s IPC runtime. GPUI and ordinary threads use runtime-neutral channels or blocking work.
- `tokio::sync` channels work under either executor.
- UI database and process work goes through background client requests; render never calls a request.
- Long waits in socket handlers use `tokio::task::spawn_blocking`.
- A blocking read that never ends gets its own thread.

## Inspecting the processes

Activity Monitor and `ps -M <pid>` on macOS show the app and `runnerd` separately. Named `runner-ipc`, `pty-reader-…` and `event-bus-…` threads belong to the daemon. On Windows, Task Manager shows `runnerd.exe` and its console helpers separately. App logs are `runner.log`; daemon logs are `runnerd.log` beside them.
