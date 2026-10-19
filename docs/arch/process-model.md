# Runner process and terminal runtime model

Current three-process implementation for [#645](../features/archive/645-session-host.md), updated on 2026-10-06 from the process map on `origin/fix/647-terminal-black-sidebar`. [Architecture](arch.md) covers domain state and coordination; [Concurrency](concurrency.md) covers executors.

## OS processes and ownership

The desktop app owns GPUI windows, AppStore and terminal mirrors. `runnerd` owns AppCore, SQLite, SessionManager, PTYs, authoritative terminal models, mission buses and routers, usage and discovery. The `runner` CLI is a separate client process per command. Multiple app windows share one connection and mirror registry. A mission groups sessions; it is not an OS process.

```mermaid
flowchart TB
    subgraph app["Runner app"]
        ui["GPUI windows and panes"]
        store["AppStore snapshots / TerminalBridge mirrors"]
        ui <--> store
    end
    subgraph daemon["runnerd — one per app-data directory"]
        core["AppCore / SQLite / buses / routers"]
        manager["SessionManager / PtyRuntime"]
        terminal["Authoritative TerminalModel"]
        core <--> manager
        manager <--> terminal
    end
    store <-->|"runnerd.sock: requests, events, snapshots and bytes"| core
    cli["runner CLI"] <-->|"runnerd.sock: typed requests and pushed events"| core
    cli -->|"mission appends"| log["events.ndjson"]
    log --> core
    manager <-->|"local PTY"| child["Agent / shell and descendants"]
    child --> cli
```

On Windows, `runnerd.exe` has a hidden console and bundled `conpty.dll` and `OpenConsole.exe` beside it in app data. Agent process trees belong to kill-on-close jobs. The app is not their process owner. A shell → SSH session still owns only the local shell and SSH process tree; stopping it does not guarantee remote jobs terminate. A remote daemon and session protocol are phase 3.

| Component | Owns |
| --- | --- |
| AppCore / SessionManager (`runner-daemon`) | Session identity, spawn/resume policy, delivery gates, status, sequence numbers and manager forwarders. |
| PtyRuntime (`runner-daemon`) | PTY and child handles, writer mutex, process-tree supervision, reader and idle workers. |
| TerminalModel (`runner-terminal`, in daemon) | Persistent parser/grid, synchronization deadlines, draft observations and terminal replies. |
| TerminalBridge / TerminalMirror (`runner-terminal`, in app) | Attached mirrors, snapshots and stream sequencing, palette and viewer leases. |
| GPUI surface (`runner-app`) | Painting, selection, links, key/mouse/IME encoding and local resize feedback. |

The app’s production dependencies are `runner-core` and `runner-terminal`; `runner-daemon` is only a test dependency. The CLI links the daemon and dispatches to it when launched under the file stem `runnerd`.

## Per-session workers and output

The daemon keeps the blocking PTY reader, idle/status worker, manager forwarder, terminal-event worker and ordered input worker. GPUI, IPC and watcher threads add work at the process level.

```text
child bytes → PTY reader → RuntimeOutput queue → manager forwarder
                                               ├─ numbered bytes → client queues → app mirror → GPUI
                                               └─ authoritative parser/grid → draft observations
hooks → status monitor → SessionModel → mission log and AppEvents → AppStore
```

The reader-to-forwarder boundary is a channel. The forwarder publishes raw bytes before authoritative parsing so the app can parse concurrently. One sequence/model lock captures a snapshot and subscribes to later output atomically. The snapshot includes unfinished input and synchronized-update state. A slow client is dropped from its bounded terminal queue and asked to resync; it cannot hold up the agent.

Authoritative parsing continues without panes or clients. Only the daemon answers terminal queries; mirrors discard replies. The app batches terminal wakes over 4 ms. That delays notification, not parsing. Agent working/idle state is separate from screen bytes: hooks and the baseline detector feed the reducer, so output silence is not proof of turn completion.

## Input and resize

The app encodes keys, IME, paste and mouse against the mirror’s current modes. Input is a one-way client frame queued by the daemon per session, for both direct chats and missions. User input still passes the delivery gate; router input uses its reserved path. Terminal replies bypass that gate and ultimately share PtyRuntime’s writer mutex.

A pane sends a one-way resize immediately. The daemon applies a controller's size to its PTY/model, emits an ordered `Resized` frame to every viewer including the sender, and persists settled geometry. Every mirror adopts its size from the attach snapshot and those frames; a request from a non-controller leaves its size unchanged. Snapshot restoration and live resize preserve parser modes and wide-character state. Runner retains its own PTY readers; it uses Alacritty’s model and VTE parser rather than its PTY EventLoop.

## Lifecycle

Spawn resolves command, environment, geometry and conversation identity in the daemon. PtyRuntime launches the child on its slave, owns process-tree cleanup and starts the reader. The manager installs the handle, creates the authoritative terminal and emits spawn. App attachment takes a snapshot and then the exact live stream.

Natural exit drains output before the manager reconciles status and releases the terminal. Explicit stop asks the runtime to stop/reap children before clearing the handle. macOS and Linux use SIGHUP and then process-group SIGKILL after its grace period, including captured descendants (Linux discovers them from `/proc`); Windows terminates the job and verifies root termination. Explicit cancellation does not promise all queued final bytes are painted.

Resume makes a new child/model under the same Runner session ID and restores the agent’s conversation when supported. Reattach connects to the existing child and current grid. These are separate operations.

The daemon keeps running without clients or live sessions; it pauses UI-only usage and discovery work. Ask/Keep/Stop controls explicit app quits, and macOS ⌥⌘Q always stops. Keep disconnects the app. Stop stamps launch resume, kills sessions and exits the daemon. OS and update quits never ask and leave it running. The daemon handles OS termination with stamping; the new build handles update hash mismatch by stopping, replacing and restarting it. Successful resumes supply the restart toast’s count. Crashes demote stale rows and leave Resume buttons; the app limits automatic recovery after three crashes in five minutes and shows a short stopped-session-count error toast after automatic recovery. After repeated crashes or a failed automatic replacement, each window shows a persistent banner beneath the main content header with Open log and Try again, excluding sidebar and side panel. Try again makes one connection/start attempt without resuming sessions; restored panes keep their Resume buttons. A failed start or another crash returns to the capped notice.

App and CLI starters keep the child handle until connection succeeds, checking for an early exit on every retry. An exited daemon fails immediately with its exit status and `runnerd.log` path; a live daemon gets a 60-second startup budget. The startup lock covers both a 15-second replacement stop and that startup budget, with five seconds of margin. Stop waits use the shared teardown budgets: eight seconds for session teardown, one second for launch-resume completion, one second for the Tokio runtime and five seconds of margin. The app’s Stop action and `runner daemon stop` use the same stop budget. These durations are defined together in `runner-core::daemon_process` and used by both clients and the server.

A failed client or legacy-sentinel accept is logged and backs off for 100 ms before serving continues; it does not stop sessions or enter teardown. Windows disconnects a failed pipe instance so the next client can connect. Explicit shutdown, an OS termination signal or loss of endpoint ownership ends the serve loop. On Unix, startup raises the soft `RLIMIT_NOFILE` toward the hard limit (capped at `OPEN_MAX` on macOS) and logs the resulting limits or failure.

On Windows, a dedicated thread pumps a never-shown top-level window for `WM_QUERYENDSESSION` and `WM_ENDSESSION`, covering restart, shutdown and sign-out. A message-only window would not receive these broadcasts. Console close, logoff and shutdown handlers remain registered, but they are insufficient alone: [loading `user32.dll` or `gdi32.dll` suppresses console logoff/shutdown events](https://learn.microsoft.com/en-us/windows/console/setconsolectrlhandler), and runnerd loads both. Startup attempts to set the process shutdown level to `0x3ff`, in the application-first range, ahead of ordinary agent and ConPTY processes at the default `0x280`. Shutdown-priority or window-creation failures log a warning and leave the daemon serving; without a window, its shutdown future waits only for console events. Unexpected closure of an established window channel logs that the window closed before entering teardown.

`WM_QUERYENDSESSION` gates new child-exit reconciliation before acknowledging the query. Exits already in progress retain their pre-exit rows in memory; database work never holds the window-state mutex. A cancelled `WM_ENDSESSION` releases the gate and discards captured exits without stopping the daemon or stamping sessions. A confirmed end-session wakes the server's OS shutdown future; teardown quiesces launch resume and new spawns, waits for in-flight exits on its worker, and commits the running-session stamp together with captured exits. Captured rows are guarded by generation and current resume eligibility, with conversation keys restored if the fast-resume-failure heuristic cleared them. Only then are the window handler and queued child exits released. The handler waits at most four seconds for the commit, logging failure or timeout explicitly; it does not wait for the full eight-second PTY teardown or indefinitely block Windows shutdown. PTY cleanup is best-effort after the stamp because Windows may end the process as soon as the handler returns. `runnerd.log` records the window messages and the committed resume stamp. Forced termination, a failed database write or an exhausted shutdown deadline cannot guarantee resume; live restart and sign-out validation remain separate from synthetic message tests.

The socket transport gives only `session_fork` at least 150 seconds, derived from the shared 120-second fork materialization bound plus 30 seconds for the remaining spawn work and response. Every other request, including ordinary non-fast calls such as session lists and sidebar renames, retains the client’s short deadline (10 seconds for the app, 30 seconds for the CLI). Longer model discovery, usage and session-key capture work runs in background workers rather than extending those request deadlines. Terminal attachment and socket writes retain the short budget, and the CLI’s pushed mission feed still bounds snapshot/drain requests separately at 30 seconds.

## Source map

- [App bootstrap](../../crates/runner-app/src/bootstrap.rs), [quit dialog](../../crates/runner-app/src/surfaces/quit_dialog.rs), and [AppStore](../../crates/runner-app/src/app_store.rs).
- [Daemon server](../../crates/runner-daemon/src/daemon/server.rs), [boot](../../crates/runner-daemon/src/daemon/boot.rs), and [resume](../../crates/runner-daemon/src/daemon/resume.rs).
- [Runtime contract](../../crates/runner-daemon/src/session/runtime.rs), [PTY runtime](../../crates/runner-daemon/src/session/pty_runtime.rs), and [session manager](../../crates/runner-daemon/src/session/manager/mod.rs).
- [Terminal model](../../crates/runner-terminal/src/terminal/model.rs), [mirror bridge](../../crates/runner-terminal/src/terminal.rs), and [terminal element](../../crates/runner-app/src/terminal/element.rs).
- [Daemon launch](../../crates/runner-core/src/daemon_process.rs), [managed connection](../../crates/runner-core/src/protocol/managed.rs), and [socket transport](../../crates/runner-core/src/protocol/socket.rs).
