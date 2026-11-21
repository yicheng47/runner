# #1332 — runtime, model and effort on mission session rows

Jason approved `neon trio` on 2026-11-21. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1332-session-runtime-rows` on `feat/1332-session-runtime-rows` (base `origin/main` `f84b1bbd`, its own `target/`). Other worktrees under `.worktrees/` belong to other work; never inspect or edit them.

## Problem and outcome

The Sessions rail in a mission's side panel does not say what each slot runs, so a slot override or an added role on another runtime (a codex `reviewer` on `gpt-6.2-astra` at `xhigh` inside a claude crew) is invisible without opening its terminal. Outcome: each Sessions card shows the handle above the status beside the avatar, and a full-width runtime line under them with the provider mark and `Runtime · model · effort`.

The spec, `docs/features/1332-session-runtime-rows.md` on this branch, is authoritative for behavior. The design is `runner/desktop/specs/1332-mission-session-runtime.pen` in `yeelinks_design`, card `RJJ6r` on board `qz5R1`; the spec wins on detail.

## Read first

- The spec above and issue #1332.
- `crates/runner-app/src/surfaces/mission_workspace/rail.rs`: `render_roles_rail`, including the handle-width fitting beside the controls and the two-up layout.
- `crates/runner-app/src/chat_icon.rs` (`runtime_mark`, `ChatIcon::for_runtime`), `crates/runner-app/src/runtime_ui.rs` (runtime display names), `crates/runner-app/src/ui/select.rs` (`effort_label`), and `chat_panel_setup` in `crates/runner-app/src/surfaces/panes/panel.rs` for how chats already show model and effort.
- `crates/runner-core/src/protocol/session.rs` (`SessionRow`), and in `crates/runner-daemon/src/repo/session.rs` `MissionSessionRow` and where it is built; `DirectSessionEntry`'s `agent_model` / `agent_effort` in `crates/runner-daemon/src/ops/session.rs` are the pattern to follow.
- `crates/runner-daemon/src/companion/core.rs`, since remote windows receive `SessionRow` through the companion snapshot.

## Deliverables

1. **Protocol.** `SessionRow` gains `agent_model` and `agent_effort` (`Option<String>`, `#[serde(default)]`), filled by `runnerd` from the session's launch definition. They are absent, never a placeholder, when not known. Remote windows get them through the companion snapshot, and an older host simply omits them.
2. **Card.** Implement the spec's card: the avatar is unchanged; the handle and status stack beside it; the controls stay on the right of the header row; a runtime line (12 px provider mark, then `Runtime · model · effort` in the meta size and muted colour, single line, ellipsis at the end, tooltip with the full text) sits above the unchanged divider and `session_key`. Unknown values drop along with their separator. A shell slot has no runtime line. It works in two-up mode and at the rail's minimum width without overlapping the controls.
3. **Guide.** Update `docs/guide/concepts/missions.md` and its zh-CN page (`python3 script/check-guide-translations.py`).

Non-goals: editing the runtime, model or effort from the card; showing them in member tabs or the feed; replacing the avatar with the provider mark.

## Validation and live checks

- **Automated:** a daemon/protocol test that a mission session's row carries the launch definition's model and effort (an added role with overrides, and a slot with none), and a serde round trip that tolerates the fields being absent. Add app tests in the rail's existing style for the runtime line text, the omission of unknown values and the shell slot. Follow the no-race test rules, and loop any new daemon-backed test 20 times with nextest stress mode, stating the count. Run the relevant crate tests, `make clippy`, `cargo fmt --all --check` and `git diff --check`.
- **Live QA (`@qa` after a clean review, macOS development app, QA lock):** the spec's five live checks. These are a mixed-runtime mission with a codex role added with model and effort overrides, unknown values left out, the status updating beside the avatar, narrow and two-up widths, and a remote window on the same-Mac two-host relay fixture. Record them in `docs/tests/runs/1332-session-runtime-rows-mac.md`.

## Authorization

Approved crew: `neon trio`. Fold this brief and the spec into the implementation commit, push, and open a PR against `main` with `Fixes #1332`. `@reviewer` adds `reviewed` on the final head, and the live-test label is handled as AGENTS.md describes. Do not merge.
