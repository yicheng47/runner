# Launch auto-resume: fork at the width the pane will actually have

## Status

Implemented. Tracking issue [#363](https://github.com/yicheng47/runner/issues/363). **Corrects [0035](0035-auto-resume-width-and-opt-in.md)**, which fixed the steady state of this bug but left the transient — see Problem.

## Problem

0035 closed two real holes: a stopped session's row can now learn its pane geometry (decision 1), and a pane re-asserts its size when its session goes live (decision 2). Both are right, and both operate *after* the PTY has already forked.

The launch path never supplies a width at all. `resumeOnLaunch` passes `cols: null, rows: null` (`src/lib/api.ts:371-377`), so `resume()` resolves the size as `cols.zip(rows).or(snap.last_cols.zip(snap.last_rows)).unwrap_or(DEFAULT_PTY_SIZE)` (`spawn.rs:1078-1080`) — meaning a launch resume always forks at **last-quit geometry**, or 80×24 when the row has none.

The fork happens first. The agent CLI immediately emits its resume banner and first repaint at that width. Only then does `reassertSizeRef` (`RunnerTerminal.tsx:830`, fired on `wentLive` at `:295`) correct the grid. For claude-code and qoder — excluded from `runtime_purges_on_resume` (`output.rs:843`), so they keep their ring across a resume — those mis-wrapped lines are permanent. No reflow undoes wrapping the agent performed itself.

So 0035 made the *eventual* state correct and left the *visible* damage in place. Two of its decisions need correcting:

- **0035 decision 3 ("do not fix this with ordering") was too strong.** The concern — don't let correctness depend on winning a race — was sound, but it hardened into a blanket ban on sequencing. Awaiting window restore is not racing; it is a data dependency. Window restore (impl 0027) lands asynchronously while `consumeResumeOnLaunch` drains its queue on a 300ms stagger (`src/lib/autoResume.ts`), so any geometry read before restore settles is wrong regardless of who wins.
- **0035 decision 4 ("no correct width to give background sessions") was too pessimistic.** `terminalGridFromPixels`, `terminalGridFromElement`, `estimateMissionTerminalGrid`, and `pickRespawnDims` (`src/lib/terminalSizing.ts`) already derive a grid with no mounted terminal. An estimate from *this* launch's window beats a persisted value from the previous quit — which is stale by construction whenever the window moved, resized, changed display scale, or the session now renders in a split.

## Key Decisions

1. **Await window-restore settle before the first fork.** The auto-resume queue must not start until the window has its final geometry. This is a correctness prerequisite for every dims computation below, not a timing optimization — no computed width is trustworthy before it. How to observe "settled" is the main open question (see below); pick one mechanism and make it explicit rather than sleeping a magic number.
2. **Supply dims at the call site; stop passing `null`.** `resumeOnLaunch` gains cols/rows. `session_resume` already threads them (`commands/session.rs:620-649`), so this is caller-side only. Precedence for what to send, best first: **measured** from the session's laid-out container when it has one → **estimated** from current window geometry via the `terminalSizing.ts` helpers → **persisted** `last_cols`/`last_rows` → `DEFAULT_PTY_SIZE`. Today's chain starts at step three; this prepends the two rungs that reflect reality.
3. **Prefer deferring over guessing for panes that are about to exist.** A session whose pane is mounting should fork against that pane's real measurement rather than an estimate. Where that can be awaited cheaply without stalling the queue, do it; where it cannot, fall to the estimate. Do not stall the whole queue on one pane — a slow or never-mounting surface must not block the rest (0035's failure-tolerance rule from #320 still holds).
4. **0035's decisions 1 and 2 stay, as the safety net.** Persisting geometry while stopped and re-asserting on going live remain correct and still catch anything dropped in flight (session exiting mid-resize, crash-and-resume, window handoff). This impl removes the need for them to be the *primary* mechanism, not the mechanisms themselves.
5. **Record the correction in 0035.** Its decisions 3 and 4 currently read as considered acceptances, and someone will hit this again if they stand unqualified. Add a short note pointing at 0038 and this issue.

## Open questions — resolved

- **Observing window-restore settle → the native frame is the authority; the viewport is the observation.** `window_state::restore` runs inside Tauri `setup`, before the webview evaluates any script, so the native frame is already final when the frontend starts; what lags is the webview's layout catching up to it (and `app_ready` reveals the window on top of that). `awaitWindowGeometrySettle` (`src/lib/windowSettle.ts`) therefore polls until the viewport width agrees with `getCurrentWindow().innerSize().width` — an agreement check against an authoritative value, not quiescence and not a sleep. It returns instantly in the common case where they already agree. `WINDOW_SETTLE_CEILING_MS = 1500` is the named ceiling: hitting it (or finding no Tauri window at all, as in browser preview) logs the outcome and proceeds rather than hanging. Width only — it is the dimension that decides wrapping, and both axes settle in the same layout pass.

  Both sides are normalized to **logical points**, and the page zoom is taken from `readAppZoom()` rather than inferred from `devicePixelRatio`. WebKit does not fold page zoom into `devicePixelRatio` (webkit#124862) — it reports the display scale and lets `innerWidth` absorb the zoom — and wry implements `setZoom` as `WKWebView.setPageZoom`. A DPR-based identity would therefore never converge at any non-100% app zoom, and every launch would silently burn the full ceiling. `windowSettleDeps` isolates that conversion so it is covered by tests rather than by assumption.

  A Tauri window event was considered and rejected as the primary path: `onResized` fires only if a resize *happens*, so the common "already correct" launch would wait for the ceiling every time — a timer wearing an event's clothes.
- **Multi-window → does not arise at launch.** Secondary windows are created on demand by `window_open` with a fresh `window-<ulid>` label (`commands/window.rs`); nothing recreates them at startup, and the main window always boots on `/runners`. At the moment the queue drains, `main` is the only window that exists, so main-window geometry *is* the destination geometry for every stamped session. If window-set restoration ever lands, this becomes real and the dims resolver needs a per-window destination.
- **Split panes → `estimateMissionTerminalGrid` does not handle the divisor, and should not.** It sizes the mission workspace, which shows one slot terminal at a time. The chat surface is where splits live, so the divisor is threaded in separately: `chatPaneAreaBox()` (`terminalSizing.ts`) yields the tab's whole pane area from `<main>` minus the chat topbar and side panel, and `paneBoxForSession()` (`paneLayout.ts`) divides that area the way `ChatPaneGroup` renders it — percentage splits over the axis minus each 1px separator, clamped to the Panels' 120px `minSize`, then the 34px pane header and 1px focus-ring border once the tab is grouped. The clamp matters because a window restored narrower than the one the sizes were dragged in makes the raw percentages a lie, and Runner sets no window `minWidth`: a pane under the floor renders at the floor with its sibling paying, and once the axis is under 2× the floor neither can be satisfied — the library pins both to the same minimum, and since panels lay out as `flexBasis: 0` with `flexGrow` set to their layout value, two equal values split the axis evenly however lopsided the drag was. Pane *count* alone would not do: `main-2` and `cols-3` are both three panes with different boxes. The layouts come from persisted tab state, so this needs nothing mounted; `App.tsx` awaits `hydratePaneLayoutsFromDb()` alongside the settle gate so the divisor is never read from an unhydrated store.

## Goals

- A session resumed at launch forks at the width it will be displayed at, so its banner and first repaint need no post-hoc correction and leave no mis-wrapped scrollback.
- Changing the window size, display, or sidebar width between quit and relaunch does not produce mis-wrapped output.
- A session whose row has no persisted geometry no longer falls to 80×24 when the window's actual geometry is knowable.
- One slow or absent pane does not block the rest of the resume queue.

## Non-Goals

- Reverting or weakening 0035 decisions 1–2.
- Repairing scrollback already mis-wrapped by earlier versions — impossible once the agent wrapped it.
- Changing the 300ms stagger, the quit-side stamp, resume-never-fresh-spawn, or crash-never-stamps (#320).
- Making auto-resume default-on again.

## Implementation Phases

### Phase 1 — settle gate

- `src/lib/windowSettle.ts`: `awaitWindowGeometrySettle` resolves once the webview viewport agrees with the native frame, or at `WINDOW_SETTLE_CEILING_MS`, or immediately when there is no native window to read. Deps are injected so the gate is testable without a window.
- `src/App.tsx`: awaited before `consumeResumeOnLaunch`, together with `hydratePaneLayoutsFromDb()` — the other input the dims computation depends on. Originally gated on the resume toggle; since #367 the settle gate runs on every launch, because the mission grid-hint push (`estimateMissionTerminalGrid()` → `mission_grid_hint_set`) needs settled geometry regardless of the toggle. Only the pane-layout hydration remains toggle-gated; the default-off path still clears stamps right after the gate.
- Tests (`windowSettle.test.ts`): settles with no wait when the viewport already agrees; waits out a lagging viewport; accepts rounding within tolerance; returns `timeout` at the ceiling instead of hanging; returns `unavailable` when the native size cannot be read.

### Phase 2 — dims at the call site

- `src/lib/api.ts`: `resumeOnLaunch` takes cols/rows.
- `src/lib/autoResume.ts`: `resolveLaunchDims` owns the precedence and guards each source; `consumeResumeOnLaunch` takes a `dimsFor` callback and resolves per session *after* the stagger, with its own guard so one unresolvable session cannot abort the drain.
- `src/lib/launchDims.ts`: wires the two frontend rungs. Measured = `terminalGridFromHostElement` on the `data-terminal-session` host RunnerTerminal now stamps (works for both surfaces, no surface-local registry needed). Estimated = chat pane box for tab members, `estimateMissionTerminalGrid()` otherwise.
- The last two rungs stay in the backend: a `null` from here is exactly what selects `last_cols`/`last_rows` then `DEFAULT_PTY_SIZE` in `spawn.rs`, so the chain is unchanged below the frontend.
- Tests (`autoResume.test.ts`, `launchDims.test.ts`, `paneLayout.test.ts`): precedence order; a session with no mounted pane still gets an estimate rather than `null`; a throwing or degenerate source falls through; one failing resolution does not abort the queue; pane boxes for single / 2-column / nested presets.

### Phase 3 — verification and record

- Regression covered at both seams: the queue sends the resolved dims rather than `null, null` (`autoResume.test.ts`), and the estimate tracks the current window rather than anything persisted (`launchDims.test.ts`).
- Append the correction note to 0035 decisions 3 and 4.
- Full check matrix: `cargo fmt --check`, `cargo clippy --workspace`, `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.

### Incidental fixes

Two pre-existing errors in `estimateMissionTerminalGrid`. Both are fixed here rather than deferred, because this impl newly feeds that helper's output to launch resume — left alone they would reintroduce #363 through a different input. `StartMissionModal`'s spawn dims get the same corrections.

- **Rail width.** The width was read as `Number(localStorage.getItem(...))`, and `Number(null)` is `0` — finite, so the "nothing stored" branch never ran and an un-dragged rail estimated at the 200px minimum instead of its 288px default, overstating the terminal by 88px. The shared `storedSideWidth` helper now falls to the default when nothing is stored.
- **Header height.** `MISSION_HEADER_HEIGHT_PX` was 88, but the mission topbar is `h-11` (44px) with only the `h-[38px]` tab strip below it — the estimate was subtracting 44px of chrome that does not exist and forking several rows short. Corrected to 44.

`missionPaneAreaBox()` was split out of `estimateMissionTerminalGrid` so both surfaces' chrome arithmetic is assertable without a real xterm fit — the mock in `launchDims.test.ts` cannot see a stale constant, which is how this one survived.

## Verification

Automated:

- [x] `resumeOnLaunch` sends non-null dims whenever any rung of the precedence chain resolves.
- [x] Precedence order holds: measured → estimated → persisted → default. The frontend covers the first two rungs and the `null` that selects the last two; the persisted → default resolution itself is 0035's Rust coverage, unchanged.
- [x] The consumer waits for the settle signal, and degrades rather than hangs if it never arrives.
- [x] A per-session dims failure does not stop the queue.

Estimates are close, not exact: both estimates model chrome from constants (topbar, tab strip, pane header, separator, focus-ring border, pane minimum) rather than measuring it, so a layout change that moves those constants silently biases the estimate. The 0035 re-assert still corrects the grid afterwards — but only the *fork* width prevents mis-wrapped scrollback, so these constants are load-bearing. `terminalSizing.test.ts` and `paneLayout.test.ts` assert them directly for that reason; a drifted constant should fail a test, not a smoke test.

Manual (Jason smoke-tests):

- [ ] Enable auto-resume, run a claude-code chat, quit, **resize the window materially**, relaunch → banner wraps at the new width, no mis-wrapped lines in scrollback.
- [ ] Same across a display with different scaling.
- [ ] Same with the sidebar widened, and with the session in a 2-pane split.
- [ ] A session with no persisted geometry no longer comes back at 80 columns.
- [ ] Several sessions resuming together all come back correct, none blocking the others.
