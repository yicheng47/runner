# 645 — runnerd: sessions outlive the app, and run on other machines

> Tracking issue: [#645](https://github.com/yicheng47/runner/issues/645)
> Priority: P1, 0.13, the sole release blocker (phase 1). Platforms: macOS and Windows.
> Status: drafted 2026-10-04 and reviewed with Jason on 2026-10-05, when it landed on `main`. Missions 1a to 1c are on the umbrella branch; on 2026-10-06 the 1d design was drawn and phase 2 was dropped (decision 13). It starts from the archived [466 spec](./archive/466-sessions-outlive-the-app.md) and the issue body. Decision 1 changes the issue's shape: the daemon owns the state, not only the processes, following the lesson Jason took from Paseo and Orca on 2026-10-04. The issue calls the process the session host; on 2026-10-05 Jason named it `runnerd`, which leaves "host" to mean a machine in phase 3. The implementation plan is in [`docs/impls/645-runnerd/`](../impls/645-runnerd/plan.md). The Remote machines section covers the launch steps [#795](https://github.com/yicheng47/runner/issues/795) asked for, so #795 can close once this spec lands.

## Motivation

Every session's PTY lives inside Runner.app. Quitting, updating or crashing the app kills every agent mid-turn, and a crew mission can never run longer than one app process. [466](./archive/466-sessions-outlive-the-app.md) declined a background process on 2026-09-02 as too much machinery for that loss. Jason reversed that on 2026-09-07 when he closed [#491](https://github.com/yicheng47/runner/issues/491) (confirm on quit) in favour of the session host.

A second reason arrived on 2026-09-17: crew members on other machines, such as a tester slot on the Windows PC or a coder on a dev box. An ssh reverse tunnel for the `runner` CLI was worked through and rejected, because it leaves hook status, Windows quoting, PATH on a non-interactive shell, sidecar version skew, `cwd` as a local path and agent detection all unsolved. A process running next to the agent solves all of them at once.

A third arrived on 2026-10-04, from comparing [Paseo](https://github.com/getpaseo/paseo) and [Orca](https://github.com/stablyai/orca): **whatever owns the state must be able to run without the UI.** In Runner the router, the delivery gate and the socket the CLI talks to all live in the app. If only the PTYs moved out, closing the app would still stop every mission's coordination, the CLI would still fail with "Runner is not running", and a phone client would only ever work while the desktop app was open.

## How it works today

- **One process.** `bootstrap::boot_core` builds `AppCore` inside the GPUI app: the SQLite pool, `SessionManager` over `PtyRuntime`, the bus and router registries, the MCP server on `mcp.sock`, usage and runtime discovery. 82 files in `runner-app` call `runner_backend` directly, reaching 99 distinct `ops::` functions and 16 `repo::` functions, plus a few `SessionManager` methods.
- **The terminal lives in the app, and the router depends on it.** `TerminalBridge` in `runner-terminal` holds one alacritty `Term` per live session, fed synchronously by the manager's forwarder through `SessionEvents::output`. The same `TerminalSession` answers terminal queries (its event worker writes the replies through `SessionManager::inject_stdin`) and derives draft observations from the grid (`observe_parsed` → `report_input_state`). Those observations hold crew deliveries at the delivery gate (arch §8.5), so the router's gate reads state the app computes.
- **The process side is already behind a seam.** `SessionRuntime` in `session/runtime.rs` sits between the manager and the processes, and `PtyRuntime::spawn` already starts each runtime's hook watcher and terminal adapter. It is the only implementation.
- **Quit kills.** `stop_running_sessions_on_quit` stamps running rows `resume_on_launch` and calls `kill_many`. The next launch mounts routers for running missions, demotes stale rows, sweeps orphan processes and resumes the stamped sessions (`consume_resume_on_launch`), whose agents restore their own conversations.
- **The CLI needs the app.** Inside a mission, `msg post`, `msg read`, `signal` and `ask` append to `events.ndjson` directly. Every other command connects to `mcp.sock` and exits 3 when the app is not running.

## Proposal

### The daemon owns the state; every interface is a client

`runnerd` is a background daemon that runs `AppCore`: the database, the sessions and their PTYs, the event buses and routers, the MCP socket, usage and runtime discovery. The GPUI app becomes a client of it. It connects over a local socket, sends requests, receives `AppEvent`s and mirrors terminals. The `runner` CLI stays a client, as it is today. A phone or web client later would be one more client, but v1 builds none.

```
Runner.app (GPUI)                    runnerd (one per app-data dir)                   agents
  windows, panes, AppStore   ──req──►  AppCore: SQLite · SessionManager ·    ──PTY──►  claude, codex, pi,
  mirror Term per session    ◄─evt──   routers · buses · usage · discovery  ◄─hooks─   copilot, agy, shells
                                       authoritative Term per session
            runnerd.sock                         mcp.sock ◄── runner CLI (people, scripts, agents)
```

What this gives:

- Closing or crashing the app stops nothing. Agents keep working, the router keeps nudging inboxes, `ask_human` cards wait in the feed, and `runner mission answer` can answer them from any terminal.
- The CLI works without the app, because it starts the daemon when none is running.
- Remote machines (phase 3) and a future phone client attach to the same owner. One mission still has one owner.

What it costs:

- Every call the app makes into the core becomes a request, about 115 kinds across 82 files. The work is mechanical, and phase 1a does it with no process split, so it lands with no behaviour change.
- The terminal splits into an authoritative copy in the daemon and a mirror in the app.
- An update still restarts the agents, as it does today (see Updates and version skew). Phase 1's gain is on quit and on crash.

### What moves where

| Piece | Today | With runnerd |
| --- | --- | --- |
| `AppCore`: SQLite, `SessionManager`, `PtyRuntime`, buses, routers, usage, discovery, window registry | app | daemon |
| MCP server on `mcp.sock` (`NativeMcpServer`) | app | daemon |
| The authoritative terminal: `Term`, parser, synchronized-update flush, query replies, colour-scheme reports, OSC 7 cwd, draft observations, fixture recorder | app (`TerminalSession`) | daemon |
| Painting, selection, scrollback viewing, links, key, IME and mouse encoding | app | app, against a mirror `Term` |
| `AppStore` snapshots, windows, tabs and layout | app | app, filled by requests and events |
| Settings the core reads (resume on launch, enabled runtimes, the Runner skill switch) | pushed into `AppCore` in-process | the daemon reads them at start; the app pushes changes |
| macOS wake observer (needs AppKit) | app | app, forwarded to the daemon as a request |
| Sparkle and the Windows updater | app | app; they mark the update quit and leave the daemon for the new build to restart |

Pure functions and types (`model`, `runtimes::for_key`, `app_paths`, constants) stay ordinary library calls. Anything that reads or writes the database, the session manager, a router, a bus, usage or discovery state goes through the daemon.

### The client protocol

The endpoint is `runnerd.sock` in app data, beside `mcp.sock`, or the named pipe `\\.\pipe\com.wycstudios.runnerd` (`runnerd-dev` for debug builds) on Windows, built on the existing `IpcListener` in `ipc.rs`. Only the user can connect, as with `mcp.sock`: anything that can reach it can start processes as the user.

The shape is length-prefixed frames on one connection. The plan settles the encoding.

- **`Hello`, `Welcome`, `Mismatch` and `Shutdown`.** Their framing and existing fields are frozen forever, so any version can always identify and stop any other. They are JSON, new fields are optional, and a reader ignores fields it does not know. That is how a credential field can be added later without breaking older versions.
- **`Request` and `Response`.** One typed variant per core call, as JSON.
- **`Event`.** The `AppEvent` stream, which is already a name plus a JSON payload.
- **Terminal frames.** `Attach` returns a `Snapshot { seq, cols, rows, bytes }`, followed by `Output { seq, bytes }` and `Resized { seq, cols, rows }` frames, which are binary. `Input` and `Resize` go the other way, one-way and ordered per session.

**Other transports later.** The protocol assumes only a byte stream. v1 relies on nothing specific to local sockets: no peer credentials and no handles passed through the socket. Reaching `runnerd` from elsewhere is therefore an extra way in, not a migration. Another Mac's Runner app can run the same frames over `ssh <host> runnerd --stdio`, as phase 3 does. A phone would need a network listener, on a Tailscale address or through a relay, with pairing and encryption and a credential in `Hello`. Paseo serves its local apps, ssh and phone clients from one daemon the same way. The local socket stays for the app and the CLI. No such listener is in this spec. Today the socket's owner-only permissions are the only authentication.

Three rules come with it:

- **No request on the render path.** Render reads `AppStore` and the mirror only. Requests run where ops run today, on GPUI's background executor.
- **Input is a one-way frame** that the daemon queues per session, so a held delivery gate can never park the app's main thread. Mission panes already queue their input this way; direct chats write inline today and will queue too.
- **A slow client never slows an agent.** Each client has bounded queues. On overflow the daemon drops that client's terminal stream and sends `Resync`, and the client re-attaches with a fresh snapshot. `AppEvent` lag already resyncs through `mission/resync`.

### The terminal: one authoritative copy in the daemon, a mirror in the app

The daemon keeps the authoritative `TerminalSession` for every live session. The app keeps a mirror `Term` fed the same bytes, which it paints, selects in and encodes keys against.

- **Only the daemon answers terminal queries:** device attributes, cursor position, colours, kitty keyboard flags and the colour scheme. The mirror discards its replies, so each query gets exactly one answer ([524](./archive/524-double-terminal-query-replies.md)), and agents still get answers while no app is attached.
- **Draft observations come from the daemon's `Term`**, next to the delivery gate that reads them.
- **Attach is a snapshot, then the live stream.** Under the lock that records `seq`, the daemon serializes its `Term` to VT bytes: the primary screen with its scrollback, the alternate screen when active, cursor position and style, SGR attributes, the title, and the modes (application cursor keys, bracketed paste, mouse reporting, focus reporting, scroll region, the kitty keyboard stack). The client feeds those bytes into a fresh `Term`, then applies every frame after `seq`.
- **The snapshot boundary is exact.** The bytes after the snapshot must land in the same parser state on both sides, so the snapshot also carries any escape sequence cut off at its boundary and any synchronized update the daemon's parser is still holding. The plan picks the mechanism; the split-point test under Verification proves it. One per-session lock covers numbering a chunk, queuing it for clients, parsing it, and taking a snapshot together with its subscription, so a client can neither miss nor repeat a chunk.
- **Snapshots are rare.** A session that starts while the app is connected is mirrored from its first byte, with an empty snapshot. A real snapshot is taken only when the app relaunches, reconnects, or falls behind and resyncs. A serializer bug therefore shows as a wrong-looking pane after a relaunch until the agent redraws; it never touches the live path or the daemon's own copy.
- **Bytes go to the app before the daemon parses them,** so the mirror never waits on the daemon's own parse, and the two parses run on different cores. The mirror parses exactly the bytes it parses today; nothing in between rewrites them, unlike tmux, which emulates the terminal itself and redraws.
- **Resize feels as it does today.** The pane that owns the size resizes its mirror at once, in the same call, as `TerminalSession::resize` does now, and sends `Resize`; the daemon resizes the PTY and its own `Term` when the frame arrives. Bytes already in flight reflow on the mirror as they do today. Another client showing the same session follows the `Resized` frame the daemon emits.
- **Everything else in the terminal stays local to the app:** painting, glyph shaping, IME, selection, copy, links and scrolling through scrollback never touch the socket. Only output bytes come in and input bytes go out.
- **Palette.** The app sends the theme palette on connect and on every theme change, and the daemon answers colour queries with it.
- **Crate boundary.** `runner-terminal` stops depending on `runner-backend`. The backend depends on it for the daemon's terminal, and the app depends on it for the mirror. More broadly, a new `protocol` module in `runner-core` holds everything that crosses the socket. After phase 1, the app depends only on `runner-core` and `runner-terminal`, never on the daemon's code, and `runner-backend` is renamed `runner-daemon` ([plan](../impls/645-runnerd/plan.md#crates)).
- **The engine moves unchanged, and it stays Runner's own reader.** [#709](https://github.com/yicheng47/runner/issues/709) asked whether to adopt alacritty's `tty` and `EventLoop` instead. A daemon has to forward raw bytes to the mirror and to its own observers, and at the pinned 0.26.0 the event loop parses bytes without exposing them, except through a reference-test file tap. #709 therefore does not go first, and its audit checklist moves into 1b ([plan](../impls/645-runnerd/plan.md#709-does-not-go-first)).

Two alternatives were rejected. A raw byte ring buffer replayed on attach starts a TUI in the wrong modes when it begins mid-stream, and is unbounded when it does not. Sending grid diffs would make a rendering protocol to maintain. The cost of the chosen design is two `Term`s per live session, which doubles terminal memory in the arch §11.4 cost model. At ten or so sessions that is acceptable.

**This was tried before, and failed for reasons that no longer apply.** The Tauri app's tmux runtime ([impl 0004](../impls/archive/0004-tmux-session-runtime.md)) rebuilt screens from `tmux capture-pane`, rendered text that drops modes such as the alternate screen, which caused the stacked redraws of #150 and [0009](../impls/archive/0009-terminal-alt-screen-reattach.md). [PR #157](https://github.com/yicheng47/runner/pull/157) then tried a daemon with a headless alacritty `Term` and a serializer, and was closed after one review for four problems ([0011](../impls/archive/0011-pty-host-terminal-runtime.md) §"Why no headless emulator"). [Spec 42](./archive/42-headless-terminal-model.md) records the raw-replay workarounds that followed and lost Claude Code's history. Against #157's four:

1. **Two parsers had to agree,** alacritty in the daemon and xterm.js in the webview. Now both sides run the same `alacritty_terminal` crate at the same version, from this workspace, over the same bytes. Since parsing is deterministic, the copies cannot disagree on the live path.
2. **The serializer had to restore every mode.** That is still true, and it is the one hard part left. It is bounded by alacritty's own mode set, and the round-trip and split-point tests check it against real recordings.
3. **Sequence numbers raced across the lock and the socket.** That is still a risk, because a socket is back. The single per-session lock above is the answer, and the split-point test covers it.
4. **Keys had to be translated in the daemon,** to match the agent's current modes. That is no longer needed: the app encodes keys against its mirror, which holds the same modes, and sends bytes, as it does today.

Orca runs this design in production with `@xterm/headless` and lists what it hit even with one engine on both sides: escape sequences split across chunk boundaries, Unicode width differences between mirror and renderer, cursor restore, SGR state before re-entering the alternate screen, and doubled query replies. The exact snapshot boundary and the daemon-only replies answer the first and last. Width cannot differ here, because the same crate computes it on both sides. The serializer tests cover the rest.

### Starting, finding and stopping the daemon

- **One daemon per app-data directory.** Production and the `make run` development build each get their own, as they each get their own `mcp.sock`. The daemon takes `runnerd.lock` before binding, so of two simultaneous starters, the second exits and its caller connects to the first.
- **`runnerd` is the bundled CLI under a second name.** The app installs the sidecar twice in `<app data>/bin`, as `runner` and as `runnerd`, and the binary runs the daemon when started under the name `runnerd`. Activity Monitor and Task Manager then show `runnerd`, and there is no second build target to sign. The sidecar already links `runner-backend` (9.5 MB in the release bundle). The app never replaces `runnerd` while a daemon runs from it, because Windows cannot replace a running executable.
- **Who starts it.** The app at launch, when nothing answers on `runnerd.sock`. The CLI, when a socket command finds nothing listening. A connection the sandbox blocks (Codex's default sandbox) still exits 5 as today and starts nothing. The CLI never starts a daemon from an ssh session (`SSH_CONNECTION` set); it says to open Runner on that machine. A daemon started there would run its agents inside the ssh logon, where macOS may keep the login keychain locked, so Claude Code could not read its credentials, and Windows may kill the session's processes when it closes.
- **Detached.** On macOS it gets its own session (`setsid`) with closed stdio. On Windows it starts with `CREATE_NO_WINDOW`, `CREATE_NEW_PROCESS_GROUP` and `CREATE_BREAKAWAY_FROM_JOB`, but not `DETACHED_PROCESS`. That gives it a hidden console of its own, which the console programs it starts inherit; with no console at all, any console child started without `CREATE_NO_WINDOW` would open a visible window. Its working directory is the home directory, and it logs to `runnerd.log` in the log directory with the same panic hook as the app.
- **A clean environment.** When started from a terminal or an agent, the daemon drops the `RUNNER_*` variables and the mission-shim and sidecar `PATH` entries it inherited. Spawns compose `PATH` from the process `PATH` (arch §5.3), and an agent's environment must not leak into the next spawn.
- **Ending when the OS asks.** At logout, restart or shutdown, `runnerd` does what Stop Sessions does before it exits, so sessions resume after a reboot as they do today (see When something dies). This applies only to the OS's request to end (SIGTERM on macOS; the logoff, shutdown and close events on Windows), never to a crash.
- **Lifetime: once started, it keeps running** (Jason, 2026-10-05, after comparing Paseo, Zeron and Docker). `runnerd` has no idle exit. It stops on four things: quitting with Stop Sessions, `runner daemon stop`, logout or restart (see Ending when the OS asks), and an update restart. Nothing starts it at login in phase 1. While no client is connected, it pauses work that exists only for a UI, plan-usage polling and agent-discovery refreshes, and resumes it when a client connects. So `runner` commands answer at once, the router and the CLI socket are always there, and an idle daemon costs almost nothing.
- **Startup is `boot_core` without GPUI:** install nothing (the app or CLI already did), open the database, mount routers for running missions, demote stale rows, sweep orphans, resume stamped sessions at their persisted sizes, start the MCP server and `runnerd.sock`, then start discovery and usage. These steps all run at app launch today, so first launch is no slower. A relaunch that finds the daemon running skips them all.
- **`runner daemon status`** prints the pid, build, uptime, live sessions and connected clients. **`runner daemon stop`** stops every session the way Stop sessions does (below) and exits.

### Quitting

- **With no live sessions,** quitting just closes the app, and `runnerd` keeps running.
- **With live sessions,** Settings → General has one row, "When Runner quits": Ask (the default), Keep running, or Stop sessions, with the live session count in its description. Ask shows a dialog with two choices, Keep them running and Stop them, the second naming any agent that is mid-turn, then Cancel and Quit. The last choice is preselected, Enter quits, and "Don't ask again" saves the selected choice into the setting (design, 2026-10-06).
- **⌥⌘Q is Quit and Stop Sessions,** whatever the setting, following the macOS convention for an alternate quit. It is how someone who chose Keep running stops everything; `runner daemon stop` does the same from a terminal.
- **Keep running** disconnects the app. The daemon keeps everything.
- **Stop sessions** is today's quit, performed by the daemon: stamp `resume_on_launch`, `kill_many`, exit.
- **On Windows,** closing the last window quits, and gets the same choice.
- **Relaunch** connects, restores windows and attaches every live session. Kept sessions need no resume; stamped ones were resumed when the daemon started.

### Updates and version skew

- **Identity is the binary, not the version string.** The daemon reports the content hash of its own executable, and the app compares it with the hash of the sidecar it ships. They must match exactly. Version strings alone are not enough, because development builds share one across rebuilds.
- **On a mismatch,** after an update, a manual install or a rebuild, the app restarts the daemon itself: the old daemon's `Shutdown` with stamping, then the new sidecar, a new daemon, and resume. It then shows a notice, "Restarted 3 sessions in Runner 0.13.1". There is no dialog, because whoever installed the build has already chosen to.
- **Installing an update** with live sessions uses the normal update dialog, which says the sessions restart with it. When agents are mid-turn, a small note beside the buttons says how many ("1 agent working"), because their turns are lost; Later waits. After the relaunch, the new daemon resumes the sessions and their agents restore their conversations.
- **An update always restarts the sessions** (Jason, 2026-10-06; decision 13). Keeping agents running through an update would need a second local process that holds the PTYs, speaking a session protocol kept stable across releases. What that saves is one in-flight turn per update, which the update dialog already flags, while restarting carries every fix to the whole stack, terminal and session code included. Remote machines still need the stable session protocol, because their Runner can be a different build, so it is built in phase 3.

### When something dies

| Event | Agents | What the user sees |
| --- | --- | --- |
| The app crashes or is force-quit | keep running | Relaunch reattaches everything. |
| `runnerd` crashes | Their PTYs close: SIGHUP on macOS, the job object's kill-on-close on Windows. The next daemon's orphan sweep catches stragglers. | The app reports that `runnerd` stopped and starts a new one. Rows are demoted to stopped, as after an app crash today, and resume from their Resume buttons. |
| `runnerd` crashes 3 times in 5 minutes | — | The app stops restarting it and shows the path to `runnerd.log`, with an Open log button. |
| The machine sleeps | paused | Nothing. Work continues on wake. |
| Logout or reboot | stop | `runnerd` treats the OS's request to end as Stop Sessions: it marks running sessions to resume, stops them and exits. That is SIGTERM on macOS, and on Windows the logoff and shutdown events its hidden console receives. With resume on launch on, the next daemon start resumes them, which is what today's app does when the OS quits it at logout. |

### Windows

The same design runs on Windows, and most of it is already there: the named pipe in `ipc.rs` serves the CLI today; each agent's job object kills the agent when its owner's handle closes, so a `runnerd` crash stops agents as an app crash does today; ConPTY works from a process with no visible window; and the hook reporters write the same feeds. Four points are specific to Windows:

- **ConPTY beside `runnerd.exe`.** `portable-pty` loads the bundled `conpty.dll` from beside the running executable ([windows.md](../arch/windows.md)). The sidecar install therefore copies `conpty.dll` and `OpenConsole.exe` into `<app data>\bin` with `runnerd.exe`. Without them, sessions fall back to the inbox conhost, and Codex redraws split again ([492](./archive/492-windows-terminal-output-latency.md)). They are replaced only while no daemon runs, like `runnerd.exe`.
- **An owner-only pipe.** Without a security descriptor, a named pipe grants read access to Everyone. `runnerd`'s pipe, and the existing CLI pipe, get an explicit owner-only one and keep rejecting remote clients.
- **Breakaway can be refused.** Runner's own session jobs forbid breakaway, so a `runnerd` that a `runner` command started from inside another Runner session would end with that session. When breakaway fails, the CLI does not start a daemon; it says to open Runner. The app retries without the flag only when its own launcher's job forbids breakaway, and logs it.
- **The installer.** The update quit leaves `runnerd` running; the new build stops and replaces it on hash mismatch. The installer already renames files in use aside, and the files `runnerd` runs from live in app data, not in the install directory.

Measuring named-pipe latency on the PC is part of 1c's preparation.

### Missions while the app is closed

The router keeps running. Messages nudge inboxes, `session_status` flows, and deliveries pass the gate as usual. `ask_human` appends its `human_question` and waits, to be answered with `runner mission answer` or when the app opens. Desktop notifications ([#701](./701-desktop-notifications.md)) are drawn by the app, so none appear while it is closed. Mission notices into a chat ([748](./748-mission-watch-delivery.md)) keep flowing, which changes that spec's "Runner quits" case.

This replaces 466's rule that the router is offline while the app is closed. Opening the app does not remount any router, because no router stopped, so there is nothing to deliver twice. Routers mount only when a daemon starts, with the replay rules of arch §7.2 unchanged.

### Remote machines (phase 3)

- **A machine reached over ssh runs a session daemon,** built in phase 3: PTYs, authoritative `Term`s, hook feeds and watchers, key capture and conversation probes, with no database, routers or missions. The local daemon stays the mission's owner.
- **The transport is `ssh <host> runnerd --stdio`,** a thin proxy that connects to the remote's own daemon over its local socket and never starts one. The remote daemon runs in the user's own logon on that machine: Runner open there, `runner daemon start` from a terminal there, or an opt-in start at login. If none is running, the pane says to open Runner on that host. Paseo works the same way: its ssh transport is `ssh -W 127.0.0.1:<port>`, and its docs say to start the daemon on the remote first. Agents therefore never run inside an ssh logon, which settles two Windows questions by construction: OpenSSH killing a session's processes when it closes, and credentials stored with DPAPI or Credential Manager that a key-based ssh logon may not be able to read. The proxy dies with the connection; the daemon and its agents do not. A dropped connection or a sleeping laptop leaves the agents running, and reconnecting reattaches with a snapshot. There is no listening port, no relay and nothing hosted; sshd is the transport.
- **Locally, `RemoteRuntime` implements `SessionRuntime`** over the session protocol: spawn with a launch spec, stop, input, resize, status, and output frames carrying bytes, agent events and draft observations.
- **The launch spec is structured:** argv, environment and working directory. The remote resolves the executable on its own login-shell `PATH`, so a Windows machine needs no quoting layer.
- **The `runner` CLI on the remote** reaches the mission through its daemon, which forwards appends and socket tools over the connection. `RUNNER_EVENT_LOG` cannot be a local path there.
- **The session protocol has its own version** and is kept stable on purpose. A mismatch shows "Update Runner on <host>". Requiring exact builds would break every remote at every release.
- **In the UI,** Start Chat and each slot in Start Mission get a host picker (recent hosts and `~/.ssh/config` names), and `sessions` gets a host column. A disconnected pane says the agent is still running on that host while it reconnects, and deliveries wait in the outbox until it does. Agent detection, runtime defaults and model discovery are asked of the remote.

#### Launch steps that must run where the process runs (#795)

These read or write files on the machine the agent runs on, so they move into the session-only daemon's half of spawn:

- Project trust seeding (`seed_trust`): Codex `~/.codex/config.toml`, Copilot `trustedFolders`, Antigravity `trustedWorkspaces`.
- pi's system prompt file (`session::system_prompt::write`) and its startup sweep.
- Shell integration scripts (`shell_integration::inject`).
- The status-hook environment pointing at app-data paths (`StatusHooks::env`), and the watchers that read those feeds.
- Conversation-key capture: the Codex rollout scan, the Antigravity log tail, and the Claude and pi rekey drops.
- Two that #795's comment did not list: resolving the runtime executable (arch §5.3), and the resume probes that check a conversation file exists (Claude transcripts, Copilot `session-state`, pi session files, agy's conversation database).

The launch spec therefore names machine-side files by role, such as the system prompt file or the hook feed, and the remote daemon fills in its own paths. Phase 1 leaves these steps where they are, since the daemon and the agent share a machine. The shell-launch tidy-up #795 described (no synthetic role for a terminal, no shell branches in resume, telling a shell apart from an unknown runtime key) can ride along with that reorganisation and gates nothing.

### The Windows PC as a remote machine (phase 4)

The Windows PC runs a crew slot for a Mac's mission: OpenSSH Server, `runnerd --stdio` on Windows, ConPTY, argv with no quoting layer, the `.cmd` first-turn paste running on the machine where [610](./archive/610-windows-hook-status.md) proved it, and the PowerShell hook reporters local to it. A Windows machine's own local daemon is part of phase 1.

## Design

Drawn on 2026-10-06 in `design/specs/645-session-host.pen`:

- The quit dialog: two choices, Keep them running and Stop them, then Cancel and Quit, with "Don't ask again".
- The "When Runner quits" row in Settings → General, with its menu open.
- The update dialog, with the note beside the buttons shown when agents are working.
- The notice after the app restarts the daemon into a different build.

Dropped in that review: a `runnerd` status row with a Stop button in Settings (⌥⌘Q and `runner daemon stop` cover it), the build-mismatch dialog (the app restarts the daemon itself), and a pane waiting for its snapshot (no slow snapshot has been seen; add it if one appears). The notice that `runnerd` stopped uses the app's existing error notice, with an Open log button after three crashes.

Phase 3's host picker and disconnected pane get designed with phase 3.

## Rules

- Agents see no change: argv, prompts and first turns are identical, and the environment differs only in the cleaned `PATH` described above.
- `events.ndjson`, `sessions` rows, MCP tools and CLI output keep their shape. The CLI gains only `runner daemon status` and `runner daemon stop`. After phase 1, the CLI moves onto the client protocol, and `mcp.sock` and the MCP server go, with the CLI's output unchanged ([plan](../impls/645-runnerd/plan.md#after-phase-1--the-cli-moves-to-the-client-protocol)).
- Tests never start a daemon in real app data. They use temporary app-data directories and endpoints, with every root passed in rather than resolved from `$HOME` (the lesson of the #648 skill leak).
- Every PR leaves both platforms working, with Windows-only paths covered on Windows CI.

## Non-goals

- Updates that keep agents running. An update restarts every session (decision 13).
- Surviving logout or reboot. Phase 1 has no login item or launchd agent; an opt-in start at login arrives with remote machines in phase 3, so a remote machine has a daemon waiting.
- A menu bar or tray item while the app is closed.
- Notifications while the app is closed, and a phone or web client. The protocol allows both later.
- Reading a remote machine's files from the app; Runner is not an ADE.
- Registering MCP servers or managing skills on a remote machine in v1.
- Sandboxing, federation between two daemons, and any hosted relay.
- Adopting alacritty's event loop (#709; see The terminal), and moving hook status to local IPC ([#797](./797-hook-status-ipc.md)). When #797 lands, its listener belongs in the daemon, which is now always running.

## Decisions

Proposed 2026-10-04 and reviewed with Jason on 2026-10-05. He settled 4 and 9 in that review and asked for the spec to be committed; the others stand unless he changes them. The branch, channel, crate and no-downgrade plans settled the same day are in the [plan](../impls/645-runnerd/plan.md).

1. **The daemon owns the state, not only the processes.** Two alternatives were weighed. (B) The issue's original split, in which the app owns state and a background process owns the PTYs. It has a smaller protocol, and an app update would not touch the agents. But nothing coordinates while the app is closed, the CLI still needs the app, and the boundary would have to carry a stream of draft observations from that process's `Term` to the app's delivery gate on every keystroke. (C) Keeping the app process alive in the background after its windows close. It is the cheapest, but every GPUI crash still kills every agent, and remote machines still need a session protocol. Jason's 2026-10-04 lesson from Paseo and Orca, that the state owner must run without the UI, picks this option.
2. **Exact build match, so an update restarts the daemon and its agents in v1.** The daemon is most of the backend, and every release changes it. A protocol kept stable across Runner's release pace is a discipline worth taking on once, for the smaller session protocol, not for every core request. Remote machines use that protocol (phase 3); a local update restarts the agents (decision 13).
3. **The authoritative terminal lives in the daemon, with a mirror in the app and a VT snapshot on attach.** The reasons are under The terminal.
4. **The daemon is `runnerd`, the bundled CLI binary under a second name** (Jason, 2026-10-05). It shows as its own process, and "host" stays free to mean a machine.
5. **The CLI starts the daemon when none is running.** This makes the CLI work with the app closed, the practical half of decision 1.
6. **The default quit behaviour is Ask,** with "Don't ask again" leading to the Settings row.
7. **0.13.0 ships phase 1 on both platforms.** Phases 3 and 4 follow in later releases and do not gate 0.13.0: remote machines (3), then the Windows PC (4). Phase 2 was dropped (decision 13).
8. **#795 closes when this spec lands,** with its launch steps carried by phase 3.
9. **#709 does not go first.** Jason closed it on 2026-10-05 as answered by this spec; its audit moves into 1b.
10. **Alacritty gets a read-only accessor patch** (Jason, 2026-10-05). The snapshot needs state alacritty keeps private. The patch is vendored during phase 1, and a fork follows only if the accessors go upstream (plan, Mission 1b).
11. **`runnerd` keeps running once started** (Jason, 2026-10-05). There is no idle exit; see Lifetime under Starting, finding and stopping the daemon.
12. **When the OS ends `runnerd`, it stops sessions the way Stop Sessions does,** so a reboot resumes them as today's quit at logout does.
13. **An update restarts every session, permanently** (Jason, 2026-10-06). Phase 2, a second local process that would have kept agents running through updates, is dropped. Agents resume their conversations after the restart, the update dialog shows when agents are working before their turns are lost, and every update reaches the whole stack. Phases 3 and 4 keep their numbers so earlier records still match.

## Implementation phases

The [implementation plan](../impls/645-runnerd/plan.md) has the detail: each mission's design, the files it touches, its verification, and the sequencing with the rest of 0.13.

**Phase 1: the local daemon, the 0.13.0 gate.** Four PRs, one mission each, in order, each leaving the app fully working. They merge into an umbrella branch, `feat/645-runnerd`, not into `main`. Each one ships on a separate `nightly-runnerd` channel. Jason's Mac install takes every cut and never moves back; problems are fixed forward. There is no downgrade guard; `runnerd` shuts down cleanly if an older app takes its sockets (plan, Branch and channel). The umbrella lands on `main` once, after Jason has daily-driven it (plan, Branch and channel).

1. **1a, one request surface.** Every app call into the core goes through `DaemonClient` over an in-process transport. Behaviour is identical.
2. **1b, the terminal moves below the session seam.** The daemon-side model, the app's mirror, frames and the snapshot, all still in one process. Behaviour is identical.
3. **1c, the daemon process.** `runnerd`, the socket, reattach, crash recovery, and the CLI starting it. Quit still stops sessions.
4. **1d, lifecycle.** The quit choice, the update dialog, the restart notice, and the docs.

**Phase 2: dropped** (decision 13). It would have kept agents running through updates.

**Phase 3: remote machines over ssh.** A session daemon on another machine, speaking a session protocol kept stable across releases, reached through `ssh <host> runnerd --stdio`, with the #795 launch steps and the host picker. Supersedes [510](./archive/510-remote-ssh-session.md).

**Phase 4: the Windows PC as a remote machine** for a crew slot.

## Risks

- **Orphaned agent processes,** the failure mode 466 feared. The daemon's lifetime rules, `runner daemon status` and `stop`, Stop sessions reaching every PTY, and the startup orphan sweep (kept as is) cover it. Verification checks the process list after every stop path.
- **A busy or frozen app stalling agents.** Bounded per-client queues and `Resync` cover it.
- **Snapshot fidelity,** the reason #157 was closed. A mode the serializer forgets shows up as a wrong screen after a relaunch. The round-trip and split-point tests over the fixture corpus catch the recorded cases. Anything unsupported, such as image protocols, is listed in the plan. Before 1b starts, the corpus gets fresh Claude Code and Codex recordings that include `/clear`, resize and the alternate screen.
- **macOS privacy prompts.** Today an agent's access to Documents, Desktop or a network volume is attributed to Runner.app. A daemon the app started should inherit that attribution, but a daemon the CLI started from Terminal or an agent is attributed to that app instead. This must be checked live, and the result written into the plan. The same check covers usage's Keychain read through `/usr/bin/security`.
- **Windows.** See the Windows section: ConPTY beside `runnerd.exe`, the pipe's security descriptor, refused breakaway, and the installer. Phases 3 and 4 never start a daemon over ssh (see Remote machines), so neither an ssh session's job nor its logon's missing credentials can reach an agent; phase 4 confirms both on the PC.
- **Request latency on the UI thread.** The 1a audit and the render rule cover it, and the verification includes typing latency.
- **Memory.** Two `Term`s per live session.
- **Each crew test that spawns a daemon is a process left behind if it fails.** Tests kill their daemon on drop and use temporary endpoints.

## Verification

Phase 1, on macOS and on Windows:

- Start a mission with Codex and Claude Code slots and a direct chat. Quit with Keep Running, wait through a turn, relaunch: the sessions are the same processes (same pids), every terminal shows the screen and scrollback it would have shown, the feed and cards are current, and nothing is delivered twice.
- With the app closed, a worker posts a message: the lead is nudged. A worker emits `ask_human`: the card appears on relaunch, and answering it delivers once. Answering instead with `runner mission answer` from a terminal while the app is closed also delivers once.
- An agent finishes or crashes while the app is closed: its exit status is right on relaunch.
- Quit with Stop Sessions behaves exactly like today's quit, including resume on the next launch, and leaves no agent process running.
- Force-quit the app mid-turn: the turn completes, and relaunch reattaches.
- Kill `runnerd` mid-turn: the app reports it and starts a new daemon, rows are demoted to stopped, and no agent process survives.
- Install an update with live sessions: the update dialog shows that an agent is working, the sessions resume after the relaunch with their conversations, and none is duplicated. Launching a manually installed build restarts them the same way and shows the notice.
- Run `make run` twice with a live development chat: the second build restarts the development daemon without asking and resumes the chat. The production app and the development app run side by side, each with its own daemon.
- With the app closed, `runner mission list` starts the daemon, `runner daemon status` shows it, it keeps running after the command and after the last session ends, its usage polling pauses while no app is connected, and `runner daemon stop` stops everything.
- Activity Monitor and Task Manager list the daemon as `runnerd`.
- In CI, every recording in `runner-terminal/fixtures` round-trips: replayed into one `Term`, serialized, and fed into a fresh one, it gives the same grid, scrollback, cursor and modes.
- **The split-point test,** in CI: for every recording, a snapshot is taken at every chunk boundary and at random byte offsets, including inside escape sequences and synchronized updates. The rest of the recording is then fed to both the original `Term` and the restored one, and their final grids, scrollback, cursor and modes must be identical. This is the test #157 never had.
- A TUI that queries the terminal gets exactly one answer, both while the app is attached and while it is closed.
- A typed draft still holds a crew delivery, as in the #791 smoke.
- **A terminal benchmark gates 1b and 1c,** run on both platforms against `main` before each change: keystroke-to-echo latency through a shell session (p50 and p99), throughput replaying a 50 MB output burst, and a drag-resize of a busy pane side by side with `main`. It passes when p99 echo latency is no more than 1 ms above `main`'s, burst throughput is at least `main`'s, and the resize shows no added stutter. For scale, a local Unix socket round trip measured 7 µs p50 and 27 µs p99 on Jason's Mac on 2026-10-05, at 1.2 GB/s, both through Python, so the real cost is lower. Windows named pipes are measured on the PC before 1c starts.
- An agent in a daemon the app started can read a file in `~/Documents` after the app quits, with no new prompt.
- The [full smoke test](../tests/full-smoke-test.md) passes before 0.13.0.

Phases 3 and 4, from the issue:

- A chat on a remote Mac: the full TUI, hook status including Approval needed, resume after relaunch, and survival through the laptop sleeping.
- A crew with one slot on the Windows PC: the launch prompt lands once, a `runner msg` round trip works, status comes from hooks, and a Bypass mission runs unattended.
- An older Runner on the remote: refused with "Update Runner on <host>", and nothing is spawned.

## Relevant code

- `crates/runner-app/src/bootstrap.rs`: `boot_core`, `NativeMcpServer`, `consume_resume_on_launch`, `stop_running_sessions_on_quit`; `main.rs`: `on_app_quit`, the updater setup, `wake::install`.
- `crates/runner-backend/src/lib.rs`: `AppCore`; `events.rs`: `AppEvent`, `EventChannel`; `ipc.rs`: `IpcListener`.
- `crates/runner-backend/src/session/runtime.rs`: `SessionRuntime`, `SpawnSpec`, `RuntimeOutput`, `OutputStream`; `session/pty_runtime.rs`: `PtyRuntime`, `cleanup_stale_running_rows_on_startup`, `cleanup_orphan_processes_on_startup`; `session/process/windows.rs`: the job object.
- `crates/runner-backend/src/session/manager/`: `SessionEvents`, `report_input_state`, `inject_stdin`, `inject_direct_stdin`, `kill_many`, the delivery gate.
- `crates/runner-terminal/src/terminal.rs`: `TerminalSession`, `TerminalBridge`, `feed_output`, `observe_parsed`, the `PtyWrite` reply path; `replay.rs` and `fixtures/` for the round-trip test.
- `crates/runner-backend/src/cli_install.rs`: `install_runner_cli`; `crates/runner-cli/`: the socket client and its exit codes.
- `crates/runner-backend/src/session/manager/spawn.rs`, `session/system_prompt.rs`, `shell_integration.rs`, `runtimes/*`: the #795 launch steps.
- [arch §5](../arch/arch.md#5-pty-session-runtime), §7.2, §8.5 and §11, [`concurrency.md`](../arch/concurrency.md), and the #647 process-model draft on branch `fix/647-terminal-black-sidebar` (`docs/arch/process-model.md`), which maps today's per-session threads.
