# M6.1 Smoke Test — observed terminal input state

Release-readiness check for M6.1 (`docs/impls/archive/gpui-rewrite/m6-consolidation.md` §M6.1, brief in `docs/impls/archive/gpui-rewrite/briefs/m6-1-input-state.md`). The native terminal now observes `Idle`, `Drafting`, and `Submitted` from key intent plus the live grid; the backend consumes that observation before the legacy byte latch. A draft has no abandonment timer: delivery remains parked until the grid shows the composer empty, including while a menu hides it.

Run the human lane on the nightly build with one two-agent mission (claude lead + codex reviewer), one shell direct chat, and a second Runner window showing the same mission. Keep one directed message ready to send from each agent so every step can test whether a nudge lands or parks.

## Agent-run checks

```sh
make verify
cargo test -p runner-terminal --test input_state_replay
cargo test -p runner-app --test terminal_ime
cargo test -p runner-app --test session_manager_integration
```

Expected: the input fixture transitions match their `.expected.txt` files; the backend observed tier overrides the legacy latch, a hidden composer still parks, and `Drafting → Idle` releases a parked delivery after `INPUT_CLEAR_FLUSH_GRACE` without injecting a stray Enter.

## Human smoke checks — claude and codex

Repeat items 1–7 in each mission slot. For every “post a message” step, send a directed `runner msg post --to <handle> "input-state smoke"` from the other agent.

1. **Type and delete.** Type `hello` without Enter, post the message, and confirm the delivery-blocked pill appears and the nudge does not enter the composer. Backspace five times. The pill clears and the nudge lands about 500 ms after the output chunk that restores the empty composer; it must not wait for Enter or Ctrl-C.
2. **Line clears.** Type text and press Ctrl-U; repeat with Ctrl-C. In each case the nudge stays parked while text is present and releases only after the grid visibly returns to the empty composer.
3. **Multiline editing.** Type a first line, press Shift+Enter, type a second line, then Backspace back to the first line. Delivery remains parked throughout. Plain Enter reports submission; the nudge does not splice into either line and may land only after the two-second submitted/recent-input window or the normal agent turn boundary.
4. **Slash menu over a draft.** Type `/` so the autocomplete menu covers or moves the composer, then post a message. The pill remains visible and delivery remains parked while the composer is hidden. Close the menu and clear the draft; the nudge releases after the 500 ms clear grace.
5. **Prompt navigation.** Trigger a permission or y/n prompt, navigate with arrows, and answer with Enter. No draft pill appears from arrows, the menu Enter, or the prompt choice. A directed nudge follows the normal delivery gate.
6. **Large paste.** Paste 20 lines. Confirm the collapsed `[Pasted text #N +M lines]` form is detected as a draft, the pill appears, and delivery remains parked until submit or clear.
7. **Pinyin composition.** Start marked Pinyin text without committing it. The pill appears before bytes reach the PTY and delivery parks. Commit the candidate, then delete it from the runtime composer; the pill clears and the nudge releases. Cancel marked text and focus another pane once as separate checks: both must clear composition state.

## Cross-surface checks

8. **Streaming output.** While each agent is producing a long response, type a draft and post a message from the peer. Transcript churn must not be mistaken for the composer echo; the pill appears only when the runtime echoes the human input, remains while the draft is present, and clears when the composer is observed empty.
9. **Second window.** Keep the same mission open in a second window. Type, hide with `/`, and clear from the owning window; both windows show the same existing blocked pill transition, with no duplicated nudge and no per-keystroke root repaint.
10. **Shell direct chat.** In zsh, type and edit a command, use Ctrl-U/Ctrl-C, submit, paste text, send arrows, and exercise mouse selection. The shell remains interactive; `\x16` image markers and mouse reports do not start a composer probe. Direct chat has no mission delivery pill.

## Fixture recording lane

Set `RUNNER_RECORD_INPUT_FIXTURE=<new-prefix>` before launching the runtimes. Each terminal session writes `<new-prefix>.<session-id>.ndjson` without touching the filesystem when the variable is unset. Record claude with the fullscreen renderer, codex, and zsh separately, performing the sequences above. Each recording must include output chunks and structured input events; bless only the transition list after checking placeholder text, prompt glyph, paste collapse, Ctrl-C behavior, menu hiding, y/n navigation, and Pinyin commit/cancel against the visible nightly.

## Pass criteria

No delivery enters visible or hidden draft text; type-then-delete releases within `INPUT_CLEAR_FLUSH_GRACE` plus the first output chunk showing the empty composer; no reported state clears on a timer; plain Enter is the only submit key; Shift/Option+Enter, composition Enter, arrows, mouse reports, and `\x16` never create a false submission or probe. Claude, codex, the shell chat, and the second window all satisfy the same run.
