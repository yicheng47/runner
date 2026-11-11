# #1263 phase 2 — The host's relay loop and flow control

Jason asked on 2026-11-11 for the codex trio to build phase 2 of [#1263](https://github.com/yicheng47/runner/issues/1263). Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1263-remote-flow-control` on `feat/1263-remote-flow-control`, cut from `main` at `c46354ef`, which includes phase 1 ([#1264](https://github.com/yicheng47/runner/pull/1264)). Use `/tmp/1263-p2-` as the prefix for any scratch file.

## Source of truth

- **Spec:** `docs/features/1263-remote-route-hardening.md`, sections Findings, Scope, Decisions, Security, Phase 2, Compatibility and Verification. It owns behavior and numbers; this brief does not repeat them. Phase 1 is merged; phases 3 and 4 are other missions: build neither here. Jason moved delivery pacing (finding 13) from phase 4 into this phase on 2026-11-11 after live QA measured about one second of flooded echo.
- **Machinery:** `docs/tech/remote-control.md`, §6 (records and credit), §7 (the adapter), §9 (the client's routes) and §11 (where the time goes).
- **Phase 1 record:** `docs/tests/1263-p1-route-robustness.md`, for the live fixture's method, its limits and its one setup incident.

There is no new screen or design file. Jason separately approved a Reconnecting floating-pill correction using existing theme/toast patterns on 2026-11-11; keep that UI fix as its own commit.

## Problem and outcome

The host's relay loop reads, writes and runs its state machine in one task, so one slow write stops every channel's reads, and each frame recomputes the trust snapshot, an X25519 operation, up to twice. Credit counts message bytes while the relay's queue and the host's inbound budget count each frame plus 64 bytes, so a stream of small messages can overflow them (close 4413) before credit stops the sender. Every handled message forces a credit grant, every relevant event pushes a rebuilt overview whether or not it changed, and every step of a window drag is its own blocking resize. After this mission the host reads while it writes, `publish` runs only on change, each end bounds its in-flight frames by the relay's measure, credit goes out in batches, an unchanged overview is not pushed, and a drag sends only the newest size per terminal.

## Read first

- `crates/runner-daemon/src/remote/mod.rs`: `run`, `send_frames`, `publish`; `remote/machine.rs` and `remote/trust.rs` for the fingerprint.
- `crates/runner-daemon/src/remote/channels.rs`: `feed` and the inbound budget.
- `crates/runner-core/src/companion/channel/records.rs` (`Records`, `handled`), `channel/mod.rs` (`INITIAL_CREDIT`), `channel/tests.rs` and `channel/vectors/channel.json`.
- `crates/runner-daemon/src/companion/mod.rs`: `Adapter::changed`, `relevant`, the overview build and the `status` pushes.
- `crates/runner-daemon/src/remote/client/mod.rs` (`queue_viewport`) and `client/route.rs`, including the single writer queue phase 1 added.

## Deliverables

The spec's Phase 2: findings 6, 8, 13, 14, 15, and the batched-credit half of 11 (input without a reply is phase 4). Hold to these points in implementation and review:

1. **No wire change and no negotiation.** No new message, field, feature or frame; the companion `VERSION` stays 1. Each end bounds what it sends and batches its grants on its own, and a peer on the old rule is no worse off than today in either direction: a new client with a phase 1 host, and a phase 1 client with a new host.
2. **The remote loop never awaits a write.** One writer task owns the host socket's sink. Channel frames, `closeChannel` and the pong reach it through one bounded queue, so a channel's last frame and its close keep their order. A write that takes 10 seconds still fails the connection.
3. **`publish` on change, with the same outcome.** It runs after a command, a handshake, a trust write or a channel opening or closing, and the machine caches its fingerprint. A Revoke, Reset identity, PIN lock or turning remote access off reaches open channels as promptly as today.
4. **In-flight frames are bounded by the relay's cost.** Both ends keep the spec's first-in-first-out list in `Records` and hold the next frame while the list would pass 768 KiB. Nothing can deadlock: a sender stalls on credit only when it has none at all.
5. **Credit in batches.** `handled` no longer forces a grant; payload-byte credit goes out once 64 KiB of payload or handled frame cost is owed, whichever comes first. Regenerate `channel/vectors/channel.json` and say in the handoff why each vector changed.
6. **An overview is pushed only when it changed.** The hash leaves out per-session status, which the `status` pushes carry, and is kept per channel; a channel's first overview is always pushed, and replies to `overview` requests are unaffected. A real change, such as a chat created, renamed or archived, a mission started or a project added, is never suppressed.
7. **Newest size only.** One pending `viewport` per terminal goes out when the writer has room; `takeControl` and `attach` are not coalesced.
8. **Security.** Pairing, the Noise patterns, the PIN lock and what the relay can see stay as they are, and no secret reaches a log or a `Debug` output.

9. **Delivery pacing.** Follow the spec’s finding 13, now in Phase 2: keep each frame’s send time, pause new terminal output beyond 128 KiB unacknowledged once the oldest frame reaches 500 ms, keep pongs/results/pushes eligible, and resume on credit without losing output or breaking snapshot/resync ordering.

**Non-goals:** `hello` in the handshake, resume, input without a reply, the raw envelope, the thread per stream (finding 19), UI beyond the separately approved Reconnecting pill, the relay service, and the host’s own terminal backlog after a sustained flood. File that host backlog as a separate issue.

**Docs in the same pull request:** `docs/tech/remote-control.md` §6, §7, §9 and §11 with its table of constants. The guide does not change: no operated command changes, and it does not describe the banner’s look. Change the spec only if a decision in it has to change, and ask Jason first through `runner signal ask_human`.

## Tests

The spec's Verification list for phase 2, each as an in-process test beside the existing ones. Say in the handoff which tests fail without the change. Gates: tests for `runner-core`, `runner-daemon` and `runner-app`, workspace Clippy with warnings denied (with the updater feature on macOS), `cargo fmt --all --check`. Drive macOS, Windows and Linux CI green. Phase 1's only CI failure was an event-ordering race in a new test on macOS: subscribe before the action under test, and bound every wait.

## Live checks (qa)

The phase 2 checks below, including Jason’s 2026-11-11 pacing and UI additions:

- `REMOTE-TERM-02`, resizing a remote terminal by dragging, because viewports are coalesced: after a fast drag the host's terminal ends at the final size with no overlapping or missing text.
- `REMOTE-WIN-03`, session states in the remote sidebar, because overview pushes are deduplicated. Select the parts that depend on pushes: states still follow changes on the host, and a chat created, renamed and archived on the host appears remotely. Say which parts of the row ran.
- New: flood one remote pane (`yes`) and type in another, because the writer split, credit and pacing changed. Run at least 100 seconds unthrottled and with the 500,000 B/s CONNECT ceiling. The route stays connected, typing is visibly responsive, and median flooded `echo_ms` is at most about 700 ms. Record idle and flooded echo and ping beside the earlier failures. Stop the flood afterwards. The native host’s delayed processing after `yes` exits is observed and filed separately, not a phase-2 gate.

- Reconnecting pill: force reconnect through the proxy block flag with two remote split terminals full of text. In dark and light themes, capture an opaque centered readable pill below the header, unchanged terminal geometry, and disappearance on recovery.
- Finish the approved scoped approval-indicator push fixture using per-chat arguments and scratch cwd; capture host/remote, deny the harmless `sleep 1`, then archive. Never change global agent configuration.

The spec asks each record to state `echo_ms` and the ping for the same link before and after the change. Measure both, idle and during the flood, on `main` at `c46354ef` (phase 1, which reports `echo_ms`) and on the candidate, over the same fixture. If the phase 1 record's CONNECT forwarder can cap the client's service traffic near 4 Mbit/s without touching system networking, repeat the flood there, since the spec's estimate assumes that link; otherwise say it was not throttled.

**Authorized fixture.** Every check needs a real route between two paired computers, and `docs/tests/regression/remote.md` says crew slots do not contact the production service or read a secret store unless Jason authorizes that session; qa also never touches the installed Runner. Jason approved reuse for this mission in message `01M74PTCVQ41CF34EWSGSYPHAW` and explicitly extended it to the pacing reruns in `01M74WXY9JGB8HD3YWDNWBQN14`: two private candidate daemons on this Mac with private storage and two test enrollments on his account, removed afterwards, with the account's computer list restored. Confirm each enrollment's code and fingerprint against the apps at the time, and never start the default development daemon.

Write the record in `docs/tests/1263-p2-host-flow-control.md`. The verdict gives pass, fail or blocked per check with evidence, and the PR description records it. A blocked check keeps `needs-live-test`.

## Handoffs

The coder implements, then hands the candidate to `reviewer`. When the review is clean, the coder hands the reviewed commit to `qa`. Any failure, and any change after review, goes back through `reviewer` before qa reruns the affected checks.

## Pull request

Against `main`, titled for phase 2, with `Refs #1263` and not `Fixes`, since two phases remain. Labels: `risk-medium` and `needs-live-test`; `core-review` is added automatically if the paths match.

## Authorization

Jason approved the codex trio (coder, reviewer, qa) for this mission on 2026-11-11. The crew may commit, push the branch and open the pull request.
