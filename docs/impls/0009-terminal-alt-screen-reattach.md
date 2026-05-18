# Terminal Alt-Screen Reattach

> **Superseded by [0011](./0011-pty-host-terminal-runtime.md).** The
> alt-screen reattach fix described here lived inside the
> tmux-backed `capture-pane` path. Under the new in-process
> `PtyRuntime`, reattach is no longer attempted at all (impl 0011
> §"Why no headless emulator") — sessions either remain alive
> across Cmd+R (live byte stream resumes) or get respawned via
> the agent's own `--resume <agent_session_key>` flow.

## Context

Opening a previously-running mission whose lead is a claude-code (or any
alt-screen TUI) session shows **stacked redraws** in xterm's main-screen
scrollback. Each window resize or tab activation appends another copy of the
agent's banner+brief+UI rather than overwriting the previous frame in place.
Image attached to the discussion thread shows four banner stacks accumulated
after a couple of resizes; user reports this gets worse the longer the mission
runs.

iTerm2 / Terminal.app / Warp do not show this behaviour with the same
claude-code process. The difference is that those terminals see claude-code's
original `ESC[?1049h` (enter alt-screen) at startup and stay on alt-screen for
the lifetime of the process — every redraw replaces the alt-screen buffer
in place, and main-screen scrollback is left untouched.

### Why our reattach path drops the alt-screen state

Two reasons stack:

1. **`tmux capture-pane -p -e` emits rendered cells as text**, with SGR colour
   escapes for styling, but **never** re-emits the mode-switching escapes
   (`ESC[?1049h`, `ESC[?47h`, etc.) that originally put the pane into
   alt-screen. `capture_replay_bytes` already detects alt-screen state via
   `is_alternate_on(...)` (`src-tauri/src/session/tmux_runtime.rs:1190`) and
   uses it to gate `-S - -E -` (full scrollback inclusion), but it doesn't
   propagate that state to the consumer.

2. **`output_buffers` is a bounded `VecDeque<OutputEvent>` of 4096 chunks**
   (`src-tauri/src/session/manager.rs:44`). For long-running missions, the
   original `ESC[?1049h` from claude-code's startup has long since rolled off
   the deque. `output_snapshot()` returns chunks that *start mid-alt-screen* —
   no enter escape anywhere in the byte stream.

So on reattach:

- xterm starts on main-screen (constructor default).
- `term.reset()` in the replay path also leaves us on main-screen.
- We write capture-pane text into main-screen.
- The live `pipe-pane` stream resumes. claude-code's SIGWINCH redraws (one per
  activation tab-switch via the resize dance, plus one per ResizeObserver tick
  during a window drag) were emitted *assuming* alt-screen — they land in
  main-screen instead, and each redraw scrolls the previous content up into
  main-screen scrollback.

### Why the SIGWINCH dance amplifies the symptom

`RunnerTerminal.tsx:484-485` resizes the backend twice per activation
(`cols-1 → cols`) to force claude-code to repaint. With xterm correctly on
alt-screen, a single resize would already cause an in-place redraw — no
stacking. With xterm on the wrong screen, the dance produces *two* visible
stacked frames per activation. Switch between three runner tabs after a
window resize and you've accumulated 6+ visible stacks.

## Approach

Re-emit the alt-screen entry escape from `capture_replay_bytes` when the pane
is currently on alt-screen, so xterm enters alt-screen at attach time and the
live byte stream lands where claude-code expects it to. Drop the SIGWINCH
dance once the underlying state mismatch is fixed.

### Trade-off

Scrolling up in a claude-code pane after this change will show only the
current alt-screen view (no "history of redraws" in scrollback). That matches
iTerm2 / Terminal.app exactly. For conversation history, users rely on
claude-code's own internal scroll (the same way iTerm2 users do).

For non-alt-screen sessions (plain shells, codex's main-screen UI, build
output), the path is unchanged — main-screen scrollback continues to work.

---

## Step 1: Prepend alt-screen-enter on reattach capture

**File: `src-tauri/src/session/tmux_runtime.rs`**

In `capture_replay_bytes` (line 1189), after invoking `tmux capture-pane` and
verifying success, if `alt_on == true`, prepend `\x1b[?1049h\x1b[H` (enter
alt-screen + cursor home) to the captured bytes before returning.

The cursor-home is defensive: capture-pane's output starts at row 1 col 1 by
default, but explicit positioning means a stale cursor state from xterm's
reset can't drift the first cell of the replay.

Do **not** modify the fresh-spawn branch (`capture_visible_region`) — fresh
spawns capture from the pane before claude-code has had a chance to emit its
own `ESC[?1049h`, and re-emitting it artificially would break the
trim-leading-blank-lines path for non-TUI fresh spawns.

```rust
fn capture_replay_bytes(cmd: &Command, session: &RuntimeSession) -> RuntimeResult<Vec<u8>> {
    let alt_on = is_alternate_on(cmd, &session.pane)?;
    // ... existing capture-pane invocation ...
    let mut bytes = out.stdout;
    if alt_on {
        // Re-emit the alt-screen entry so xterm.js enters alt-screen at
        // attach time. capture-pane gives us rendered cells, not the
        // mode-switching escapes the agent originally emitted; without
        // this, subsequent live redraws from claude-code land in
        // main-screen and stack in scrollback (see docs/impls/0009).
        let mut with_alt = Vec::with_capacity(bytes.len() + 8);
        with_alt.extend_from_slice(b"\x1b[?1049h\x1b[H");
        with_alt.append(&mut bytes);
        bytes = with_alt;
    }
    Ok(bytes)
}
```

## Step 2: Drop the SIGWINCH dance in activation

**File: `src/components/RunnerTerminal.tsx`**

Replace the double-resize at lines 484-485:

```ts
void api.session.resize(sessionId, Math.max(2, cols - 1), rows)
  .then(() => api.session.resize(sessionId, cols, rows))
```

with a single:

```ts
void api.session.resize(sessionId, cols, rows).catch(() => {
  // session may have exited
});
```

The dance existed to "wake claude-code into repainting" after an attach where
the new cols equalled the old cols (no SIGWINCH delta to detect). With Step 1
correctly entering alt-screen, claude-code's *one* redraw on the actual
resize lands in alt-screen and overwrites in place — there's no scrollback
to corrupt with the dance's extra redraw, and the visible viewport ends up
identical either way.

## Step 3: Update comments at attach_streaming

**File: `src-tauri/src/session/tmux_runtime.rs`**

In the `attach_streaming` block-comment (lines 805-829), add a note to Step 2
("Reattach only: snapshot via capture-pane") that the snapshot now prepends
`ESC[?1049h` for alt-screen panes, with a one-line rationale pointing at
`docs/impls/0009`.

## Files to modify

| File | Change |
|------|--------|
| `src-tauri/src/session/tmux_runtime.rs` | Prepend `ESC[?1049h\x1b[H` in `capture_replay_bytes` when alt-screen is on. Update attach_streaming comment. |
| `src/components/RunnerTerminal.tsx` | Replace SIGWINCH dance (two resizes) with a single resize call. |
| `docs/impls/0009-terminal-alt-screen-reattach.md` | This document. |

No backend API shape change. No DB migration. No new dependencies.

## Verification

### Manual (the canonical reproducer)

1. Open a long-running mission with a claude-code lead. Confirm reopen works.
2. Scroll up in the `@architect` pane after reattach. **Expected:** no banner
   stacking — scrollback shows nothing above the current view (because
   we're on alt-screen). **Regression signal:** stacked banners or stacked
   conversation excerpts in scrollback.
3. Resize the window a few times (drag the edge slowly). Scroll up again.
   **Expected:** still no stacking.
4. Switch tabs (`@architect` → `@impl` → `@reviewer`) several times. Scroll
   up in each. **Expected:** alt-screen view is current, no stacked redraws.
5. Run codex (alt-screen TUI) — same expectations.
6. Run a plain shell command in a session (`echo hello`, `ls`). Scroll up.
   **Expected:** main-screen scrollback works normally — this path is
   gated on `alt_on == false`, unchanged.

### Edge cases

- **Fresh mission spawn.** `attach_streaming` takes the `is_fresh_spawn=true`
  branch which uses `capture_visible_region`, not `capture_replay_bytes`.
  Behaviour unchanged — fresh spawn captures the visible region and trims
  leading blanks; claude-code's own `ESC[?1049h` flows through `pipe-pane`
  naturally on first output.
- **Agent exits while we're on alt-screen.** When the agent emits
  `ESC[?1049l` (exit alt-screen) on shutdown, xterm returns to main-screen
  with its prior contents — empty since we never wrote anything there.
  Matches iTerm2 behaviour.
- **Reattach to a session that's mid-transition from main to alt or back.**
  `is_alternate_on` reads `#{alternate_on}` synchronously at capture time,
  so we honour whatever state tmux thinks the pane is in at that instant. A
  race where the agent toggles alt-screen between our `is_alternate_on`
  check and the `capture-pane` call is theoretically possible but vanishingly
  rare; worst case is one frame of mis-rendering that the next live byte
  corrects.

### Automated

No new unit tests proposed — the bug lives in the interaction between
tmux's capture format, xterm's mode machine, and the agent's redraw timing,
none of which are easily testable in isolation without a full integration
harness. Manual verification covers the regression path.
