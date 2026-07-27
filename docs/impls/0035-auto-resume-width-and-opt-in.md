# Auto-resume: correct fork width, and make it opt-in

## Status

Implemented. Follows [0032](archive/0032-in-place-resume-seam-and-width-hardening.md) (which introduced the persisted-dims fallback this bug lives in) and the auto-resume feature shipped in 0.4.2 ([#320](https://github.com/yicheng47/runner/issues/320), spec `docs/features/45-auto-resume-on-launch.md`). Two changes: fix the fork width, and flip the setting to opt-in.

## Release note

Auto-resume is now opt-in, including for existing users who previously received it through the default. Turn on **Settings → General → Resume running agents on launch** to restore the prior launch behavior.

## Problem

Sessions restored by auto-resume-on-launch come back at the wrong terminal width. Manual resume does not have this bug. The asymmetry is the tell, and it traces to a five-step mechanism where two independent holes line up:

1. **Auto-resume asks for no particular size.** `api.session.resumeOnLaunch` invokes `session_resume` with `cols: null, rows: null` (`src/lib/api.ts:371`), unlike manual resume which measures the live pane first — `const dims = terminals.get(targetId)?.measure() ?? null` (`src/pages/RunnerChat.tsx:1185`, whose own comment at `:1182` already notes "measure() can be null right after a restart").
2. **So the backend falls back to persisted dims.** `resume()` resolves `cols.zip(rows).or(snap.last_cols.zip(snap.last_rows)).unwrap_or(DEFAULT_PTY_SIZE)` (`src-tauri/src/session/manager/spawn.rs:1056-1059`) — exactly the 0032 decision-5 chain. Correct by design; the inputs are what's wrong.
3. **Hole one — `last_cols`/`last_rows` can't be updated while a session is stopped.** The only writer outside spawn is `SessionManager::resize` (`output.rs:488`), and its first line is `let rt_session = self.live_runtime_session(session_id)?` (`:489`), which returns `Err("session not found")` whenever the session has no live handle (`manager/mod.rs:1029-1035`). The `update_last_size` call sits *after* that early return (`:492`). A pane that mounts and measures while its session is still stopped therefore records nothing.
4. **Hole two — the frontend believes it already pushed.** `RunnerTerminal`'s mount effect fits, sets `lastPushedColsRef.current = term.cols`, then fires `api.session.resize(...).catch(() => {})` (`RunnerTerminal.tsx:531-543`). The ref is updated unconditionally and the rejection from step 3 is swallowed. Since `pushSize` dedupes against that ref, the frontend will **never re-send** that size unless the pane's geometry genuinely changes.
5. **Result.** At launch the pane mounts and pushes `W_real` → dropped, because the session hasn't been resumed yet. Auto-resume then forks the PTY at the stale persisted `W_old`. The agent's resume banner and first repaint are emitted hard-wrapped at `W_old`. Nothing corrects it: the frontend thinks `W_real` is already in flight, and the backend never heard it. For claude-code — which *keeps* its ring on resume (`runtime_purges_on_resume`, `output.rs`) — those miswrapped lines stay in scrollback permanently, since no reflow can undo wrapping the agent performed itself.

Manual resume escapes all of this because step 1 supplies real dims from an already-laid-out pane.

Note this is a genuine race, not a fixed ordering: `consumeResumeOnLaunch` fires from `App.tsx:51-61` immediately after `app_ready`, concurrently with route/pane mounting. Whether the mount push or the resume wins is timing-dependent — which is why the fix must not be a timing fix.

## Key Decisions

1. **Persist pane geometry independently of PTY liveness.** Split `SessionManager::resize` so recording `last_cols`/`last_rows` happens before the missing-live-handle early return, and applying the resize to the PTY happens only when a handle exists. Rationale: the dims are a fact about the *pane that will display this session*, not about the process — a stopped session's row should learn its geometry so the next spawn or resume forks correctly. This closes hole one and makes the persisted fallback trustworthy on its own. A resize for a nonexistent session still returns a genuine not-found error and does not create manager state.
2. **Re-assert size when a session goes live, instead of trusting a dedupe ref that may describe a dropped call.** A dedicated disabled-state effect detects the stopped→running transition, clears `lastPushedColsRef`/`lastPushedRowsRef`, and invokes `reassertSizeRef` to fit and issue one plain deduped push. This closes hole two and, unlike decision 1, also repairs any *other* path where a resize was rejected in flight (session exiting, crash-and-resume, window handoff).

   Stopped and transitional panes deliberately differ: stopped panes keep pushing every laid-out geometry measurement so later zoom, sidebar, split-layout, and window changes can correct the persisted size, while transitional panes suppress resize until the fork completes. Stopped pushes are plain RPCs that skip both the local TUI clear and the forced resize dance, and they preserve `replayJustDrainedRef` so a geometry-only push cannot consume the next clear-capable push's replay protection.
3. **Do not fix this with ordering.** The obvious alternative — defer `consumeResumeOnLaunch` until layout settles, or have it await a "surfaces ready" signal — makes correctness depend on a race we'd have to keep winning as the boot sequence evolves. Decisions 1 and 2 make the outcome correct whichever side wins: if the mount push lands first, the fork is already right; if the resume lands first, the correction fires the moment the pane is live and measurable. No new timing dependency.

   **Corrected by [0036](0036-launch-resume-fork-width.md) / [#363](https://github.com/yicheng47/runner/issues/363).** The concern was sound but stated too broadly. Both decisions here act *after* the PTY forks, so they fix the eventual state and leave the agent's resume banner wrapped at the fork width — permanently, for runtimes that keep their ring across a resume. Awaiting window-restore settle is not the race this warned about: restore lands asynchronously, so *any* geometry read before it is wrong regardless of who wins. 0036 gates the queue on that settle.
4. **Accept that background sessions still fork at their last known width.** Most auto-resumed sessions have no mounted pane at all — only the active tab and the persistent surfaces are mounted. There is no correct width to give them, and the persisted one (now trustworthy per decision 1) is the best available guess. When the user later opens such a tab, activation pushes real dims and the agent repaints. Explicitly not solved here: history a background agent hard-wrapped before you ever looked at it. That's the same irreducible limit 0032 recorded.

   **Corrected by [0036](0036-launch-resume-fork-width.md) / [#363](https://github.com/yicheng47/runner/issues/363).** "No correct width" was too pessimistic. `terminalSizing.ts` already derives a grid with nothing mounted, and the persisted tab layouts already say which pane a session returns into — so a background session can be estimated from *this* launch's window and its own share of the split. That beats a value persisted at the previous quit, which is stale by construction whenever the window moved, resized, changed display scale, or the session now renders in a split. The persisted value stays as the rung below the estimate.
5. **Auto-resume becomes opt-in, default off.** Export one `DEFAULT_RESUME_ON_LAUNCH = false` constant and use it in both the `App.tsx` consumer and the `GeneralPane.tsx` toggle. Sharing the constant structurally prevents the two defaults from drifting and rendering a toggle state the consumer does not act on.
6. **No migration for existing users, but the behavior change is real and must be stated.** The setting is stored under `settings.resumeOnLaunch` and read with a default; anyone who never touched the toggle has no stored key, so flipping the default silently turns auto-resume off for them too. That is the intent — spawning agents unprompted at launch should be something you choose, not something you discover. It shipped only two releases ago (0.4.2), so the blast radius is small, and writing a one-time "preserve previous default" key would leave a permanent wart to explain. Call it out in the release notes instead.
7. **The quit-side stamp stays unconditional.** The backend continues to mark sessions at graceful quit regardless of the toggle, and a launch with the toggle off still *clears* pending stamps without resuming (the edge case #320 built deliberately). This keeps enabling the setting a forward-looking act rather than something that resurrects a session set from an old quit.

## Goals

- A session restored by auto-resume forks its PTY at the width of the pane that will display it, and its resume banner is not hard-wrapped at a stale width.
- A resize issued against a stopped session updates the row, so the next spawn or resume uses it.
- A session that goes live re-asserts its pane geometry rather than assuming an earlier push landed.
- Auto-resume is off unless the user turns it on; the toggle and the consumer agree on that default.

## Non-Goals

- Repairing scrollback already hard-wrapped at a previous width — impossible once the agent wrapped it (0032 non-goal, restated).
- Giving background, never-displayed sessions a "correct" width (decision 4).
- Reordering or gating the boot sequence (decision 3).
- Any change to the quit-side stamping, the 300ms stagger, resume-never-fresh-spawn, or the crash-never-stamps rule from #320.

## Implementation Phases

### Phase 1 — backend: record dims regardless of liveness

- `src-tauri/src/session/manager/output.rs`: restructure `resize` so the `update_last_size` write runs before the missing-live-handle early return, and the runtime resize plus the `cols_changed`/`runtime_clears_on_resize` purge run only when a live handle exists. Preserve current behavior exactly for live sessions — the ring-purge rules and the rows-only-keeps-ring reasoning are unchanged.
- A resize on a nonexistent session returns a genuine not-found error and does not create manager state. Persisting geometry for an unknown id must not silently create state.
- Tests in `session/manager/tests.rs`: resizing a stopped session updates `last_cols`/`last_rows` and does not error; a subsequent resume with `None` dims forks at those dims; resizing a live session still purges the ring on a cols change and still keeps it on a rows-only change.

### Phase 2 — frontend: re-assert geometry on going live

- `src/components/RunnerTerminal.tsx`: use a dedicated disabled-state effect to reset `lastPushedColsRef`/`lastPushedRowsRef` on the transition into a running session, then fit and issue a plain deduped push through `reassertSizeRef`. Keep geometry enabled for stopped panes but disabled for transitional panes; do not disturb the transitional latch's dance-suppression behavior, which #352 depends on.
- Verify the interaction with #352's activation change: the re-push must be a plain deduped push, **not** a forced resize dance — the goal is to inform the backend of a size it never received, not to make the agent repaint.
- Helper tests in `src/lib/terminalResize.test.ts`: a push that the backend rejects must not leave the frontend believing it succeeded; the stopped→running transition resets the dedupe state so the current dims need one push and are deduped afterward. The xterm effect wiring remains a manual check because the repository has no component harness for `RunnerTerminal`.

### Phase 3 — opt-in default

- `src/App.tsx` and `src/components/settings/GeneralPane.tsx`: use the shared false default at both sites.
- Check the GeneralPane row's sub-copy still reads correctly for an off-by-default control.
- Test that the consumer does not resume when the key is absent, and still clears pending stamps in that case (decision 7).

### Phase 4 — docs

- `docs/features/45-auto-resume-on-launch.md`: the spec says "One toggle in Settings … default on." Update it and record why.
- The GitHub release notes must manually include this document's release note stating that auto-resume is now opt-in, including for users who had it working by default (decision 6); the repository has no changelog that publishes it automatically.

## Verification

Automated:

- [x] Resize a stopped session → `last_cols`/`last_rows` updated, no error.
- [x] Resume with `None` dims after such a resize → forks at those dims, not `DEFAULT_PTY_SIZE` and not the pre-quit value.
- [x] Live-session resize behavior unchanged: cols change purges the ring for full-repaint runtimes, rows-only keeps it.
- [x] Push-state helpers clear the pushed-dims refs on going live and dedupe the next size after one push; the xterm effect wiring has no component harness and remains part of manual verification.
- [x] Toggle absent → no resume, stamps cleared.

Manual (Jason smoke-tests):

- [ ] With the toggle **on**, run two chats and a mission, quit, relaunch → restored panes are wrapped at the current window width, with no short-wrapped resume banner and no shredded box-drawing above it.
- [ ] Same, but resize the window materially before relaunching → restored panes still match the new width.
- [ ] Switch to a background auto-resumed tab → it repaints at the correct width on activation.
- [ ] Fresh profile (or cleared `settings.resumeOnLaunch`) → nothing auto-resumes; the General toggle reads off.
- [ ] Turn the toggle on, quit, relaunch → auto-resume works as before.
- [ ] Resume a visible stopped pane and confirm the log records one plain size push, without a forced resize dance.
