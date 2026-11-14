# Mission Brief — #1315 Remote unread green dot persists after reading Codex chat

- **Issue:** [#1315](https://github.com/yicheng47/runner/issues/1315) (`bug(remote): unread green dot persists after reading a Codex chat on 0.19.8`). Related historical context: [#1026](https://github.com/yicheng47/runner/issues/1026).
- **Starting point:** worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1315-remote-unread-dot`, branch `fix/1315-remote-unread-dot`, off `origin/main` (`627797fd`). Work only in this worktree, with its own `target/` (never share `CARGO_TARGET_DIR`).
- **Parallel isolation:** no other `runner` repo worktrees are active; `runner_ios` mission `#32` shares the macOS host screen and `script/qa-lock` when running live QA, so `qa` must serialize strictly via `script/qa-lock`.

## Problem And Goal

In Runner remote use on `0.19.8`, after a direct Codex chat finishes a turn and raises its unread green dot (`unread_since` on the host, `unread: true` in companion status), viewing the session's messages—and even submitting a follow-up prompt in the same conversation—leaves `unread_since` set on the host and the green dot persistent. Reading or replying in a selected/viewed remote chat must clear that turn's unread marker on both host and remote client, keep it cleared when switching away and back or across status snapshots, and allow a later genuinely unread completion to set it again.

## Read First

1. `crates/runner-app/src/surfaces/remote_client/workspace/sync.rs` — `mark_remote_stopped_sessions_viewed`, `attach_remote_panes`, and `viewed_tabs` (note `mark_remote_stopped_sessions_viewed` currently filters to `Status::Stopped | Status::Error`).
2. `crates/runner-app/src/surfaces/remote_client/bridge.rs` and `crates/runner-app/src/surfaces/remote_client/model.rs` — remote read-ack triggers, status synchronization, and `unread_since` projection.
3. `crates/runner-daemon/src/companion/mod.rs`, `core.rs`, and `actions.rs` — `Control::Take` / `Control::IfFree` / `Control::Viewport` attach handling, `markViewed` handler, and status snapshot pushes.
4. `crates/runner-daemon/src/ops/node.rs` and `crates/runner-daemon/src/session/state/mod.rs` — `mark_direct_sessions_viewed`, `node_mark_viewed`, and in-memory `unread_since` / `viewed` transitions across Codex completion and follow-up input.
5. `crates/runner-daemon/src/session/manager/` — `browser_resize`, `browser_input`, `mark_status_viewed`, and status broadcast paths.

## Deliverables And Boundaries

- **Root-cause fix across client and daemon/companion as needed:**
  - Ensure viewing an active/running or completed remote chat sends/applies the viewed acknowledgement without requiring `Stopped`/`Error` or a fresh attach/control transfer.
  - Ensure replying/sending input or viewing in an already-selected remote chat clears `unread_since` on the host and propagates the cleared state to remote sidebars without stale status snapshots restoring the dot.
  - Preserve genuinely unread completions: a new turn completing while unviewed sets `unread_since` / the green dot again.
- **Automated tests:** targeted unit/integration tests in `runner-daemon` (companion/session state/ops) and `runner-app` (`remote_client`) covering reading a running completed Codex chat remotely, replying in an already-selected chat, switching away/back, stale snapshot ordering, and a subsequent unread completion.
- **Non-goals:** unrelated sidebar visual redesign, `runner_ios` repo changes (unless a protocol bug is uncovered—escalate to Manager if cross-repo changes are needed), or unrelated session-lifecycle refactors.
- **Commit hygiene:** fold this brief commit into your implementation commit(s) before final push so PR history contains clean change commits and no standalone brief commit.

## Validation And Handoffs

1. **coder:** run `make fmt`, `cargo nextest run --locked --workspace --cargo-profile ci`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and the `--features updater` clippy pass. Do not run the live app. Hand the candidate SHA to `reviewer`.
2. **reviewer:** review the diff, focusing on host/remote read-ack races, `Control::Take`/`IfFree`/`Viewport` vs `markViewed`, `session_attention` persistence vs in-memory `SessionStatus`, stale status snapshot protection, and test coverage. Hand the clean SHA to `qa` with the live checks below plus any the diff adds.
3. **qa:** run bounded live checks on macOS in this worktree after taking `script/qa-lock acquire --timeout 540` and launching with `make run` (inexpensive Codex model, low effort, tiny prompts in fresh `/tmp` folders; preserve pre-existing dev sessions; archive only QA-created sessions). Verify in a real remote-client-to-host flow (or loopback remote connection) that:
   - completing a direct Codex turn sets the green dot when unviewed;
   - reading/selecting the chat (and replying in an already-selected chat) clears both the remote dot and host `unread_since`;
   - switching away and back keeps it cleared, and a subsequent unviewed turn sets it again.
   Record the run in `docs/tests/runs/1315-remote-unread-dot-mac.md` and in the PR description. Note explicitly if Windows host behavior could not be tested live on macOS and keep `needs-live-test` if any required platform/check remains pending.

## Authorization

The `agy trio` is authorized for [#1315](https://github.com/yicheng47/runner/issues/1315): commit, push `fix/1315-remote-unread-dot`, open a PR against `main` closing `#1315`, apply the appropriate `risk-*` label, keep any automatic `core-review`, apply `reviewed` for the final head, and stop at the open PR with CI green. Do not merge.
