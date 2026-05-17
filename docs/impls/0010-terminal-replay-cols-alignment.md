# Terminal Replay Cols Alignment

> **Status: investigated, not implemented.**
> The fix below pre-resizes xterm to the snapshot's cols/rows so the
> *current* claude-code page paints correctly on reattach. After
> implementing it locally, we discovered the residual symptom that
> made us shelve it: **claude-code's previous "pages" (its own
> internal scroll history) are still drifted**, because claude-code
> only re-emits the current viewport on SIGWINCH — historical pages
> are written at whatever cols was current at their original write
> time, and no SIGWINCH from us causes the agent to re-emit them.
> The terminal layer can't repair what the agent never re-paints.
>
> Keeping this plan committed as the durable record of the
> investigation: the diagnosis (Ink absolute positioning vs.
> codex's relative wrap, why quit+restart amplifies the cols
> mismatch, why partial-fix the previous-page rendering is
> impossible from our side) should save the next person from
> re-deriving it. If someone returns to this with a strategy for
> coaxing agents into re-emitting their full scroll history,
> Steps 1–4 below are still the right shape for the *current*-page
> half of the fix.

## Context

Follow-up to `0009-terminal-alt-screen-reattach.md`. After landing the
alt-screen prepend + dropping the SIGWINCH dance, the *stacking* symptom
went away — but quit-and-restart of the app still corrupts claude-code's
terminal output. The user's example screenshot shows the architect's brief
text rendered with progressively-shifting indentation on every line, with
the visible right edge of the rendering box mid-screen (well inside xterm's
actual cols) and large empty space to the right.

### Why it only affects claude-code (not codex)

The two TUIs differ in rendering strategy:

- **claude-code** is built on Ink (React for CLI). Ink computes absolute cell
  positions for every glyph and emits `ESC[<row>;<col>H` to place text. Its
  layout is sized to *the cols it was rendered for*. Replaying those bytes
  into an xterm at a different cols sends every positioning escape to the
  wrong absolute coordinate — text scatters with the rendering-box edges
  baked in at the wrong place. xterm cannot reflow alt-screen content (and
  even main-screen reflow only works for `isWrapped` runs xterm emitted
  itself, not agent-emitted absolute positions).
- **codex** renders with relative positioning — print rows sequentially,
  rely on terminal wrap. A cols mismatch leaves trailing space on the right
  but the text reads correctly.

So this is claude-code-specific because of Ink's absolute positioning.

### Why quit+restart triggers it

State at quit:

- xterm at the user's current container cols (e.g. 200).
- We pushed that cols to tmux on the last activation, so tmux pane is at 200.
- claude-code is on alt-screen, drawing for 200.

Quit + relaunch:

1. `RunnerTerminal` mounts at the constructor default **cols=80, rows=24**
   (`src/components/RunnerTerminal.tsx:142-144`).
2. Mount effect fetches the snapshot. `attach_streaming` had already run in
   the backend and queued a `Replay` from `capture_replay_bytes` — captured
   at **tmux's current cols (200)**, now (post-0009) also prefixed with
   `ESC[?1049h`.
3. Frontend writes those 200-cols-wide bytes **into an 80-cols xterm**.
4. Activation effect runs `fit.fit()` → xterm grows to container cols (200)
   and we resize tmux back to 200.
5. claude-code SIGWINCHes and does a *diff-based* redraw — only changed cells
   are rewritten. The cells we mis-painted from step 3 stay broken.

The corruption is permanent for the lifetime of the alt-screen view (until
claude-code itself triggers a full redraw, which it rarely does).

## Approach

Replay the snapshot at *exactly* the cols claude-code was drawing for.
Backend reports tmux's current pane cols/rows alongside the snapshot bytes;
frontend resizes xterm to those dims *before* writing the bytes; only after
the snapshot is in does `fit.fit()` resize to container cols and trigger the
agent's clean in-alt-screen redraw at the new dims.

For codex / plain shells / build output this is a no-op improvement — they
were already rendering correctly at any cols; we're just no longer relying
on that property.

---

## Step 1: Runtime accessor for pane cols/rows

**File: `src-tauri/src/session/runtime.rs`**

Add a method to the `SessionRuntime` trait:

```rust
/// Current cols/rows of the pane in the runtime's view. Used by
/// the snapshot path to align xterm's grid with the bytes it's
/// about to replay (see docs/impls/0010).
fn pane_size(&self, session: &RuntimeSession) -> RuntimeResult<(u16, u16)>;
```

**File: `src-tauri/src/session/tmux_runtime.rs`**

Implement via `tmux display-message -p -t <pane> '#{pane_width},#{pane_height}'`.
Reuse the `clone_cmd` / status-check pattern already established by
`is_alternate_on` (line 1218). Parse `"<cols>,<rows>\n"` defensively; map
parse failure to `RuntimeError::TmuxFailed`.

## Step 2: Bundle cols/rows into the snapshot API

**File: `src-tauri/src/session/manager.rs`**

New type adjacent to `OutputEvent`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSnapshot {
    pub events: Vec<OutputEvent>,
    /// Pane dimensions at the moment of the snapshot. Frontend
    /// resizes xterm to these *before* writing `events` so
    /// absolute positioning escapes (claude-code/Ink) land at the
    /// correct grid coordinates. Optional — `None` for sessions
    /// whose runtime can't report the size.
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}
```

Replace `pub fn output_snapshot(&self, session_id: &str) -> Vec<OutputEvent>`
with one that returns `OutputSnapshot`. Internally:

1. Collect buffered events (current behaviour).
2. Look up the `rt_session` for `session_id` in the live map.
3. If present, call `runtime.pane_size(&rt_session)`. Treat any error as
   "we don't know" → leave `cols`/`rows` as `None`.

Sessions that have already terminated (no live rt_session) just get
`cols: None, rows: None` — the events still replay; xterm just doesn't
pre-resize. Acceptable degradation for dead sessions.

**File: `src-tauri/src/commands/session.rs`**

Update `session_output_snapshot` (line 138) return type from
`Result<Vec<OutputEvent>>` to `Result<OutputSnapshot>`.

## Step 3: Frontend pre-resize before replay

**File: `src/lib/api.ts`**

Update the `outputSnapshot` typing (line 178) to match the new struct:

```ts
interface SessionOutputSnapshot {
  events: SessionOutputEvent[];
  cols: number | null;
  rows: number | null;
}
outputSnapshot: (sessionId: string) =>
  invoke<SessionOutputSnapshot>("session_output_snapshot", { sessionId }),
```

**File: `src/components/RunnerTerminal.tsx`**

In the mount effect, replace:

```ts
let snapshot: OutputEvent[] = [];
try {
  snapshot = await api.session.outputSnapshot(sessionId);
} catch (e) { … }
termRef.current?.reset();
for (const ev of snapshot) { … }
```

with:

```ts
let snapshot: SessionOutputSnapshot = { events: [], cols: null, rows: null };
try {
  snapshot = await api.session.outputSnapshot(sessionId);
} catch (e) { … }
const t = termRef.current;
t?.reset();
if (t && snapshot.cols && snapshot.rows) {
  // Match xterm's grid to the cols/rows the captured bytes were drawn
  // for. claude-code/Ink emit absolute-positioning escapes (`ESC[r;cH`);
  // replaying them into the wrong cols paints the alt-screen in the
  // wrong cells and the agent's subsequent diff-based SIGWINCH redraw
  // doesn't repair the miswritten regions. See docs/impls/0010.
  try { t.resize(snapshot.cols, snapshot.rows); } catch { /* xterm guard */ }
}
for (const ev of snapshot.events) { … write … }
```

The activation effect's existing `fit.fit()` + `api.session.resize(cols,
rows)` continues to run after replay — xterm grows from snapshot-cols to
container-cols, tmux follows, claude-code SIGWINCHes and redraws once into
the same alt-screen, in place. The redraw is diff-based so it cheaply
updates the cells that need to change.

## Step 4: Cleanup

**File: `src-tauri/src/session/manager.rs`**

Find any tests that assert on the shape of `output_snapshot`'s return type
(grep for `output_snapshot`) and update to the new struct shape.

Drop any now-redundant comments in the mount effect that explained the
"replay-before-fit cols mismatch" rationale — Steps 1-3 supersede them.

## Files to modify

| File | Change |
|------|--------|
| `src-tauri/src/session/runtime.rs` | Add `pane_size` to `SessionRuntime` trait. |
| `src-tauri/src/session/tmux_runtime.rs` | Implement `pane_size` via `tmux display-message`. |
| `src-tauri/src/session/manager.rs` | New `OutputSnapshot` struct; `output_snapshot` returns it. Update tests. |
| `src-tauri/src/commands/session.rs` | Update `session_output_snapshot` return type. |
| `src/lib/api.ts` | Update `outputSnapshot` typing. |
| `src/components/RunnerTerminal.tsx` | Pre-resize xterm to snapshot cols/rows before writing. |
| `docs/impls/0010-terminal-replay-cols-alignment.md` | This document. |

No DB migration. No new dependencies. One new IPC field (`cols`, `rows`)
on the existing `session_output_snapshot` response.

## Verification

### Manual (the canonical reproducer)

1. Open a long-running mission with claude-code lead. Confirm the *first*
   open looks correct (this was already working post-0009).
2. **Quit the app** (Cmd+Q). Wait until process is fully gone.
3. **Relaunch the app.** Open the same mission.
4. Inspect the `@architect` pane. **Expected:** brief text reads correctly,
   no progressive indentation drift, no mid-screen "right edge" of the old
   render visible. The view should be indistinguishable from running the
   mission for the first time.
5. Resize the window after restart. **Expected:** claude-code redraws
   cleanly into alt-screen at the new cols, no stacking (0009 handles this).
6. Same test with codex — **expected:** unchanged (codex was already fine).
7. Same test with a plain shell session that's accumulated scrollback —
   **expected:** scrollback reads correctly; main-screen reflow continues
   to work for `isWrapped` runs.

### Edge cases

- **Session terminated between attach and frontend mount.** `pane_size`
  errors out (pane gone). Snapshot returns `cols: None, rows: None`. Frontend
  skips the pre-resize. xterm replays at default cols. Dead session — the
  content is a static remnant; user can't interact further; minor visual
  imperfection is acceptable.
- **First-time spawn (no prior render).** `attach_streaming`'s fresh-spawn
  branch uses `capture_visible_region`, not `capture_replay_bytes`; the
  buffer at snapshot time is small or empty. `pane_size` returns whatever
  size tmux is at (matches `initial_size` from spawn spec). Frontend
  pre-resizes to that. Activation then grows to container cols. Net visual:
  one extra resize tick on first open; should be imperceptible.
- **Container size changed between quit and restart.** Snapshot is at the
  cols tmux remembers (pre-quit user size). Frontend resizes xterm to that
  → replay correct. Then `fit.fit()` grows/shrinks to the new container
  size, tmux resizes, claude-code SIGWINCH redraw → clean final state at
  the user's new dims. This is the primary case the fix is designed for.
- **Two RunnerTerminals for the same session (split view).** Both call
  `outputSnapshot` independently. Both pre-resize their xterm to the same
  cols. After activation, each fits to its own container; the runtime
  resize calls race but tmux is single-writer so the last one wins. No
  worse than today.

### Automated

The interesting test would be: capture a known PTY byte stream with
positioning escapes targeting cols=200, replay it into an 80-cols xterm,
assert layout corruption; then re-run with pre-resize, assert correct
layout. That requires a headless xterm.js harness which we don't have.
Defer to manual verification.

## Risk

The IPC shape change (`Vec<OutputEvent>` → `OutputSnapshot`) is a breaking
change for any code calling `session_output_snapshot`. Only one consumer
exists (`api.ts:177`), so the blast radius is contained — both sides ship
together in the same release.
