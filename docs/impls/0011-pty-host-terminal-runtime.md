# PTY Host Terminal Runtime

## Context

The current terminal runtime uses tmux as both:

1. the process-survival layer, and
2. the replay/snapshot source for xterm.js.

That second role is the problem. The latest Claude Code stacking screenshot
shows repeated banners and duplicated prompt panels after reopen. This is not a
frontend paint bug; it is a source-of-truth bug. `tmux capture-pane` gives
Runner a rendered cell approximation of a pane, not the original terminal event
stream. For modern TUIs that redraw on resize, especially Claude Code,
`capture-pane` can include prior redraws, resize artifacts, and main-screen
history that was never meant to be replayed as fresh terminal input.

Relevant current code:

- `src-tauri/src/session/tmux_runtime.rs:816` wires `pipe-pane`,
  `capture-pane`, and live FIFO streaming into one `OutputStream`.
- `src-tauri/src/session/tmux_runtime.rs:872` uses full scrollback
  `capture-pane` on reattach for main-screen panes.
- `src/components/RunnerTerminal.tsx:489` fetches the backend output snapshot,
  resets xterm, then writes the returned bytes.
- `src-tauri/src/session/manager.rs:1715` stores only a bounded in-memory
  output snapshot, so app restart relies on the runtime reattach path.

The most recent attempt to patch the existing architecture
(branch `fix/terminal-attach-resize-replay`, PR #154 — closed) routed every
resize through a tmux `capture-pane` + `term.reset()` + replay round trip.
It did not fix the stacking. Main-screen claude-code's SIGWINCH redraws
land at the current cursor position, and `capture-pane`'s text dump can't
distinguish "live frame" from "previous frame in scrollback". The dropped
PR is the final piece of evidence that the tmux-as-replay-source model is
unrepairable for modern TUIs — we have to own the byte stream end-to-end.

VS Code's integrated terminal architecture is closer to what Runner needs:
the app owns a PTY host process and the terminal byte stream. The display is
xterm.js, but tmux is not the replay source.

## Persistence guarantees

This is what survives what after the migration:

| Event                          | Agent CLI | PTY host | Tauri app | Webview |
|--------------------------------|-----------|----------|-----------|---------|
| Webview reload (Cmd+R)         | ✅        | ✅       | ✅        | ❌      |
| Tauri app crash / force-quit   | ✅        | ✅       | ❌        | ❌      |
| PTY host process death         | ❌        | ❌       | (n/a)     | (n/a)   |

The first two rows are the goal — they're what tmux gives us today; losing
either would be a regression. The third row is "agents die when their PTY
owner dies"; the host owns the master fd, so this is by construction.

The agents-survive-app-crash guarantee requires the sidecar to be
**detached** from the Tauri main process at boot, not a managed child. See
Step 2 for the mechanism.

## Resize-stack fix mechanism

Owning the byte stream is *necessary but not sufficient* for the
resize-stack fix. If the host just streams every raw byte to xterm and
faithfully replays the byte tape on attach, claude-code's repeated
SIGWINCH redraws still stack in xterm scrollback — the host has merely
preserved the original problem with higher fidelity.

The first draft of this doc proposed a "live-path scrollback clear" —
the host emits `ESC[3J ESC[H ESC[2J` to subscribers on every Resize
before forwarding SIGWINCH. That approach is rejected: plain shells
are also main-screen sessions with meaningful scrollback (e.g. the
output of an earlier `ls` or `cargo test`), and clearing it on every
window-edge drag is a real regression that no heuristic distinguishes
from claude-code's case at the wire level.

The fix lives in the **snapshot**, not the live byte stream:

- The host parses every PTY byte through a headless terminal emulator
  (`wezterm-term` — the embeddable parser inside the WezTerm terminal
  emulator; production-tested, designed-for-library use, broad and
  aggressively-updated escape-sequence coverage). The host wraps it
  with a small screen → ANSI walker that emits cursor positioning +
  SGR state + cell content per row, capturing the exact bytes needed
  to recreate the current visible state.
- The live `Output` events are still raw PTY bytes; xterm's behaviour
  on the live path matches Terminal.app / iTerm2 / any other PTY
  viewer. Claude-code's redraw-on-SIGWINCH still stacks in xterm
  scrollback during a live session — same as it does in any other
  terminal — and we deliberately do not fight that.
- `Attach` returns the headless emulator's serialized state, *not* the
  raw event tape. After an app restart or webview reload (the actual
  reported bug), xterm receives "exactly the visible region claude-code
  is rendering right now" instead of "the entire history of every
  frame claude-code has ever painted, including the stale ones".

This means the headless emulator is **v1 infrastructure, not a
deferred optimization**. The raw event tape is still kept on disk for
durability and post-mortem debugging, but the wire-level snapshot is
the parser's output. See Steps 3 and 5 for the parser integration and
the snapshot contract.

For alt-screen TUIs (codex), the parser tracks state correctly across
the `?1049h` / `?1049l` toggle and the snapshot covers the active
alt-screen frame — which is the entire visible UI for an alt-screen
app, so nothing meaningful is lost.

For plain shells, the v1 snapshot is the **visible region only** —
`screen_to_ansi` walks the cells the user can currently see, not the
scrollback rows above the viewport. That means a webview reload or
host-side reattach to a plain shell session loses pre-reload
scrollback (e.g. the output of an earlier `ls` that has since scrolled
out of view). This is a real fidelity trade vs. tmux's
`capture-pane -S - -E -` and is the one v1 regression in the migration.

The trade is deliberate. The naive fix — have `screen_to_ansi` walk the
scrollback grid too — would re-stack every historical SIGWINCH redraw
frame for main-screen TUIs like claude-code (the v2.1.143 case that
motivated this whole migration), and there's no wire-level signal that
distinguishes meaningful shell scrollback from stale TUI redraws.
Phase 2 work can revisit with a per-runtime flag or an OSC-based
annotation scheme; v1 ships with the visible-region-only contract and
documents it in release notes.

## Approach

Introduce a Runner-owned PTY host process:

```text
Runner UI (xterm.js)
  <-> Tauri session manager
  <-> runner-pty-host sidecar over local IPC
  <-> portable-pty
  <-> agent CLI
```

The PTY host becomes the source of truth for:

- spawning and stopping agent processes,
- stdin, paste, key, and resize delivery,
- live output events,
- durable raw terminal event logs,
- session status and exit codes.

The rule for this migration:

> Do not use `tmux capture-pane` as terminal replay again.

v1 ships a headless terminal model in the host (`wezterm-term` —
see "Resize-stack fix mechanism" above and Step 5's snapshot
semantics). On `Attach`, the host returns the headless model's
serialized current state via `screen_to_ansi`, not a replay of the
raw event tape. Frontend writes those bytes into a pre-sized xterm
and starts processing live events from `last_seq + 1`.

The raw event tape (`terminal.ndjson` — see Step 7) is kept on disk
for durability, debugging, and post-mortem audit, but it is **not**
on the attach replay path. Earlier drafts of this plan considered
raw-tape replay as the v1 approach with headless-model snapshots
deferred; that design is rejected because faithful tape replay would
re-stack every historical SIGWINCH redraw frame just like the failed
PR #154 did with `tmux capture-pane`.

---

## Step 1: Define the PTY Host Protocol

**File: `crates/runner-core/src/pty_host.rs`**

Add shared serde types for app <-> host IPC.

Core request types:

- `Spawn { spec: SpawnSpecWire }`
- `Attach { session_id }`
- `Input { session_id, data_base64 }`
- `Paste { session_id, data_base64 }`
- `Key { session_id, key }`
- `Resize { session_id, cols, rows }`
- `Stop { session_id }`
- `Status { session_id }`
- `List`

Core response/event types:

- `HostAck`
- `HostError { message }`
- `HostSessionStatus { alive, exit_code, pid, command }`
- `HostSnapshot { events, last_seq, cols, rows }`
- `HostEvent::Output`
- `HostEvent::Resize`
- `HostEvent::Exit`
- `HostEvent::RunnerStatus`

Terminal replay events should be explicit:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerminalReplayEvent {
    Output {
        seq: u64,
        // base64 of a terminal byte stream the frontend should write
        // verbatim into xterm. Origin varies by carrier:
        //   - on `HostEvent::Output` (live path): raw PTY bytes
        //     forwarded from the child;
        //   - on `HostSnapshot.events[0]` (attach path): synthetic
        //     bytes produced by the host's `screen_to_ansi`
        //     serializer over the headless `Terminal`'s current
        //     screen. See Step 5's snapshot semantics.
        data: String,
    },
    Resize {
        seq: u64,
        cols: u16,
        rows: u16,
    },
}
```

Why resize belongs in the replay tape: raw terminal output only replays
correctly if xterm sees the same geometry transitions the agent saw. Without
resize events, historical wrapping and absolute-positioned TUI redraws can
still drift.

Add protocol round-trip tests in `runner-core`.

## Step 2: Add the PTY Host Sidecar

**Files:**

- `src-tauri/src/bin/runner-pty-host.rs`
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`

Add a sidecar binary that runs independently of both the webview and the
Tauri main process. It should listen on a private Unix socket under app
data, for example:

```text
<app_data>/pty-host/runner-pty-host.sock
```

On startup:

- if invoked with `--detach`, the entry point performs a `setsid` +
  double-fork before any other work so its parent (the Tauri main
  process) reaping it does not propagate SIGHUP to the agent children;
- create the socket directory with owner-only permissions,
- acquire a lock file (`pty-host.lock` with `fs2::FileExt::try_lock_exclusive`)
  so only one host owns the socket,
- remove stale sockets only after proving no live host responds,
- load known session metadata/log directories,
- begin accepting one or more app connections.

The Tauri side resolves the bundled sidecar path explicitly — *not*
`std::env::current_exe()`, which would point at the main app binary
rather than the bundled sidecar binary. The sidecar is registered in
`tauri.conf.json` under `bundle.externalBin` with target-triple
suffixing (e.g. `runner-pty-host-aarch64-apple-darwin`), and Tauri's
`PathResolver` exposes it at runtime:

```rust
let sidecar_path = app
    .path()
    .resolve("runner-pty-host", BaseDirectory::Resource)?;
```

In dev (`cargo tauri dev`), Tauri's path resolver returns the path
under `src-tauri/target/debug/`; in a packaged bundle, it returns the
path inside `Runner.app/Contents/Resources/`. Invoke that path with
`--detach`; the app does not retain the child handle. The sidecar is
single-instance and discoverable by socket path. On subsequent app
launches, the existing host is reused if its lockfile proves liveness.

Initial platform scope: macOS/Linux. Windows can use named pipes later
and the `setsid` trick has no direct equivalent — defer until needed.

A note on macOS hardened runtime: the entitled production bundle may
restrict double-forking. If `posix_spawn` + `POSIX_SPAWN_SETSID` is the
only path that survives notarization review, swap the detach
implementation. Validate this during phase 2 before bundling.

Add dependencies:

- `portable-pty` for PTY creation,
- `wezterm-term` for the host-side headless terminal model
  (see "Resize-stack fix mechanism" and Step 5's snapshot semantics),
- `fs2` for the single-instance lockfile,
- no tmux dependency in the new path.

## Step 3: Implement Host-Owned Sessions

**Files:**

- `src-tauri/src/bin/runner-pty-host.rs`
- `src-tauri/src/session/launch.rs`

Each host session owns:

- `portable_pty::MasterPty`,
- child process handle,
- writer handle for stdin,
- reader thread for stdout/stderr PTY bytes,
- `wezterm_term::Terminal` (mirror of the agent's terminal state —
  see the resize-stack mechanism section above) with a
  `screen_to_ansi` serializer adjacent to it (see Step 5),
- current `cols` / `rows`,
- monotonic `seq`,
- raw terminal replay log file (for durability; not used on the
  attach replay path),
- subscriber list for connected app clients.

Reuse the existing launch-script machinery from `src-tauri/src/session/launch.rs`
so PATH, cwd, runner shims, and environment filtering stay consistent with the
current tmux runtime.

The host reader thread should:

1. read raw PTY bytes,
2. feed them through `wezterm_term::Terminal::advance_bytes(bytes)`
   to update the headless terminal state,
3. append `TerminalReplayEvent::Output` to the session log,
4. broadcast a live `Output` event to connected app clients (raw
   bytes, same as today's `pipe-pane` stream — frontend xterm
   behavior on the live path is unchanged),
5. feed the existing busy/idle detector logic currently in
   `src-tauri/src/session/tmux_runtime.rs`.

On resize:

1. resize the headless model first: `terminal.resize(TerminalSize { rows, cols, pixel_width, pixel_height })`
   (the `pixel_*` fields can be zero — wezterm only consults them
   for pixel-addressable escape sequences we don't emit) so the
   parser grid is ready to absorb the agent's SIGWINCH redraw at the
   new geometry. Snapshot fidelity depends on the headless model
   staying in lockstep with the child PTY — resizing only the master
   would leave the parser stuck at the old size and the next
   snapshot would be wrong;
2. then resize the PTY master: `master.resize(PtySize { rows, cols, ... })`
   so the child receives SIGWINCH;
3. append `TerminalReplayEvent::Resize` to the replay log;
4. broadcast resize if the frontend needs to mirror it.

On stop:

- terminate the child,
- close PTY handles,
- append/emit exit status,
- keep the replay log until explicit archive/delete.

## Step 4: Add a `PtyHostRuntime`

**Files:**

- `src-tauri/src/session/pty_host_runtime.rs`
- `src-tauri/src/session/mod.rs`
- `src-tauri/src/session/runtime.rs`
- `src-tauri/src/lib.rs`

Implement `SessionRuntime` by talking to the sidecar instead of tmux.

Mapping:

| `SessionRuntime` method | PTY host behavior |
|---|---|
| `spawn` | send `Spawn`, receive runtime handle, subscribe to live events |
| `resume` | send `Attach`, receive replay snapshot + live stream |
| `send_bytes` | send `Input` |
| `paste` | send `Paste` with bracketed-paste semantics in host |
| `send_key` | send `Key` |
| `resize` | send `Resize` |
| `status` | send `Status` |
| `stop` | send `Stop` |
| `capture_visible` | temporary compatibility path for paste verification; implement via host terminal model or a host-visible-screen readback in Step 8 |

For the first cut, store PTY host metadata in the existing runtime columns:

- `runtime = 'pty-host'`
- `runtime_socket = <socket path>`
- `runtime_session = <session_id>`
- `runtime_window = 'main'`
- `runtime_pane = <session_id>`

This avoids a migration during the initial cutover. A cleanup migration can
replace the tmux-shaped fields with a runtime JSON blob later.

Keep `TmuxRuntime` behind a fallback flag until the PTY host path proves out:

```text
RUNNER_SESSION_RUNTIME=tmux
```

Default should become `pty-host`.

## Step 5: Replace Snapshot Replay Semantics

**Files:**

- `src-tauri/src/session/manager.rs`
- `src-tauri/src/commands/session.rs`
- `src/lib/types.ts`
- `src/lib/api.ts`
- `src/components/RunnerTerminal.tsx`

Replace `session_output_snapshot -> Vec<OutputEvent>` with a structured
terminal snapshot:

```ts
interface SessionTerminalSnapshot {
  events: TerminalReplayEvent[];
  last_seq: number;
  cols: number;
  rows: number;
}

type TerminalReplayEvent =
  | { kind: "resize"; seq: number; cols: number; rows: number }
  | { kind: "output"; seq: number; data: string };
```

Frontend replay algorithm:

1. Register live listeners first.
2. Fetch `session_output_snapshot`.
3. Reset xterm.
4. **Resize xterm to `snapshot.cols` / `snapshot.rows` before writing
   any output.** The serialized bytes from the host's `screen_to_ansi`
   walker are positioned for the headless terminal's screen, which is
   the agent's view. Writing into a different grid (e.g. xterm's
   default 80×24) corrupts the replay with the same cols-mismatch
   that the failed PR #154 demonstrated. The top-level
   `cols`/`rows` fields on `SessionTerminalSnapshot` exist for this
   step; do not rely on per-event resize entries to land first
   (the v1 snapshot returns a single Output event).
5. For each replay event:
   - `resize`: call `term.resize(cols, rows)`;
   - `output`: decode base64 and `term.write(bytes)`.
6. Mark `last_seq`.
7. Flush pending live events with `seq > last_seq`.
8. Fit to the visible container and push one resize to the host
   (host updates `Terminal` + PTY master + appends `Resize` event).

This preserves the original terminal stream without asking tmux to
reconstruct it.

### Snapshot semantics (v1)

The snapshot returned by `Attach` is **the host's `screen_to_ansi`
serialization of the headless emulator's current state**, not the raw
event tape. See "Resize-stack fix mechanism" above for why — this is
the load-bearing choice that makes the reattach view correct at the
current geometry. It intentionally does not promise full shell scrollback
for plain-shell sessions in v1; only the visible terminal state is
reattached.

Concretely the host wraps the serialized bytes in a single synthetic
`TerminalReplayEvent::Output { seq: last_seq, data: <base64> }` and
returns it as the only entry in `HostSnapshot.events`, with
`HostSnapshot.cols` / `rows` set to the current `Term` geometry so the
frontend can pre-size xterm before writing the bytes (see Step 5's
replay algorithm, line 4). The frontend then writes the bytes, marks
`last_seq`, and flushes pending live events with `seq > last_seq`.

The raw event tape (`terminal.ndjson` — see Step 7) is still kept on
disk for durability, debugging, and post-mortem audit, but it is not
on the attach replay path.

#### Why `wezterm-term` for v1

- It's the embeddable parser inside [WezTerm](https://github.com/wezterm/wezterm)
  — production-tested in a major terminal emulator, and unlike
  `alacritty_terminal`, **designed as a reusable library from the
  start**. WezTerm's whole architecture is modular: `termwiz`
  (rendering primitives), `wezterm-term` (the terminal model),
  `wezterm-mux` (session multiplexing). The mux server uses
  `wezterm-term` exactly the way our pty-host will — that's the
  upstream reference design for this exact use case.
- Aggressive modern-protocol coverage: synchronized updates
  (`?2026h`), kitty keyboard / graphics protocol, OSC 8 hyperlinks
  (which claude-code already emits and our existing
  `RunnerTerminal.tsx` routes through `plugin-opener`),
  bracketed-paste, mouse reporting, application keypad / cursor.
  Wez Furlong tracks the modern spec aggressively; gaps close in
  weeks, not Alacritty's quarters.
- The whole point of this architecture migration is fidelity (the
  failed PR #154 was a fidelity failure). Picking the parser with
  the broader and more aggressively-updated protocol footprint
  upfront avoids discovering gaps in production after agents adopt
  new sequences.
- Cost: heavier transitive dep tree (`termwiz`, `wezterm-bidi`, etc.)
  than `alacritty_terminal`. The dep weight is contained inside the
  sidecar binary — the main Tauri app doesn't pull it. We accept the
  ~10 MB binary delta for a parser with first-class library posture.

The one shared cost with the alacritty alternative is that
`wezterm-term` does not ship a built-in "serialize the grid as ANSI"
helper (unlike `vt100`'s `Screen::contents_formatted()`). We write
that ourselves in a `screen_to_ansi(terminal: &Terminal) -> Vec<u8>`
adjacent to the parser: iterate `terminal.screen().lines()` row by
row, emit `\x1b[<r>;1H` to position the cursor, then for each
contiguous run of cells sharing the same SGR state emit one SGR
escape + the cell glyphs, finally restore alt-screen state and
cursor position from `terminal.get_mode()` and
`terminal.cursor_pos()`. Expect ~150 lines plus a focused unit test
that round-trips constructed screens through the serializer.

Keep the host's parser behind a small trait (`HeadlessTerminal` with
`process_bytes`, `resize`, `snapshot_ansi`) so the binding point is
mechanical if we ever need to swap.

## Step 6: Wire App Startup to the Host

**Files:**

- `src-tauri/src/lib.rs`
- `src-tauri/src/session/manager.rs`
- `src-tauri/src/session/pty_host_runtime.rs`

During Tauri startup:

1. connect to an existing host, or launch one if none responds;
2. construct `PtyHostRuntime`;
3. call `SessionManager::reattach_running_sessions`.

Reattach policy:

- Direct chats: attach to the host session and keep them running.
- Mission sessions: keep the current conservative policy at first unless the
  mission router/event-bus is also moved into the host. The terminal host can
  keep the PTY alive, but mission coordination still depends on the app-side
  router. Do not silently pretend a mission remained fully coordinated while
  the router was down.

This means direct-chat restart survivability lands first. Full mission
survivability is a separate follow-up: move mission router/event-bus ownership
into the host process or add a host-side router worker.

## Step 7: Disk Layout and Cleanup

**Files:**

- `src-tauri/src/bin/runner-pty-host.rs`
- `src-tauri/src/session/manager.rs`
- `src-tauri/src/commands/session.rs`

Use a per-session directory:

```text
<app_data>/pty-host/sessions/<session_id>/
  meta.json
  terminal.ndjson
```

`terminal.ndjson` contains `TerminalReplayEvent` rows. `meta.json` contains:

- session id,
- runner id,
- mission id,
- cwd,
- command/args summary,
- current cols/rows,
- started/stopped timestamps,
- last seq,
- exit status.

Cleanup rules:

- archive direct chat: stop host session if live, remove replay directory;
- runner delete: remove associated replay directories;
- mission archive/reset: remove associated replay directories;
- host startup: remove directories for sessions no longer present in DB.

## Step 8: Restore Paste Verification Without tmux

**Files:**

- `src-tauri/src/session/manager.rs`
- `src-tauri/src/session/pty_host_runtime.rs`
- `src-tauri/src/bin/runner-pty-host.rs`

Current first-prompt delivery uses `capture_visible` for paste verification
(`src-tauri/src/session/runtime.rs:356`). The tmux implementation reads the
visible pane with `capture-pane`.

For PTY host, implement one of these, in order:

1. Preferred: host maintains a lightweight terminal parser and exposes
   `VisibleText { rows }`.
2. Acceptable first cut: host records recent raw output and verifies paste
   placeholders from the event tape when the agent emits them.
3. Fallback: keep argv delivery as the dominant path and reduce paste
   verification scope, but do not call tmux.

If a parser crate is added, use it only inside the host. The frontend remains
xterm.js.

## Step 9: Remove tmux From the Default Path

**Files:**

- `src-tauri/src/session/tmux_runtime.rs`
- `src-tauri/src/session/tmux.rs`
- `src-tauri/src/session/mod.rs`
- `src-tauri/Cargo.toml`
- `docs/impls/0004-tmux-session-runtime.md`
- `docs/impls/0009-terminal-alt-screen-reattach.md`

(`docs/impls/0010-terminal-replay-cols-alignment.md` is deleted as part
of this plan's landing commit — its diagnosis is preserved here in
"Resize-stack fix mechanism" above.)

After PTY host validation:

- stop constructing `TmuxRuntime` by default,
- remove `capture-pane` replay from any app-start attach path,
- keep tmux fallback only if we still need a short migration window,
- update older docs to mark tmux replay as superseded by this plan.

Do this after, not before, the PTY host has parity for direct chat spawn,
resume, paste, resize, kill, archive, and app restart.

## Files to Modify

| File | Change |
|---|---|
| `crates/runner-core/src/pty_host.rs` | Shared host protocol and replay event types. |
| `src-tauri/src/bin/runner-pty-host.rs` | New sidecar daemon owning PTYs and replay logs. |
| `src-tauri/Cargo.toml` | Add sidecar binary and `portable-pty` dependency. |
| `src-tauri/tauri.conf.json` | Bundle or locate the sidecar. |
| `src-tauri/src/session/pty_host_runtime.rs` | New `SessionRuntime` implementation backed by the sidecar. |
| `src-tauri/src/session/runtime.rs` | Keep trait; adjust docs and snapshot/readback contracts. |
| `src-tauri/src/session/manager.rs` | Use PTY host runtime, structured snapshots, and host reattach policy. |
| `src-tauri/src/commands/session.rs` | Return structured terminal snapshots. |
| `src/lib/types.ts` | Add `SessionTerminalSnapshot` and `TerminalReplayEvent`. |
| `src/lib/api.ts` | Update `outputSnapshot` typing. |
| `src/components/RunnerTerminal.tsx` | Pre-size xterm to snapshot dims, then write the host's `screen_to_ansi` bytes; resume live events from `last_seq + 1`. |
| `src-tauri/src/session/tmux_runtime.rs` | Keep fallback initially; later remove from default path. |
| `src-tauri/src/session/tmux.rs` | Keep fallback initially; later remove from default path. |

## Verification

### Unit Tests

- Protocol serde round-trips for every request/response/event.
- Host replay log preserves monotonic seq across output and resize events.
- `Attach` snapshot bytes round-trip: feed a known PTY byte sequence
  into `wezterm_term::Terminal`, snapshot via `screen_to_ansi`, then
  feed those bytes into a second fresh `Terminal` and confirm both
  terminals' grid contents and modes match. Catches serializer
  regressions early.
- Snapshot bytes pre-sized correctly: `HostSnapshot.cols`/`rows`
  match the host `Terminal`'s current geometry at capture time.
- Snapshot plus pending live event merge never duplicates or drops
  events when the snapshot is followed by live events with seq
  > snapshot's `last_seq`.
- Host cleanup removes replay logs on archive/delete/reset.
- `PtyHostRuntime` maps `SessionRuntime` methods to host requests.

### Integration Tests

- Spawn `/bin/cat`, inject bytes, verify output event.
- Spawn shell command that exits, verify exit status and DB row transition.
- Resize session, then immediately `Attach`; verify
  `HostSnapshot.cols` / `rows` match the post-resize geometry and the
  serialized bytes in `events[0]` reflect the redrawn screen at the
  new dims. (v1 returns a single synthetic Output event plus
  top-level dims; there is no intra-snapshot Resize event to assert
  on.)
- Kill session, verify child exits and host forgets live handle.
- Restart Tauri-side manager while host remains alive; verify direct session
  reattaches without tmux.

### Manual Tests

1. Start a direct Claude Code chat.
2. Let the initial banner and prompt render.
3. Resize the window several times.
4. Quit the Runner UI and relaunch.
5. Reopen the chat.
6. Expected: no stacked Claude banners, no duplicated prompt panels, no
   staircase indentation.
7. Type into the chat after reopen.
8. Expected: input lands in the same live agent process.
9. Open two direct chats, switch between them while both remain mounted,
   and verify each visible terminal state stays correct without relying on
   hidden xterm panes.
10. Kill the host process manually.
11. Expected: Runner marks affected sessions stopped/crashed and does not
    show stale "running" state.

### Regression Checks

- `pnpm exec tsc --noEmit`
- `pnpm run lint`
- `cargo test -p runner --lib`
- Host integration tests, gated if they spawn real PTYs.
- Manual Claude Code restart repro from issue #150.

## Non-Goals

- Do not build full mission survivability in the first terminal-host PR.
  Mission coordination still needs router/event-bus ownership outside the UI
  process.
- Do not replay the raw event tape on `Attach` — the v1 snapshot path
  is the headless terminal's `screen_to_ansi` output. The tape exists
  for durability and debugging only.
- Do not use tmux control mode as a halfway solution; it still keeps tmux as
  the terminal owner.
- Do not add a UI redesign. This is runtime architecture, not product surface.
