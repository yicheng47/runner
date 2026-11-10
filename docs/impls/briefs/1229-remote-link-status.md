# #1229 — Host usage in the remote footer, link status in the host popup

Jason asked on 2026-11-09 for the codex trio to rework [PR #1233](https://github.com/yicheng47/runner/pull/1233) for [#1229](https://github.com/yicheng47/runner/issues/1229) under the revised spec. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1229-remote-link-status` on `feat/1229-remote-link-status`. The branch holds `109a2817` (the first spec), `fc33e746` (the first implementation, with link status in the footer) and the commit that revises the spec and this brief, all on `b2c15e38`; `main` is at `704b5f30`. Rebase when the repo's rules call for it.

## Source of truth

- **Spec:** `docs/features/1229-remote-link-status.md`, as revised, owns behavior, thresholds, phases and verification. This brief does not repeat them.
- **Design:** frame `BGIfa` in `runner/desktop/specs/1229-remote-link-status.pen`, in `~/repos/yeelinks/yeelinks_design` at `397fed1`: option C (the host's usage in the footer), P1 (the mark leads the popup row's status line), the footer's Slow state and the read-only usage popover, signed off by Jason on 2026-11-09. Frame `kLrRu` (option B) is superseded. Read the design through the Pencil tools only, since `.pen` files are encrypted, and do not edit or commit it. The design wins on visual detail, the spec on behavior; if they disagree on something a person would notice, ask rather than choose. If the Pencil tools are unavailable in your session, say so before building the footer, the popover and the popup row.

## Problem and outcome

A window showing another computer shows this computer's usage in the footer, where it reads as the host's. After this mission it shows that computer's usage, and clicking it opens a read-only usage popover. The host popup's connected row leads its status line with latency bars or a state icon, a slow link turns the host button, bars and text amber, and Unreachable turns red. A window on this computer is unchanged.

## Starting point

`fc33e746` was reviewed against the first spec. Keep what the revised spec still asks for: the negotiated `ping` and `pong` with their fixtures, the route's 5-second ping, median of 3, 10-second rule and per-route reset, the `remote/latency` event, answering `ping` from the channel loop with feed reads moved off it, and red Unreachable in `host_tone` and `connection_subtitle`. Remove the footer link status (`render_link_status`, its use in `sidebar.rs`, its strings and anything only it uses) with its tests and docs wording. Add the `usage` action end to end, the host's refresh rules, the host's usage in the footer, the read-only popover, the P1 row marks and the Slow state with its 180 ms margin. The first spec and feature commits are superseded: fold them into the new history rather than stacking reverts.

## Read first

- The spec, in full, and `git show fc33e746`.
- `crates/runner-core/src/companion/actions.rs` and its fixtures (`catalog` is the model for `usage`); `crates/runner-daemon/src/companion/actions.rs` and `mod.rs` (`FEATURES`).
- `crates/runner-daemon/src/usage.rs`: `UsageService::snapshot`, how `fetch` filters runtimes, `RefreshReason` and its intervals, and the scheduler, which polls only while an app is connected. `crates/runner-daemon/src/daemon/boot.rs` enables and restores usage at boot.
- How the remote client forwards `catalog`, under `crates/runner-daemon/src/remote/client/`.
- `crates/runner-app/src/surfaces/remote_client/model.rs` and `picker.rs`; `crates/runner-app/src/surfaces/app_shell/sidebar.rs` (`sidebar_usage_element`, `show_usage`) and `usage.rs` (the popover).
- `docs/tech/remote-control.md`, `docs/guide/guides/remote-access.md` and its zh-CN page, `docs/tests/regression/remote.md`.

## Deliverables

The spec's four phases: protocol and host, daemon client, app, docs. Hold to these points in implementation and review:

1. **Negotiated, not versioned.** `ping` and `usage` are welcome features; the companion `VERSION` stays 1, an older host is sent neither, and the iPhone app ignores both.
2. **The ping never affects the connection.** The route's existing failure handling stays the only source of Reconnecting and Unreachable.
3. **Nothing waits.** `pong` never waits on overview, feed or usage work, and `usage` is answered from the snapshot without waiting for a fetch.
4. **The result leaks nothing.** No account folders, commands, paths or configuration; the active Claude account's name only when the host has several.
5. **The host refreshes only when due.** Periodic requests use the 15-minute schedule and a popover request the 5-minute one, so a remote window never makes a host fetch more often than its own app would, and a host with no app open still refreshes when asked.
6. **The footer follows the window's computer.** The same element and visibility as local usage, 45% opacity when the route is not Connected, an empty slot before the first reply and for a host without `usage`, unknown agents skipped.
7. **One slow flag.** Amber from 200 ms, normal again below 180 ms, driving the host button and the row together; the bar count follows the plain boundaries.
8. **Scope of the colours.** `host_tone`, `connection_subtitle` and the popup row only; the reconnecting banner and the remote workspace's connection card stay as they are.

**Non-goals:** the spec's non-goals, no change to the relay heartbeat, and no iPhone work.

## Tests

The spec's Automated list, each at the layer it names. Say in the handoff which tests fail without the change. Gates: tests for `runner-core`, `runner-daemon` and `runner-app`, workspace Clippy with warnings denied, `cargo fmt --all --check`. Regenerate any golden protocol fixture through its existing workflow and inspect the change. Drive macOS, Windows and Linux CI green.

## Live checks (qa)

The spec's nine live checks, each with the reason its table gives: L1 connected, L2 switching computers, L3 the read-only popover and the host's account, L4 a host with its app closed, L5 slow, L6 Reconnecting, L7 Unreachable, L8 a 0.19.5 host, L9 appearance.

**Prerequisite, for qa to raise at handoff.** Every check needs a real route to a second paired computer. `docs/tests/regression/remote.md` says crew slots do not contact the production service or read a secret store unless Jason explicitly authorizes that session, and qa never touches the installed Runner, which is the 0.19.5 on this Mac. So when qa receives the reviewed candidate, before launching anything, it asks Jason through `runner signal ask_human` how to run them, with choices such as: authorize a second candidate daemon on this Mac with private storage and test enrollments on his account, cleaned up afterwards, as in the [#1025 record](../../tests/archive/1025-remote-switch-repaint.md); or leave the checks to Jason on his second computer. The question also says that L5 needs added network delay, which may need administrator rights, and that L8 needs a real 0.19.5 host. Do not guess or start pairing before he answers.

The verdict gives pass, fail or blocked per check with screenshot evidence, and the PR description records it. A blocked check keeps `needs-live-test`.

## Handoffs

The coder implements and commits, then hands the candidate to `reviewer`. When the review is clean, the coder hands the reviewed commit to `qa`. Any failure, and any change after review, goes back through `reviewer` before qa reruns the affected checks.

## Pull request

PR #1233 is open, with `risk-high`, `needs-live-test` and `reviewed` from the first implementation. Retitle it and rewrite its description for the new scope, keep `Refs #1229` until the feature is complete and use `Fixes #1229` in its final form. Remove `reviewed` when the reworked code is pushed, until the reviewer passes the new head. Keep `risk-high` and keep `needs-live-test` until every selected check has passed; `core-review` is added automatically if the paths match. No co-author trailers, per AGENTS.md.

## Authorization

The codex trio may commit, push this branch, force-push it with lease after a rebase or a reshaped history, and update PR #1233 as above. It does not merge.
