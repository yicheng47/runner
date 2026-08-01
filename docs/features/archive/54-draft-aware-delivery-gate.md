# 54 — Draft-aware delivery gate

> Tracking issue: [#359](https://github.com/yicheng47/runner/issues/359)
> Priority: P1.

## Motivation

Typing a single stray character into a crew runner's terminal blocks its inbox indefinitely, and the only ways to unblock it are destructive.

The gate itself is right and should stay. The router injects into a slot's PTY stdin to wake an idle agent; if the human has a half-typed draft in that agent's input box, an injection collides with it — corrupting the draft or submitting a merged line. Runner deliberately refuses to deliver in that case (`reserve_delivery` → `DeliveryReservation::PendingInput`, `session/manager/mod.rs:821`). That is the correct trade, and better than the alternative: Orca, which faces the same problem, has no draft concept at all — its Native Chat sends `\x15` (Ctrl-U) to kill the input line before writing (`native-chat-runtime-send.ts:37`) and its automation path injects into a running agent with no readiness check, appending to the human's draft and submitting the merged result.

What's wrong is not the gate but its **state model**. `local_input_pending` is a boolean latch (`session/manager/mod.rs:536`) set by `classify_local_input` (`session/manager/output.rs:16-37`) on any printable byte, and cleared only by `\r` (Enter), `\x03` (Ctrl-C), respawn (`mod.rs:884`), or session lifecycle transitions (`lifecycle.rs:76,309`). Everything else — backspace (`\x7f`), Ctrl-U (`\x15`), Escape, arrow keys — classifies as `ActivityOnly`, which refreshes the 2-second `RECENT_LOCAL_INPUT_WINDOW` (`mod.rs:51`) but leaves the latch set.

The consequence: type `a` into the wrong pane, press backspace, and the input box is empty while the latch is still true. The runner's inbox is now blocked forever. Recovery requires Enter (submits a line to the agent) or Ctrl-C (interrupts it) — there is no non-destructive escape and no UI affordance. Because a stalled inbox is silent, the mission just quietly stops coordinating until someone notices.

## Scope

### In scope

- **Replace the boolean latch with a draft line model.** Track the human's unsubmitted input as a length/emptiness model rather than a one-way flag, so the gate closes when a draft exists and opens again when the draft is gone. Minimum behaviors: printable bytes extend the draft; backspace and word-delete shrink it; Ctrl-U, Ctrl-C, and Escape clear it; Enter clears it (submitted). An empty model means delivery is allowed.
- **Abandonment backstop.** Even a perfect model can be defeated (a draft left half-typed for an hour, or a TUI whose editing keys we mis-model). A draft that has seen no input for a generous interval should stop blocking delivery. The interval is a decision, not a given — long enough not to interrupt someone thinking, short enough that a forgotten keystroke doesn't strand a mission.
- **A non-destructive manual clear.** Whatever the model, the human needs one honest way out that neither submits nor interrupts. The runner rail already shows per-slot state; a "clear pending input" affordance there, or a keybinding, closes the loop when heuristics fail.
- **Observability.** A slot whose inbox is gated on a draft should say so — the current state is indistinguishable from a busy agent in the UI, which is why it goes unnoticed.

### Out of scope

- Removing or weakening the gate. Delivering into a live draft is worse than a delayed delivery; this spec makes the gate accurate, not permissive.
- Hook-based agent status ([#347](https://github.com/yicheng47/runner/issues/347) / spec 52). That resolves *agent working vs agent idle*; this resolves *input box empty vs input box dirty*. They are orthogonal dimensions of "is it safe to deliver," and #347 does not subsume this — a hook can never report a human's unsubmitted keystrokes.
- Screen-scraping the TUI's input line to read the draft directly. Per-runtime, per-version fragile; the keystroke model is runtime-agnostic and already sits on a byte stream Runner owns.
- Changing the router's injection mechanism, the inbox pull model, or `RECENT_LOCAL_INPUT_WINDOW`.

### What to borrow from Orca

Orca does not solve the problem, but it **has already written the algorithm** — for a different purpose, and explicitly disabled in the case that matters here.

`observeAcceptedShellCommandInput` (`src/renderer/src/components/terminal-pane/pty-connection.ts:1492-1567`) maintains `pendingShellCommandLine`: a real line model with a cursor that appends printables (`char >= ' '`), deletes on `\x7f`/`\b`, word-deletes on `\x17`, resets on `\x03`/`\x15`, consumes CSI arrow sequences, and commits on `\r`/`\n`. It exists to notice a human typing `claude` at a shell prompt — and at `:1505` it bails out precisely inside agent TUIs:

```
// Why: bytes typed inside a live agent TUI are prompt text, not shell
// commands, even if they spell another agent binary name.
if (hasFreshPaneAgentSurface()) { resetPendingShellCommandLine(); return }
```

So the borrowable artifact is the state machine, not the design: Runner wants that same model applied to exactly the context Orca skips, feeding the delivery gate instead of command detection. Runner's version lives in Rust on the input path (`classify_local_input`) rather than in the renderer, and needs no cursor — only whether the draft is empty.

## Open questions

- **Where the model lives.** `classify_local_input` sees raw input bytes and is the natural home, but it is currently stateless per call. A per-session draft model belongs on `SessionState` beside the flag it replaces.
- **How faithfully to model editing.** Agent TUIs are not readline; claude-code and codex handle kill-line, word-delete, and history differently. The model should fail *closed* (assume a draft still exists when unsure) with the abandonment timeout and manual clear as the release valves — never fail open into a corrupted draft.
- **Multi-line and paste.** Bracketed paste and `\x16` already set the latch. A pasted block followed by Enter is a normal submit; a pasted block left unsubmitted is a draft. The model should treat paste as content like any other.
- **Whether the 2-second recency window survives.** With an accurate draft model, `RECENT_LOCAL_INPUT_WINDOW` may be redundant, or may still be worth keeping as protection against delivering mid-keystroke.

## Implementation phases

1. **Draft model.** Replace `local_input_pending` with a draft-state struct on `SessionState`; port the editing state machine into `classify_local_input`'s caller; keep `reserve_delivery`'s contract unchanged apart from consulting emptiness. Unit-test the byte sequences directly: type-then-backspace-to-empty opens the gate, Ctrl-U opens it, partial draft keeps it closed, paste keeps it closed, Enter opens it.
2. **Backstop and escape hatch.** Abandonment timeout plus a non-destructive manual clear, wired to whatever UI surface the runner rail exposes.
3. **Surface the state.** Make "inbox gated on your unsent input" visible on the slot, distinct from "agent busy."

## Verification

- [ ] Type a character into a mission slot, backspace it away → the inbox delivers without pressing Enter or Ctrl-C.
- [ ] Type a real draft and leave it → delivery stays gated.
- [ ] Ctrl-U on a partial draft → delivery resumes.
- [ ] Paste a block without submitting → delivery stays gated; submit it → delivery resumes.
- [ ] An abandoned draft stops gating after the chosen interval.
- [ ] The manual clear neither submits to nor interrupts the agent.
- [ ] A gated slot is visually distinguishable from a busy slot.
- [ ] Delivering into a live draft never happens — the regression this spec must not introduce.
- [ ] `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint` clean.
