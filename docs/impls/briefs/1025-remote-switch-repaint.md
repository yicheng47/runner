# #1025 — Switching remote chats replays a full repaint

Jason requested a claude trio mission (coder, reviewer, qa) on 2026-11-02 for all three fixes below. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1025-remote-switch-repaint` on `fix/1025-remote-switch-repaint`, based on `origin/main` at `0b4eae1f83d71d0a0a7bbcd7393f79f215bab7f4`. The root checkout stays on main. Use this worktree's own `target/`.

## Problem

[#1025](https://github.com/yicheng47/runner/issues/1025): through Runner remote access, switching between existing agent chats shows a long burst of page-flipping scroll before the view settles. First reported on a Windows host (Codex 0.164.0, Runner 0.18.0). Jason hit it again on 2026-11-02 controlling a Linux host (Ubuntu 22.04, Runner 0.18.2 nightly `caba8e7`) from macOS (Runner 0.19.0).

New evidence from the Linux case: neither the Linux host nor the macOS client logged `synchronized update timed out` that day, so on Linux the partial-frame flush from the issue's code reading is not the visible mechanism. Linux also ignores a terminal resize to an unchanged size, so the agent only redraws when the size actually changes. The host showed two remote devices connected all morning, so two clients showing the same chat at different sizes may each flip the terminal size when they take control. No attach or resize trace exists yet.

Suspected chain, still unconfirmed:

1. `RemoteWindow::attach_remote_panes` (`remote_client/workspace/sync.rs`) retains `terminals` only for `visible_sessions`, so switching tabs drops the mirror of the chat being left.
2. Switching back re-attaches with `take_control: !remote.recovery.contains(&id)`, so the host receives a take at the client's size every time.
3. `resize_controlled` (`runner-daemon/src/session/manager/output.rs`) pushes `Resized` and calls `runtime.resize` without checking whether size or controller changed. ConPTY repaints even on a same-size resize.
4. Codex and Claude Code clear and reprint their whole history on a real resize. The client paints that repaint chunk by chunk as it arrives over the relay, which is the visible scroll. The attach snapshot itself is applied atomically in `runner-terminal/src/terminal.rs` and is not the cause.

## Read first

The issue and its code-reading comment; `remote_client/workspace/{mod.rs,sync.rs}` and `remote_client/transport.rs` in `runner-app`; `session/manager/{output.rs,terminal.rs}` (`ResizeControl::apply`) and `remote/client/terminal.rs` in `runner-daemon`; `runner-terminal/src/terminal.rs` and `terminal/model.rs` (`flush_sync_update`, the sync timeout); `docs/tests/regression/remote.md`.

## Deliverables

Start by confirming the chain in tests before changing behavior: a test that switches tab A → B → A over a fake remote and records attaches, takes and host resizes. Add a debug-level log in `resize_controlled` with old and new size, whether it changed, and the controller, so the next field report carries a trace.

1. **Keep remote chats attached while their tab is open.** Switching between open tabs, or into a mission and back, must not re-attach, take control or resize. Detach only when the tab closes, the session stops, or the connection drops. Hidden panes must not send viewport or size updates, including zero sizes. Reconnect recovery keeps its current no-take behavior. Keep the number of hidden attachments bounded by open tabs and say in the PR what an attached hidden pane costs in relay traffic.
2. **Skip resizes that change nothing.** In `resize_controlled`, when size and controller are both unchanged, push no `Resized` frame and make no runtime resize. A real size or controller change behaves exactly as today. Keep the existing settle and resume handling intact.
3. **Paint large remote repaints once.** For remote mirrors only, when output arrives as a burst (a clear-scrollback sequence or a large volume within a short window), hold painting until the stream is briefly quiet, with a hard cap, then paint once. Typing echo and small interactive output must not be delayed, local panes must not change, and an explicit synchronized update keeps its current semantics. Choose and justify the thresholds; around 80 ms quiet and 500 ms cap is the starting point. If the explicit sync timeout is the better place to extend for remote mirrors, explain why.

Non-goals: no relay protocol version bump unless unavoidable (stop and ask first), no change to Desktop take-back UX, no Windows-only workaround, no unrelated refactor of the remote workspace.

## Validation

Tie each check to the behavior it covers. In-process: the A → B → A test shows zero takes and zero host resizes after the first attach; a closed tab detaches; a stopped session detaches; a same-size same-controller resize emits nothing and a real one still resizes; two remote clients alternating takes at different sizes resize only on an actual change; burst coalescing paints once for a large repaint, does not delay a single keystroke echo, and releases at the cap. Run the affected `runner-app`, `runner-daemon` and `runner-terminal` tests with locked dependencies, `cargo fmt --all --check`, workspace Clippy with warnings denied, and `git diff --check`. Drive the required CI green.

Live checks (qa, after the clean code review, on the same candidate): use the isolated development app under Test Scope. Remote checks need a second host; follow how `docs/tests/archive/836-connect-host.md` set up and removed a test host. Never operate the installed Runner, Jason's office Linux host, or any pre-existing host or chat. If a second host cannot be staged, record the remote check as Blocked rather than substituting a local check. Select from `docs/tests/regression/remote.md` (remote terminals and chat switching) plus a new check: with two long agent chats open on a remote host, switching A → B → A shows the chat at once with no scroll burst; resizing the client window still repaints correctly. Record results in `docs/tests/1025-remote-switch-repaint.md`. Jason will confirm on his Linux and Windows hosts after merge; list those as not run.

## Authorization and handoff

Jason authorized this three-agent claude trio mission: implementation, review, qa live checks in the development app, commits, pushing this branch, and opening one PR against main. coder implements and owns all commits; reviewer reviews the working-tree diff first; qa runs the live checks after a clean review. Fold this brief into an implementation commit. Fold review and CI fixes into the commits they correct and use `git push --force-with-lease` when rewriting pushed history. Do not add tool-specific co-author trailers. The PR links #1025 without a closing keyword, since the host-side confirmation is Jason's.

Stop at an open PR with green CI and qa's verdict. Report through Runner with the PR URL, final SHA, changes, validation and the live checks not run. Do not merge, release, cut a nightly, archive the mission, delete branches or remove worktrees. When waiting for a teammate or Jason, finish your turn and rely on the pushed inbox notification.
