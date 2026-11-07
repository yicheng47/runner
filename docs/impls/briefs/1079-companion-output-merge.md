# #1079 — merge companion terminal output before sending it

Jason approved this mission on 2026-11-07 with a claude duo. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1079-companion-output-merge` on `fix/1079-companion-output-merge`, based on `origin/main` at `1db23e604ab24d93b02b9afe8db669bac8a56503`. Issue #1079 is the authority on the expected behavior; its line numbers are from an older commit, so go by symbol. This brief adds what reading the code settled.

## Problem

The companion stream sends every terminal read as its own `output` message. `Stream::chunk` takes one `TerminalFrame::Output` per call and splits large frames, but never merges small ones, even when several are already queued. A fullscreen TUI's single redraw can leave the host as several messages, each paying its own JSON encoding, encrypted record and relay hop, and a client drawing between them can show a half-finished screen. On the iPhone, scrolling Claude Code moves in uneven steps.

## Read first

- `crates/runner-daemon/src/companion/mod.rs`: `Queue`, `Stream`, `Stream::chunk`, `Adapter::next` (send order, terminal credit, round-robin between streams) and `Adapter::changed` (the wake-ups; `overview_due` is the existing timed-wake pattern).
- `crates/runner-daemon/src/companion/tests.rs`: `terminal_chunks_order_fairness_overflow_and_resync_sequence`, the `next` helper, and `#[tokio::test(start_paused = true)]`.
- `crates/runner-daemon/src/remote/client/terminal.rs`: the desktop viewer refuses any `output` whose `seq` isn't `stream.seq + 1`, with "Remote terminal output sequence changed". A merged message must therefore raise `seq` by exactly one.
- `docs/tech/remote-control.md`: the terminal and send-order descriptions.

## Deliverables

1. **Merge.** When `chunk` takes an `Output` frame, it also takes the `Output` frames directly behind it, up to `TERMINAL_CHUNK_SIZE` in total, and sends them as one `output` message. A frame that would overflow stays queued for the next message. A single frame larger than the limit still splits as today. A `Resized` frame ends the merge. `Queue.bytes` stays exact for every frame taken.
2. **Window.** Add a window of about 5 ms. Output with nothing sent in the previous window goes at once, so typing echo keeps its latency. Output arriving within the window waits for the rest of it and goes out merged. These never wait:
   - a merge that already fills `TERMINAL_CHUNK_SIZE`, because waiting can't add to it, and holding it would push a flood towards the queue's overflow and a resync it would not have hit before;
   - the remaining chunks of a split frame;
   - output with a `Resized` behind it, or on a stream that has ended, so size changes and `Closed` are not delayed.
3. **Timed wake.** `changed` wakes when held output comes due, with no further frame or event needed. Held output must never sit until something unrelated happens. Use `tokio::time::Instant` so paused-time tests stay deterministic.
4. **Fairness and credit.** A held stream does not block other streams or feeds in `next`, and holding spends no terminal credit. Output that builds up while credit is out merges when credit returns.
5. **Docs.** Update `docs/tech/remote-control.md` to describe the merge and the window.

## Unchanged

The message shape, the protocol `VERSION`, and snapshot, resync, closed and clipboard behavior stay as they are, and `seq` rises by one per message, so existing clients need no update.

## Boundaries

No client-side render pacing, no gesture-based window as in Orca, no setting and no protocol change. Keep the change inside `companion/` and leave `session/manager/output.rs` alone: the host-side stream is the only thing that changes. The phone probe mentioned in the issue is not a prerequisite.

## Validation

Name the changed behaviors and the check covering each before running them. Selected checks in `companion/tests.rs`, with paused time wherever timing matters:

- First output after a quiet period is delivered without time advancing.
- Several frames within the window arrive as one `output`, with bytes in order and `seq` up by one.
- A merge stops at `TERMINAL_CHUNK_SIZE`, and the next message carries the rest with no bytes lost.
- A frame larger than the limit still splits, without waiting between its chunks.
- `Resized` ends a merge and is not delayed: the order is output, resized, output.
- Held output is delivered by `changed` alone, with no new frame arriving. This guards against output getting stuck.
- An ended stream flushes queued output, then sends `Closed`.
- A sustained flood past the chunk size does not wait on the window and does not resync where it would not have before; a real overflow still gives a snapshot with `seq` continuing.
- A held stream doesn't hold back another stream, and output queued without credit merges once credit returns.

Existing `remote/client` tests must still pass, since they enforce the `seq` rule from the viewer's side. If an existing companion test needs changing, say why in the PR.

**Gates:** `cargo nextest run --locked -p runner-core -p runner-daemon --cargo-profile ci`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check` and `git diff --check`, using this worktree's own `target/`. Then CI on every required platform.

**Live checks:** expected, because this changes the terminal output a remote viewer sees. The reviewer names the final list from the diff. Expected:

- `REMOTE-TERM-01`, because merged output carries typing echo and pastes, which must arrive once and in order.
- `REMOTE-TERM-08`, because a warm switch sends a snapshot followed by output, and that order must hold.
- Scrolling a long Claude Code chat in a remote terminal, because it is the case the issue reports: no garbled or half-drawn screen and no sequence error.
- The issue's iPhone scroll pass, which is Jason's on his device.

The mission's operator adds the codex `qa` role for these after the mission settles. The coder and reviewer do not run the app. A failure comes back to the coder, and the fixed candidate is reviewed again before qa reruns the affected checks. The PR carries `needs-live-test` until the required checks pass.

## Authorization and handoff

The approved crew is this claude duo. The crew may commit, push this branch and open one PR against `main`.

The coder implements, opens the PR and hands the reviewer the PR number and head commit. The reviewer posts its result as a PR comment. A clean review adds `reviewed`, names the reviewed commit and lists the required live checks.

Finish by posting the PR URL, final head, what changed, validation results and the live-check list with `runner msg post`.
