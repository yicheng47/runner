# M6.6 Smoke Test — terminal resize smoothness + Claude fullscreen default

Release-readiness check for the M6.6 nightly (`docs/impls/archive/gpui-rewrite/m6-consolidation.md` §M6.6, brief in `docs/impls/archive/gpui-rewrite/briefs/m6-6-terminal-resize-smoothness.md`). What changed underneath: the PTY ioctl now fires on every resize instead of 175 ms after the drag stops; the settle only purges the output ring and nudges a repaint — it no longer injects `ESC[2J ESC[H` into live grids; `last_size` and the resize policy are no longer read or written per frame on the UI thread; claude-code spawns get `--settings {"tui":"fullscreen"}` unless the runner's own args already pass `--settings`.

Two lanes, ten minutes total. Run the human lane on the nightly build, not `make run`, with one Runner instance and no crew mission in flight.

## Agent-run checks

```sh
make verify
RUNNER_RESIZE_TRACE=1 make run   # only if the crew shipped the trace flag; watch ioctls-per-drag in the log
```

Expected in `crates/runner-backend` tests: the rewritten M3.3 coalescing suite (ioctl per resize, one purge/nudge per storm, `last_size` persisted once per storm, policy cached), the width-chain tests (fork, reset, resume, attach at the recorded size), and the `claude_settings_args` cases (inject / skip on runner `--settings` / codex and shell empty).

## Human smoke checks — ordered by what could hurt daily use

Setup: resume-on-launch on; one shell chat, one direct claude chat, one two-slot mission (claude lead + codex), one four-pane tab.

1. **Width drag over the codex slot.** The frame tracks the drag continuously; no snap when you let go; scroll up afterwards — history has no duplicated block and no wrong-width garbage rows. This is the headline fix.
2. **Width drag over the claude slot.** Full-frame repaint tracks the drag; `/tui` inside the slot reports the fullscreen renderer; no history artifacts.
3. **Width drag over the shell chat.** Prompt and prior output reflow; `stty size` in the shell equals the visible grid after the drag.
4. **Split-gutter drag in the four-pane tab.** All four panes track; none stays at a stale width (check `stty size` in a shell pane; codex/claude frames fill their pane).
5. **Re-attach after a resize.** Resize, switch to another tab and back; then open the same session in a second window (⇧⌘N). Both paint immediately at the right width — a blank pane that only fills on the next output means the purge landed after the nudged repaint (reviewer hunt 2).
6. **Fork/reset after a resize.** Reset the mission (and fork if available) — the new sessions spawn at the current width, no narrow first paint.
7. **Quit right after a resize, relaunch.** With resume-on-launch, the resumed panes come up at the right width.
8. **Kill during a drag.** Kill a slot mid-drag — no crash, the pane shows the exited state, `~/Library/Logs/com.wycstudios.runner/runner.log` has no `settled resize failed` spam.
9. **Status during a drag.** Drag over an idle claude slot — the sidebar status does not flap to working; after the drag it reads idle within ~1.5 s.
10. **Agent plumbing under the fullscreen renderer.** Start a fresh mission with a claude lead: the launch brief is pasted and submitted on the first turn; a `runner msg` from the codex slot lands in the lead's composer and is delivered; the mission feed shows both. Jason's user file says `"tui": "default"` on purpose: a fresh Runner slot must still come up fullscreen, which proves the injection end to end; `/tui default` typed inside the slot switches it live for that session only.

## What this cannot break

`main` (the Tauri app) is untouched; this is `gpui-nightly` only. Shell chats already resized immediately before M6.6, so items 3 and 4 are regressions-only checks. A user who wants Claude's default renderer inside Runner opts out per runner with `--settings {"tui":"default"}` in the runner's args; outside Runner their own settings are untouched.

## Known risk, in order

Ring purge ordering on re-attach (item 5) is the one place a wrong implementation shows as a user-visible bug rather than a cosmetic one; everything else in the resize path degrades to "one extra repaint". The fullscreen default only touches claude-code argv and overrides the user file by design; the residual risk is Claude Code versions without the `tui` key printing a settings warning at startup.

## M6.8 addendum — long-lived terminals (nightly after `6cfae2d`)

The ring is gone; panes borrow a per-session `Term` that lives in the `TerminalBridge` registry. Item 5 above is now the headline: after a resize, switch tabs away and back, change mission ↔ chat route, close and reopen a pane — content is there immediately, no blank, no flash. Add:

11. Start a mission with its pane hidden, open it later — full history from the first byte.
12. Resume and reset a session — fresh paint, no leftover frame. Kill a session — the pane shows the Ended/Resume card over a neutral background (the final screen is no longer kept; deviation recorded in the impl_log).
13. Answer an ask card on a stopped slot — one "queued until the session resumes" warning in the feed, then delivery on resume.
14. Two windows on the same tab set: assign a chat to a pane from window B and watch window A's pane attach (the render-time attach safety net is gone; no repro was found, this is the check).
15. Activity Monitor after an hour with several live sessions — memory is bounded by live `Term`s (10,000-line scrollback each) and drops when sessions are archived.
