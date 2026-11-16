# Mission Brief: Fix free remote terminal auto-take and remote duplicate-window detection (#1347)

**Tracking issue:** [#1347](https://github.com/yicheng47/runner/issues/1347)
**Branch:** `fix/1347-remote-control-and-duplicate` | **Base:** `origin/main` (`e2d24cc2`)
**Worktree:** `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1347-remote-control-and-duplicate`
**Crew:** `carbon trio` (`coder`, `reviewer`, `qa`)

**Parallel isolation:** Stay strictly in this worktree, never edit the root checkout `/Users/jason/repos/yicheng47/runner`, and take `script/qa-lock acquire --timeout 540` before launching any app/wrapper during live QA (another mission `runner_ios` `01M7GW1AQ3KWJYD8583KMYNNEX` may share the macOS host/screen lock).

## Authoritative Scope & Commit Hygiene

- Follow [#1347](https://github.com/yicheng47/runner/issues/1347) strictly. Fold `docs/impls/briefs/1347-remote-control-and-duplicate.md` into an implementation commit before final push so no standalone brief commit remains in PR history.
- Changes touch client protocol and daemon/PTY control, so GitHub applies `core-review`. Keep companion wire compatibility intact and never remove `core-review`.

## Read First

1. `crates/runner-app/src/surfaces/remote_client/workspace/{render.rs,sync.rs}` (`render_remote_terminal_content`, `mark_remote_sessions_viewed`, `attach_remote_panes`, `recovery` -> `IfFree`)
2. `crates/runner-daemon/src/remote/client/terminal.rs` (`Attach` vs `IfFree` / `Viewport` / `TakeControl`, `reattach` with `ControlMode::IfFree`)
3. `crates/runner-daemon/src/session/manager/terminal.rs` (`control_group`, `browser_resize`, split-tab `IfFree` check at `:966-984`)
4. `crates/runner-core/src/protocol/window.rs` & `crates/runner-daemon/src/windows.rs` (`Subject`, `WindowRegistry`, `set_subjects`, `primary_for`, `any_focused_displaying`)
5. `crates/runner-app/src/surfaces/windowing/mod.rs`, `panes/pane.rs`, `panes/tree.rs`, `chat/attachment.rs` (`current_subjects`, `sync_subject_ownership`, `refresh_chat_secondaries`, `DuplicateSubjectOverlay`, pane menu/resume secondary checks)

## Deliverables

1. **Auto-take free remote terminals & split-tab `IfFree`:**
   - When a visible, connected, running, primary (non-secondary) remote pane reports `TerminalController::None`, automatically take control without waiting for a click, and remove the steady `client.no_controller` prompt.
   - Keep `{computer} took this terminal back` (`client.taken_back`) + **Take control** when held by host `Desktop` or another `Browser` client; never auto-take when someone else holds control, and never loop/steal back and forth (deduplicate auto-take per uncontrolled episode; reset on manual **Take control** click or reconnect/new attachment; show `REMOTE_CONTROL_FAILED` on error).
   - Fix host `browser_resize` `IfFree` (`session/manager/terminal.rs:966-984`) so a `Controller::None` pane isn't left uncontrolled merely because another session in the same host tab is owned by this same channel (`Browser { channel: owner } if *owner == channel`) or when claiming free sessions without stealing from `Desktop` or another channel. Audit `TerminalHub` (`remote/client/terminal.rs`) `attach`/`reattach` across `replay_valid` and recovery.
2. **Duplicate-window detection for remote chats (`Subject::RemoteChat`):**
   - Add `Subject::RemoteChat { computer: String, session: String }` to `runner-core/src/protocol/window.rs` (preserving `Mission(String)` and `DirectChat(String)` serde shape) and update `protocol/tests.rs` `Sample` / round-trip tests.
   - Keep remote subjects isolated from local `DirectChat` `viewed_session_id` / `any_focused_displaying` notification suppression while arbitrating `primary_for` by `focused_at` per `(computer, session)`.
   - Report active remote tab leaf sessions as `Subject::RemoteChat` from `NativeRoot::current_subjects` and run `sync_chat_subject_ownership` / `refresh_chat_secondaries` on remote windows (open, computer switch, tab switch, focus/blur, `window_focus_map` / reconnect). Clear when switching away or closing.
   - Apply secondary state in `panes/pane.rs`, `panes/tree.rs`, `chat/attachment.rs`, and `remote_client/workspace/sync.rs`: render blank background + `DuplicateSubjectOverlay` (**Focus other window** / **Stay here**), skip attach/auto-take/typing/resize/`mark_viewed` in secondary panes, and reattach/auto-take when focus makes the window primary.
3. **Guide/Docs & Tests:**
   - Check `docs/guide/{,zh-CN/}guides/remote-access.md` and `docs/tests/regression/remote.md` for user-visible banner/two-window updates and verify `python3 script/check-guide-translations.py`.
   - Add unit + visual tests across `runner-core`, `runner-daemon` (`session/manager/terminal/tests.rs`, `windows.rs`), and `runner-app` (`remote_client/tests.rs`).

## Role Boundaries, QA & PR Gate

- **`coder`:** Implement, run `cargo test -p runner-core`, `cargo test -p runner-daemon`, `cargo test -p runner-app`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, and `python3 script/check-guide-translations.py`, commit cleanly, and hand off to `reviewer`. Do not run the live app.
- **`reviewer`:** Review the full diff against `origin/main` (including `core-review` protocol/daemon/session paths), verify tests, and confirm the live checks for `qa`.
- **`qa`:** Acquire `script/qa-lock acquire --timeout 540`, run the macOS live checks from `#1347`: (1) free remote chat auto-takes without clicking; (2) host `Take back` keeps taken-back banner without client stealing it back; (3) same remote chat in two windows shows `DuplicateSubjectOverlay` on secondary and transfers ownership on focus. Follow `docs/tests/full-smoke-test.md#same-mac-two-host-relay-fixture-macos` if a second physical host is unavailable (two isolated wrappers + short private `HOME`s under `/private/tmp/` via Chrome's signed-in account, strictly cleaning up/revoking disposable enrollments), or record `Blocked` if relay sign-in is unavailable. Record in `docs/tests/runs/1347-remote-control-and-duplicate-mac.md` and release `script/qa-lock release` after cleanup.
- **PR & Labels:** After clean review and QA, fold commits cleanly (no standalone brief commit), push `fix/1347-remote-control-and-duplicate`, open a **non-draft** PR closing `#1347` (`core-review` stays, add `risk-medium`, `reviewed` on final head SHA, and `needs-live-test` for unrun Windows/Linux or blocked live checks), and wait for all CI jobs `SUCCESS`. **Do not merge.**
