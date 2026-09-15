# 587 — Display terminal-provided titles

> Tracking issue: [#587](https://github.com/yicheng47/runner/issues/587)
> Priority: P1. Platforms: macOS and Windows.
> Design: frame "Spec — Terminal-provided titles (587) · v1" (`x3miq`) in `design/runner.pen`, in the chat feature-spec row beside the #347 frames.

## Motivation

Codex already sends terminal title updates over OSC 0, and Ghostty displays that text in its window/tab label. Current Codex titles can include activity, the thread name, the project name, and an action-required message. Runner parses and stores terminal titles but does not display them in its pane/tab labels, losing useful context that the application already provides.

This is a display feature, separate from the current status heuristic (#584), its hook-based replacement (#347), and shell status (#586). Showing a spinner supplied by the child process must not classify that session as Busy or Idle or change inbox routing.

## Scope

- Display nonempty OSC 0/2 titles in live terminal pane headers and corresponding tab/sidebar labels on macOS and Windows. Support any child application that supplies a title; no Codex-specific parsing, hooks, or shell injection.
- Explicit user names take precedence. An empty/reset title or pane replacement falls back to the existing label, and stale titles must not leak into another session. An exited session is the exception: it keeps its last title, dimmed — this reverses the issue's original wording, see Design decisions.
- Keep title state per session/pane. A grouped tab with no name of its own is labelled by its first pane in layout order, not by whichever pane has focus — this replaces the issue's "selected pane" wording, so that moving focus inside a tab never relabels it. Changing focus must not change any tab's label.
- Keep live titles transient in the UI and persist exactly one value: `sessions.live_title`, the last meaningful title, for agent runtimes only. Spinner frames must never repeatedly write to SQLite — write after the dedupe, never per frame. Deduplicate identical updates, coalesce repaint notifications, and retain bounded single-line labels with truncation.
- Keep runner names, crew slot handles, mission identities, activity state, completion/unread indicators, and router eligibility independent of this text.

## Design decisions

Settled in frame `x3miq`; the frame is the reference for the labels, the code is the reference for everything else.

The frame lives in `design/runner.pen` rather than a feature-scoped file, against this spec's original line. Every surface it touches — pane header, tab strip, sidebar row, mission roster — is already drawn there, and the #574 and #347 specs set the precedent of a `Spec — … · v1` frame in the shared canvas. A separate file would have meant redrawing all four.

**Precedence.** A name the user typed always wins, and the child may go on writing titles that are no longer shown; clearing the name hands the slot back to the live title. The live title fills the slot nobody claimed. With no user name and no title yet — or after a reset — the runtime's own name stands.

**Nothing is named at creation**, which is what makes the rule above safe: `session_start_direct` takes no title, so `sessions.title` is NULL until `session_rename` writes one, and a tab node carries a user name only once `node_rename` does. A live title fills a hole and never overwrites a name the user set.

**Stripping, for agent runtimes only.** Only a leading braille run (U+2800–U+28FF) and surrounding whitespace come off, the same range `classify_title` already keys on, so status and label can never disagree about what counts as a spinner frame. Everything else the child sends is its own wording and is shown verbatim.

**Terminal tabs take the shell's title verbatim.** A shell has no spinner convention — whatever `precmd` or the prompt writes is the title — so a `Runtime::Shell` pane is not stripped, and a tab whose panes are all shells (`tab_is_terminal`, already in `chat.rs`) is labelled the way every other terminal emulator labels one. How much of a shell's own title is worth showing, and whether the prompt should be taught to write a better one, is a separate conversation.

**Tabs.** A grouped tab keeps `nodes.name` while it stays grouped — `PaneLayout::name` already filters that name out whenever a tab is ungrouped (`pane_layout.rs:400`), so the name goes dormant on unsplit and returns on resplit. With no name, the tab is labelled by its **first pane in layout order** (`root.leaves()`), replacing both today's join of every pane's label (`rows_render.rs:46`) and the issue's selected-pane wording. Focus must never relabel a tab: a tab you cannot find in the rail is worse than a tab named after only half of itself. The label changes only when the tab is restructured — the first pane closed, or panes reordered (#568).

**Mission surfaces.** The handle is the name: `@builder` stays `@builder` for a whole run and its live title reaches the tooltip only. Identity is the thing you address, so it may not move under you.

**Lifecycle.** A reset restores the fallback at once — only the child's own reset clears the title — and a pane replaced by a new chat inherits nothing from the old one.

An *exited* session is `PaneOverlayState::Ended`: the child process is gone (the agent quit, the shell got `exit`, or it crashed) while the pane stays open showing the ended card with Restart/Resume and Close. **The last title stays, dimmed, for agent panes and shells alike.** The dead terminal is still on screen at 0.45 opacity behind the card (`panes.rs:1712`), so the `TerminalSession` and its title are still in memory and the title it earned is still the truest label that pane has; the card, not the label, is what says the process is gone.

**The last meaningful title is persisted, in a column of its own.** `ALTER TABLE sessions ADD COLUMN live_title TEXT` as `0022_session_live_title.sql`, registered after line 168 of `db.rs` — the shape 0016, 0017 and 0019 already use for nullable session fields (0021's side table was for coupled fields with a backfill). No index, no default, no backfill; NULL means nothing was ever reported. It goes on `sessions` because the process does: one PTY, one title. Tabs run nothing, so `nodes` needs no counterpart — `session_label` is the single point every surface goes through, and a grouped tab reaches it through its first leaf. Last wins rather than first, because an agent re-emits a title only once it is working again, so the most recent one is the only thing there is to show at launch.

**The column is the provenance.** `sessions.title` and `nodes.name` mean a human typed it; `sessions.live_title` means a process reported it. Which column a string sits in is what says who wrote it, so no source flag is needed and no rename dialog ever prefills with a machine's words. This is the one thing termio and cmux both had to retrofit — see Prior art.

**Written for agent runtimes only, and cleared when it stops being true.** A shell's title is never persisted: it would strand a stale topic on a bare prompt across restarts. The column is cleared when the child resets the title and when a session resumes into a new conversation, so a row never goes on describing work that is over.

## Prior art

Checked against three apps solving the same problem locally.

**Termio** (`Sources/termio/TermioStore/`) keeps four fields: `givenTitle` (the user's), `liveTitle` (the OSC title, persisted, "display-only; `title` stays untouched"), `promptTitle` (derived from the first prompt, for agents whose title names only the project), and the composed placeholder. It persists every meaningful title, agent sessions only — persisting a shell's "would strand a stale topic on a bare shell across restarts" — and labels plain terminals from OSC 7's cwd instead. `TermioStore.swift:1770` records the scar: `title` once held both kinds of name, they were told apart by pattern-matching, and that "used to freeze a remote row at `<project> · <host>` for the rest of its life."

**Orca** (`src/shared/tab-title-resolution.ts:5`) resolves `customTitle` → `quickCommandLabel` → live title *if meaningful* → `aiVaultTitle` → `generatedTitle` → raw live title → fallback, so the live title appears twice: gated high, ungated low. Its rename editor snapshots the label on open so OSC churn mid-edit cannot overwrite what is being typed.

**cmux** persists `title`, `customTitle` and `customTitleSource` in `SessionPanelSnapshot` (`SessionPersistence.swift:1711`), restoring `customTitle ?? title ?? displayTitle`, and runs title updates through a 1/30s burst coalescer that is flushed before any snapshot is persisted (`DockSplitStore.swift:1265`).

Not adopted for v1: termio's meaningfulness filter (reject titles that repeat the agent name, the folder, or look like a path or `user@host`). With a real column a bad title is one rename away from being fixed by hand, and Codex's behavior here is worth observing before filtering it.


## Implementation Phases

1. Design the pane/header and tab/sidebar title precedence before implementing UI changes; preserve visible runner/slot identity on mission surfaces. Done: frame `x3miq`, decisions recorded above.
2. Expose the existing terminal title state to the relevant UI labels and wire refreshes through the normal terminal event path; keep the last title on exit rather than clearing it.
3. Add persistence: migration 0022, the write after the dedupe, the clears, and the resolution order `nodes.name` (grouped) → `sessions.title` → live in-memory title → `sessions.live_title` → derived default.
4. Verify reset, lifecycle, multi-pane, manual-name, and cross-platform behavior with recorded OSC sequences and a live Codex session.

## Verification

- OSC 0 and OSC 2 update labels with both BEL and ST terminators, including sequences split across PTY reads.
- Codex thread-name changes and animated titles appear without affecting session activity, unread/completion behavior, or routing.
- Manual names win; clearing a manual name restores the live title or existing fallback.
- The persisted title survives a relaunch and is the last meaningful one; it updates only when the words change, never per spinner frame, is never written for a shell, and is cleared on reset and on resume into a new conversation. A typed name is never overwritten, and clearing one falls back to the live title, then to the persisted one.
- Empty titles, resets, exit/resume, and pane replacement restore the appropriate fallback.
- Multi-pane focus selects the correct title, and updates stay scoped to the owning pane/tab/window.
- Validate macOS and Windows; repeated spinner frames cause no persistent-name writes or layout churn.

## References

- Runner: `crates/runner-terminal/src/terminal.rs`, `crates/runner-app/src/surfaces/panes.rs`, and `crates/runner-app/src/surfaces/chat.rs`.
- Codex title writer: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/terminal_title.rs
- Ghostty title display: https://github.com/ghostty-org/ghostty/blob/09a2724c23fd13f7cd24c093c568a4b6792a66a2/macos/Sources/Features/Terminal/BaseTerminalController.swift#L888
