# 466 — Sessions outlive the app process

Tracking: [#466](https://github.com/yicheng47/runner/issues/466). Status: planned.

## Motivation

Today a session's PTY lives inside Runner.app. Quitting, updating, or crashing the app kills every running agent; stale rows demote to stopped on next launch and resume means respawning the agent process. That caps every autonomous run at the app process's lifetime, and makes an auto-update restart — the moment the app itself chooses — interrupt every agent mid-turn.

The vision doc states the current contract as "Sessions outlive the UI window, not the app process" (§4.2). This feature upgrades the second half: sessions outlive the app process too.

Two things make this the right time. The GPUI rewrite moved the terminal emulator from xterm.js in a webview to a Rust-side `alacritty_terminal` model, so terminal content can be rebuilt host-side on reattach — impractical in the Tauri architecture. And it is the one capability where herdr's design is strictly ahead of Runner rather than differently scoped; everything else in that comparison Runner covers or deliberately excludes.

## Feature

Closing or restarting Runner.app no longer stops running agents.

- Agents keep working while the app is closed. Agent-to-agent mission coordination (the append-only event log the `runner` CLI writes to) keeps flowing.
- On relaunch, live sessions reattach: the terminal's visible screen and recent scrollback are restored, busy/idle reads correctly, and the mission workspace picks up where it left off.
- Human-facing coordination pauses cleanly while the app is closed: HITL cards and router stdin delivery wait, then reconcile on relaunch — pending `ask_human` cards surface, nothing is lost, and nothing fires or injects twice.
- Quitting becomes an explicit choice: quit and leave sessions running, or quit and stop everything (today's behavior). Stop Mission / Archive semantics are unchanged.
- App auto-update restarts no longer interrupt running sessions.

## Shape

Session PTYs move out of the app process into a small background host the app talks to over a local socket; the app becomes a client that spawns it on demand and reattaches to it on launch. The existing `SessionRuntime` seam is the boundary — the mission model, router, event bus, DB, and MCP server all stay in the app, which remains the state owner. Detail belongs to the impl plan (`docs/impls/`), not this spec.

## Scope

**In:** mission sessions and direct chats, on one machine, across app quit / relaunch / crash / update.

**Out:**

- Surviving machine reboot or logout.
- Remote or SSH attach (vision non-goal).
- Sandboxing (vision non-goal).
- Moving mission coordination out of the app: while the app is closed the router is offline by design — a worker that emits `ask_human` waits, because the router's job is reaching the human and the human is only reachable through the app.

## Implementation Phases

1. Extract session hosting from the app process behind the existing runtime seam; app spawns/reconnects to the host.
2. Reattach: terminal content restore, busy/idle and exit-status reconciliation, router remount without duplicate delivery.
3. Lifecycle UX: quit choices, update-restart flow, app/host version-skew handling.

## Verification

- Start a mission, quit the app, relaunch: sessions are still running, terminals show correct content, no duplicate stdin injections, feed and cards are current.
- Emit `ask_human` while the app is closed: the card surfaces on relaunch and answering it delivers once.
- Agent finishes or crashes while the app is closed: exit status is correct on relaunch.
- Quit-and-stop behaves exactly like today's quit.
- Auto-update restart keeps sessions running across the app swap.
- Vision doc §4.2 updated to the new contract when this ships.
