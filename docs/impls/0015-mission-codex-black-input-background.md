# Mission Codex Input Field Renders Black Instead of Gray

## Symptom

A `codex` runner spawned in a mission slot renders its input composer with a black background. The same `codex` runner spawned as a direct chat renders the composer with its normal gray background. Two further observations turned out to be decisive:

- Opening the slot's tab (which triggers a refit + SIGWINCH) does **not** fix it.
- Stopping and resuming the mission slot **does** fix it — the resumed composer is gray.

## Root cause (confirmed)

Codex probes the terminal's background color at startup and computes the composer's gray shade *relative to the detected background*. If that probe is not answered within codex's startup window, codex falls back to a default and the composer renders black. Codex performs this detection **once at startup and caches it for the process lifetime**.

This was confirmed by spawning codex under a bare PTY and capturing its first bytes. Before painting anything, codex emits:

```
\x1b[?2004h   \x1b[6n   \x1b]10;?\x1b\\   \x1b]11;?\x1b\\   \x1b[c
bracketed    DSR-pos   OSC10 fg query   OSC11 bg query    DA1
```

The `\x1b]11;?\x1b\\` is an **OSC 11 background-color query**; codex waits for the terminal to report its background. The reply must come from xterm.js, which sends it back to the PTY via `onData → api.session.injectStdin`. Two corroborating facts:

- Codex emits the OSC 11 query **even though** `~/.codex/config.toml` already sets `appearanceTheme = "dark"`. It always probes the live terminal for the actual background rather than trusting config.
- Because the detected background is cached for the process lifetime, a later resize (tab open) repaints with the stale/default theme — which is exactly why a resize can't fix the black composer but a respawn (resume) can.

### Why mission slots lose the handshake but direct chats and resumes don't

The determinant is whether the slot's pane is the visible/active surface during codex's startup window. Hidden mission slot panes buffer their PTY output (`pendingLiveRef` in `RunnerTerminal.tsx`) and do **not** drain it into xterm until the pane is activated. So xterm never parses codex's OSC 11 query in time, never replies, and codex times out into the black fallback.

| Case | Pane during codex startup | OSC 11 answered in time? | Result |
|------|---------------------------|--------------------------|--------|
| Direct chat | the active chat (`block`; drains when `starting` clears ~1s via TUI-ready) | yes | gray |
| Mission, fresh start | `activeTab="feed"`, so the slot `Pane` is `hidden` (`display:none`); its `active` prop never flips true until the user clicks the tab — far past codex's startup timeout | no | black |
| Mission, after resume | the user resumes the slot they are already viewing → visible + active | yes | gray |

When the user finally opens the slot's tab, xterm drains the buffered bytes and replies to the now-stale OSC 11 query — but codex has already cached its theme, so the composer stays black until a full respawn.

## Superseded hypothesis (shipped, but not the fix)

An earlier version of this doc attributed the black composer to a local `\x1b[2J\x1b[H` clear emitted by the `ResizeObserver`-driven `pushSize` racing codex's SGR-dependent repaint, before the snapshot replay had drained. That change shipped in `fdf41d2` (and a follow-up tweak): `pushSize` now early-returns while `!replayDoneRef.current`, and `skipLocalClear` is widened to cover the pre-drain window.

That guard is a defensible defensive change — it prevents a real clear-vs-repaint race — but it does **not** address this symptom. The black composer is not a repaint artifact; codex never selected the gray shade in the first place because its **startup** OSC 11 query went unanswered. No clear-suppression or resize-dance can recover a value codex only reads once at boot.

## Approach (recommended fix)

Answer codex's startup terminal queries in the **backend PTY forwarder**, independent of any frontend attachment — the same thing terminal multiplexers (tmux) do for detached panes. A mission slot is effectively a detached pane until the user opens its tab, so the terminal contract must be honored by something that is always present at spawn: the backend.

In the session reader / forwarder (`src-tauri/src/session/`, `src-tauri/src/event_bus/`), watch the child's output stream for the startup queries and immediately write canned replies back to the PTY master:

- OSC 11 background query (`\x1b]11;?`) → reply `\x1b]11;rgb:RRRR/GGGG/BBBB\x1b\\` with the user's terminal theme background.
- OSC 10 foreground query (`\x1b]10;?`) → reply with the theme foreground.
- DSR cursor-position (`\x1b[6n`) → reply `\x1b[1;1R`.
- DA1 device attributes (`\x1b[c`) → reply with a standard `xterm-256color` attributes string.

The OSC 10/11 colors should match what xterm.js would report, so plumb the user's terminal theme foreground/background into the spawn (the frontend already resolves these via `resolveTerminalTheme`); default to the dark palette if none is supplied. DSR/DA1 replies are static.

This makes codex — and any future TUI runtime — detect the terminal correctly regardless of which tab is active when it boots, with no dependency on frontend drain/visibility/`disabled` timing. xterm.js answering the same queries again later (when the tab is opened) is harmless; codex honors the first reply and ignores the rest.

## Alternatives considered

- **`COLORFGBG` env at spawn** (e.g. `15;0` for white-on-black/dark) in `base_spawn_spec`. If codex consults `COLORFGBG`, it would skip the OSC 11 round-trip entirely — a one-line fix. But codex was observed probing OSC 11 even with `appearanceTheme = "dark"` already set, which suggests it ignores ambient hints and always probes; treat this as likely-insufficient on its own. Cheap to test, not to be relied on.
- **Frontend: answer while hidden.** Construct each mission slot's xterm at the `estimateMissionTerminalGrid()` size and let hidden panes drain immediately (so xterm parses and replies to the OSC 11 query in time), and stop gating xterm's automatic protocol replies on `disabled` (the `disabled` flag is meant to suppress *user keystrokes*, not terminal-protocol responses). More invasive — it reworks the careful deferred-drain logic that exists to prevent 80-col replay drift — and remains timing-sensitive. The backend responder is preferred.
- **Eagerly activate every slot at mission start**, then snap back to feed so each xterm answers its codex's queries. Causes flicker and is race-prone. Rejected.

## Files to modify (recommended fix)

| File | Change |
|------|--------|
| `src-tauri/src/session/` (PTY forwarder / reader) | Add a startup terminal-query auto-responder: detect OSC 10/11, DSR `\x1b[6n`, DA1 `\x1b[c` in child output and write canned/configured replies to the PTY master. |
| spawn plumbing (`commands` / `session/manager.rs`) + frontend spawn calls | Pass the user's terminal theme foreground/background to the spawn so the responder reports the right OSC 10/11 colors; default dark when absent. |

No change to the `RunnerTerminal.tsx` drain logic is required for the recommended fix; the shipped `replayDoneRef` clear-suppression guard can stay as defensive hardening.

## Verification

### Manual

1. Configure a `codex` runner and add it to a crew with a lead + at least one worker.
2. Start a mission. The workspace opens on the Feed tab, so every slot terminal mounts hidden.
3. Without resuming, click each codex slot's PTY tab.

**Expected (with fix):** every codex composer shows its normal gray background on first reveal — identical to a direct chat and to a post-resume slot.

**Expected (before fix):** the composer background is black until the slot is stopped and resumed.

4. Cross-check direct chat is unregressed: "Chat" into the same codex runner — composer stays gray.
5. Switch the user's terminal theme to a light palette in Settings, start a fresh mission, and confirm the composer shade matches the light background (validates that the responder reports the configured colors, not a hardcoded dark value).

### Edge cases

- **Light vs dark theme:** the composer shade should track the active terminal theme, since the responder reports the theme's background.
- **Resume still works:** resuming a slot must remain gray (the backend responder answers the respawned codex too).
- **Non-codex runtimes:** claude-code / shell slots should be unaffected — the canned replies are standard terminal responses any well-behaved client tolerates.
- **Tab opened mid-startup:** opening a slot tab while codex is still booting should not double-paint or corrupt — the backend reply plus xterm's later reply are both benign.

### Capturing codex's startup queries (repro for the root cause)

Spawn codex under a PTY with `TERM=xterm-256color`, `COLORTERM=truecolor`, and no `COLORFGBG`, read the first ~1KB without replying, and confirm the stream contains `\x1b]11;?` (OSC 11), `\x1b]10;?` (OSC 10), `\x1b[6n` (DSR), and `\x1b[c` (DA1). This is the handshake the backend responder must satisfy.

### CI gates

Backend change: `cargo fmt`, `cargo clippy`, and `cargo test --workspace` (per the run-CI-checks-before-pushing convention). Any frontend spawn-plumbing edit also needs `pnpm exec tsc --noEmit` + `pnpm run lint`.
