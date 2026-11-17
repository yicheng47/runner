# #1369 — Mid-turn inbox delivery on every runtime that accepts it

Jason approved this mission for the codex trio on 2026-11-17. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1369-mid-turn-delivery` on `feat/1369-mid-turn-delivery`, which holds the spec commit on top of `origin/main` at `be023414`. Rebase onto the current `origin/main` before you start. The worktree uses its own `target/`. The branch already has draft PR [#1370](https://github.com/yicheng47/runner/pull/1370).

## What to build

The spec is authoritative: [`docs/features/1369-mid-turn-delivery.md`](../../features/1369-mid-turn-delivery.md). In short:

- Rename `RuntimeMetadata::queues_input_mid_turn` to `accepts_input_mid_turn` and set it for Codex, so a working Codex member gets its inbox notice during the turn through the existing `DeliveryReadiness::Working` path.
- Mid-turn delivery is best-effort. Do not add confirmation or retries for it: an unconfirmed attempt falls back to #1076's leftover handling and to the confirmed idle delivery.
- A leftover `Unsent` notice from a mid-turn attempt starts its `REDELIVERY_BACKOFF` stall warning when the member next reports idle, not while it is still working (`Dispatcher::hold` in `crates/runner-daemon/src/router/dispatch.rs`).
- After qa's per-runtime probe (spec, live check 5), set the flag for each of pi, Copilot, Antigravity, OpenCode, Grok and Cursor that passes. Trae and Shell stay idle-only.
- Update the guide paragraph in `docs/guide/concepts/missions.md` and its zh-CN copy, `docs/tech/mission-message-delivery.md`, and regression rows MIS-MSG-01 and MIS-MSG-03.

Read first: the spec; `crates/runner-daemon/src/router/dispatch.rs` (the attempt, `Unsent` and `hold` logic); `delivery_readiness` in `crates/runner-daemon/src/session/manager/mod.rs`; `crates/runner-core/src/protocol/runtime_metadata.rs`; `docs/tech/mission-message-delivery.md`.

Non-goals: reports to a mission's linked chat (they stay idle-only), #1108's Settings switch, hook or app-server delivery, and changes to `SUBMIT_DELAY` or paste-burst handling unless the Windows check shows they are needed (report that instead of tuning it here).

## Validation

Automated tests as listed in the spec's Verification section, using the router's fake injector. Router tests drive time through the caller's clock; do not add sleeps. Run `cargo test -p runner-daemon` and `cargo test -p runner-core`, `make clippy`, `cargo fmt --all --check` and `git diff --check`.

Live checks: the six in the spec, run by qa on the reviewed candidate, with `script/qa-lock` as AGENTS.md describes, and recorded in `docs/tests/runs/1369-mid-turn-delivery-<platform>.md`. Use the scratch prefix `/tmp/1369-qa-*`. Another mission (#1213 m2) shares the development app through the lock. The Windows check needs a Windows host: if qa cannot reach one, it records that check as Blocked, `needs-live-test` stays on, and Jason runs it.

## Authorization

Approved crew: codex trio (coder, reviewer, qa). The crew may commit, push this branch and take draft PR #1370 out of draft once review is clean and CI is green; it closes #1369. Keep `needs-live-test` while any selected check has not passed. Fold this brief into the implementation commit.
