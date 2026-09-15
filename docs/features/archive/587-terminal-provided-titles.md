# 587 — Conversation titles for agent tabs

> Tracking issue: [#587](https://github.com/yicheng47/runner/issues/587)
> Implemented in [#594](https://github.com/yicheng47/runner/pull/594). Platforms: macOS and Windows.
> Design: frame “Spec — Terminal-provided titles (587) · v1” (`x3miq`) in `design/runner.pen`. The naming policy below incorporates Jason’s September 15 live-test feedback.

## Behavior

A new Codex chat starts as **Codex**. A startup title containing only its directory does not rename it. A useful conversation title such as **Discuss cars | yicheng47** becomes **Discuss cars**. Spinner frames, status text, empty titles, and title-stack resets cannot replace that topic.

Names resolve in this order:

1. An explicit user name (`nodes.name` for a grouped tab, then `sessions.title` for a session).
2. A meaningful provider conversation title, from the live terminal or `sessions.live_title`.
3. The existing runtime or runner default, such as **Codex** or **@coder**.

A later genuine provider topic may replace an earlier provider topic. Runner does not derive titles from user prompts. A manual name always wins, and clearing it restores the best automatic name. Rename editors prefill only the manual name.

New Chat saves its name only when the user edited the name field and left it nonempty. An untouched suggested name such as “Codex” leaves `sessions.title` NULL. Existing manually stored names are preserved, including old defaults whose provenance was never recorded; clearing one once restores automatic naming.

## Title sources

`session::title::provider_title` bounds incoming text to 512 characters, keeps labels on one line, strips terminal spinner/status decorations, and removes known directory or runtime-name segments around the topic. It recognizes both Unix and Windows paths. The same filter handles persisted titles from the earlier implementation, so old `topic | directory` values display cleanly without rewriting the database. Meaningful topic wording and internal punctuation are retained.

Provider title filtering and persistence work on both macOS and Windows. Until the provider emits a meaningful title, the session keeps its runtime or runner default.

Shell tabs keep the child’s terminal title verbatim apart from bounded single-line sanitization. Their titles are transient; shell reset restores the default, and shell output never populates `sessions.live_title`.

## Surfaces and lifecycle

Pane headers, sidebar rows, the command palette, and archived chats use the same session precedence. A grouped tab with no explicit name uses its first pane in layout order; changing focus inside the tab cannot rename it. Group names keep the existing dormant-on-unsplit behavior. Mission identities stay **@handle**, with the conversation title available in the tooltip.

The terminal bridge exposes titles for hidden sessions and other windows, so title repainting does not depend on the active pane. Exited panes retain their last title and terminal buffer, with the existing ended styling. Replacing a pane with another session inherits nothing.

Migration 0022 (`0022_session_live_title.sql`) adds `sessions.live_title`. It remains separate from the user-authored `sessions.title`; no extra source flag or backfill is needed.

Agent titles survive empty/reset events, process exit, app relaunch, and resuming the same conversation. Starting a fresh conversation clears `sessions.live_title`. Writes are guarded by the process start timestamp so an old process cannot overwrite the new conversation. Provider updates are deduplicated before persistence, so repeated titles and spinner frames do not rewrite the database.

Title text does not change runner names, crew handles, mission identity, activity, completion/unread indicators, or routing. The existing status classifier continues to receive the raw terminal title independently of label filtering.

## Prior art

Reviewed locally against Orca (`src/shared/tab-title-resolution.ts`, `agent-tab-title.ts`, and `renderer/src/store/slices/terminal-tab-title-batch.ts`), Termio (`TermioStore.swift` and `TermioStore+AgentStatus.swift`), and cmux (`Workspace+TitleOwnership.swift`). Runner uses separate ownership for explicit names and meaningful provider titles. Its fallback is the runtime or runner default; it does not generate names from prompts. Genuine provider topics can update over time.

## Verification

Validated on macOS, 2026-09-15: 1,226 tests passed across `runner-app`, `runner-backend`, and `runner-terminal` (one existing manual measurement ignored). Workspace Clippy, updater-enabled app Clippy, formatting, and diff whitespace checks passed.

Automated coverage includes title precedence, directory and status rejection, decorated topic cleanup, Unicode title cleanup and bounded single-line labels, deduplicated title persistence, migration from the existing version 0021 schema, database reopen, fresh versus resumed conversations, and shell exclusion. Recorded OSC 0/2 sequences cover BEL and ST terminators, split reads, spinner deduplication (including Claude’s `◐–◓` frames), and title-stack resets.

Live smoke tests:

- Start an unnamed Codex chat in `yicheng47`: it stays **Codex** through startup.
- Submit “Let’s discuss cars”: the name stays **Codex** until the provider emits a useful title. A provider title such as **Discuss cars | yicheng47** displays as **Discuss cars**. If no useful provider title arrives, the default remains.
- Send follow-up messages, wait for completion, and trigger an approval: status changes and spinner frames leave the topic intact.
- Rename to **My cars**: it survives subsequent provider updates. Clear the name to restore the current automatic title.
- Switch tabs/windows, exit, and resume the same conversation: the useful name remains. Start a fresh conversation: the old automatic name is cleared.
- Split a tab and change pane focus: its unnamed tab label stays tied to the first pane. Confirm mission handles stay fixed and shell titles retain their normal behavior.
- Repeat provider-title checks on Windows; the naming policy is the same on both platforms.

Jason confirmed the macOS live smoke test passed on 2026-09-15, including the Codex naming flow and Claude spinner cleanup, before the prompt fallback was removed. The provider-only revision still needs a live smoke check; native Windows smoke testing also remains a manual check.
