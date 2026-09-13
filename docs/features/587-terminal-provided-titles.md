# 587 — Display terminal-provided titles

> Tracking issue: [#587](https://github.com/yicheng47/runner/issues/587)
> Priority: P2. Platforms: macOS and Windows.
> Design: required before implementation; feature-scoped Pencil file, no frame yet.

## Motivation

Codex already sends terminal title updates over OSC 0, and Ghostty displays that text in its window/tab label. Current Codex titles can include activity, the thread name, the project name, and an action-required message. Runner parses and stores terminal titles but does not display them in its pane/tab labels, losing useful context that the application already provides.

This is a display feature, separate from the current status heuristic (#584), its hook-based replacement (#347), and shell status (#586). Showing a spinner supplied by the child process must not classify that session as Busy or Idle or change inbox routing.

## Scope

- Display nonempty OSC 0/2 titles in live terminal pane headers and corresponding tab/sidebar labels on macOS and Windows. Support any child application that supplies a title; no Codex-specific parsing, hooks, or shell injection.
- Explicit user names take precedence. An empty/reset title, session exit, or replacement falls back to the existing label; stale titles must not leak into another session.
- Keep title state per session/pane. For a tab containing multiple panes, use its selected pane's title when the tab has no explicit user name. Changing focus must not change another tab's label.
- Keep live titles transient: spinner frames must not repeatedly write chat/session names to SQLite. Deduplicate identical updates, coalesce repaint notifications, and retain bounded single-line labels with truncation.
- Keep runner names, crew slot handles, mission identities, activity state, completion/unread indicators, and router eligibility independent of this text.

## Implementation Phases

1. Design the pane/header and tab/sidebar title precedence in a feature-scoped Pencil file before implementing UI changes; preserve visible runner/slot identity on mission surfaces.
2. Expose the existing terminal title state to the relevant UI labels and wire refreshes through the normal terminal event path.
3. Verify reset, lifecycle, multi-pane, manual-name, and cross-platform behavior with recorded OSC sequences and a live Codex session.

## Verification

- OSC 0 and OSC 2 update labels with both BEL and ST terminators, including sequences split across PTY reads.
- Codex thread-name changes and animated titles appear without affecting session activity, unread/completion behavior, or routing.
- Manual names win; clearing a manual name restores the live title or existing fallback.
- Empty titles, resets, exit/resume, and pane replacement restore the appropriate fallback.
- Multi-pane focus selects the correct title, and updates stay scoped to the owning pane/tab/window.
- Validate macOS and Windows; repeated spinner frames cause no persistent-name writes or layout churn.

## References

- Runner: `crates/runner-terminal/src/terminal.rs`, `crates/runner-app/src/surfaces/panes.rs`, and `crates/runner-app/src/surfaces/chat.rs`.
- Codex title writer: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/terminal_title.rs
- Ghostty title display: https://github.com/ghostty-org/ghostty/blob/09a2724c23fd13f7cd24c093c568a4b6792a66a2/macos/Sources/Features/Terminal/BaseTerminalController.swift#L888
