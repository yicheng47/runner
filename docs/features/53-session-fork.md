# 53 — Session fork

> Tracking issue: [#348](https://github.com/yicheng47/runner/issues/348)

## Motivation

There is no way to branch a conversation: take a chat that has accumulated valuable context and explore a different direction in parallel, without steering or destroying the original. Orca ships this as "Fork Agent Session" (source studied 2026-07-25): serialize the last 800 lines of xterm scrollback, strip ANSI and trim to a 36,000-char newest-first budget, wrap it in a fenced handoff prompt, create a `<name>-fork` worktree, and launch a fresh agent with the handoff as an editable unsubmitted first-turn draft. No system prompt and no CLI session flags are involved.

Runner can do strictly better for claude-code: we already persist `agent_session_key` (impl 0017 for codex; claude keys captured at spawn), so a native full-history fork via `claude --resume <key> --fork-session` needs no lossy capture at all. Orca structurally cannot do this — it drops provider session IDs during hook normalization.

## Scope

- **Native fork tier (claude-code, preferred):** a "Fork chat" action spawns a new direct chat with the same runner and cwd, launched with `--resume <agent_session_key> --fork-session` — full conversation history, new session identity, original untouched.
- **Context fork tier (generic fallback):** for runtimes without a native fork path, build a bounded, ANSI-cleaned transcript server-side from the output ring (no xterm serialization needed), wrap it in a handoff preamble ("this is a fork of an existing session — acknowledge and wait"), and deliver it as an unsubmitted first-turn draft via the existing spawn-time prompt delivery plumbing (impls 0005/0007; bracketed paste without Enter).
- UX entry points: chat header kebab and sidebar chat context menu → new chat appears beside the original.
- Capture bounds for the fallback tier follow Orca's proven envelope as a starting point: newest-first retention with an explicit omission marker, fenced so transcript backticks cannot break the prompt.

## Out of scope

- Missions and mission slots (v1 is direct chats only).
- Worktree coupling. Runner chats are not worktree-bound; an optional different-cwd picker at fork time is a spec-level open question, not v1.
- Summarization or structured transcript parsing for the fallback tier — the handoff is a verbatim bounded transcript.
- Cross-runtime forks (fork a claude chat into a codex chat).

## Open questions

- Does codex resume-as-a-second-instance fork cleanly, or does it contend for the same session file? Decides whether codex gets the native tier or the context tier.
- Per-runtime draft-delivery behavior: paste-without-submit must land in the input box (not auto-execute) in each TUI.
- Fallback capture bounds: adopt Orca's 36k-char budget or size to runner's ring retention.

## Implementation phases

1. **Native claude-code fork** — chat row creation + fork spawn flags + UX entry points.
2. **Context fork from the ring** — transcript cleanup/bounding helpers, handoff preamble, draft delivery for non-native runtimes.
3. **codex native-fork investigation** — promote codex to the native tier if resume-fork proves safe; qodercli after #341.

## Verification

- [ ] Forking a claude-code chat produces a new chat whose agent recalls full pre-fork context; the original session continues independently with its own key.
- [ ] The forked chat's first turn is an editable draft, not an auto-submitted message.
- [ ] Context-fork fallback produces a cleaned, bounded transcript with an omission marker when history exceeds the budget, and the fenced block survives transcripts containing backticks.
- [ ] Forking never writes to or mutates the source session's ring, key, or process.
