# `×` on a chat pane archives the chat

Tracking issue: [#567](https://github.com/yicheng47/runner/issues/567). Status: spec, 2026-09-12; Pencil-first, code waits for the frame sign-off. Priority P2.

## Motivation

The `×` on a split chat pane is a layout-only action today. Spec 64 says it outright: "× is a layout action; Archive is a session action." It removes the pane from the tab and leaves the session untouched, and on the next tree read `ensure_active_sessions` gives the now-uncovered session a fresh unnamed single-pane tab, appended at the end of its project. From the user's side the chat did not close: it reappeared as its own sidebar row outside the split, at the bottom of the list, with no name.

Everywhere else (Zed, VS Code, browsers, iTerm) `×` makes the thing go away. Runner's terminal `×` already does: it kills the shell and drops the pane, confirming only when a foreground process is running. The chat `×` is the odd one out, and nothing in the UI says so. Decided 2026-09-12: `×` on a chat archives it. Archive is the existing reversible operation, so the chat never reappears in the sidebar on its own and Settings → Archived brings it back.

## Behavior

### What `×` does

- **Chat pane with a session, running or stopped:** `×` opens **Archive chat?**. Confirm stops the agent if it is running, archives the session, and drops the pane; the split re-flows exactly as today. Cancel leaves everything as it was.
- **Empty pane stub:** layout-only close, no dialog. Unchanged; there is no session to archive.
- **Terminal pane:** unchanged. Foreground-process confirm, then kill and drop.
- **`⌘W` in a split** runs the same request as `×`, so a chat pane has one close semantics. On a single-pane tab `⌘W` still closes the window.

Every chat `×` confirms, whether the agent is running or already stopped (decided 2026-09-12). `×` is a wordless one-click button in a corner whose meaning changes in this release; archive is reversible, but it stops a live agent and takes the chat out of the split, and the dialog is the one place that says so. A menu item labelled Archive is its own confirmation, so `⋯ → Archive chat` and the sidebar's Archive stay silent. Two narrower thresholds were considered and rejected: confirm only while the agent is busy (a chat whose session key has not been captured yet is idle and not resumable, and the user has no model for that anyway), and confirm only while the process is alive (a stopped chat would then vanish on one click, which is the exact surprise this spec removes).

### The dialog

The existing `ConfirmDialog` with the archive icon and the same red danger button every other confirm uses (Archive all, Close terminal); only the fork dialog is primary, and one archive dialog in a different colour would read as a different kind of action. The body follows the state the pane already shows on its card, and `<name>` is `session_label`, the same label the sidebar and the fork dialog use.

Running:

```
Archive chat?
<name> is still running. Archiving stops it. You can restore the chat from Settings → Archived.

[Cancel]  [Archive chat]        pending: Archiving…
```

Stopped:

```
Archive chat?
You can restore <name> from Settings → Archived.

[Cancel]  [Archive chat]        pending: Archiving…
```

**Drawer terminals ride along** the way `archive_chat_sessions` already handles them. When the closing chat is the tab's only session and the tab has drawer shells, `archive_targets_for_chats` pulls the shells into the set and the existing **Archive all?** dialog shows instead, with its counted body and the permanent-close warning. One dialog either way: confirm archives the chat, closes the shells, and the node reconciler removes the tab that is now empty.

The `×` tooltip stays **Close pane** on every pane; the dialog carries the meaning.

### What does not change

- **`⋯ → Archive chat`** keeps its meaning: archive the session, leave the pane in place, empty, no dialog. `×` is archive plus close.
- **Sidebar Archive and Archive all** are untouched.
- **Restore.** Settings → Archived → Restore clears the archive marker; on the next tree read the reconciler wraps the active, uncovered session in a fresh single-pane tab at the end of its project. The old split is not remembered, and the reconciler's doc comment says so on purpose. Nothing here touches the restore path.
- **Single-pane tabs** have no identity line and no `×`; sidebar Archive is their path.

## Non-goals

- A **Move to new tab** item in `⋯` to recover the old pop-out. Not asked for; file separately if it turns out to be missed.
- An **Undo** toast after archiving. `ToastHost` has no action slot, and Settings → Archived is the undo.
- Making `⋯ → Archive chat` or the sidebar's Archive confirm.
- Remembering the split a chat was in so Restore can put it back.

## Design

`design/runner.pen`: one frame, `Spec — Pane × archives the chat (567) · v1`, built from the identity line and two-pane surface of `Spec — Split panes, slimmed chrome (64) · v1`. It shows the split with **Archive chat?** over it in both bodies (running, stopped) and the **Archive all?** variant for the drawer-shell case. Dialog chrome matches Archive all and Close terminal (danger button); only the icon changes to the archive glyph.

## Implementation Phases

1. **Design.** The frame above. Stop for sign-off.
2. **Close request.** One `request_close_chat_pane(pane_id, session_id)` on the root: if `archive_targets_for_chats` grows the set, hand off to `request_archive_all`; otherwise set a new `TerminalCloseTarget::ArchiveChatPane { pane_id, session_id }` and let `render_terminal_close_confirm` pick the copy by the entry's status. On confirm, archive through the existing `archive_all_sessions` path, then drop the pane once the archive has completed and tabs have reloaded: at that point the leaf is an empty stub, so the drop is the existing layout-only `close_pane` and no tree read can re-adopt the session in between.
3. **`⌘W` and the fork.** `pane_close_behavior` gains the chat case so the button and the `close-pane` handler in `main.rs` share one fork instead of two copies.
4. **Docs.** Supersede the "× is a layout action" paragraph in `docs/features/archive/64-native-terminal.md` with a pointer here, and rewrite the chat `×` line in `docs/tests/64-terminal-as-pane-option-smoke.md`.

## Verification

- Split with two live chats: `×` on one shows **Archive chat?** with the running body. Cancel keeps both panes and both agents. Confirm stops that agent, drops the pane, the split re-flows to one pane, the chat is gone from the sidebar and listed in Settings → Archived. Restore brings it back as a single-pane row at the end of its project, resumable.
- The same on a chat showing its ended card: the stopped body, and confirm archives with no kill.
- `×` on an empty stub: no dialog, pane gone. Unchanged.
- Chat plus empty pane, with a drawer terminal open: `×` on the chat shows **Archive all?** counting one chat and one terminal; confirm removes the whole tab.
- Chat plus terminal split: `×` on the chat shows **Archive chat?**; confirm leaves a single-pane terminal tab and the drawer untouched.
- `⌘W` on a chat pane inside a split behaves as `×`; on a single-pane tab it closes the window.
- Terminal `×` is unchanged: silent at a bare prompt, confirm under `sleep 60`.
- `⋯ → Archive chat` is unchanged: no dialog, the pane stays in place, empty.
- Unit tests pin `pane_close_behavior` for a chat runtime and the dialog copy per status.
- `make verify` green.
