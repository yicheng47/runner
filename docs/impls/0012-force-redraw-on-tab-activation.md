# Force Agent Redraw on Tab Activation

> Fixes [#177](https://github.com/yicheng47/runner/issues/177).

## Context

Switching back to a runner's PTY tab after it has accumulated output
while hidden shows a mostly-black canvas with scattered text fragments
instead of the agent's current rendered state. A manual window resize
fixes it. Reproduces for every TUI runtime (`codex`, `claude-code`,
`gemini`, …) once enough output landed while the tab was inactive.

The activation effect at `src/components/RunnerTerminal.tsx:656–707`
already does the right local-side work — `fit.fit()`, atlas clear,
`term.refresh()`, focus — but `refresh()` only repaints the current
xterm buffer. The buffer itself is the problem: while the pane was
hidden, live PTY bytes wrote straight into it
(`RunnerTerminal.tsx:605–607`). By the time the user returns the
buffer is either mid-frame (the agent was halfway through a redraw at
the moment of switch-away) or dim-mismatched (the window or a side
panel resized while the pane was hidden, so xterm's grid no longer
matches what the buffer was painted at). Refreshing it just shows the
broken state.

The pushSize path inside the activation effect is deduped against the
previously-pushed cols/rows (`:694–697`) — a same-dim return never
re-pushes, so the agent never receives a SIGWINCH and never knows to
repaint.

The "drag the window edge a pixel" workaround works because a real
dim change releases the dedup, the backend resize fires, and the
agent's full SIGWINCH redraw lands.

## Approach

Force the agent to repaint on every tab activation by doing a
SIGWINCH dance (resize one row below current → resize back) so the
kernel's TIOCSWINSZ dedup can't suppress the signal. Pair it with a
viewport wipe for TUI runtimes so the user sees a clean black canvas
during the few-ms gap before the agent's redraw lands, instead of the
mid-frame mess.

Why a dance and not just an unconditional `api.session.resize`: both
Linux and macOS pty drivers compare the incoming `struct winsize`
against the cached value and only signal `SIGWINCH` when the bytes
differ
(`drivers/tty/tty_io.c:tty_do_resize` on Linux,
`bsd/kern/tty.c:ttioctl_locked` TIOCSWINSZ branch on Darwin). A
single-call same-size resize is a no-op at the kernel level. Two
resizes — `(cols, rows-1)` then `(cols, rows)` — guarantee the cached
winsize changes between calls, so each ioctl produces a SIGWINCH.

Why we perturb **rows**, not **cols**. An earlier draft of this plan
used `(cols-1, rows) → (cols, rows)`. That worked for the alt-screen
case (claude-code's alt buffer) but corrupted scrollback under any
main-screen TUI mode: claude-code (and similar agents) wrap their
own text by emitting explicit `\n` at their computed cols boundary,
not relying on terminal auto-wrap. The intermediate `(cols-1)` paint
deposited hard-wrapped narrow lines into xterm's buffer. xterm can
only soft-reflow width-wrapped lines; explicit newlines stick. Every
tab return then layered another stripe of narrower lines into
scrollback, visible on scroll-up even though the current viewport
looked fine. Row perturbation keeps content width at `cols`
throughout — the `(rows-1)` intermediate paint is just one line
shorter, and the second SIGWINCH's `\x1b[2J`-led repaint at the
final row count cleans it up. No width pollution.

Tradeoff: a one-cycle `(rows-1)`-tall intermediate frame may flash
through the buffer before the second SIGWINCH's `rows`-tall repaint
lands. The viewport wipe hides this for TUI runtimes. For plain
shells (no `runtime` in `runtimeClearsOnResize`), the intermediate
is harmless — shells don't redraw on SIGWINCH.

---

## Step 1: Replace the deduped pushSize in the activation effect with a SIGWINCH-guaranteeing dance

**File: `src/components/RunnerTerminal.tsx`**

In the activation effect (`useEffect` at `:650`), replace the
conditional pushSize block at `:684–703` with an unconditional
viewport wipe + dance. The wipe matches the existing pushSize behavior
at `:417–421`: clear the visible region only for runtimes where
`runtimeClearsOnResize` is true (claude-code, codex; plain shells
keep their history).

Replace the conditional pushSize block with an unconditional wipe +
rows-perturbing dance:

- For TUI runtimes (`runtimeClearsOnResize(runner)`), pre-wipe via
  `t.write("\x1b[2J\x1b[H")` so the user sees a clean canvas during
  the few-ms gap before the agent's redraw lands. Plain shells skip
  the wipe (matches pushSize at `:417–421`) — they keep their
  history and don't repaint on SIGWINCH.
- Update `lastPushedColsRef`/`lastPushedRowsRef` to the *final*
  `(cols, rows)` — not the intermediate `(cols, nudgedRows)` — so
  `refitAndPush`'s dedup semantics for the non-bug off-screen-layout
  paths are preserved.
- Call `api.session.resize(sessionId, cols, nudgedRows)` then chain
  `.then` to `api.session.resize(sessionId, cols, rows)`. Sequencing
  matters: parallel calls could collapse to a single kernel state
  change.
- `const nudgedRows = rows > 1 ? rows - 1 : rows + 1` so the
  pathological 1-row case still produces a real winsize-diff in both
  directions of the dance.

History note: an earlier draft of this plan used `(cols-1, rows) →
(cols, rows)` instead. That worked for the alt-screen case but
corrupted scrollback under main-screen TUIs — see the **Approach**
section above for why row perturbation is required.

## Step 2: Drop the stale comment fragment about the cols-1 → cols dance being obsolete

**File: `src/components/RunnerTerminal.tsx`**

The current comment at the deleted block (`:686–693`) says:

> Single resize is enough once xterm enters alt-screen at attach
> time (see docs/impls/0009). The earlier cols-1 → cols dance was
> there to coax claude-code into a repaint that would land where
> the user could see it; with the alt-screen state correct, the
> agent's single SIGWINCH redraw lands in the right buffer.

That's no longer true — Step 1 reintroduces the dance for a different
reason (forcing SIGWINCH on same-dim returns, not buffer-state
alignment). The replacement comment in Step 1 covers the new
reasoning; just make sure the old text is fully replaced, not
appended to.


## Step 3: No backend changes

`api.session.resize` already maps to
`commands::session::session_resize → SessionManager::resize →
PtyRuntime::resize → MasterPty::resize`. The backend is a thin
pass-through to `portable_pty`'s `tcsetwinsz`. No changes needed.

## Files to modify

| File | Change |
|------|--------|
| `src/components/RunnerTerminal.tsx` | Replace the deduped pushSize at the end of the activation effect (`:684–703`) with an unconditional SIGWINCH dance + viewport wipe for TUI runtimes. |

## Verification

### Manual

1. Start a mission with at least two runners; pick `codex` and
   `claude-code` for the most visible coverage.
2. Open a long-running task on one runner — get it producing output
   that exceeds the viewport (file dumps, a long agent turn, etc.).
3. Switch to another tab (Feed or another PTY) while output is
   actively streaming.
4. Wait ~10s for more output to accumulate.
5. Switch back to the original PTY tab.

**Expected (with fix)**: viewport flashes black briefly (TUI
runtimes), then the agent's current frame lands fully painted.

**Expected (before fix)**: black canvas with scattered, mispositioned
text fragments until a manual window resize.

Repeat for plain shells:

1. Start a `shell` runner.
2. `yes "long line of text"` or any output-flooding command.
3. Switch tabs, wait, switch back.

**Expected**: scrollback intact, current shell prompt visible. No
viewport wipe (shells aren't in `runtimeClearsOnResize`).

### Edge cases to manually verify

- **Window resize between switch-away and switch-back.** Resize the
  app window while a different tab is active, then come back. The
  pane should land at the new dims with no residual content from
  the old dims.
- **Side-panel toggle while hidden.** Collapse / expand the right
  rail while the source pane is hidden; the pane should refit and
  repaint correctly on return.
- **Rapid tab switching.** Click between tabs quickly. Each
  activation should kick a fresh redraw; no race-induced stale
  state.
- **Session exited while hidden.** If the runner exited while the
  tab was inactive, the dance's `.catch` should swallow the resize
  rejection silently. Verify by killing the runner from another
  pane and switching to the dead tab — should not throw.
- **Direct-chat (RunnerChat) panes.** Same component is used; cover
  by switching from a mission workspace to a direct chat and back
  while output is flowing.

### Automated

The activation flow is a React effect driven by xterm + IPC, so a
useful automated test would need to mock both. Existing tests in
`src/` don't cover RunnerTerminal end-to-end, and adding a JSDOM-based
xterm test for a one-edge-case redraw isn't worth the surface. Manual
verification per the steps above is the bar.

### CI gates

Standard for runner Rust changes (per
[`feedback_run_ci_checks_before_pushing.md`](../../) memory): nothing
Rust changes here, so just `pnpm tsc --noEmit` + `pnpm lint`. Run
`make lint` if reaching the full CI parity is needed.
