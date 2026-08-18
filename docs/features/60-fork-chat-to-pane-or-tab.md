# Fork a chat into a split pane or a new tab

Tracking issue: [#398](https://github.com/yicheng47/runner/issues/398). Status: planned. Supersedes the native tier of [53 — Session fork](./53-session-fork.md) (closed won't-do in [#348](https://github.com/yicheng47/runner/issues/348); its revisit trigger — actually reaching for a fork in real work — has now fired).

## Motivation

A chat that has accumulated valuable context can't be branched: exploring a second direction means steering the original or losing the context. #348 declined to build this because nobody had reached for it and because the generic transcript-capture tier was lossy per-runtime machinery; the close-out explicitly reserved the native tier as "small and worth building" once someone actually wanted a fork. That happened. What's new in this spec versus 53 is the destination: the fork should land where the comparison is useful — beside the original as a split pane in the current tab, or out of the way in a new tab.

Orca reference (source re-checked at `~/repos/gui/orca`, `terminal-agent-session-fork.ts`): its "Fork Agent Session…" always creates a new worktree workspace and launches the forked agent in a new tab there, delivering an 800-line ANSI-stripped scrollback capture as an editable draft; a sibling "Copy context" puts the bare transcript on the clipboard. Two things carry over: fork never mutates the source pane, and the forked turn arrives as a draft, not an auto-submitted message. Two things don't: the lossy capture (Runner has persisted `agent_session_key`, so claude-code forks natively with full history) and the single fixed destination (Runner's pane-layout system supports an in-tab split).

## Scope

- **Native fork only.** Fork spawns a new direct chat with the same runner, cwd, model/effort as the source session, launched via the runtime's native fork flags (claude-code: `--resume <agent_session_key> --fork-session`). Full history, new session identity, source session untouched. No transcript-capture fallback tier — #348 refused that machinery and this spec keeps refusing it.
- **Two destinations, chosen at the entry point** ("Fork to split pane" / "Fork to new tab"):
  - *Split pane*: the current tab's layout grows by one pane (same mechanism as the pane-level New chat affordance) and the forked session lands in it, focused, beside the source.
  - *New tab*: a fresh single-pane tab in the same window (the ⌘T shape) holding the forked session; the source tab stays as-is.
- **Entry points**: chat pane header kebab and the sidebar chat context menu. Both actions disabled with an explanatory tooltip when the session's runtime has no native fork path or no `agent_session_key` has been captured yet.
- **Naming**: forked chat titles derive from the source ("<title> (fork)"), editable like any chat.

## Non-Goals

- The lossy transcript/context tier for non-native runtimes, including Orca-style "Copy context" affordances.
- Missions and mission slots — direct chats only.
- Cross-runtime forks and fork-time cwd/worktree pickers (chats are not worktree-bound).

## Implementation Phases

1. **Backend fork spawn** — a fork variant of the direct-chat spawn: new session row cloning runner/cwd/model/effort from the source row, resume plan composed with the fork flag, fork-eligibility exposed on the session (runtime capability + key presence).
2. **Destinations + entry points** — pane-kebab and sidebar context-menu actions; split-pane destination through the pane-layout grow path, new-tab destination through the tab-creation path; disabled-state tooltips.
3. **codex investigation** — promote codex to native fork if a second `codex resume` instance forks cleanly without contending for the source session file (open question inherited from 53).

## Verification

- Forking a claude-code chat yields a new session whose agent recalls pre-fork context in full; the source session keeps its own key and continues independently.
- Split destination: layout gains a pane beside the source with the fork focused; tab destination: new tab holds the fork, source tab unchanged.
- Fork actions disabled (with tooltip) for a runtime without native fork and for a session with no captured key.
- Forking never writes to the source session's ring, key, or process.
- `cargo test --workspace` for spawn-plan composition; vitest for entry-point state; manual pass for both destinations.
