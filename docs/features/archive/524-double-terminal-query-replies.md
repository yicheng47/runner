# Answer each terminal query once

Tracking issue: [#524](https://github.com/yicheng47/runner/issues/524). Status: **shipped 2026-09-08 in [#526](https://github.com/yicheng47/runner/pull/526)**, merged after the 0.8.4 tag and released in 0.8.5. Priority P1: a 0.8.4 regression on Windows that puts stray text in every Codex and Claude Code composer. Both platforms change; macOS is affected invisibly.

## Motivation

Since 0.8.4 shipped the Windows Terminal ConPTY (#492, [spec](./492-windows-terminal-output-latency.md)), a resumed Codex chat on Windows opens with `[6c` in its composer, sometimes the longer `[6c]10;rgb:dcdc/dcdc/e0e0\]11;rgb:1515/1616/1b1b\`, and a resumed Claude Code chat opens with `C`. The user has to delete the text before typing. The bytes are Runner's own replies to the terminal queries the CLI and the ConPTY host send at startup, arriving a second time after the CLI has stopped waiting for them.

## Findings

### Runner answers every startup query twice

Two independent responders answer the same query:

- **The PTY reader's canned responder.** `TerminalQueryResponder` in `crates/runner-backend/src/session/pty_runtime.rs` scans the first 8 KiB of child output for OSC 10 and 11 color queries, DSR (`ESC[6n`), and DA1 (`ESC[c`, `ESC[0c`), and writes fixed replies (Runner's dark foreground and background, `ESC[1;1R`, `ESC[?1;2c`) straight to the PTY writer from the reader thread. It was added for #213 in the Tauri era, when a mission slot's xterm.js pane buffered its output until the user opened the tab, so nothing answered Codex's OSC 11 background probe in time and the composer painted black. The #213 note assumed the front end answering again later was harmless because "codex honors the first reply and ignores the rest".
- **The terminal itself.** `TerminalSession::attach_with_input_mode` in `crates/runner-terminal/src/terminal.rs` runs an event thread that turns alacritty's `Event::PtyWrite`, `Event::ColorRequest`, and `Event::TextAreaSizeRequest` into `inject_stdin` writes, with the live palette (`query_color_for`) and the real cursor position. Since the GPUI rewrite, `TerminalBridge::spawned` creates this terminal for every session at spawn and `TerminalBridge::output` feeds it every chunk whether or not a pane shows it, so the #213 gap no longer exists: hidden mission slots are answered by the terminal within the same forwarder hop as visible ones.

The reader thread's canned reply always lands first; the terminal's reply follows a few hundred microseconds later.

### The inbox conhost hid the duplicate, OpenConsole forwards it

The standalone probe under the ignored `target/double-reply-probe/` (`probe.exe <canned|term|both|none> <seconds> <cmd…>`; real `codex.exe` and `claude.exe`, 149 × 49, both responders modelled on Runner's, replies logged with timestamps, the alacritty screen dumped at the end) was run with `conpty.dll` and `OpenConsole.exe` from `target\debug` beside one copy and nothing beside another:

| ConPTY host | responders | Codex composer | Claude Code composer |
| --- | --- | --- | --- |
| OpenConsole 1.24 (0.8.4) | canned + terminal, Runner today | `› [6c` | `❯ C` |
| OpenConsole 1.24 | canned only | clean | not run |
| OpenConsole 1.24 | terminal only | clean | not run |
| inbox conhost (0.8.3) | canned + terminal | clean, twice | not run |

`codex resume --last` under OpenConsole with both responders reproduces the screenshot from the report byte for byte.

The host changes what reaches the terminal side. In the probe's runs the inbox conhost sent only its own DSR to the terminal and swallowed Codex's DA1 and OSC 10/11 (with a `cmd.exe` child, as in the backend's batch tests, it also sends a DA1 of its own); the duplicate replies were a second `ESC[1;1R` and `ESC[?6c` that conhost consumed itself. OpenConsole sends its own DA1 at 8 ms, before the client prints anything, forwards the client's OSC 10/11 (Codex at 150 to 350 ms) and DA1 (Claude Code at 330 ms and again at 3.7 s), and returns every reply to the client as input. The client consumes the first reply; the second `ESC[?6c` and the second color reports go through conhost's input parser, which drops the ESC and the private marker, and the rest is typed into the composer.

On macOS the PTY forwards every query, so both responders have been writing there since the GPUI cutover. No stray text has been reported on macOS and it was not measured here; the likely reason is that the CLIs' own input parsers drop an unrecognised CSI or OSC on read, where conhost's input parser passes the remainder through as characters.

### The host's own handshake must be answered, or the client stalls

Both ConPTY hosts send a DSR, and depending on the client a DA1, to the terminal before the child prints anything, and hold the child until the reply arrives or an internal timeout passes. In the probe's `none` mode Codex's own color queries moved from 150 to 350 ms after spawn to 3.2 s. In the app the terminal answers the host within a millisecond of the first chunk, so no session ever waits. Two backend tests (`windows_batch_first_turn_*`) spawn a real ConPTY with no terminal attached and had been relying on the canned responder to answer that handshake; with it gone, the 3 s stall crossed the test-profile first-turn deadline and the first turn arrived at the child "before readiness". They now answer the handshake themselves, standing in for the terminal. Four more (`spawn_exit_seven_records_exit_code_windows`, `spawn_batch_roundtrips_arguments_windows`, `spawn_emits_idle_after_silence_and_busy_on_more_output_windows`, `foreground_process_tracks_job_and_stop_reaps_child_windows` in `pty_runtime.rs`) spawn a bare runtime the same way and kept passing on JASONPC, where the Windows 11 conhost releases the child after its internal timeout; on the CI image (`windows-2025`) the inbox conhost held the child past every deadline, and the batch test's only output in 15 s was the host's `ESC[6n`. They now use the same stand-in.

Resume is where the user sees it because nothing else touches the composer afterwards. On a fresh Windows spawn of a `.cmd` runner the first turn is pasted and submitted after the TUI-ready signal (`deliver_windows_batch_first_turn`), which would carry the stray bytes into the prompt; the argv first-turn path leaves the composer as it is. Neither was measured.

## Behavior

- **The terminal is the only responder.** `TerminalQueryResponder`, `terminal_query_patterns`, `find_subsequence_positions`, `answer_terminal_queries`, the four `DEFAULT_*` / `DSR_*` / `DA1_*` reply constants, `TERMINAL_QUERY_STARTUP_BUDGET`, `TERMINAL_QUERY_TAIL`, the `query_responder` argument to `reader_thread`, and the four `terminal_query_responder_*` tests are deleted from `pty_runtime.rs`. The reader thread only forwards bytes.
- **Every query is answered from the live terminal state.** OSC 4/10/11/12 report the session's current palette, light or dark, including runtime overrides; DSR reports the real cursor position; DA1 stays alacritty's `ESC[?6c`; XTWINOPS size reports keep working. This is the existing event-thread path, unchanged.
- **Hidden panes are still answered.** The terminal answers from `TerminalBridge::output`, which does not consult pane visibility. No readiness wait, no visibility gate, no first-8-KiB window.
- **Both platforms run the same code.** The deletion is not `cfg(windows)`; macOS loses its silent duplicate too.

## Non-goals

- Keeping the canned responder as a fallback for a terminal that is slow to attach. The bridge creates the terminal at `spawned`, before the first output chunk, and `output` creates one lazily if a chunk ever arrives first; there is no window where the canned reply is the only one.
- Deduplicating the two responders by timing or by tracking answered queries. One responder is simpler than two that agree.
- Filtering the CLI's input for stray reply bytes, or working around conhost's input parser.
- Changing what alacritty reports (DA1 identity, cursor position format).
- The `deliver_windows_batch_first_turn` readiness wait and the first-turn paste; unchanged.

## Implementation Phases

1. **Delete the canned responder.** Done. Removed the items listed under Behavior from `crates/runner-backend/src/session/pty_runtime.rs` (155 lines); `reader_thread` and its `spawn` call site lost the responder parameter. The existing `runner-terminal` tests in `osc_color_query.rs` (OSC 4/10/11/12 replies from live state, `primary_da_query_emits_pty_write`) remain the regression coverage for the surviving path. The two `windows_batch_first_turn_*` tests in `manager/tests.rs` answer the ConPTY host's DSR and DA1 once through `inject_stdin` when the DSR shows up in output, because they run a real ConPTY with no terminal. The four bare-runtime `*_windows` tests in `pty_runtime.rs` share a `HostHandshake` stand-in that watches the output stream for the DSR and answers once through `send_bytes`; the foreground test, which never read its stream, drains it until the handshake is answered before typing its command.
2. **Prove the terminal answers hidden panes.** Done. `hidden_terminal_answers_each_query_once` in `crates/runner-terminal/src/terminal.rs` drives the real chain: a `RecordingRuntime` (a `SessionRuntime` that hands the manager a live output channel and records stdin writes) behind `spawn_direct`, the `TerminalBridge` creating the terminal on `spawned` with no view attached, an output chunk carrying `ESC]11;?ESC\` and `ESC[c` pushed through the forwarder, and exactly two writes back: the OSC 11 report with Runner's background, then `ESC[?6c`. `OutputStream::new` went from `pub(crate)` to `pub` so a runtime stand-in outside the backend crate can build one.
3. **Verify live on Windows.** Rebuild the dev app, start a Codex chat and a Claude Code chat, stop and resume each; the composer must be empty after every resume. Record whether a fresh Codex chat's first user turn starts with `[6c` on 0.8.4 (installed app) and confirm it does not on the dev build. Run the probe once more in `term` mode as the reference.
4. **macOS check.** `cargo test -p runner-backend -p runner-terminal` on a Mac; Codex startup still paints the gray composer (the #213 symptom) with the canned responder gone.

## Verification

- [x] `cargo test -p runner-backend` and `cargo test -p runner-terminal` green on Windows; workspace Clippy in the `ci` profile and `cargo fmt --all --check` clean. 2026-09-08: 586 backend tests, 55 terminal unit tests plus the integration suites.
- [x] The hidden-pane test from phase 2 passes; it asserts the exact reply bytes from the event thread, so it fails on zero writes and on a duplicate. 2026-09-08.
- [x] Dev app on JASONPC: resume a Codex chat; the composer is empty. Jason, 2026-09-08 dev build: "the previous issue is gone". Claude Code not separately re-checked.
- [ ] Dev app on JASONPC: a fresh Codex mission slot opened from the Feed tab paints the gray composer (#213 does not return).
- [ ] macOS: a fresh Codex chat paints the gray composer; `runner-backend` and `runner-terminal` tests green.

## History

- 2026-06-20: #213 added the canned responder because hidden xterm.js panes buffered output and Codex cached a black composer; its note assumed a later duplicate reply was harmless.
- 2026-08-23: the GPUI cutover made the alacritty terminal answer every session from spawn, so both responders have been writing since; the inbox conhost consumed the only duplicate that reached it.
- 2026-09-08: #522 shipped OpenConsole 1.24 in 0.8.4; the duplicate DA1 and color replies started reaching the CLI as input. Measured the same day with `target/double-reply-probe`; issue #524 filed.
- 2026-09-08, later: implemented inline in this session rather than by a crew, at Jason's call for a deletion this size. Jason's dev-build feel test confirmed the stray text is gone; the separate missing-background finding from that same test is [#525](https://github.com/yicheng47/runner/issues/525), where the probe shows Codex still emits the highlight after this fix. Phases 1 and 2 landed in the working tree; the two real-ConPTY backend tests exposed the host handshake stall and now answer it themselves. Windows checks green; dev app rebuilt for the feel test.
- 2026-09-08, PR [#526](https://github.com/yicheng47/runner/pull/526): the Windows CI job failed twice on the four bare-runtime `pty_runtime` tests that the local Windows 11 conhost had let through; the CI conhost holds the child until the DSR is answered. Patched the tests with the `HostHandshake` stand-in; no implementation change.
