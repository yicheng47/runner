# #1263 phase 1 — Route robustness and an echo reading

Jason asked on 2026-11-11 for the codex trio to build phase 1 of [#1263](https://github.com/yicheng47/runner/issues/1263). Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1263-remote-route-hardening` on `feat/1263-remote-route-hardening`, cut from `main` at `1b1873f0`. Use `/tmp/1263-p1-` as the prefix for any scratch file.

## Source of truth

- **Spec:** `docs/features/1263-remote-route-hardening.md`, sections Findings, Decisions, Security and Phase 1. It owns behavior and numbers; this brief does not repeat them. Phases 2 to 4 are other missions: build none of them here.
- **Machinery:** `docs/tech/remote-control.md`, §9 (the client's routes), §10 (a keystroke) and §11 (where the time goes).

There is no design file. Nothing a person sees changes except how soon a lost route shows Reconnecting.

## Problem and outcome

The client's `runnerd` re-reads secrets and may call the service before every request on an open route, never notices a dead route, ends the whole route when one request is slow, reconnects in step with every other client, and drops typing while it writes. After this mission a request on a live route touches neither the secret store nor the service, a silent route reconnects within about 15 seconds, a slow request fails alone, reconnects are jittered, typing during a short write stall arrives in order, and the route reports how long an echo takes.

## Read first

- `crates/runner-daemon/src/remote/client/mod.rs`: `Client::route`, `computers`, `context`, `keys`, `input`, `pair`, `forget`, `reset`.
- `crates/runner-daemon/src/remote/client/route.rs`: `Handle`, `Latency`, `read`, `flush`, `Session` and `run`, with its tests and `client/tests.rs`.
- `crates/runner-daemon/src/remote/client/terminal.rs`: `Terminals::refused`, `disconnected`, `reattach`.
- `crates/runner-daemon/src/remote/mod.rs`, `run`: the host loop's explicit pong.
- `crates/runner-core/src/remote/mod.rs` (`HostKey`, `Secret`) and `crates/runner-core/src/companion/channel/handshake.rs` (`HostPair`).
- `crates/runner-app/src/surfaces/remote_client/model.rs`, the `remote/latency` handler.

## Deliverables

The spec's Phase 1, findings 1 to 5, 7 and 16 to 18. Hold to these points in implementation and review:

1. **No wire change.** No new message, field, feature or frame; the companion `VERSION` stays 1 and every host keeps working, including one without `ping`. The WebSocket ping to the relay is the only new thing on the socket.
2. **The open route comes first.** With a live route, a request reads no secret and calls no service, and it succeeds while `ListAccountHosts` fails or is rate-limited. The cached credential and keys are dropped when the remote status changes origin, host id or sign-in state, and follow `pair`, `forget` and `reset`.
3. **Liveness is not the latency reading.** Fifteen seconds with nothing received ends the route through the existing reconnect path. An unanswered ping is still only a 10,000 ms sample (#1229).
4. **A slow request fails alone.** Its late reply is dropped, a timed-out `attach` is cleaned up like a refused one, and no unmatched `result` is ever emitted as an overview.
5. **Typing never crosses a reconnect.** Buffered input is discarded with today's `session/input-error` when the route's generation changes or the `c::MAX_MESSAGE_SIZE` (1 MiB) cap is passed. Within one route it arrives once and in order.
6. **Frames are sealed in the order they are written.** The writer task takes sealed frames from one queue; nothing else seals or writes.
7. **`echo_ms` is additive.** It rides in `remote/latency` beside `ms` and in a log line once a minute; the app ignores it and no screen changes.
8. **Secrets.** `Zeroizing` for `HostKey::private`, `HostPair::psk` and the intermediate session key, zeroize-on-drop for `Secret`; no secret reaches a log or a `Debug` output.

**Non-goals:** the host's relay loop beyond removing its explicit pong, credit, the handshake, resume, encodings, any UI, and the relay service.

**Docs in the same pull request:** `docs/guide/guides/remote-access.md` and its zh-CN page (a silent route reconnects after about 15 seconds; typing during a short stall is kept and typing across a reconnect is still discarded), and `docs/tech/remote-control.md` §9 to §11 with its table of constants. Change the spec only if a decision in it has to change, and ask Jason first.

## Tests

The spec's Verification list for phase 1, each as an in-process test beside the existing route tests. Say in the handoff which tests fail without the change. Gates: tests for `runner-core`, `runner-daemon` and `runner-app`, workspace Clippy with warnings denied, `cargo fmt --all --check`. Drive macOS, Windows and Linux CI green.

## Live checks (qa)

The spec's four phase 1 checks, each with the reason its table gives: `REMOTE-TERM-01`, `REMOTE-TERM-05`, sleep and wake, and tab switches and an action while the directory call fails. Write the record in `docs/tests/1263-p1-route-robustness.md`, with `echo_ms` and the ping for the same link.

**Prerequisite, for qa to raise at handoff.** Every check needs a real route to a second paired computer, and `docs/tests/regression/remote.md` says crew slots do not contact the production service or read a secret store unless Jason authorizes that session; qa also never touches the installed Runner. So when qa receives the reviewed candidate, before launching anything, it asks Jason through `runner signal ask_human` how to run them, with choices such as: authorize a second candidate daemon on this Mac with private storage and test enrollments on his account, cleaned up afterwards, as in the [#1025 record](../../tests/archive/1025-remote-switch-repaint.md); or leave the checks to Jason on his second computer. The question also says that the sleep check cannot run on the Mac qa drives and that failing the directory call needs a way to block it, and asks whether those two stay with their automated tests. Do not guess or start pairing before he answers.

The verdict gives pass, fail or blocked per check with evidence, and the PR description records it. A blocked check keeps `needs-live-test`.

## Handoffs

The coder implements, then hands the candidate to `reviewer`. When the review is clean, the coder hands the reviewed commit to `qa`. Any failure, and any change after review, goes back through `reviewer` before qa reruns the affected checks.

## Pull request

Against `main`, titled for phase 1, with `Refs #1263` and not `Fixes`, since three phases remain. Labels: `risk-medium` and `needs-live-test`; `core-review` is added automatically if the paths match. No co-author trailers, per AGENTS.md.

## Authorization

Jason approved the codex trio (coder, reviewer, qa) for this mission on 2026-11-11. The crew may commit, push the branch and open the pull request.

Jason amended the input budget on 2026-11-11 after review found that 64 KiB collided with the 64 KiB IPC paste chunking: use `c::MAX_MESSAGE_SIZE` (1 MiB), add a whole 200 KiB IPC paste regression, and keep discarding input across a reconnect.
