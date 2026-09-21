# 647 — Terminal runtime integration: evaluate Alacritty first

Tracking issue: [#647](https://github.com/yicheng47/runner/issues/647). Priority: P1. Status: draft, revised 2026-09-21; implementation has not started. Platforms: macOS and Windows.

## Motivation

With one Runner window open, Jason saw a terminal pane turn completely black after toggling the sidebar while running Codex on a remote server through SSH. Further toggles and window resizes did not restore it. Cursor visibility, whether input still reached Codex, and whether changing tabs recovered the pane remain unknown.

Investigation found a missing synchronized-update timeout in Runner's custom terminal runtime. Comparing the local Zed checkout shows a broader boundary to evaluate: Zed uses Alacritty's PTY and I/O event loop as well as its terminal model; Runner uses `portable-pty`, its own reader/forwarder/reply workers, and manual parser calls. Taking over that event loop also takes over its scheduling, I/O, and lifecycle obligations.

Jason's direction is to evaluate adopting Alacritty's PTY/event loop first, while preserving Runner's process cleanup, agent-status hooks, input routing, and output observations. This revision replaces the earlier plan to extend the current reply worker with timeout handling. The original SSH incident remains unconfirmed; a synthetic parser reproduction alone cannot establish that #647 is resolved.

## Evidence and limits

Runner currently pins `alacritty_terminal` 0.26.0 and `vte` 0.15.0. `TerminalSession::feed_output` advances a persistent `Processor`, which buffers bytes between `ESC[?2026h` and `ESC[?2026l`. The default timeout handler records a deadline, currently 150 ms after a begin or extension, but does not independently flush it. Alacritty's event loop services that deadline with `stop_sync` and a wakeup. Runner does neither. A recognized end sequence or the parser's buffer limit can release the buffer; a grid resize cannot.

A headless probe on 2026-09-21 populated a 100 × 30 grid, cleared it and hid the cursor before beginning a synchronized update, then buffered redraw text and a cursor-show command without an end sequence. After 250 ms it remained blank with 19 buffered bytes. Four resizes through 80, 120, 80, and 100 columns plus more ordinary output left 67 bytes buffered. Calling `stop_sync` restored text and cursor. A complete redraw succeeded at all 38 possible two-chunk split positions. This exercised the dependency's parser/grid, not Runner's full runtime or live Codex.

Codex uses synchronized redraws, also documented in [#492's captures](./archive/492-windows-terminal-output-latency.md). That does not explain why an end sequence would be absent in the incident; ordinary SSH fragmentation alone is insufficient. Remote inspection found Codex 0.155.0 and tmux 3.2a with four detached Codex sessions, but those sessions are not established as the failing session. Direct SSH versus tmux must be recorded during live validation.

## Architecture decision to establish

The preferred candidate is upstream Alacritty `tty` + `EventLoop` + one shared `Term` per live process, controlled by Runner's session layer and observed by GPUI. Evaluate it against the contracts below before replacing the production path. Start with the pinned upstream version so runtime integration and dependency upgrades are separable decisions.

The local Zed reference is `~/repos/gui/zed`, commit `e0931d5`; `crates/terminal/src/alacritty.rs` calls `tty::new`, constructs `EventLoop`, and exposes input, resize, and shutdown through its channel. That checkout uses Zed's fork at `4c129667ce56611becdc82de6e28218c80e2e88f`, labelled 0.26.1-dev. Record any required fork behavior separately instead of assuming upstream has it. Zed remains responsible for application event handling and its GPUI view; adopting the engine does not supply every terminal feature automatically.

The [GPUI rewrite plan](../impls/archive/gpui-rewrite/plan.md) deliberately retained `portable-pty` so `SessionManager` could own sessions independently of views. Preserve that ownership goal. It does not require the manager to implement terminal I/O itself: a session-owned Alacritty runtime can also outlive a pane. This spec reopens the engine choice, not mission/session ownership.

If Alacritty's native PTY exposes a concrete compatibility blocker, evaluate whether a small `EventedPty` adapter preserves its event loop while satisfying Runner's process requirements. That interface requires poll registration, I/O, resize, and child-exit behavior; wrapping `Read`/`Write` alone is insufficient. Retaining `portable-pty` with a custom loop is the fallback only after recording the blockers and the guarantees Runner would explicitly own and test. These are evaluation alternatives, not three permanent backends to ship.

## Ownership and compatibility contracts

| Responsibility | Owner after integration | Required behavior |
| --- | --- | --- |
| Session identity and policy | `SessionManager` | Keep DB rows, mission membership, spawn/resume/restart claims, agent keys, permissions, and router delivery rules. |
| PTY I/O, parsing, deadlines | Session-owned terminal engine | Exactly one PTY reader, parser, and authoritative live grid; progress without any attached view. |
| Process termination | Runner process supervision with engine child events | Preserve descendant cleanup, exit status, normal-exit draining, and stop/reap guarantees on both platforms. |
| User input and terminal replies | Runner routing into one engine write path | Keep user delivery gates and paste/Enter ordering; terminal protocol replies bypass user draft gates and are answered once in order. |
| Geometry | UI measures; runtime applies | Preserve initial/resume size selection, effective PTY/grid sizes, and settled size persistence. |
| Output and agent observations | Runtime adapters into session observers | Preserve ordered observations, status hooks, input-state tracking, first-paint/readiness signals, and fixture capture without parsing bytes twice. |
| Terminal events | Explicit application adapter | Handle or intentionally decline every event; retain title, palette/query, and color-scheme behavior. |
| Rendering and interaction | GPUI terminal surface | Observe the existing grid and modes, encode input, and request sizes; no parser timer or process lifetime depends on paint. |

### Resolve the crate boundary before wiring the engine

Today `runner-backend` owns the PTY and emits raw `RuntimeOutput::Stream`, while `runner-terminal` depends on the backend, consumes those bytes, and owns the `Processor`/`Term`. Simply making the backend depend on `runner-terminal` creates a cycle. The evaluation must show the proposed dependency graph, engine lifetime, event interfaces, and ownership of the shared grid. Keep the engine independent of GPUI and database access. Choose the smallest extraction or dependency change that achieves this; a new general-purpose framework is unnecessary.

`EventLoop` owns parser advancement internally. Identify concrete attachment points for raw-byte observation and post-parse state observation. A byte tap may observe the sole reader, but must not consume competing reads or feed a second live parser. Distinguish bytes received from state applied: synchronized bytes may arrive before first visible content, and a timeout may change the grid without another output chunk. Preserve sequence ordering and real receipt timestamps; document any intentional change in chunk granularity and update its consumers together. UI wake batching cannot be the only source of lifecycle or input-state correctness.

### Preserve process and input semantics

Prove how the selected PTY exposes the PID, foreground process group or Windows process handle, and termination/reaping signals that Runner needs. `Msg::Shutdown` is an I/O-loop command; do not assume it fulfills Runner's stronger stop contract. Define who reaps the child, drains final bytes and pending synchronized data, cleans descendants, and publishes completion so there is no double wait or premature stopped state.

Keep hook watchers and agent-status semantics outside terminal emulation. Preserve interrupt notifications, idle detection's resize grace, first-turn delivery, and direct versus mission input routing. Queuing bytes into Alacritty changes the meaning of a successful write relative to the current synchronous `write_all`: define acceptance, ordering, cancellation, and error reporting before relying on it in a manager delivery result. Queued input, replies, deadlines, and observations must belong to a particular process instance; an old instance must never target a resumed process just because the session ID was reused.

### Audit runtime guarantees and event policy

Compare Runner, pinned upstream Alacritty, and the local Zed integration for synchronized deadlines, partial writes/backpressure, read fairness, resize ordering, terminal replies, EOF/error handling, child exit, final-output draining, and shutdown. Classify each difference as an upstream guarantee, Runner adapter obligation, intentional product policy, or demonstrated defect. Zed parity alone is not the acceptance criterion.

Map every Alacritty event explicitly. Runner currently handles query replies, titles, and color-scheme interception but silently ignores other variants. Clipboard requests, bells, and cursor-blinking changes need a stated policy; do not automatically enable new clipboard behavior or treat every omitted feature as a cause of #647. Correct the tech note that inaccurately says `alacritty_terminal` has no PTY support.

## Scope

The work covers the terminal runtime boundary, the minimal session/terminal adapters it requires, regression coverage, and architecture documentation. It applies to shell tabs including SSH, local agent chats, mission slots, drawers, and hidden sessions on macOS and Windows.

Preserve the current UI, layouts, keybindings, terminal palettes, and externally visible session behavior. A renderer or glyph rewrite, session-host daemon, new persistence protocol, new terminal settings, and unrelated feature parity are outside scope. Preserve the bundled Windows ConPTY path, command quoting, environment/PATH composition, and process-tree handling. Do not alter remote server configuration as a workaround.

## Implementation Phases

### 1. Audit and record the baseline

Produce a concise architecture decision record under `docs/impls/` covering the ownership table, current dependency graph, differences from upstream/Zed, and named integration blockers. Record baseline results for terminal startup, queries, resize, exit/stop/resume, and representative local/SSH output. Inspect existing tests before adding cases; retain the guarantees they assert even when implementation-specific test helpers change.

Reduce the timeout probe into a durable regression: recovery must happen through a running runtime, without the test directly calling `stop_sync`. Keep the complete-frame/chunk-split control. Record time and resize events where needed; `RUNNER_RECORD_INPUT_FIXTURE` captures output timing but no resize events, while `Fixture::output_bytes` strips timing. A concatenated replay cannot prove runtime scheduling or resize behavior.

Exit criterion: the runtime contracts, baseline, and unresolved fit questions are concrete enough to judge a candidate implementation.

### 2. Prove Alacritty fits before production migration

Build a bounded development/test harness with upstream `tty::new` and `EventLoop`, a real disposable child, the shared grid, and Runner's proposed observation adapter. First validate deterministic child scripts for incomplete/complete redraws, query replies, slow input consumption, busy output, resizing, and exit. Exercise the proposed lifecycle adapter through Runner's session contracts, including stop with descendants and resume under the same ID. Validate startup and input with actual supported agent CLIs as a separate live check.

Run the platform-specific fit checks on native macOS and Windows. Windows checks include the bundled ConPTY DLL, batch-command first-turn paths, quoting, process handles/Job Objects, and output draining; macOS checks include signal masks, foreground process groups, and descendant cleanup. No platform is considered proven by compilation or results from the other platform.

The decision record must identify the selected engine, exact dependency version, raw-output and post-parse observation mechanisms, process supervision integration, dependency graph, and remaining obligations. Adopt the upstream engine if these contracts pass. If they fail, document the reproducible blocker and evaluate a small adapter or the custom-loop fallback against the same tests. A passing timeout demonstration alone is insufficient to choose or roll out the engine.

Exit criterion: an evidence-backed architecture choice and implementation plan, with no unresolved ownership, input-delivery, or process-cleanup gaps. This replaces the earlier assumption that adding a timer to the reply worker is the chosen implementation.

### 3. Integrate through one runtime per process

Implement the chosen boundary in reviewable steps: engine/lifecycle ownership, input and query routing, output/status adapters, then view attachment. Preserve the current manager-facing contracts where possible; change a contract explicitly if the fit work proves it necessary. Keep exactly one authoritative engine per live process throughout development. Historical fixture replay remains separate from live parsing.

The old implementation may remain available temporarily for development comparison, but do not run two engines against one PTY or silently switch an existing session. Define rollback as using the previous working build after a normal stop/restart, not hot-swapping a live parser. When the replacement is validated, remove the redundant production reader/parser/reply plumbing and `portable-pty` dependency if unused. Ship one selected path, with any required platform adaptations documented.

Keep critical locks out of blocking I/O and UI callbacks. Document lock order, event ordering, and shutdown ownership. Verify that completed or replaced engines release their threads/handles and that stale events cannot target a new process while a view still holds the old grid.

Exit criterion: session and terminal tests pass through the integrated path, the UI reads its grid, and legacy code removal leaves no competing terminal authority.

### 4. Validate behavior and the original incident

Run the verification matrix below and compare normal interaction and busy-output responsiveness against the baseline. Update `docs/arch/arch.md`, `docs/tech/alacritty-terminal.md`, and `docs/tech/terminal-rendering.md` with the final ownership and event flow. Record dependency/fork decisions and any intentional behavior differences.

Reproduce one-window macOS Runner → SSH → Codex, recording versions, direct execution versus tmux, idle versus streaming, and primary versus alternate screen. Toggle the sidebar repeatedly and resize the window. If it goes black, record cursor visibility, input response, tab-switch recovery, grid geometry, and synchronized-update state where observable. Add focused diagnostics at the selected runtime boundary if needed; avoid making hidden upstream parser internals a prerequisite for all logging.

Separate the runtime integration result from #647's resolution. If the migration and synthetic regression pass but the SSH incident cannot be reproduced or verified by Jason, leave the issue's resolution unconfirmed. A failure without a pending synchronized update needs its own evidence-based investigation.

## Verification

- [ ] The chosen dependency and ownership graph has one live reader/parser/grid, works without GPUI or a DB in the engine, and keeps sessions running across view detach/reattach.
- [ ] An incomplete synchronized redraw recovers after expiry without another byte or UI action. Test begin-only updates, delayed end sequences, valid extensions, ordinary output after expiry, and traffic that could starve deadlines. Complete updates preserve batching and apply exactly once across chunk splits.
- [ ] Queries held inside synchronized output receive exactly one ordered reply after completion/expiry, including with no viewer. Color overrides, size queries, and color-scheme subscription behavior remain correct. No query bytes appear in agent input fields.
- [ ] Input ordering, bracketed paste, paste/Enter delivery, interrupts, mission draft gates, and error reporting retain their documented semantics. Slow PTY consumers and busy output do not block the UI or corrupt partial writes.
- [ ] Spawn, natural exit, crash, explicit stop, spawn failure, and resume/restart retain correct status and agent keys. Final output is preserved, descendants are cleaned up, workers/handles are released, and delayed work cannot reach a replacement process or sibling mission slot.
- [ ] Hook-based status and baseline idle detection, resize grace, readiness/first-paint observations, and input-state tracking remain correct with hidden sessions and delayed synchronized output. Fixture/output observers preserve ordering without duplicate parsing or invented byte arrivals.
- [ ] Initial size, resume during a resize, sidebar toggles, split panes, and window resizes keep PTY/grid dimensions coherent. Verify primary-screen reflow, alternate-screen redraw, scrollback, selections, and attach/detach behavior.
- [ ] Native Windows validates bundled ConPTY, command/environment composition, first-turn delivery, process-tree cleanup, and shutdown/drain behavior. Native macOS validates foreground groups, signals, process cleanup, and local/SSH interaction.
- [ ] `cargo test --locked -p runner-backend -p runner-terminal -p runner-app`, workspace Clippy, and formatting checks pass. Both platforms run relevant runtime regressions. Live checks cover shell, local Codex/Claude Code, mission input, and SSH Codex, including idle and streaming resize cases.
- [ ] Baseline comparison, architecture decision, removed legacy paths, remaining limitations, and Jason's live #647 result are recorded separately. Cross-platform validation and process-lifecycle parity are rollout gates, not deferred follow-up work.

## Code references

- `crates/runner-backend/src/session/runtime.rs`, `pty_runtime.rs`, `process/`, and `manager/`: runtime contract, spawn/stop, process supervision, input gates, status hooks, output sequence, and resize persistence.
- `crates/runner-terminal/src/terminal.rs`, `input_state.rs`, `fixtures.rs`, and `replay.rs`: current parser/model ownership, replies, observations, view leases, and capture/test seams.
- `crates/runner-app/src/app_store.rs`, `terminal/element.rs`, and `terminal_resize.rs`: bridge installation, UI wake batching, grid rendering, and geometry measurement.
- Local Zed `crates/terminal/src/alacritty.rs` and `terminal.rs`: PTY/event-loop construction, input/resize/shutdown commands, and explicit event adaptation; reference revision and fork are recorded above.
- Upstream `alacritty_terminal` 0.26.0 `src/event_loop.rs`, `src/tty/`, and `src/event.rs`; `vte` 0.15.0 `src/ansi.rs`: I/O ownership, scheduling, child events, and synchronized-update semantics to verify.
