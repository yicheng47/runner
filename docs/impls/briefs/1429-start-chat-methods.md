# #1429 — start_chat slice: move launcher and setup methods off `NativeRoot`

## Starting point

- Issue: [#1429](https://github.com/yicheng47/runner/issues/1429), item 1 of the [#1248](https://github.com/yicheng47/runner/issues/1248) cleanup tracker.
- Branch: `refactor/1429-start-chat-methods`
- Worktree: `/Volumes/samsung/repos/yicheng47/runner/.worktrees/refactor-1429-start-chat-methods` (base `origin/main` `5869f774`, own `target/`).
- Other live worktrees: `chore-1477-gpui-pre-0.3.14` (running neon trio, touches app files but not `start_chat/`), `fix-818-terminal-round-trips` and `refactor-1431-claude-accounts-seam` (neon trio started alongside this one, daemon-side). Leave them alone. If #1477 merges first, rebase onto `origin/main`.
- Fold this brief into the implementation commit before the final push.

## Problem and outcome

#1429's field clusters have landed: usage (#1457), chat lifecycle (#1462) and panes and splits (#1468) are now plain state structs owned by `NativeRoot`. What remains is the issue's second bullet: methods in the large `impl NativeRoot` blocks that serve only one surface. This slice takes the start_chat surface, the largest of them:

- `surfaces/start_chat/launcher.rs`: three `impl NativeRoot` blocks of about 1,250 lines, from `refresh_remote_agent_versions` to `launch_from_launcher_with_prompt` (launcher visibility and refresh, gated and ready launches, the launcher page, `render_launcher`, update-pane reservation).
- `surfaces/start_chat/setup.rs`: one `impl NativeRoot` block of about 1,380 lines, from `setup_visible` to `render_setup_footer` (opening, rechecking, skipping and finishing first-run setup, and all of its rendering).

Outcome: methods that only read and write launcher or setup state live on `ChatLauncher`, on `SetupState`, or on a new launcher-collection state struct that owns `NativeRoot.launchers` and the launcher page's bookkeeping. `NativeRoot` keeps thin entry points only where the work coordinates across surfaces: reserving tabs and panes, routing focus, daemon requests whose results reach other surfaces, and `cx.notify()` on the root. Setup's render helpers take explicit inputs instead of `&mut NativeRoot`. Turn setup or the launcher collection into an `Entity` only where it re-renders on its own and that removes real coupling, and say in the PR which you chose and why. No behaviour or visual change.

## Read first

- `crates/runner-app/src/surfaces/start_chat/{launcher.rs,setup.rs,mod.rs}` and `start_chat/launcher/` (`tests.rs`, `remote_tests.rs`, `mission.rs`, `worktree.rs`), `start_chat/setup_tests.rs`
- `crates/runner-app/src/surfaces/root/mod.rs` (`NativeRoot`: `launchers`, `setup`)
- The pattern from the earlier slices: `surfaces/panes/state.rs` (#1468), `surfaces/app_shell/usage.rs` (#1457), `surfaces/chat/mod.rs` (`ChatLifecycleState`, #1462)
- Callers outside start_chat: `settings/agents/{mod,render}.rs` (`open_setup`), the sidebar, front page, windowing and remote client call sites of the launcher methods

## Deliverables

1. Move the launcher and setup methods as described above, updating call sites across the app. `start_chat/mod.rs` (`StartChatModal`) and `launcher/mission.rs` change only as far as the moves require.
2. Tests move with the code they cover and keep their assertions; any new test of async or background work follows the repo's no-race rules and is stress-looped.
3. The PR states before and after counts: lines of `impl NativeRoot` in `launcher.rs` and `setup.rs`, and the number of `NativeRoot` methods.
4. No guide change, since nothing user-visible changes.

Non-goals: the other large blocks (`settings/remote.rs`, `panes/header.rs`, `panes/pane.rs`), which are later slices; behaviour, layout or copy changes; new comments explaining moved code.

## Validation and handoffs

- **@coder:** `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, `git diff --check`. Hand `@reviewer` an unpushed commit SHA with a summary and the counts.
- **@reviewer:** This changes GPUI ownership of launcher and setup methods, so check that every `cx.notify()`, `observe`/subscription, `FocusHandle`, `cx.spawn` task and its lifetime, and render tree behaves as before, and that no test was weakened. The clean review names the live checks below (adjusting them if the diff warrants) and the reviewed SHA.
- **@qa:** Take `script/qa-lock acquire --timeout 540` before building or launching the macOS development app; another mission may hold it, so wait as Test Scope describes. Use only your own `/tmp` folders, chats and missions, an inexpensive model at low effort, and tiny prompts. Back up any development setting you change and restore it.
  1. **Setup page (SET-SETUP-01 subset):** open setup from Settings → Agents and from the launcher; expand and collapse the supported-agent disclosure and scroll it, Recheck, copy an install command, Skip, and resize between compact and tall windows. Reason: every setup render and action method moved.
  2. **Finish setup (SET-SETUP-02 subset):** choose Start your first chat, start it in a scratch folder with an inexpensive model, and confirm the starter crew and chat appear once and the launcher opens afterwards. Delete the crew you created. Reason: `complete_setup_and_open_launcher` and `finish_setup` moved.
  3. **Open and launch (WS-CREATE-06/07 subset):** open the launcher with ⌘N from a chat and a terminal tab and on the default page with no tab; launch an agent chat and a terminal and check Recent. Reason: `open_launcher_page`, `reset_launcher_page`, `launch_from_launcher` and the page helpers moved.
  4. **Close and keys (WS-CREATE-18 and WS-CREATE-12 subsets):** close the launcher with the page's ×, the row's hover × and ⌘W, landing on the tab it opened from; use number keys, Escape and Enter, starting a row once. Reason: `close_launcher_page`, `prune_launchers` and `focus_visible_launcher` moved.
  5. **Split-pane launcher (WS-CREATE-09 subset):** split an owned chat with ⌘D and start a terminal from the new pane's launcher. Reason: the per-pane `launchers` map moved.
  6. **Gated launch (SET-UPDATE-12 subset):** with a scratch executable override that reports an outdated version, start that agent from the launcher, let the update run, and confirm the launch proceeds once. Use only the override setup and restoration from SET-AGENT-02. Reason: `retry_gated_launches`, `drain_ready_launches` and `reserve_update_pane` moved.

  Record pass/fail, candidate SHA and screenshots in `docs/tests/runs/1429-start-chat-methods-mac.md`, archive only your own chats and missions, clean `/tmp`, run `script/qa-lock release`, and report the verdict to `@coder`.

## Authorization

Approved crew: `neon trio` (`coder`, `reviewer`, `qa`). After `@reviewer` passes and `@qa` passes, `@coder` may push the branch and open a PR against `main` with `Part of #1429` (the issue stays open for the remaining slices), labelled `frontend-only`, `no-behavior-change` and `risk-medium`, and drive CI green on both platforms. Do not merge.
