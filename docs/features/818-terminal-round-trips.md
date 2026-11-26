# Keep runnerd round trips off typing and terminal output

Tracking issue: [#818](https://github.com/yicheng47/runner/issues/818). No new screen and no visible change apart from speed. The change touches the client protocol's transport, runnerd's connection server and the session manager's terminal input, so its pull request carries `core-review`.

Line references are to `main` at `bf31c0ff` (2026-11-24). The issue's references predate the code moving around since 0.13.0; the behavior it describes has not changed.

## Motivation

Since #645 phase 1c, two terminal hot paths in the app wait on runnerd:

- **Every keystroke, typed text and paste waits for a reply before its bytes are sent.** It runs on the UI thread.
- **Every output frame of every live session costs a request.** That request asks for metadata that rarely changes.

The 1c benchmark passed (echo p99 0.38 ms on macOS, `docs/tests/archive/645-m3-daemon-process.md`), but it measured one quiet session over a Unix socket. Nobody has measured several sessions streaming at once, or Windows named pipes. The case this spec addresses is local: several sessions streaming at once. The remote-control routes, the hosted service and Tailscale ([#954](https://github.com/yicheng47/runner/issues/954)), do not take this path, because the remote window's transport answers `terminal_observe_input` itself. Running sessions on other machines over SSH ([#808](https://github.com/yicheng47/runner/issues/808)) moved to the P3 backlog on 2026-11-26; when it is designed, typing must not wait on a reply from the other machine, which this spec's one-way observation already provides.

## Current behavior

### Typing

1. `send_key_bytes`, `send_text` and `paste` call `observe_input` before `write_typed_bytes` (`crates/runner-terminal/src/terminal.rs:1068-1184`). `observe_input` is a synchronous `terminal_observe_input` request. `SocketTransport::call` blocks until the reply arrives or the 10-second `REQUEST_TIMEOUT` passes (`crates/runner-core/src/protocol/socket.rs:12`, `:621-640`). Only then are the input bytes sent, as a one-way `INPUT` frame.
2. In runnerd, the request is not handled on the connection's ordered input path. It joins the connection's request pool: 8 requests in flight, 64 queued, shared by everything the app asks on that connection (`crates/runner-daemon/src/daemon/server.rs:451-452`, `:1212-1240`). When the queue is full, the request is refused with "too many pending requests on connection". The client ignores the error, so the key still reaches the PTY, but the input tracker never sees it.
3. The worker takes the connection's `owners` lock (`server.rs:676-692`), then the manager-wide `terminal_control` lock and the session's state (`crates/runner-daemon/src/session/manager/terminal.rs:740-785`). It then calls `TerminalModel::observe_input`, which takes the model's `input_tracker` and `term` locks (`crates/runner-terminal/src/terminal/model.rs:648-657`). The output parser holds `parser`, `input_tracker` and `term` for the whole chunk it is parsing (`model.rs:377-379`). So a keystroke into a session that is printing waits for the current chunk. Meanwhile it holds `terminal_control`, which every other session's `desktop_input` also takes (`terminal.rs:729-738`).

The observation exists for the input tracker: draft-aware delivery (MIS-DRAFT-01), unsent-notice detection and input status. Its order relative to the bytes matters. Today the blocking call is what keeps the observation ahead of the bytes, because requests and input frames take different paths in runnerd.

### Output

1. The mirror's frame worker calls `refresh_metadata()` after every frame it applies (`terminal.rs:608`). That is a synchronous `terminal_metadata` request for title, live cwd, size and controller. In runnerd it locks the session's state and reads the model (`crates/runner-daemon/src/session/manager/terminal.rs:1226-1245`).
2. The bridge attaches a mirror to every live session, whether or not a pane shows it (`terminal.rs:1608-1613`). So every streaming agent issues one request per output frame on the app's one connection. Those requests compete with keystroke observations for the same 8 in-flight slots.
3. While the frame worker waits, frames pile up in runnerd's per-subscriber queue. At 64 frames the queue is cleared and replaced with `Resync` (`crates/runner-daemon/src/session/manager/terminal.rs:15`, `:118-131`). The mirror then reattaches and restores a full snapshot.
4. The metadata mostly reaches the mirror another way already:
   - **Title:** a title change raises `terminal_metadata_changed` for shells and `updated` for agents (`crates/runner-daemon/src/session/manager/terminal.rs:321-342`). Either becomes `session/updated`, and the bridge refreshes on it (`terminal.rs:1643-1647`).
   - **Size and controller:** these arrive in `Resized` frames (`terminal.rs:574-593`).
   - **Live cwd:** the shell's OSC 7 reports update it with no event (`model.rs:361-368`). The per-frame refresh is the only way it reaches the terminal tab labels (`crates/runner-app/src/app_store.rs:509`) and the start cwd of a new chat (`crates/runner-app/src/surfaces/start_chat/mod.rs:295`).

## Scope

In scope:

- A benchmark that measures typing under contending output, on macOS and Windows.
- Metadata that reaches the mirror when it changes, not after every frame.
- Key and paste observations sent one-way, ordered with their bytes, off the UI thread and outside the request pool.
- Moving the tracker update onto the session's own input worker, only if the measurement shows that one busy session still slows typing in another (phase 3).

Out of scope:

- IME composition (`InputEvent::Composing`) stays a request. It happens once per composition, not per key, and `set_composing` uses the reply to retry clearing after a handover (`terminal.rs:1104-1146`).
- The remote window's transport, which already answers `terminal_observe_input` locally (`crates/runner-app/src/surfaces/remote_client/transport.rs:130`).
- Remote control latency ([#1263](https://github.com/yicheng47/runner/issues/1263), [#1349](https://github.com/yicheng47/runner/issues/1349)).
- Events raised when a shell sets the same title again. That predates this issue.

## Design

### Output: metadata when it changes

- runnerd raises the existing `terminal_metadata_changed` (`crates/runner-daemon/src/session/manager/events.rs:285`) when a shell's OSC 7 report moves its live cwd to a different directory. A report of the same directory, which shells send at every prompt, raises nothing. The model tells its host through a new `TerminalHost` callback next to `set_live_title` (`model.rs:7`). The event is raised after the model's locks are released.
- The mirror stops calling `refresh_metadata()` after `Output` and `Resized` frames. It keeps refreshing:
  - after a `Resync` restore;
  - in `reattach` and `connect` (`terminal.rs:631`, `:706`);
  - on `session/updated`.

### Typing: the observation travels with the input

- A new one-way client frame, `wire::INPUT_EVENT` (20), carries a session ID and a JSON `InputEvent` (`crates/runner-core/src/protocol/wire.rs`).
- `Transport` gets `typed_input(session_id, input, bytes)`:
  - **`SocketTransport`:** sends `INPUT_EVENT` and then the `INPUT` chunks under the existing `one_way` lock, so no other input can come between an observation and its bytes.
  - **The managed transport** (`crates/runner-core/src/protocol/managed.rs`): forwards it.
  - **Default implementation:** does today's two steps, a `terminal_observe_input` call with its result ignored, then `input`. Test transports and the remote window's transport therefore keep working unchanged.
- `TerminalMirror::send_key_bytes`, `send_text` and `paste` call `typed_input` while the desktop controls the terminal. Its private `observe_input` helper goes away. `write_typed_bytes` keeps its count and wake.
- runnerd's connection loop routes `INPUT_EVENT` to the connection's input thread with `INPUT` and `RESIZE` (`server.rs:1295`). That thread runs `observe_terminal_input(session, connection, input)` (`server.rs:1099-1130`), then applies the bytes that follow. Frames arrive in order on one connection, so the observation is applied before its bytes, as it is today, without the wait. Errors are logged at debug level; the client ignores them today.
- The input thread already stops at the connection's cancellation, so nothing from a closed connection is observed after its cleanup. That is the guarantee the `owners` lock gives the request path today.
- The app and runnerd come from one build, and the hello exchanges the daemon's executable hash (`wire::Hello`). A daemon that does not know the new frame is replaced before the app talks to it.

### Phase 3, only if measured: observation on the session's own input worker

After phase 2, the connection's input thread still makes the tracker update wait for the session's current output chunk while it holds `terminal_control`. Typing into one quiet session can then wait behind an observation of a busy one, on the same connection or another. If the measurement shows this, the session's input worker (`start_terminal_input`, `crates/runner-daemon/src/session/manager/terminal.rs:842`) changes:

- Its queue carries both observations and bytes.
- The controller check stays under `terminal_control`, but the tracker update moves out of it, onto that session's thread.
- An observation queued before a handover or a `reset_input_state` must not be applied after it. The input reset guard (`input_reset_guard`) exists for this.

## Implementation Phases

0. **Measure.** Extend `socket_terminal_benchmark` (`crates/runner-cli/tests/daemon_process.rs:1315`, real daemon socket, `#[cfg(unix)]`) with a contended run: four sessions printing bounded continuous output while 2,000 single-character echoes are timed in a fifth. Report echo p50, p99 and maximum, the burst throughput, and the number of `Resync` frames the mirrors received. Add the same run over the Windows named pipe. Record a baseline from `main` on macOS and on the Windows PC in `docs/tests/runs/818-terminal-round-trips-both.md`.
1. **Output.** Raise the cwd change event and drop the per-frame refresh. Rerun phase 0.
2. **Typing.** Add `INPUT_EVENT` and `typed_input` and route the frame to the input thread. Rerun phase 0.
3. **Session-local observation**, only if phase 2's contended run shows typing in a quiet session slowed by a streaming one. Otherwise record the numbers that justify skipping it and leave it out.

Phases 1 and 2 can be separate commits in one pull request. Phase 3, if it happens, is its own commit.

## Verification

Automated:

- **Mirror:** applying `Output` and `Resized` frames issues no `terminal_metadata` request. A `session/updated` event refreshes the title and live cwd.
- **runnerd:** an OSC 7 report to a new directory raises one metadata change; the same directory again raises none.
- **Socket transport and runnerd:** `typed_input` sends the observation before its bytes. The input tracker sees the key before the echo reaches the model. Use the real daemon socket, as `crates/runner-daemon/src/daemon/server_tests.rs` and `crates/runner-cli/tests/daemon_process.rs` do.
- Existing input-tracker tests that intercept `Request::terminal_observe_input` (`crates/runner-app/src/terminal/control/tests.rs`, `crates/runner-app/src/terminal/input.rs`) keep passing through the default `typed_input`.
- New tests wait on the expected state, never sleep, and run 20 times under `cargo nextest run --stress-count 20` before handoff.

Benchmark (phase 0's run, before and after, on macOS and Windows):

- Quiet echo p99 stays within 0.1 ms of the baseline.
- Contended echo p99 and maximum go down.
- No `terminal_metadata` request is made while output streams.
- `Resync` count is no higher than the baseline.

Live checks: a neon trio's `qa`, in the development app. Typing, paste and metadata changes can all be exercised there.

| Check | Why |
| --- | --- |
| TERM-INPUT-01 | The path every key and paste takes changes. |
| TERM-IME-01 | Composition stays a request while the keys around it move to the new frame, so their order needs checking with a real IME. |
| TERM-INPUT-03 | Paste observations take the new path; the runtime's collapsed-paste draft must still be held. |
| MIS-DRAFT-01 | The input tracker is what the draft hold reads; it must still see typed text. |
| Typing under load | With four owned shells printing bounded continuous output, type a marker sentence into a fifth pane and into a Claude Code composer. Every character arrives once and in order, and no streaming pane repaints from a reattach. |
| Live cwd | `cd` into two owned scratch folders in a terminal tab. The tab's label follows each one, and a new chat started from that tab's folder starts there. |
| Shell title | `printf '\e]0;marker-818\a'` in an owned shell. The tab shows the title without further output. |

Docs: `docs/arch/arch.md` §5 and `docs/arch/process-model.md` describe input as one-way frames to a per-session queue. They gain the observation frame and say that metadata follows change events. No guide page changes.
