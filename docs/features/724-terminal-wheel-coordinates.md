# 724 — Preserve terminal wheel coordinates

Tracking issue: [#724](https://github.com/yicheng47/runner/issues/724). Priority: P2. Status: ready for implementation. This is a terminal input correction with no visual or layout changes; no Pencil design is needed.

## Motivation

In Codex CLI 0.157.1's fullscreen transcript, wheel scrolling over conversation content does nothing while a pinned prompt header is visible, although PageUp moves through the same history. Jason reports that ordinary scrollback mode works. Keep both modes usable without changing how Codex launches.

Runner's `on_scroll` and `on_mission_scroll` retain wheel deltas and Shift but discard the pointer position. `TerminalSession::scroll` delegates to `encode_scroll`, which always emits terminal cell `(1,1)` in both SGR and legacy mouse reports. Its assumption that agent TUIs ignore wheel coordinates is incorrect for this Codex version.

Codex's matching-version [mouse handler](https://github.com/openai/codex/blob/rust-v0.157.1/codex-rs/tui/src/transcript_view/input.rs) rejects events outside the transcript unless a selection drag is active. Its [renderer](https://github.com/openai/codex/blob/rust-v0.157.1/codex-rs/tui/src/transcript_view.rs) moves the content area down one row when the pinned prompt header is present. Runner's synthetic top-left position therefore targets the header. PageUp uses a separate keyboard path. Ordinary inline scrollback without mouse reporting uses Runner's local terminal buffer and avoids this coordinate-dependent path. Codex transcript ownership is separate from macOS window fullscreen.

This diagnosis was checked against Codex 0.157.1 source and Runner main `b9a0d6b`. Runner's existing wheel encoder test passes because it explicitly expects the incorrect fixed position. No live UI reproduction was performed during the investigation.

## Scope and behavior

1. **Report the cell under the pointer.** Convert the wheel event's window position to zero-based viewport column and row using the displayed terminal element's content bounds, cell width and line height. Use the same geometry and edge handling as clicks and drags. Account for pane offsets, padding, splits, terminal font size and zoom. Do not use the OS window origin, generic text metrics, a fixed row below the header, or a scrollback-relative grid row. Convert to one-based coordinates only when encoding terminal bytes.
2. **Keep the terminal routing rules.** With mouse reporting enabled, send wheel-button reports at those coordinates. Without mouse reporting, retain alternate-screen arrow forwarding when `ALT_SCREEN | ALTERNATE_SCROLL` is enabled, including application-cursor sequences; otherwise scroll Runner's local buffer. Shift continues to bypass reporting and use local scrollback. Do not disable fullscreen, mouse reporting or Codex's pinned header to mask the bug.
3. **Support both protocols.** Encode the real coordinates in SGR and legacy mouse reports, preserving wheel direction and accumulated line count. Use the existing mouse encoder's legacy byte-range policy; unrepresentable coordinates must not wrap, produce malformed packets, or accidentally fall through to local history scrolling in mouse-reporting mode.
4. **Handle each gesture once.** Preserve fractional-delta accumulation, existing scroll sensitivity, and input-ownership gates. A gesture must not be handled by both the terminal element and an enclosing surface, sent to a different pane, or blocked by a stale hitbox. Do not add another layout measurement path just for wheels.
5. **Cover every existing caller.** Direct chat panes and terminal drawers, mission slot panes and mission terminal drawers, and the agent-update terminal must use correct coordinates wherever they forward wheel reports. Keep local-only selection autoscroll and stopped/read-only terminal scrolling working. Preserve the existing policy on which views may forward input.
6. **Let the child decide what its regions do.** Scrolling over the actual pinned header still reports that header cell; Runner must not redirect it into transcript content. Click, drag, selection, copy, links, keyboard scrolling and paste retain their behavior. The change is shared across macOS and Windows.

## Implementation phases

1. Trace wheel delivery and terminal geometry through `crates/runner-app/src/terminal/element.rs`, `surfaces/chat.rs`, `surfaces/panes.rs`, `surfaces/mission_workspace/{input,view,terminal_pane}.rs` and `surfaces/agent_update.rs`. Choose the smallest shared path that has the displayed pane's geometry and respects current interactivity. Reuse `TerminalGeometry` and `hit_test` where practical; remove superseded listeners if ownership moves into the terminal element.
2. Thread viewport coordinates through the terminal scrolling API and `crates/runner-terminal/src/mappings.rs`. Keep local-only scroll calls explicit and preserve existing protocol behavior. Update all callers and remove the comment claiming wheel coordinates are ignored. Avoid unrelated terminal refactors, new dependencies, runtime-specific branches, settings or persistent state.
3. Add focused regression coverage, run the checks below, and complete the coder/reviewer loop. The handoff must distinguish automated evidence from the live smoke checks Jason still needs to perform.

## Verification

Automated coverage must exercise observable input behavior, not only a new helper in isolation:

- A wheel over a non-origin cell reports that cell for both directions and repeated lines. For example, zero-based column 7, row 4 produces SGR `ESC[<64;8;5M` for wheel-up; legacy coordinates use the corresponding byte offsets. Test origin, an interior cell and protocol bounds.
- Exercise pointer-to-cell conversion with a nonzero pane origin and changed cell metrics. An event over transcript content below row zero must reach the encoder as that content cell; a real header event remains on row zero. Include a check that one gesture produces one set of reports.
- Plain scrollback, Shift bypass, alternate-screen arrow forwarding, application-cursor mode, zero delta, fractional accumulation and local selection autoscroll retain their current behavior. A legacy coordinate outside its representable range is consumed without malformed output or unintended local scrolling.
- Verify the shared wheel path covers chat and mission surfaces and respects existing read-only/input-ownership gates. Prefer focused integration coverage using synthetic terminal output and recorded PTY input where existing test infrastructure supports it.

Required local checks: `cargo test --locked -p runner-terminal -p runner-app --profile ci --no-fail-fast`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check`, and `git diff --check`. The PR must pass macOS and Windows CI.

Jason's live smoke check: open a Codex fullscreen transcript with enough history for a pinned prompt header; wheel over the body to reach earlier content and back down, then verify PageUp/PageDown still work. Repeat in a split chat pane and a mission pane, after resizing and changing terminal font size or zoom. Check ordinary inline/shell scrollback and Shift bypass, and confirm selection and dragging still work. Compare an ordinary macOS window with OS fullscreen; the window state must not change the outcome. Native Windows and any unperformed UI checks must be listed as unverified rather than inferred from unit tests.
