# 645 m4 — Lifecycle (runnerd 1d)

Program [#645](https://github.com/yicheng47/runner/issues/645) (`runnerd`), phase 1, mission 1d, the last mission of phase 1. Jason requested this mission on 2026-10-06. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-645-m4-lifecycle`, on the existing branch `feat/645-m4-lifecycle`, created from the umbrella `origin/feat/645-runnerd` at `dc3cc07a`. That commit is `main` (with the 1d design and today's spec and plan) plus the umbrella's CI commits and missions 1a, 1b and 1c. The root checkout stays on `main`. Do not create another branch or worktree, and do not share a Cargo target directory. This lands on the umbrella, never on `main`.

## The goal

1c moved the core into `runnerd`, so an app crash or force-quit leaves every session running, but a normal quit still stops them all and a new build restarts them silently. **This mission gives the user the choice and the words:** a quit setting and dialog, ⌥⌘Q to quit and stop everything, the update dialog's note on Windows, a toast when a new build restarts the sessions, and a proper notice when `runnerd` stops. It also renames `runner-backend` to `runner-daemon` and brings the docs up to the three-process shape. Phase 1 is complete when this lands.

## Read first

- `AGENTS.md`, especially Worktrees and Crew Missions.
- `docs/features/645-session-host.md`: "Quitting", "Updates and version skew", "When something dies", "Design", and decisions 6, 12 and 13.
- `docs/impls/645-runnerd/plan.md`: "Mission 1d" and "Crates". This brief wins on any detail.
- **The design**, exported from `design/specs/645-session-host.pen` (signed off by Jason on 2026-10-06) into `target/design-ref/` in this worktree. Match them:
  - `quit-dialog.png` and `quit-dialog-screen.png`;
  - `settings-when-runner-quits-row.png`, `settings-quit-menu-open.png` and `settings-general-screen.png` (only the new Sessions section is new; the rest of that screen is an old snapshot, so keep today's rows);
  - `update-dialog-windows.png` and `update-dialog-screen.png`;
  - `restart-toast.png` and `restart-toast-screen.png`.
- Code at `dc3cc07a`:
  - `crates/runner-app/src/main.rs`: `on_app_quit` (around 1421), the `Quit` action, and the app menu (around 1541–1556);
  - `crates/runner-app/src/keymap.rs` (`system-quit`, `cmd-q`, and the Windows table's rule of no quit binding);
  - `crates/runner-core/src/protocol/managed.rs` (`shutdown`, around 94) and `crates/runner-core/src/daemon_process.rs` (`connect_or_spawn`, the mismatch restart);
  - `crates/runner-app/src/app_settings.rs` (`AppSettings`) and `crates/runner-app/src/surfaces/settings_page.rs` (the General rows, around 1968);
  - `crates/runner-app/src/toast.rs` (`ToastTone`) and `surfaces/app_shell.rs` `render_toast` (around 1321);
  - `crates/runner-app/src/app_store.rs` (`daemon_disconnected`, `record_error`, the `daemon/disconnected` and `daemon/reconnected` events);
  - `crates/runner-app/src/updater.rs` (the Sparkle delegate), `updater/windows.rs` and `surfaces/update_dialog.rs`;
  - `docs/tests/645-m3-daemon-process.md`, 1c's record, for what is already proven.

## Deliverables

1. **The setting.** `AppSettings` gains `quitBehavior`: `ask` (default), `keep` or `stop`, a new key that older builds ignore. Settings → General gets a "Sessions" section under Window with one row, "When Runner quits", as in the design: the description says what the setting does and the live session count, and the control is a menu of the three values with the tick on the right.
2. **The quit dialog.** When sessions are live and the setting is Ask, quitting shows the dialog in `quit-dialog.png`: two choices, Keep them running and Stop them, then Cancel and Quit, and "Don't ask again".
   - The Stop line names the agent that is mid-turn ("Refactor the settings store" is working and loses its current turn); with several, it gives the count.
   - The last choice is preselected (Keep them running the first time), Enter quits, Escape cancels.
   - "Don't ask again" writes the selected choice into `quitBehavior`.
   - It appears on ⌘Q, on the Quit menu item, and on Windows when the last window closes. With no live sessions, quitting just quits.
3. **The two outcomes.** Keep running: the app disconnects and exits, and `runnerd` keeps every session (1c already supports this). Stop sessions: today's `Shutdown { stop_sessions: true }` path, unchanged.
4. **Quits that never ask.** A quit the OS starts (logout, restart, shutdown) never shows the dialog and leaves the sessions to `runnerd`, which already stops and stamps them on SIGTERM and the Windows console events. An update's quit never shows the dialog either (next item).
5. **⌥⌘Q, Quit and Stop Sessions,** on macOS: the Option alternate of the Quit menu item if GPUI supports alternate menu items, otherwise a visible item below Quit, bound to `alt-cmd-q`. It stops the sessions without a dialog, whatever the setting. Windows gets no new binding.
6. **Updates.** An update restarts every session (spec decision 13), and 1c's mismatch restart already does it when the new build starts, so the old app does not stop `runnerd` itself.
   - macOS: Sparkle's flow and window stay exactly as they are (Jason, 2026-10-06). The only change is a delegate hook (such as `updaterWillRelaunchApplication:`) that marks the coming quit as an update, so it skips the quit dialog and leaves `runnerd` running.
   - Windows: the in-app update dialog (`surfaces/update_dialog.rs`) says the sessions restart with it, and when agents are mid-turn shows the amber dot and "1 agent working" (or "N agents working") to the left of its buttons, as in `update-dialog-windows.png`. Installing quits the same way, without the quit dialog.
7. **The restart toast.** When the app restarts `runnerd` into its own build (a mismatch: an update, a manual install or a rebuild) and resumes one or more sessions, it shows the toast in `restart-toast.png`, "Restarted N sessions in Runner <version>", with the restart icon (`rotate-cw`) in the accent colour. Toasts gain a way to carry that icon, as a new tone or an icon field; the existing three tones stay as they are.
8. **The notice that `runnerd` stopped.** Replace 1c's raw text ("restart limit reached; see runnerd.log") with plain wording, in the existing notice: that Runner's background service stopped and its sessions were stopped, which resume from their panes. After the third crash in five minutes, it says the service keeps stopping and offers an Open log button that reveals `runnerd.log`.
9. **The rename.** `runner-backend` becomes `runner-daemon`: its directory, package name, every `runner_backend::` path, `Cargo.toml` and `Cargo.lock` entries, the Makefile, scripts and docs. Nothing else changes in that commit.
10. **Docs**, matching the shipped behaviour:
    - `docs/arch/` §1, §5.5, §5.8, §11 and bets 3, 10 and 12; `concurrency.md`, which now has three processes; `windows.md`; and vision §4.2.
    - `AGENTS.md`: the crate names in Stack and Project Map, and in the development notes that `make run` restarts the development daemon and `runner-dev daemon stop` stops it.
    - The process map: read `docs/arch/process-model.md` on `origin/fix/647-terminal-black-sidebar` (`git show origin/fix/647-terminal-black-sidebar:docs/arch/process-model.md`) and add an updated version to `docs/arch/`.

## Out of scope

A Sparkle UI change or any warning in Sparkle's window; an "install when idle" option; a downgrade guard; a `runnerd` status row or Stop button in Settings; a pane waiting for its snapshot; any schema, event-log or CLI output change beyond renamed paths. No behaviour change beyond what this brief names.

## Rules

- **The umbrella's rules:** no migration, no event-log shape change, no CLI output change. New settings are new keys.
- **No test starts a daemon in real app data;** 1c's rules for daemon tests stand.
- **Before every handoff, check the Windows build** by hand: list every `cfg` gate and every moved `use`, and gate imports used only by `cfg(unix)` tests. Windows CI is the real gate.
- No new dependency without a reason in the handoff. Stage by path, never `git add -A`.
- **Nobody runs the app** except QA under Jason's authorization. The CLI in the build tree is `target/debug/runner-agent-cli`; never run `target/debug/runner`, which is the GUI.
- For UI tests, use the existing headless harness and its known traps (one `ThemeGuard` at a time, seed before the first draw, assert on rendered output).
- No extra agents.

## QA

**QA does not run the app, any agent, or `runnerd` outside the tests until Jason authorizes it on the feed.** Until then QA reads the diff, the tests and the record, and prepares its live checklist in `docs/tests/645-m4-lifecycle.md`. When authorized, on the development build and development data only, it covers:
- the quit dialog from ⌘Q, the Quit menu item, Cancel, Enter, Escape, the preselection, and "Don't ask again" for each choice;
- Keep running (sessions survive, relaunch reattaches) and Stop sessions (sessions stop, relaunch resumes);
- the Settings row and menu, and each value's effect on the next quit;
- ⌥⌘Q with each setting;
- a rebuild with a different build stamp while sessions are live (1c's L9 method): the restart toast and its count;
- killing the development `runnerd`: the notice, and after three crashes the Open log button;
- 1a's and 1c's smoke rows again, briefly.

It never logs Jason out, never touches the production app, production data, or `~/.claude`, `~/.codex` or `~/.pi` settings, archives what it creates and deletes its test role and crew, and notes each live agent session's cost. The Windows rows (last-window close, the update dialog's note, the installer quit) go to Jason's PC run.

## Verification

1. Unit and headless UI tests for: the dialog's rules (when it appears, the preselection, "Don't ask again", the Stop line with zero, one and several working agents); the setting's round trip; the update quit and the OS quit skipping the dialog; ⌥⌘Q stopping the sessions; the toast's icon and count; and the notice's two states.
2. Every existing test passes. Record workspace test counts (`--profile ci`, with pipefail) at `dc3cc07a` and after.
3. **The rename:** no `runner_backend` or `runner-backend` remains outside `docs/impls/archive/`, the archived records and git history; record the grep.
4. **The commands:** `make verify`, workspace clippy with `-D warnings`, `--features updater` clippy, `cargo fmt --all --check`, `git diff --check`, and the Windows audit. Record each exit code.

Record everything in `docs/tests/645-m4-lifecycle.md`.

## Review

The coder implements and hands off on the Runner feed. The reviewer reviews the full diff against this brief and the design PNGs, must-fix first with file:line. It checks:
- every quit path: which ask, which never ask, and that Keep running never stops a session while Stop always does;
- that no OS or update quit can show the dialog or block logout;
- the dialog, Settings row, update note and toast against the design;
- the rename for leftovers, and that its commit changes nothing else;
- the Windows gates;
- that nothing in Out of scope changed.

Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`.

## Authorization

After a clean review, Jason authorizes the following:
- **Two commits**, because they are unrelated and could each be reverted alone: first the rename (`refactor: rename runner-backend to runner-daemon (#645 1d)`), then everything else, the brief and the test record included (for example `feat(app): quit choices, update and restart notices for runnerd (#645 1d)`), both on top of current `origin/feat/645-runnerd`. Say why there are two in the PR body.
- If the umbrella has moved, rebase; never merge.
- Push the branch and open the PR against `feat/645-runnerd` **as a draft**. The body says `Refs #645` and carries the gates and the review verdict, with no Claude session link, and says that QA's live checks are pending Jason's authorization.
- Review or CI fixes are amended into the commit they belong to and pushed with `git push --force-with-lease`.
- Drive `Rust / macOS` and `Rust / Windows` green.

Do not mark the PR ready, merge it, delete the branch or worktree, or cut a nightly or release. The handoff after CI is green carries the PR URL, what changed, tests and exit codes, CI and the reviewer's verdict. Then stand by for Jason.
