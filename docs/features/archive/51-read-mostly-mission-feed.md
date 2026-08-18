# 51 — Read-mostly mission feed

> Tracking issue: [#330](https://github.com/yicheng47/runner/issues/330)

## Motivation

Mission workspaces expose two competing human-to-runner paths. The native one is direct pane input: click a runner's terminal and type. The duplicate one is the `MissionInput` feed composer, which emits a `human_said` signal that the router then injects into the same PTY anyway (`router/handlers.rs::human_said`) — the composer is pane input with extra steps, plus a synthetic transcript, a recipient picker, and a reply protocol.

The reply protocol is the expensive half. To make composer conversations two-way, every generated prompt teaches a `human` virtual handle: the worker coordination preamble spends a whole "Replying to the human" section on it, the lead prompt spends two coordination bullets, and the CLI reserves the handle in roster validation (`cli/src/roster.rs::HUMAN_HANDLE`). It was the worst-followed part of the crew protocol — #128 exists because agents over-triggered on it with reply spam — and every instruction removed from the preamble gives the surviving ones more weight.

The feed should answer *what is happening across the crew*; the selected pane is where the human talks to a runner. This completes the direction started by #128.

## Scope

- **Remove the `MissionInput` composer.** The feed pane becomes read-mostly: the only remaining feed-side control is the `ask_human` card's choice buttons, which post correlated `human_response` signals exactly as today.
- **Remove the reply-to-human protocol end to end:**
  - CLI: drop the `HUMAN_HANDLE` carve-out from roster validation. `runner msg post --to human` fails with a teaching error — "the operator reads your terminal; answer in your TUI output" — so a resumed session whose baked prompt still advertises the old verb self-corrects instead of hitting a generic unknown-handle error.
  - Worker preamble: delete the "Replying to the human" section and the `human` entry from the valid-handles line. Workers end with zero human-related instructions.
  - Lead prompt: delete both `--to human` bullets; add one line stating the operator watches the terminals and types directly into a runner's pane. `ask_human` stays as the lead's single structured escalation verb.
  - Empty-goal fallback text "(no goal set; await human_said)" becomes "await the operator's instructions in your terminal."
- **Keep the coordination feed intact:** runner-to-runner mail and inbox nudges, coordination signals (`ask_lead`, `ask_human`/`human_question`/`human_response`), router warnings, status transitions, and historical event rendering all stay.
- **Keep historical log compatibility:** `EventFeed` retains its render paths for `human_said`, `human_response`, and messages addressed to `human`, so pre-51 mission logs replay unchanged. The router's `message_nudge` skip for the `human` target also stays as replay-compat belt-and-braces.
- **Keep the programmatic channel (decision):** `mission_post_human_signal` (Tauri command and MCP tool) continues to accept `human_said`, and the router's `human_said` handler stays live. It costs nothing toward the simplicity goal — agents never see it, it appears in no prompt — and it is the only programmatic human-to-mission channel, used by external orchestrators to relay mid-flight operator instructions into a running lead. The whitelist comment is updated to name MCP (not the workspace UI) as the producer.
- Accept that direct pane typing is PTY conversation, not a synthetic event-log transcript.

## Out of scope

- Removing `human_response`, the pending-ask map, or any part of the HITL ask path.
- Changes to the #328 pending-input outbox or the #332 reconciliation tick.
- Recording pane keystrokes into the event log.
- A replacement "type into pane" MCP tool — `human_said` via MCP already covers programmatic injection.

## Implementation phases

1. **UI removal** — delete `MissionInput.tsx` and its `MissionWorkspace` mount; verify feed-pane layout without the dock (paused overlay, ask cards, scroll anchoring). Keep all `EventFeed` human-event render paths.
2. **CLI + prompt simplification** — roster carve-out becomes a teaching rejection; rewrite `WORKER_COORDINATION_PREAMBLE` and `compose_launch_prompt` coordination bullets; update prompt tests (the #128 tone-guardrail test becomes an absence assertion: no `--to human`, no `human` handle in either prompt).
3. **Feed + HITL verification** — `ask_human` → card → choice → `human_response` → injection to asker unchanged; historical logs replay; MCP `human_said` still injects; sync `docs/arch/arch.md` §5.5.0 to the read-mostly feed model.

## Verification

- [ ] Mission workspace shows no composer; feed renders and scrolls; ask-card buttons still answer and route to the asker.
- [ ] `runner msg post --to human` fails with the teaching message; other handles unaffected.
- [ ] Generated lead and worker prompts contain no `--to human`, no `human` handle, no `[human_said]` reference (test-asserted).
- [ ] A pre-51 mission log containing `human_said` and messages to `human` renders exactly as before.
- [ ] MCP `mission_post_human_signal` with `human_said` still injects into the target pane; `mission_goal`/`ask_lead` remain rejected.
- [ ] `cargo fmt`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint` pass.
