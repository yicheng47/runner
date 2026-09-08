# Ask about a selection — a side thread forked from a chat

Tracking issue: [#511](https://github.com/yicheng47/runner/issues/511). Status: planned. Priority P2.

## Motivation

A Runner user asked for the side chat in ChatGPT's desktop app (screenshot in the request, 2026-09-08): the main conversation on the left, and on the right a panel titled after the follow-up question, holding a focused explanation of one passage from the main answer. The side thread knows everything the main conversation knows, and the main conversation never sees the detour. It also appears in the chat list as its own entry.

In Runner the detour has two bad shapes today. Typing the question into the main chat spends the main context and the agent's attention on it, which is exactly what the user wants to avoid mid-task. Forking by hand (60) preserves the main chat, but the user then re-types or re-pastes the passage and the fork is titled "(fork)".

Everything the feature needs already shipped. Terminal selection exists: a plain drag in a shell pane, shift-drag in a claude-code or codex pane because those TUIs capture the mouse (`reporting` in `crates/runner-app/src/terminal/element.rs`), and right-click makes a semantic selection. The native fork (60, `session_fork`) spawns a new chat with the whole history and a new session key, and the source is untouched; both claude-code and codex are native. Spawn-time first-turn delivery (`apply_runtime_args` in `crates/runner-backend/src/session/manager/spawn.rs`, argv up to 32 KB with a post-spawn paste fallback) exists for missions. This spec is the glue between those three.

## Behavior

### Trigger

- **Right-click on a selection** opens a small terminal context menu: **Ask about this…** and **Copy**. Today a right-click outside a selection starts a semantic selection and a right-click inside one does nothing; the menu takes the second case and the first is unchanged.
- **Keymap entry `ask-about-selection`**, default ⌥⌘A, terminal scope, listed in Settings → Keyboard shortcuts like `toggle-terminal-drawer`. With no selection it does nothing.
- **Command palette** row **Ask about selection…**, enabled only while the focused pane has a selection.
- **Availability follows fork.** The entry is hidden on a shell pane and in the mission workspace, and disabled with the fork captions from 60 on a trae chat ("Forking needs claude-code or codex") and on a chat with no session key yet ("No session key captured yet").

### Prompt box

A small composer anchored to the selection, in the confirm-dialog family (`ConfirmDialog`, `crates/runner-app/src/ui/overlay.rs`):

- One text field prefilled with `Explain this in more detail:` and focused with the whole line selected so typing replaces it.
- Below it the quoted selection, read-only, trimmed of trailing whitespace on each line, capped at 4 KB with an ellipsis and a "(truncated)" caption.
- **Enter** sends, **Esc** or the backdrop cancels. No confirm step: this path replaces the Fork chat? dialog because the user already expressed intent by selecting and asking.
- The first turn is the question, a blank line, then the selection as a Markdown blockquote. The blockquote keeps the passage findable by the agent even when the same text sits in its history.

### The side thread

- **Fork with a first turn.** `session_fork` gains an optional `first_turn`. For claude-code the plan `--resume <key> --fork-session --session-id <new>` gets the body as the positional prompt through `first_turn_argv`, so the question is the fork's first user entry and the copy-on-write fork materializes at once, closing the untouched-fork degradation 60 accepted. For codex the headless materializer is unchanged (its provenance note never reaches the transcript); the body is pasted into the visible resumed TUI through the existing post-spawn paste path, which already waits for TUI readiness.
- **Title** derives from the question, not "(fork)": the composer's text, trimmed, first 48 characters; when the text is still the default, the first 48 characters of the selection. Editable like any chat.
- **Destination: beside the source.** The tab grows by one pane through `PaneLayout::prepare_new_pane` (`crates/runner-app/src/pane_layout.rs`): an empty pane is filled, otherwise the layout moves to the next split preset, and the side thread lands in it focused. At the three-pane cap the fork opens in a new tab, which is what 60 does today, with a toast "Opened in a new tab — this tab is full".
- **Sidebar.** A side thread is an ordinary direct chat in the tab's member list, so grouped-tab rules, archive, rename, pin, and unread apply unchanged. The 60 fork row and header icon keep their behaviour; this spec adds a path, it does not alter theirs.
- **The source is untouched.** No focus change until the fork attaches, no write to its ring or key, and its next turn does not include the side question.

### Non-goals

- A markdown-only answer panel driven by headless `claude -p --resume --fork-session`, with a streaming renderer and its own composer. It is the ChatGPT look, but it is a second execution mode; the pane version delivers the isolation and the quote with shipped parts. Recorded as a possible later phase, not planned.
- Side threads from mission slots. Fork is direct-chats-only and the mission workspace stays as it is.
- A transcript-capture tier for runtimes without native fork (53 and 60 both refused it).
- Selecting across the agent's own scrollback: claude-code and codex run in the alternate screen, so selection covers the visible viewport only. The user scrolls the TUI to the passage first.
- Multi-selection, or asking about a selection in another pane than the focused one.

## Design

Feature-scoped Pencil file `design/ask-about-selection.pen`, designed first and reviewed before code:

1. A claude-code pane with a shift-selected passage and the right-click menu (Ask about this…, Copy).
2. The prompt box: question field, quoted selection, Enter / Esc captions.
3. The two-pane result: source on the left, the side thread on the right with its derived title in the header and the sidebar member list.
4. The disabled and hidden states, reusing 60's captions.

## Implementation Phases

1. **Design.** Pencil frames above; stop for review.
2. **Backend.** `first_turn` on `session_fork` and on the fork plan; claude argv and codex post-spawn paste delivery; title derivation; unit tests for the composed argv, the paste path, the 4 KB cap, and the title rules.
3. **App.** Terminal context menu on a selection, `ask-about-selection` keymap entry, command palette row, the prompt box, the split-pane destination with the cap fallback and toast.
4. **Docs.** Add the side-thread path to 60's archive record and a line in `docs/arch/arch.md` §5 on selection-driven forks.

## Verification

- In a claude-code chat, shift-select a passage, ⌥⌘A, Enter: a new pane opens beside the source, the agent answers about that passage with full prior context, the pane is titled from the question, and the source's next turn shows no trace of the side question.
- Same flow on a codex chat: the side thread shows the accepted "Conversation interrupted" banner from 60 and then the pasted question as its first turn.
- Right-click on a selection shows the menu; right-click outside a selection still makes a semantic selection.
- Shell pane and mission workspace: no entry. trae and unkeyed chats: entry disabled with the right caption. No selection: ⌥⌘A and the palette row do nothing.
- A selection over 4 KB is truncated with the caption; the composed first turn stays under the 32 KB argv limit or falls to the paste path.
- A tab already at three panes opens the side thread in a new tab with the toast.
- Existing Fork chat (header icon, sidebar row, confirm dialog, "(fork)" title) is unchanged. `make verify` passes; `runner-app` tests cover the keymap entry and header state.
