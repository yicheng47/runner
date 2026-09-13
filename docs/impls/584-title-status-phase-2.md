# 584 — Phase 2: a shell is busy while it has a foreground process

Tracking issue: [#584](https://github.com/yicheng47/runner/issues/584). Spec: [584](../features/584-title-status-detection.md) — read it first, especially why the title route was abandoned for shells. Feature, P1. Phase 1 shipped in [#585](https://github.com/yicheng47/runner/pull/585). Branch: **`feat/584-shell-foreground` already exists and is checked out** — it carries this brief; work on it, do not create another.

**Read [`584-title-status-phase-1.md`](./584-title-status-phase-1.md) too.** It is one commit old, it established the plumbing this sits beside, and its review found three defects in its own brief. Assume this one has some too.

## What ships

A shell session stays Busy while its PTY has a foreground process, instead of flipping Idle after two seconds of silence. `sleep 4` reads Busy for the full four seconds; today it reads Idle at 4.58 s, two seconds before the command finishes.

**Scope, stated so nobody expects more.** Shell is not a selectable runtime — `runtime_catalog_options` offers Codex, Claude Code and TRAE only — so no runner and no crew slot is ever a shell, and nothing routes crew work to one. This buys a correct status dot on terminal tabs, panes and drawer shells. It is small and exact, not load-bearing.

## Where the code is

Every piece exists. This is a gate, not a detector.

- `crates/runner-backend/src/session/manager/output.rs:159` — the consumer arm for `RuntimeOutput::StatusTransition`. The gate goes here, before `note_forwarder_transition`.
- `crates/runner-backend/src/session/manager/output.rs:365` — `SessionManager::has_foreground_process(session_id) -> Result<bool>`, already written, already resolves the live runtime session and returns `false` when there is none.
- `crates/runner-backend/src/session/pty_runtime.rs:456` — the runtime impl behind it, reading `handle.process_tree.has_other_processes()`. Already used to decide the close-confirmation prompt (`surfaces/chat.rs:1990`, `mission_workspace/drawer.rs:314`).
- `crates/runner-backend/src/ops/session.rs:378` — the existing shape for "is this session a shell", `effective_runtime(conn, session_id) == Some(Runtime::Shell.key())`.
- `crates/runner-backend/src/session/pty_runtime.rs:674` `IdleDetector` — **do not change it.** The gate is on what the consumer does with a transition, not on how the detector decides.

## Shape

In the `StatusTransition` arm, when the incoming state is Idle **and** `source == "forwarder"` **and** the session's effective runtime is Shell **and** `has_foreground_process` is true: drop the transition and continue. Everything else is untouched.

Deliberately at the consumer and not in `idle_monitor_thread`: the runtime layer has no notion of a shell, so gating there would mean a new `SpawnSpec` field and a runtime-layer concept it does not need. Gating in the manager reuses `has_foreground_process` exactly as the close prompt does.

**Know this consequence and confirm it in a test.** Dropping the transition leaves the detector internally Idle while the manager still reports Busy. That resynchronises on its own: the command's output flips the detector back to Busy (deduped by `note_forwarder_transition`, so no event), and the next two seconds of real silence produce a fresh Idle that passes the gate. Net effect is that Idle lands two seconds after the command finishes, which is exactly when it lands today. **There is no path where a shell gets stuck Busy — prove that, do not assert it.**

Cost: one process-tree read per would-be Idle transition on a shell session, so at most one per two seconds per shell, and only for shells.

## Rules of the road

- Agent sessions must be completely unaffected. An agent has subprocesses whenever it runs a tool, which is not the same as thinking, so the gate is wrong for them and must never apply. Runtime check first, foreground query second.
- Do not touch `IdleDetector`, phase 1's title path, or the `source: "title"` flow.
- No shell integration, no `OSC 133`, no injected `precmd`/`preexec`, no `ZDOTDIR`. The process tree answers this without touching the user's shell config, and that is the point.
- No new config, no settings surface, no UI change.
- Stage by path, never `git add -A`. The tree carries nothing else at launch; if it does, ask.
- Do not launch the app (`make run`) — Jason smoke-tests. Verify with `make verify`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit, push, open the PR titled `feat(session): a shell stays busy while it has a foreground process (584 phase 2)`, drive CI green on both required checks. Do not merge. No worktrees, no extra checkouts, no extra agents.

## Tests

- **The regression:** a shell session with a foreground process does not go Idle on the two-second tick. Model it the way `pty_runtime.rs:1243` models the detector, or drive a real PTY as `pty_runtime.rs:1537` already does with `has_foreground_process`.
- **No stuck Busy:** foreground process appears, silence, transition dropped; process exits; output arrives; silence again; Idle is delivered. Assert the delivered sequence, not the detector's internals.
- **Agents unaffected:** a non-Shell session with a foreground process still goes Idle on silence. This is the one that matters most — it is the regression that would be silent.
- **No shell, no foreground:** a shell at its prompt goes Idle exactly as today.
- `make verify` green, plus `cargo test -p runner-backend`. Read cargo's exit status directly or set `pipefail`.

## Handoff must report

The gate condition verbatim and its evaluation order, proof that agent sessions cannot reach it, and the delivered transition sequence for the no-stuck-Busy case. If the shell check needs a database read on the consumer thread, say so and say what it costs — that thread also drains PTY output.

## Non-goals

Phase 3 (reconciling `InputTracker` as a third signal), hung-agent detection, OSC 9;4 and OSC 133, and anything touching the title path phase 1 shipped.
