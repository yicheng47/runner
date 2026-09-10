# Confirm quit while work is still running

Tracking issue: [#491](https://github.com/yicheng47/runner/issues/491). Status: **closed as not planned 2026-09-07** — Jason chose to make sessions outlive the app (a detached session host, the direction #466 declined), which leaves nothing to confirm at quit; kept as a record. Priority was P2.

## Motivation

Quitting Runner kills every PTY in the process: `on_app_quit` (`crates/runner-app/src/main.rs:1135`) stamps running sessions for auto-resume and then `stop_running_sessions_on_quit` (`crates/runner-app/src/bootstrap.rs:251`) kills them. ⌘Q is one keystroke — easy to hit in a terminal pane by reflex, or from muscle memory in another app while Runner happens to be frontmost — and there is no warning. An agent three minutes into a refactor loses its turn; a `cargo build` or `npm run dev` in a drawer shell just dies.

Auto-resume ([45](./45-auto-resume-on-launch.md)) makes this survivable but not free: it restores the *conversation* on relaunch, so an idle chat costs nothing, but a mid-turn agent loses the turn, and a foreground process in a shell terminal is gone for good. [#466](./466-sessions-outlive-the-app.md) (sessions outlive the app) was declined on 2026-09-02 precisely because the daemon was too much machinery for that loss, and its decision named the cheap follow-up: *confirm-on-quit when sessions are mid-turn*. This spec is that follow-up.

Runner already does the pane-scale version of this. Closing a shell terminal with a foreground process opens a **Close terminal?** confirm (`request_close_terminal_pane`, `crates/runner-app/src/surfaces/chat.rs:1878`; `render_terminal_close_confirm`, `crates/runner-app/src/surfaces/panes.rs:1110`) built on the shared `ConfirmDialog` overlay (`crates/runner-app/src/ui/overlay.rs:345`). Quit is the same question at app scale, and Terminal.app is the precedent: "Terminal has running processes. Quit anyway?"

## Behavior

### What counts as running

Quit asks only when something would actually be lost. Two signals, both of which already exist in the backend:

- **Agents mid-turn.** Any live session whose byte-flow detector reports `Busy` (`session_activity_snapshot`, `crates/runner-backend/src/ops/session.rs:80`). Direct chats and mission slots alike; the snapshot covers both. An agent that is idle, waiting at a permission prompt, or blocked on `ask_human` is not mid-turn: quitting stamps it and auto-resume brings it back, so it is not a reason to interrupt the quit.
- **Terminals with a foreground process.** Any live shell session — pane, tab, chat drawer, or mission drawer — where `has_foreground_process` is true (`session_shell_has_foreground_process`, `crates/runner-backend/src/ops/session.rs:533`, the check the pane-close confirm already uses).

A running mission whose slots are all idle does not count either. Its status stays `running` across the restart and its buses re-mount at launch; nothing is lost.

The busy signal is the 2 s byte-flow `IdleDetector` (`crates/runner-backend/src/session/pty_runtime.rs:666`), so the gate inherits its blind spot: a tool call that stays silent for longer than the threshold reads as idle. Both CLIs animate a spinner while a tool runs, so in practice this is rare, and the gate is meant to catch the obvious case, not to be a guarantee. Hook-based status ([52](./52-hook-based-session-status.md), declined) would sharpen it; it is not a prerequisite.

### The dialog

When the live-work query returns anything, quit stops and the active window shows a `ConfirmDialog`:

- Title: **Quit Runner?**
- Body, first line: a summary in words — "2 agents are still working and 1 terminal has a running process. Quitting stops them." Singular and plural forms; drop whichever half is zero.
- Body, then a list, one line per item: the agent's display name (chat title for a direct chat; `mission · @handle` for a mission slot) or the terminal's name plus its foreground command when the runtime can name it. Cap at five lines with "+N more" after that.
- Body, last line, only when **Resume running agents on launch** is on: "Idle chats resume on next launch." (The mid-turn ones do too, but they lose the turn; the line is a reassurance, not a promise about the turn.)
- Buttons: **Cancel** (default; Escape) and **Quit anyway** (destructive variant). Quit anyway runs today's quit unchanged — stamp, kill, exit. Cancel closes the dialog and nothing else changes.

The list is computed once when quit is requested. If the work finishes while the dialog is open, Quit anyway is still correct and Cancel costs nothing, so the dialog does not live-update.

When nothing is running, quit is exactly as fast and silent as today. The gate must never add a dialog to an app with only idle chats.

### Every quit path

Quit reaches the process three ways, and the gate has to sit in front of all of them or it is theatre:

- **⌘Q and the app menu.** Both dispatch the `Quit` action, which today is `cx.on_action(|_: &Quit, cx| cx.quit())` (`crates/runner-app/src/main.rs:1145`). The handler becomes: query live work; if empty, `cx.quit()`; otherwise open the dialog on the active window, and the dialog's confirm calls `cx.quit()`.
- **Dock → Quit, `osascript`, logout and shutdown (macOS).** These send `terminate:` to `NSApplication` directly. gpui-ce 0.3.3 registers only `applicationWillTerminate:` on its delegate (`platform/mac/platform.rs:100`), which runs `on_app_quit` and cannot veto. Runner adds an `applicationShouldTerminate:` responder to the delegate — the app already touches AppKit from the main-thread init callback for the wake bridge (`runner_backend::wake::install`) and the app icon — that runs the same query, returns `NSTerminateCancel` and raises the dialog when there is work, and `NSTerminateNow` otherwise. On logout, cancelling shows the system's standard "Runner canceled logout" sheet, which is what Terminal.app does in the same situation; the human either finishes the work or chooses Quit anyway and logs out again.
- **Last window close (Windows).** GPUI's Windows run loop ends when its window list empties (`close_one_window` returns `is_empty`), so closing the last Runner window is a quit. The `on_window_should_close` hook (`crates/runner-app/src/main.rs:1379`) runs the query when the closing window is the last one, keeps the window open and shows the dialog when there is work, and lets `finish_window_close` proceed otherwise. On macOS closing the last window does not quit and is unaffected.

**No window open (macOS).** ⌘Q with every window closed reaches the `Quit` action with nowhere to draw the dialog. Runner reopens the main window through the existing reopen path (`application.on_reopen`, `main.rs:1081`) and shows the dialog there. Quitting silently in that state would be the one case where the gate misses on purpose, and it is also the state where the human is least aware what is running.

### Where the state lives

The existing confirms (`terminal_close_confirm`, `fork_confirm`, `project_delete_confirm`) are per-window fields on `NativeRoot`. Quit is app-level: one pending quit, one dialog, whichever window is active. The impl plan decides the mechanics — likely a small global (`PendingQuit { items }`) set by the gate and rendered by the active window's root — but the invariant is that two windows never both show it, and a second ⌘Q while it is open is a no-op, not a second dialog.

## Non-goals

- A **"don't ask again"** setting. The dialog appears only when something is genuinely running, so it is never noise; if it turns out to be, that is the signal to add the setting, not a reason to ship it now.
- A **wait-until-idle** ("quit when they finish") option. Nice, but it needs a live-updating dialog and a countdown state; out of scope until someone reaches for it.
- Sparkle's **Install and Relaunch**. The human chooses that moment in the update prompt (466's reasoning), and Sparkle drives the relaunch through its own terminate call. It will hit the new `applicationShouldTerminate:` hook and get the same dialog, which is acceptable; making the update prompt itself count running work is a later polish.
- **Crash, SIGKILL, `kill -9`.** Nothing to do; the crash path already skips the resume stamp by design.
- Changing what quit *does*. Stamp-and-kill stays; this spec only adds the question in front of it.
- Sessions outliving the process. Declined in #466; this is the alternative, not a step toward it.

## Design

No new Pencil node: the dialog is the existing `ConfirmDialog` overlay with a list body. If the per-item list needs its own layout beyond stacked text rows, add a frame for it in `design/runner.pen` before implementing phase 3.

## Implementation Phases

1. **Live-work query and the action gate.** `ops::session::live_work(state) -> LiveWork { agents: Vec<…>, terminals: Vec<…> }` in the backend, joining the activity snapshot with session rows for display names and the foreground-process check for shell sessions; unit-tested against the manager's fake runtime. Replace the `Quit` action body with the gate and the dialog (summary line plus Cancel / Quit anyway). ⌘Q and the menu are covered. Ships on its own.
2. **The other quit paths.** `applicationShouldTerminate:` on macOS (Dock, `osascript`, logout) and the last-window close on Windows, both routed to the same gate. The no-window reopen case.
3. **Copy and polish.** Per-item list with the five-line cap, the foreground command name for terminals, the auto-resume reassurance line, keyboard defaults.

## Verification

- [ ] Two idle chats, no shells: ⌘Q quits immediately, no dialog.
- [ ] One agent mid-turn: ⌘Q shows the dialog naming it; Cancel leaves the agent running and the window focused; Quit anyway quits and the chat comes back on relaunch with auto-resume on.
- [ ] A drawer shell running `sleep 300`: the dialog names the terminal; the pane-close confirm still works on its own.
- [ ] A mission with one busy slot and one idle slot: the dialog lists the busy slot as `mission · @handle` and nothing else.
- [ ] Dock → Quit with work running shows the same dialog; with nothing running it quits.
- [ ] Log out with work running: the OS reports Runner cancelled the logout, the dialog is up; Quit anyway then log out again succeeds.
- [ ] Windows: closing the last window with work running shows the dialog and the window stays; with nothing running the app exits.
- [ ] All windows closed on macOS, agent busy, ⌘Q: the main window reopens with the dialog on it.
- [ ] ⌘Q twice while the dialog is open: still one dialog.
- [ ] `make verify` clean; a unit test covers `live_work` with busy, idle, and shell-with-foreground sessions.
