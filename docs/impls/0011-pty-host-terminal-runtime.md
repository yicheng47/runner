# In-Process PTY Terminal Runtime

## Context

The current terminal runtime uses tmux as both the process-survival
layer and the replay/snapshot source for xterm.js. The replay role is
the problem: `tmux capture-pane` returns a rendered text approximation
of the pane, and modern TUIs (claude-code especially) redraw on resize
in ways `capture-pane` can't faithfully reproduce. The stacking
screenshot on issue #150 is the visible symptom.

Two earlier design attempts went further:

- **PR #154** — route every resize through `capture-pane` + reset +
  replay. Didn't fix the stacking; closed.
- **PR #157** — host-side PTY sidecar with a headless emulator
  (`alacritty_terminal::Term` + `screen_to_ansi` + key translation +
  bracketed-paste handling). Compiled and passed tests, but the first
  review pass surfaced a seq race, an incomplete protocol contract,
  and missing mode restoration in the snapshot. The pattern of "every
  architectural layer must be correct across all of xterm's protocol
  surface" was the fragility signal. Closed.

This rewrite picks the smallest design that fixes the original
problem:

> Replace tmux with `portable-pty` in the **same** Tauri process.
> xterm.js is the terminal model. Agents die with Tauri; a manual
> Resume button respawns them via the existing `agent_session_key`
> flow.

There is no sidecar, no headless emulator, no IPC protocol, no host
serialization layer.

## Persistence guarantees

| Event                                  | Agent process | Tauri app | Webview | Conversation recoverable |
|----------------------------------------|---------------|-----------|---------|--------------------------|
| Webview reload (Cmd+R)                 | ✅            | ✅        | ❌      | ✅ (live stream resumes) |
| Tauri quit (graceful / crash / kill)   | ❌            | ❌        | ❌      | ✅ (`agent_session_key` resume) |

Cmd+R remounts the webview but keeps the Tauri main process and every
PTY it owns. The frontend resubscribes, raw bytes flow again from the
agent's next emit forward — xterm starts blank until the agent
repaints (which TUIs do on the next user interaction).

Every other "app went away" event kills the agents. Recovery is via
the existing `session_resume` flow — `--resume <uuid>` for
claude-code, `codex resume <uuid>` for codex once the key-capture
path lands. See `src-tauri/src/commands/session.rs:556` and
`src-tauri/src/session/codex_capture.rs`. Sessions surface as
`stopped` in the sidebar, the user clicks **Resume**, the backend
respawns the agent CLI with its prior conversation key, and xterm
renders the freshly-redrawn UI from the agent's own re-render.

**No auto-resume on app start.** Sessions surface as `stopped`; the
user decides which (if any) to resume. Matches the existing
`DirectSessionEntry` click contract
(`src-tauri/src/commands/session.rs:230`) and avoids surprise
wakeups firing N agents' worth of LLM context loads.

## Approach

```text
Runner UI (xterm.js)
  <-> Tauri SessionManager (existing)
  <-> PtyRuntime (in-process, this PR)
  <-> portable-pty
  <-> agent CLI
```

Same shape as today's `TmuxRuntime`: a `SessionRuntime` impl that
lives in the Tauri main process. The only thing changing is the
backend (`portable-pty` instead of tmux) and the contract
(`Replay` snapshots on `resume` go away).

PtyRuntime **owns**:

- agent process + PTY master fd (one per session, in
  `HashMap<session_id, SessionHandle>`),
- the reader thread that pumps raw PTY bytes into the existing
  `RuntimeOutput::Stream` channel,
- writer mutex for `send_bytes` / `paste`,
- resize ioctl,
- child kill on `stop`.

PtyRuntime **does not own**:

- any terminal-state mirror (no headless emulator, no `Term`, no
  `Processor`),
- any serialized snapshot (no `screen_to_ansi`),
- bracketed-paste wrapping for the user-input path — xterm.js wraps
  client-side based on its own mode tracking before calling
  `send_bytes`.

### Per-runtime clear-on-resize

The first cutover surfaced a UX regression: dragging the window edge
while a `claude-code` chat is open visibly stacks the prior frame on
top of the post-SIGWINCH redraw. Same root cause as the original
issue #150 stacking — claude-code's TUI repaints fully on SIGWINCH
and the old frame stays in xterm scrollback.

The fix is a small surgical hack in the frontend, gated on the
runner's `runtime` field:

- `RunnerTerminal` reads the runner's runtime kind via a prop
  (`runnerRuntime`) plumbed in from `RunnerChat` / `MissionWorkspace`.
- `SessionRow` carries `runtime` denormalized off the runner row so
  mission sessions can reach it without a second lookup.
- On the resize push path, before calling `session.resize(...)`, the
  terminal writes `\x1b[3J\x1b[2J\x1b[H` (hard clear: scrollback +
  visible region + cursor home) into xterm — **only** when the
  runtime is a known full-screen TUI agent (today: `claude-code`,
  `codex`). The SIGWINCH-driven repaint then lands on a clean
  buffer. Plain shells (`shell`, unknown runtimes) skip the clear
  and keep their scrollback intact.

Trade-off: TUI sessions lose their own xterm scrollback on resize.
That's fine — claude-code / codex use alt-screen-style full redraws
that have no meaningful inter-frame scrollback anyway. The user
gets a clean repaint on every resize, which is what they wanted.

This is the v1 answer to the "stacking on resize" complaint. It is
*not* the same surface as the headless-emulator path PR #157
attempted; this only writes a clear sequence to xterm at resize
time, no parser, no mode round-trip, no protocol expansion.

### Why no headless emulator

PR #157 tried to give the user visible state after reattach by
mirroring the agent's terminal in `alacritty_terminal::Term` and
re-serializing it on `Attach`. That introduced four correctness
problems in one review pass:

1. Two parsers had to agree (host's alacritty + frontend's xterm.js).
2. The serializer had to round-trip the screen *and* every user-facing
   mode (`APP_CURSOR`, `BRACKETED_PASTE`, mouse modes, focus, alt-
   screen, line-wrap, …).
3. The seq numbering had to be race-free across term-lock + atomic +
   subscriber-list boundaries.
4. `Key` + `Paste` had to be host-side translated to honor the agent's
   current mode bits.

Accepting "no reattach-state" deletes all four. The Cmd+R UX cost is
real (blank xterm until the agent's next emit) and we accept it. If
real usage shows the blank-xterm window is too jarring, v2 can add a
tiny ring buffer of recent raw bytes — pure UX patch, no emulator.

### Why no sidecar

The sidecar in PR #157's design was justified by "agents survive
Tauri quit/crash". Once we trade that away in favor of manual resume,
the sidecar buys nothing:

- Cmd+R doesn't kill the Tauri main process — in-process PTYs survive
  Cmd+R fine.
- All the IPC machinery (Unix socket, framed JSON, protocol crate,
  bundle externalBin, stage script) would exist purely to cross a
  process boundary that no behavior depends on.

In-process is also cheaper to debug, drops a build target, and
eliminates the seq / subscriber race surface entirely (the manager
holds the `OutputStream` directly, no IPC fan-out).

## Step 1: `PtyRuntime`

**File:** `src-tauri/src/session/pty_runtime.rs`

A `SessionRuntime` impl backed by `portable-pty`. Per session:

```rust
struct SessionHandle {
    runtime_id: RuntimeSession,         // existing identifier shape
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send>>,
    cols: AtomicU16,
    rows: AtomicU16,
    exit_code: Mutex<Option<i32>>,
    alive: AtomicBool,
    output_tx: mpsc::Sender<RuntimeOutput>,   // same channel SessionManager listens on
}
```

Trait method mapping:

| Method | PtyRuntime behavior |
|---|---|
| `spawn(spec)` | `native_pty_system().openpty(PtySize)` → `slave.spawn_command(CommandBuilder)`. Drop the slave immediately. Spawn the reader thread. Return the `(RuntimeSession, OutputStream)` pair. |
| `resume(session)` | The runtime is in-process and the session is already in the manager's registry → no work needed at the runtime layer. The contract changes here: we **do not** emit a `RuntimeOutput::Replay` first. The frontend already accepts the no-snapshot trade. |
| `send_bytes(session, bytes)` | `writer.lock().write_all(bytes)` |
| `paste(session, payload)` | Same as `send_bytes` — xterm.js wraps in `\x1b[200~`/`\x1b[201~` client-side based on its own mode bit. Rust-side callers that want bracketed paste prefix it themselves. |
| `send_key(session, key)` | Translate via a small Rust-side name table: `"Enter"` → `\r`, `"Escape"` → `\x1b`, `"C-c"` → `\x03`, `"C-d"` → `\x04`, `"Tab"` → `\t`, plus the arrows/Home/End/Function keys for completeness. Only names Rust callers (e.g., the first-prompt `paste`-then-Enter machinery) actually use need to land in v1; unknown names error. **Not** the full xterm.js `KeyboardEvent.key` space — xterm.js routes user typing through `send_bytes`. |
| `resize(session, cols, rows)` | `master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })` |
| `status(session)` | Read atomic `alive` + `exit_code`. |
| `stop(session)` | `killer.kill()`. SIGTERM-style; the reader thread observes EOF, marks `alive = false`, and emits the final `RuntimeOutput::Stream` empty + `RuntimeOutput::Exit` if/as the manager expects. |
| `capture_visible(session)` | v1 scope: return empty bytes. The paste-verification path (`inject_paste_with_verify` in `manager.rs`) currently uses this to readback the visible pane. With no host-side terminal state, we can't readback. Drop the verify-then-Enter chain in favor of always-argv-or-paste-then-Enter at the manager level. |

Reader thread, per session:

```rust
let mut buf = [0u8; 8192];
loop {
    match reader.read(&mut buf) {
        Ok(0) => break,                 // EOF
        Ok(n) => {
            if output_tx.send(RuntimeOutput::Stream(buf[..n].to_vec())).is_err() {
                break;                  // manager dropped the receiver
            }
        }
        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
        Err(_) => break,
    }
}
// Mark alive=false, try_wait for exit code, emit Exit, drop.
```

Dependencies added to `src-tauri/Cargo.toml` (cfg(unix)-gated to
match the existing tmux runtime; Windows keeps the tmux fallback
until a separate effort lands):

```toml
portable-pty = "0.9"
```

## Step 2: Lifecycle wiring

**Files:**

- `src-tauri/src/lib.rs` (startup hook, quit hook)
- `src-tauri/src/session/manager.rs` (startup DB cleanup)

### Tauri startup

1. **DB cleanup first.** Before mounting the UI:
   ```sql
   UPDATE sessions
       SET status = 'stopped', stopped_at = COALESCE(stopped_at, ?1)
       WHERE status = 'running';
   ```
   Defensive — covers the force-quit / crash path where the on-quit
   hook didn't run. Resumable rows surface in the sidebar with the
   existing `DirectSessionEntry.resumable` flag.
2. Build `SessionManager` with `PtyRuntime`. No reattach pass — the
   DB has zero `running` rows at this point.

### Tauri on-quit hook

`tauri::Builder::on_window_event` + `RunEvent::ExitRequested` /
`RunEvent::Exit`:

1. Walk the DB for `status = 'running'` direct-chat sessions.
2. For each, call `SessionManager::stop` (which routes to
   `PtyRuntime::stop` → `ChildKiller::kill()`).
3. Brief wait (~500ms total budget) for reader threads to observe
   EOF and emit `Exit`.
4. Mark rows `status = 'stopped'`, `stopped_at = now()`.
5. Let Tauri continue exit.

If the user force-quits before this hook runs, step 1 of the
**startup** cleanup catches it on next launch.

### Webview reload (Cmd+R)

- Tauri main process, PtyRuntime, every PTY all stay alive.
- Frontend remounts. For rows still marked `running`, the existing
  `DirectSessionEntry.status === "running"` branch calls
  `session_attach` → manager rewires its existing `OutputStream` to
  the new frontend listener.
- xterm starts blank; live bytes flow on the next agent emit.
- This is the **only** path where Attach actually has a live
  session to attach to.

### User clicks Resume

- Frontend calls `session_resume(session_id, cols, rows)`
  (`src-tauri/src/commands/session.rs:556`) — already implemented.
- `SessionManager::resume` reads `agent_session_key` off the row,
  builds the resume command (`--resume <key>` etc.), and calls
  `PtyRuntime::spawn` with the resulting `SpawnSpec`.
- The agent CLI re-renders its UI from its own conversation store.
  xterm receives the freshly-rendered frame as the first batch of
  bytes.
- Status flips back to `running`, `started_at` updates.

### Mission sessions

Out of scope for this PR. Mission coordination still depends on the
app-side router/event-bus; moving it host-side is a separate effort.
Mission rows behave like direct-chat rows for the lifecycle paths
above (stopped at quit, resumed by user click), but the router
doesn't auto-rewire for resumed mission sessions.

### Fallback flag

`RUNNER_SESSION_RUNTIME=tmux` keeps the tmux path alive during
cutover. Default flips to `pty` once the manual-test pass below
succeeds.

## Step 3: Remove tmux

**Files:**

- `src-tauri/src/session/tmux_runtime.rs` — delete (~1850 lines)
- `src-tauri/src/session/tmux.rs` — delete
- `src-tauri/src/session/mod.rs` — drop the tmux mods
- `src-tauri/Cargo.toml` — drop tmux-only deps if any
- `docs/impls/0004-tmux-session-runtime.md` — mark superseded by 0011
- `docs/impls/0009-terminal-alt-screen-reattach.md` — mark superseded
- `docs/impls/0010-terminal-replay-cols-alignment.md` — already
  shelved; delete in this step's landing commit if still present

After PtyRuntime parity for spawn / send_bytes / paste / resize /
stop / status / resume on direct chats:

- Stop constructing `TmuxRuntime` by default.
- Delete the tmux runtime module and its capture-pane / pipe-pane /
  send-keys plumbing.
- Update older docs to mark them superseded.

## Verification

### Unit tests

- `PtyRuntime::spawn` + `send_bytes` round-trip via `/bin/cat`.
- `PtyRuntime::resize` updates `MasterPty::get_size()`.
- `PtyRuntime::stop` causes child to exit and reader thread to
  observe EOF.
- Key-name translation table covers the names Rust callers use;
  unknown names error.

### Integration tests

- Spawn a `sh -c "exit 7"` session; verify `RuntimeOutput::Exit`
  with `exit_code: Some(7)` and DB transition to `stopped`.
- Two concurrent sessions don't cross output streams.
- Webview reload simulation (drop + reconstruct the frontend-side
  listener) preserves live `RuntimeOutput::Stream` delivery.

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
7. Sidebar shows the prior session as `stopped` with a Resume
   affordance.
8. Click Resume. Agent re-spawns, re-renders the prior conversation.
   ✓

### Regression checks

- `pnpm exec tsc --noEmit`
- `pnpm run lint`
- `cargo test --workspace`

## Non-Goals

- **Visible state on Cmd+R reload.** Deliberate. Blank xterm until
  next emit; ring buffer is a possible v2 patch.
- **Cross-Tauri-process agent survival.** Deliberately removed in
  favor of manual resume.
- **Mission router / event-bus migration.** Separate effort.
- **Windows support.** Defer until a portable-pty + child-spawn pass
  on Windows lands.
- **Persistent terminal scrollback across reattach.** xterm.js's
  in-memory scrollback during a single mount is enough.
