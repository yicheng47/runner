# PTY Host Terminal Runtime

## Context

The current terminal runtime uses tmux as both the process-survival
layer **and** the replay/snapshot source for xterm.js. The replay role
is the problem: `tmux capture-pane` gives a rendered text approximation
of the pane, and modern TUIs (claude-code especially) redraw on resize
in ways `capture-pane` can't faithfully reproduce. The latest stacking
screenshot (issue #150) is the visible symptom.

Two earlier design attempts went further:

- **PR #154** — route every resize through `capture-pane` + reset +
  replay. Didn't fix the stacking; closed.
- **PR #157** — host-side headless emulator
  (`alacritty_terminal::Term`) + `screen_to_ansi` serializer + bracketed-
  paste handling + key-name translation. Compiled and passed tests, but
  the first review pass surfaced a seq race, an incomplete protocol
  contract for `Key` / `Paste`, and missing terminal-mode restoration in
  the snapshot. The pattern of "every architectural layer we add must
  be correct across all of xterm's protocol surface" was the fragility
  signal. Closed.

This rewrite picks a different point on the trade-off curve:

> The PTY host owns the agent process and forwards raw bytes.
> xterm.js is the terminal model. **We do not try to restore visible
> state across app restart or webview reload.**

## Persistence guarantees

What survives what, after the migration:

| Event                          | Agent CLI | PTY host | Tauri app | Webview |
|--------------------------------|-----------|----------|-----------|---------|
| Webview reload (Cmd+R)         | ✅        | ✅       | ✅        | ❌      |
| Tauri app crash / force-quit   | ✅        | ✅       | ❌        | ❌      |
| PTY host process death         | ❌        | ❌       | (n/a)     | (n/a)   |

Process-survival guarantees match what tmux gives us today (and what
PR #157 promised). **Visible state on reattach is explicitly NOT a goal
in v1.** After a webview reload or app restart, xterm starts blank; the
agent is still running and bytes will appear once it next emits. TUIs
that redraw on user interaction (claude-code, codex, vim) repaint
quickly. Plain shells stay blank until the user hits Enter.

The agents-survive-app-crash guarantee requires the sidecar to be
**detached** from the Tauri main process at boot, not a managed child.
See Step 2.

## Approach

```text
Runner UI (xterm.js)
  <-> Tauri session manager
  <-> runner-pty-host sidecar over local IPC
  <-> portable-pty (PTY allocation + child spawn)
  <-> agent CLI
```

The PTY host **owns**:

- agent process and PTY master fd,
- stdin / paste / key forwarding — verbatim from xterm.js, no host-side
  translation,
- resize forwarding — `MasterPty::resize` ioctl, nothing else,
- live raw-byte broadcast to subscribed clients,
- session status (alive / exit code).

The PTY host **does not own**:

- any terminal-state mirror (no headless emulator, no `Term`,
  no `Processor`),
- any serialized snapshot (no `screen_to_ansi`),
- any key-name translation table — xterm.js converts
  `KeyboardEvent.key` to bytes before sending,
- any bracketed-paste wrapping — xterm.js wraps client-side based on
  its own mode tracking.

The rule:

> Bytes in from agent → bytes out to xterm. Bytes in from xterm →
> bytes out to agent. The host does not interpret either direction.

### Why no reattach-state

PR #157 tried to give the user visible state after reattach by
mirroring the agent's terminal in `alacritty_terminal::Term` and
re-serializing it on `Attach`. That introduced a class of correctness
problems:

1. Two parsers had to agree (host's alacritty + frontend's xterm.js).
   Any mismatch in protocol coverage produced visible drift.
2. The serializer had to round-trip the screen *and* every user-facing
   mode (`APP_CURSOR`, `BRACKETED_PASTE`, mouse modes, focus reporting,
   alt-screen, line-wrap, …).
3. The seq numbering had to be race-free across the term-lock + atomic
   + subscriber-list boundaries.
4. `Key` + `Paste` had to be host-side translated to honor the agent's
   current mode bits.

Each layer needed to be correct individually *and* in composition with
the others. The first review pass produced findings on three of them.
The fourth was probably next.

Accepting "no reattach state" deletes all four problems. The trade is
real (a webview reload leaves the user staring at a blank xterm for a
moment until the agent next emits) and we accept it. If real usage
shows the blank-xterm window is too jarring, v2 can add a tiny ring
buffer of the last few KB of raw bytes — *purely a UX patch*, no
emulator, no serializer.

## Step 1: Define the PTY Host Protocol

**File:** `crates/runner-core/src/pty_host.rs`

Requests:

| Op | Payload | Response |
|---|---|---|
| `Spawn` | `SpawnSpecWire` | `Spawned { session_id, pid }` |
| `Attach` | `session_id` | `Ack` (registers subscription; **no snapshot**) |
| `Input` | `session_id, data_base64` | `Ack` |
| `Resize` | `session_id, cols, rows` | `Ack` |
| `Stop` | `session_id` | `Ack` |
| `Status` | `session_id` | `SessionStatus` |
| `List` | (none) | `Sessions` |

Push events:

| Kind | Payload |
|---|---|
| `Output` | `session_id, seq, data_base64` — raw PTY bytes |
| `Exit` | `session_id, seq, exit_code` |

`HostMessage { type: response | event }` envelope at the top level so
the Tauri side routes without inspecting inner tags.

`seq` is per-session monotonic. The frontend uses it for ordering;
receive-out-of-order isn't expected on a single socket, but the seq
makes the contract explicit and gives the Tauri side a knob for
duplicate detection during a brief reconnect window.

**Explicitly dropped from PR #157's protocol:** `Paste`, `Key`,
`HostSnapshot`, `TerminalReplayEvent`, `RunnerStatus`. xterm.js
owns the user-input translation surface; the host doesn't see
keys or pastes as anything other than `Input` bytes.

## Step 2: Sidecar Daemon + Sessions

**File:** `src-tauri/src/bin/runner-pty-host.rs`

Daemon scaffold (mostly inherited from PR #157's design, which got
that piece right):

- `--detach`: setsid + double-fork before bind; macOS hardened
  runtime may force a swap to `posix_spawn` +
  `POSIX_SPAWN_SETSID` — validate in the bundling pass.
- `--socket-dir <PATH>`: directory for the lockfile + Unix socket.
- `fs2::FileExt::try_lock_exclusive` on `pty-host.lock` — second
  invocation exits cleanly so the Tauri startup connects to the
  existing host.
- 0o700 dir / 0o600 socket perms.
- Stale-socket liveness probe before unlink.
- 4-byte big-endian length-prefixed JSON frames, capped at 16 MiB.

Per-session ownership:

- `portable_pty::native_pty_system()` for PTY allocation.
- `MasterPty` (the master fd) wrapped in `Mutex` for the resize
  ioctl.
- `Child` handle (`Box<dyn Child + Send + Sync>`) from
  `slave.spawn_command(CommandBuilder)`. The slave is dropped
  immediately after spawn to release our side of the pair.
- Reader thread per session: `try_clone_reader()` → blocking read
  into an 8 KiB buffer → base64-encode → broadcast
  `HostEvent::Output` to subscribers.
- Writer: `take_writer()` → `Mutex<Box<dyn Write + Send>>` for input.
- Resize: `master.resize(PtySize { rows, cols, .. })`.
- Stop: `child.clone_killer()` returns a `Send`-able `ChildKiller`;
  call `kill()` on that from the dispatch thread.

There is **no `Term`, no `Processor`, no `FairMutex` around
terminal state**. The reader thread's only state is the master fd
and a seq counter.

`SpawnSpecWire` keys the host trusts verbatim:

```rust
struct SpawnSpecWire {
    command: String,
    args: Vec<String>,
    cwd: Option<String>,
    env: BTreeMap<String, String>,
    cols: u16,
    rows: u16,
}
```

Env / cwd filtering is the Tauri side's job (`session::launch`).
The host doesn't second-guess it.

## Step 3: `PtyHostRuntime`

**Files:**

- `src-tauri/src/session/pty_host_runtime.rs`
- `src-tauri/src/session/runtime.rs` (existing trait; lightly
  re-document for the no-snapshot contract)
- `src-tauri/src/session/manager.rs`

`SessionRuntime` impl that talks to the sidecar over the local socket:

| Method | Host op |
|---|---|
| `spawn` | `Spawn` |
| `resume` | `Attach` — subscribes to live `Output` stream, no snapshot returned |
| `send_bytes` | `Input` — verbatim |
| `paste` | `Input` — xterm.js sends bracketed-paste-wrapped bytes when its mode is set, the host doesn't care |
| `send_key` | `Input` — frontend translates `KeyboardEvent` → bytes; the host stays neutral |
| `resize` | `Resize` |
| `status` | `Status` |
| `stop` | `Stop` |
| `capture_visible` | Paste verification's legacy contract. v1 scope: argv-only delivery; this method returns empty and paste verification falls back to argv. Revisit if a real need surfaces. |

Fallback flag: `RUNNER_SESSION_RUNTIME=tmux` keeps the tmux path
alive during cutover. Default becomes `pty-host` once the
manual-test pass in Step 4 succeeds.

## Step 4: Wire Startup + Reattach Policy

**Files:**

- `src-tauri/src/lib.rs`
- `src-tauri/src/session/manager.rs`
- `src-tauri/tauri.conf.json` (register sidecar in `bundle.externalBin`)
- `scripts/stage-runner-pty-host.mjs` (mirrors `stage-runner-cli.mjs`)
- `package.json` (wire stage script into `tauri:before:dev` and
  `tauri:before:build`)

At Tauri startup:

1. Locate sidecar via
   `app.path().resolve("runner-pty-host", BaseDirectory::Resource)?`
   — **not** `std::env::current_exe()`.
2. Launch with `--detach --socket-dir <app_data>/pty-host`.
3. Connect to the resulting socket, build `PtyHostRuntime`.
4. `reattach_running_sessions`:
   - **Direct chats:** send `Attach` per known session, subscribe to
     the live `Output` stream. xterm renders blank; the agent's next
     emit fills it.
   - **Mission sessions:** keep the conservative policy until the
     router/event-bus also moves host-side. Don't pretend the
     mission stayed fully coordinated while the router was down.

Webview reload (Cmd+R):
- Tauri main process and sidecar both stay alive.
- Frontend remounts, calls `Attach` per session.
- xterm starts blank, live bytes flow.

Tauri app restart (force-quit + relaunch):
- Sidecar stays alive (detached at boot).
- New Tauri process discovers the existing host via the lockfile.
- Same reattach flow as above.

## Step 5: Remove tmux

**Files:**

- `src-tauri/src/session/tmux_runtime.rs` — delete
- `src-tauri/src/session/tmux.rs` — delete
- `src-tauri/src/session/mod.rs` — drop the tmux mod
- `src-tauri/Cargo.toml` — drop tmux-only deps
- `docs/impls/0004-tmux-session-runtime.md` — mark superseded
- `docs/impls/0009-terminal-alt-screen-reattach.md` — mark superseded

After PtyHostRuntime parity for spawn / resume / input / resize /
kill / archive / app restart:

- Stop constructing `TmuxRuntime` by default.
- Remove `capture-pane` from any attach path.
- Delete tmux session-management code.
- Update superseded-by markers on the older impls.

## Verification

### Unit tests

- Protocol round-trips for every request / response / event.
- `PtyHostRuntime` method-to-op mapping.
- Subscriber retain-on-broadcast-failure correctly garbage-collects
  closed connections.

### Integration tests

- Spawn `/bin/cat`, inject bytes via `Input`, observe matching
  `Output` event.
- Spawn `/usr/bin/false` (or `sh -c "exit 7"`); verify `Exit` event
  with `exit_code: Some(7)` and DB row transition to `stopped`.
- Restart Tauri-side manager while host stays alive; verify each
  direct-chat session `Attach` succeeds and live stream resumes.

### Manual tests

1. Start a direct claude-code chat, let the banner render.
2. Type a prompt, get a response — confirm input + output flow.
3. Resize the window several times. Claude-code's SIGWINCH redraw
   lands in xterm; expect the same stacking-during-live-session
   behavior as Terminal.app / iTerm2 (we deliberately do not fight
   this).
4. Cmd+R reload. xterm goes blank.
5. Type "hi" — claude-code receives, responds, output flows.
6. Force-quit the Tauri app. Relaunch.
7. Open the same direct chat — xterm blank, agent still alive.
8. Type again — agent responds. ✓

### Regression checks

- `pnpm exec tsc --noEmit`
- `pnpm run lint`
- `cargo test --workspace`

## Non-Goals

- **Visible state on reattach.** Deliberate. See "Why no reattach-state".
- **Mission router / event-bus host-side migration.** Mission
  coordination still depends on the app-side router today; moving
  it into the sidecar is a separate effort.
- **Windows support.** Defer until a named-pipe + child-spawn
  equivalent of the detach mechanism lands.
- **Persistent terminal scrollback across reattach.** xterm.js's
  in-memory scrollback during a single mount is enough; agents that
  need history paint it themselves on user interaction.
- **Recovering visible state via a ring buffer of recent bytes.**
  Not in v1; if usage feedback demands it, that's a small v2 patch
  (bounded `Vec<u8>` per session, replay-on-Attach).
