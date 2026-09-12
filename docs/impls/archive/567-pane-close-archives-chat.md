# 567 — `×` on a chat pane archives the chat

Tracking issue: [#567](https://github.com/yicheng47/runner/issues/567). Spec: [567](../../features/archive/567-pane-close-archives-chat.md). Feature, P2. Shipped 2026-09-12 in [#569](https://github.com/yicheng47/runner/pull/569) (mission `01M2A53W681P3VY7K5XQYHVRQ9`, claude crew). Design: `design/runner.pen` frame `Spec — Pane × archives the chat (567) · v1` (`Id1kj`), committed `50085cc`. Branch: **`feat/567-pane-close-archives-chat` already exists and is checked out** — it carries the spec, the design and this brief; work on it, do not create another. Phases 2–4 of the spec.

## What ships

`×` on a split chat pane opens **Archive chat?** whether the agent is running or stopped. Confirm stops the agent if it is running, archives the session, and drops the pane; the split re-flows as today. When the chat is the tab's only session and the tab has drawer shells, the existing **Archive all?** shows instead. `⌘W` in a split does what `×` does. The empty stub `×`, the terminal `×`, `⋯ → Archive chat`, the sidebar's Archive, and Restore are untouched. Tooltip stays **Close pane**.

## Where the code is

- `crates/runner-app/src/surfaces/panes.rs:1989` the `×` button's `on_press`, matching on `pane_close_behavior` (`:2447`, enum `PaneCloseBehavior` `:2338`, both private) — `LayoutOnly` is the branch that changes. Tests at `:2767` and `:2771`.
- `crates/runner-app/src/main.rs:235` the `⌘W` `CloseTarget::Pane` arm: a second copy of the same fork, inline.
- `crates/runner-app/src/surfaces/chat.rs:1736` `close_pane` (layout-only; persists, reloads tabs); `:1904` `request_close_terminal_pane` (the request shape to mirror); `:2012` `request_archive_all`; `:2044` `confirm_terminal_close`.
- `crates/runner-app/src/main.rs:285` `enum TerminalCloseTarget` (`Pane`, `Tab`, `Drawer`, `MissionDrawer`, `ArchiveAll`), `:306` `TerminalCloseConfirm`, `:317` `ArchiveAllSource`.
- `crates/runner-app/src/surfaces/panes.rs:1113` `render_terminal_close_confirm`: one match yielding title/body/labels, then `ConfirmDialog::new(title, body, confirm, pending, false, …)` (`ui/overlay.rs:417`; `.icon(…)`, `.variant(…)` builders, default is `trash.svg` + `Danger`).
- `crates/runner-app/src/surfaces/sidebar.rs:645` `archive_chat_sessions` → `:90` `archive_targets_for_chats` (grows the set with the tab's drawer shells only when every pane session is requested) → `:660` `archive_all_sessions` → `:1490` `archive_sessions` (kills a running chat, then `session_archive`) → `:1566` `finish_sidebar_archive` on the root (`refresh_sessions`, `reload_tabs`, `ensure_active_tab_attached`, then focus). `:110` `archive_all_confirmation_body` returns `Some` only when terminals are in the plan.
- `crates/runner-app/src/surfaces/sidebar.rs:4078` `session_label`; `DirectSessionEntry.status == SessionStatus::Running` is "alive".
- `crates/runner-app/src/pane_layout.rs:581` `PaneLayout::close_pane` refuses on a single leaf.
- `crates/runner-backend/src/repo/node.rs:646` `ensure_active_sessions`: re-adopts an active, uncovered session as a new tab and (`:790`) deletes a tab with no sessions and an empty drawer. Archived sessions are excluded. Do not change it.
- `docs/features/archive/64-native-terminal.md:49` and `docs/tests/64-terminal-as-pane-option-smoke.md:54`, `:60`.

## Fix shape

1. **One fork.** `PaneCloseBehavior` gains `ArchiveChat`: shell → `CloseTerminal`, any other runtime → `ArchiveChat`, no session → `LayoutOnly`. Make the enum and `pane_close_behavior` `pub(crate)` and use them from both the `×` button and the `⌘W` arm in `main.rs`, which stops carrying its own runtime check.
2. **One request.** `request_close_chat_pane(pane_id, session_id, window, cx)` in `chat.rs` beside `request_close_terminal_pane`. If `archive_targets_for_chats(vec![session_id], self.tabs.tabs())` grew the set, hand the grown set to `request_archive_all(None, …, ArchiveAllSource::Chat, …)` and stop — the reconciler removes the tab once its only session is archived, so there is no pane to drop. Otherwise set `TerminalCloseTarget::ArchiveChatPane { pane_id, session_id }` and notify.
3. **Dialog.** A new arm in `render_terminal_close_confirm`: title `Archive chat?`, confirm `Archive chat`, pending `Archiving…`, icon `archive.svg`, default danger variant (the match now also yields the icon). Body by the entry's status, `<name>` = `session_label(entry)`, fallback `this chat` as the fork dialog does: running → `<name> is still running. Archiving stops it. You can restore the chat from Settings → Archived.`; otherwise → `You can restore <name> from Settings → Archived.` Put the two strings in a pure `archive_chat_confirm_body(label, running)` so they are testable. Copy exactly as written.
4. **Confirm: archive first, drop after.** The `ArchiveChatPane` arm of `confirm_terminal_close` records `pending_pane_close = Some(pane_id)` on the root and calls `archive_all_sessions(vec![session_id], ArchiveAllSource::Chat, …)`. In `finish_sidebar_archive`, after `reload_tabs` has succeeded, take the pending id: if the active layout still has that leaf and the leaf has no session, call `close_pane` on it; otherwise just drop the id. Clear it on an archive error. The invariant: the pane leaves the persisted layout only after the archive marker is set — closing first lets a tree read re-adopt the session as a new tab, which is the bug being fixed. `close_pane` already skips a single-leaf layout.
5. **Docs.** In the 64 spec, prefix the `:49` paragraph with `**Superseded by [567](../567-pane-close-archives-chat.md), 2026-09-12:**` and rewrite its body to the new rule in two sentences; keep the rest of the bullet list. Smoke test: rewrite `:54` (`×` on a chat → the dialog; cancel keeps the pane; confirm stops a running agent, archives, drops the pane; the chat is under Settings → Archived, not the sidebar), add a line for the stopped-chat body, and make `:60` say `⌘W` inside a split matches `×` for chats and terminals alike.

## Rules of the road

- No change to `ops`, the reconciler, `archive_sessions`' plan, `⋯ → Archive chat`, sidebar Archive, or Restore. No new settings, no toast, no tooltip change.
- Stage by path, never `git add -A`. The tree carries nothing else at launch; if it does, ask.
- Do not launch the Runner app (`make run`); Jason smoke-tests. Verify with `cargo test -p runner-app`, `make clippy` (with `--features updater` too), `make fmt`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit on this branch, push, open the PR, drive CI green (`gh pr checks <n> --watch`; the required check is `Rust / macOS`). Do not merge: Jason merges after his own check. No worktrees, no extra checkouts, no extra agents.

## Tests

- `panes.rs`: `pane_close_behavior` → `None` is `LayoutOnly`, `"shell"` is `CloseTerminal`, `"codex"` and `"claude-code"` are `ArchiveChat`; `archive_chat_confirm_body` for both statuses.
- The pending-close decision (leaf present and empty → close; present with a session, or gone → skip) as a pure function over the layout, tested on a two-pane and a single-pane layout.
- Existing suites stay green: `cargo test -p runner-app`.

## Jason's smoke test (after landing)

1. Two live chats in a split: `×` → **Archive chat?**, running body, red button. Cancel: both stay. Confirm: agent stops, pane gone, one pane left, chat under Settings → Archived; Restore → its own row at the end of the project, resumable.
2. A chat on its ended card: stopped body; confirm archives with no kill.
3. Empty stub `×`: no dialog. Terminal `×`: silent at a prompt, confirm under `sleep 60`.
4. Chat + empty pane with a drawer terminal open: `×` on the chat → **Archive all?** (1 chat, 1 terminal); confirm removes the tab.
5. Chat + terminal split: `×` on the chat → **Archive chat?**; confirm leaves a single-pane terminal tab, drawer untouched.
6. `⌘W` on a chat pane in a split = `×`; on a single-pane tab it closes the window. `⋯ → Archive chat`: no dialog, pane stays empty.

